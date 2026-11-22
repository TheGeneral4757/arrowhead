//! Global hotkeys with `RegisterHotKey` on a message-only window (spec 05).
//! Same actions and `start` shape as the Linux `PortalHotkeys`.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, mpsc};

use arrowhead_core::config::Hotkeys;
use arrowhead_core::input::{Hotkey, InputError};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    HOT_KEY_MODIFIERS, MOD_NOREPEAT, RegisterHotKey, UnregisterHotKey,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW, HWND_MESSAGE,
    MSG, PostThreadMessageW, RegisterClassW, WINDOW_EX_STYLE, WM_HOTKEY, WM_QUIT, WNDCLASSW,
    WS_OVERLAPPED,
};
use windows::core::w;

use crate::winutil::parse_hotkey;

pub struct WinHotkeys {
    thread_id: Arc<AtomicU32>,
}

unsafe extern "system" fn proc(h: HWND, m: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    // SAFETY: forwarding the window procedure's own arguments.
    unsafe { DefWindowProcW(h, m, w, l) }
}

impl WinHotkeys {
    /// Registers the hotkeys on a background thread and sends each press to `tx`. The bind
    /// result (or the error) arrives as one message on the returned receiver.
    pub fn start(
        keys: &Hotkeys,
        tx: mpsc::Sender<Hotkey>,
    ) -> Result<(Self, mpsc::Receiver<Result<String, String>>), InputError> {
        let binds: Vec<(Hotkey, String)> = vec![
            (Hotkey::Toggle, keys.toggle.clone()),
            (Hotkey::Panic, keys.panic.clone()),
            (Hotkey::NudgeUp, keys.nudge_up.clone()),
            (Hotkey::NudgeDown, keys.nudge_down.clone()),
            (Hotkey::CycleStyle, keys.cycle_style.clone()),
        ];
        let thread_id = Arc::new(AtomicU32::new(0));
        let tid = thread_id.clone();
        let (btx, brx) = mpsc::channel();
        std::thread::Builder::new()
            .name("hotkeys".into())
            .spawn(move || {
                // SAFETY: standard message-only window, hotkey registration and message loop,
                // all on this thread (hotkeys belong to the thread that registered them).
                unsafe {
                    tid.store(GetCurrentThreadId(), Ordering::SeqCst);
                    let fail = |e: String| drop(btx.send(Err(e)));
                    let Ok(inst) = GetModuleHandleW(None) else {
                        return fail("GetModuleHandle failed".into());
                    };
                    let class = w!("arrowhead_hotkeys");
                    let wc = WNDCLASSW {
                        lpfnWndProc: Some(proc),
                        hInstance: inst.into(),
                        lpszClassName: class,
                        ..Default::default()
                    };
                    RegisterClassW(&wc);
                    let hwnd = match CreateWindowExW(
                        WINDOW_EX_STYLE(0),
                        class,
                        w!("arrowhead hotkeys"),
                        WS_OVERLAPPED,
                        0,
                        0,
                        0,
                        0,
                        Some(HWND_MESSAGE),
                        None,
                        Some(inst.into()),
                        None,
                    ) {
                        Ok(h) => h,
                        Err(e) => return fail(format!("CreateWindow: {e}")),
                    };
                    let mut list = Vec::new();
                    let mut problems = Vec::new();
                    for (i, (_, text)) in binds.iter().enumerate() {
                        let Some((mods, vk)) = parse_hotkey(text) else {
                            problems.push(format!("cannot read key {text:?}"));
                            continue;
                        };
                        match RegisterHotKey(
                            Some(hwnd),
                            i as i32,
                            HOT_KEY_MODIFIERS(mods) | MOD_NOREPEAT,
                            vk,
                        ) {
                            Ok(()) => list.push(text.clone()),
                            // Another program owns the key.
                            Err(e) => problems.push(format!("{text}: {e}")),
                        }
                    }
                    if list.is_empty() {
                        let _ = DestroyWindow(hwnd);
                        return fail(problems.join("; "));
                    }
                    let mut msg = format!("registered {}", list.join(", "));
                    if !problems.is_empty() {
                        msg.push_str(&format!(" (failed: {})", problems.join("; ")));
                    }
                    let _ = btx.send(Ok(msg));
                    let mut m = MSG::default();
                    while GetMessageW(&mut m, None, 0, 0).as_bool() {
                        if m.message == WM_HOTKEY
                            && let Some((a, _)) = binds.get(m.wParam.0)
                            && tx.send(*a).is_err()
                        {
                            break;
                        }
                        DispatchMessageW(&m);
                    }
                    for i in 0..binds.len() {
                        let _ = UnregisterHotKey(Some(hwnd), i as i32);
                    }
                    let _ = DestroyWindow(hwnd);
                }
            })
            .map_err(|e| InputError(e.to_string()))?;
        Ok((Self { thread_id }, brx))
    }
}

impl Drop for WinHotkeys {
    fn drop(&mut self) {
        let id = self.thread_id.load(Ordering::SeqCst);
        if id != 0 {
            // SAFETY: posting WM_QUIT to our own hotkey thread ends its message loop.
            unsafe {
                let _ = PostThreadMessageW(id, WM_QUIT, WPARAM(0), LPARAM(0));
            }
        }
    }
}
