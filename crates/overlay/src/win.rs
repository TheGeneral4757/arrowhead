//! Windows backend: a layered, topmost, click-through, non-activating popup covering one
//! monitor. Each redraw pushes the tiny-skia pixmap (premultiplied BGRA) with
//! `UpdateLayeredWindow`. Redraws happen only on state change, at most once per monitor refresh.

use std::cell::RefCell;
use std::ffi::c_void;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tiny_skia::Pixmap;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM};
use windows::Win32::Graphics::Gdi::{
    AC_SRC_ALPHA, AC_SRC_OVER, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BLENDFUNCTION,
    CreateCompatibleDC, CreateDIBSection, DEVMODEW, DIB_RGB_COLORS, DeleteDC, DeleteObject,
    ENUM_CURRENT_SETTINGS, EnumDisplayMonitors, EnumDisplaySettingsW, GetDC, GetMonitorInfoW,
    HBITMAP, HDC, HGDIOBJ, HMONITOR, MONITORINFO, MONITORINFOEXW, ReleaseDC, SelectObject,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW, KillTimer, MSG,
    PostMessageW, PostQuitMessage, RegisterClassW, SW_SHOWNOACTIVATE, SetTimer, ShowWindow,
    TranslateMessage, ULW_ALPHA, UpdateLayeredWindow, WM_APP, WM_DESTROY, WM_TIMER, WNDCLASSW,
    WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};
use windows::core::{BOOL, w};

use crate::model::{Draw, Msg};
use crate::render::{render, to_bgra};

const WM_MSGS: u32 = WM_APP;
const WM_EOF: u32 = WM_APP + 1;
const TIMER: usize = 1;

struct Win {
    hwnd: HWND,
    rect: RECT,
    dib: HBITMAP,
    bits: *mut u8,
    mem: HDC,
    pm: Pixmap,
    draw: Draw,
    dirty: bool,
    last: Instant,
    min_gap: Duration,
    inbox: Arc<Mutex<Vec<Msg>>>,
}

thread_local! {
    static WIN: RefCell<Option<Win>> = const { RefCell::new(None) };
}

struct Mon {
    rect: RECT,
    name: String,
}

unsafe extern "system" fn collect(m: HMONITOR, _: HDC, _: *mut RECT, data: LPARAM) -> BOOL {
    let out = unsafe { &mut *(data.0 as *mut Vec<Mon>) };
    let mut mi = MONITORINFOEXW::default();
    mi.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;
    if unsafe { GetMonitorInfoW(m, &mut mi as *mut MONITORINFOEXW as *mut MONITORINFO) }.as_bool() {
        let n = mi.szDevice.iter().position(|&c| c == 0).unwrap_or(0);
        out.push(Mon {
            rect: mi.monitorInfo.rcMonitor,
            name: String::from_utf16_lossy(&mi.szDevice[..n]),
        });
    }
    true.into()
}

fn refresh_hz(device: &str) -> u32 {
    let wide: Vec<u16> = device.encode_utf16().chain(Some(0)).collect();
    let mut dm = DEVMODEW {
        dmSize: size_of::<DEVMODEW>() as u16,
        ..Default::default()
    };
    let ok = unsafe {
        EnumDisplaySettingsW(
            windows::core::PCWSTR(wide.as_ptr()),
            ENUM_CURRENT_SETTINGS,
            &mut dm,
        )
    }
    .as_bool();
    if ok && dm.dmDisplayFrequency > 1 {
        dm.dmDisplayFrequency
    } else {
        60
    }
}

impl Win {
    fn redraw(&mut self) {
        if !self.dirty {
            return;
        }
        let since = self.last.elapsed();
        if since < self.min_gap {
            unsafe {
                SetTimer(
                    Some(self.hwnd),
                    TIMER,
                    (self.min_gap - since).as_millis() as u32 + 1,
                    None,
                )
            };
            return;
        }
        render(&self.draw, &mut self.pm);
        let len = self.pm.data().len();
        // SAFETY: `bits` points at the DIB section, which is exactly width * height * 4 bytes.
        let dst = unsafe { std::slice::from_raw_parts_mut(self.bits, len) };
        to_bgra(&self.pm, dst);
        let (w, h) = (self.pm.width() as i32, self.pm.height() as i32);
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: 255,
            AlphaFormat: AC_SRC_ALPHA as u8,
        };
        unsafe {
            let screen = GetDC(None);
            let _ = UpdateLayeredWindow(
                self.hwnd,
                Some(screen),
                Some(&POINT {
                    x: self.rect.left,
                    y: self.rect.top,
                }),
                Some(&SIZE { cx: w, cy: h }),
                Some(self.mem),
                Some(&POINT { x: 0, y: 0 }),
                Default::default(),
                Some(&blend),
                ULW_ALPHA,
            );
            ReleaseDC(None, screen);
        }
        self.dirty = false;
        self.last = Instant::now();
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_MSGS => {
            WIN.with_borrow_mut(|w| {
                if let Some(w) = w {
                    let msgs = std::mem::take(&mut *w.inbox.lock().unwrap());
                    for m in msgs {
                        w.dirty |= w.draw.apply(m);
                    }
                    w.redraw();
                }
            });
            LRESULT(0)
        }
        WM_TIMER => {
            let _ = unsafe { KillTimer(Some(hwnd), TIMER) };
            WIN.with_borrow_mut(|w| {
                if let Some(w) = w {
                    w.redraw();
                }
            });
            LRESULT(0)
        }
        WM_EOF => {
            let _ = unsafe { DestroyWindow(hwnd) };
            LRESULT(0)
        }
        WM_DESTROY => {
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wp, lp) },
    }
}

pub fn run(output: Option<String>, size: Option<(i32, i32)>) -> Result<(), String> {
    unsafe {
        // Physical pixels, like the captured stream.
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let mut mons: Vec<Mon> = Vec::new();
        let _ = EnumDisplayMonitors(
            None,
            None,
            Some(collect),
            LPARAM(&mut mons as *mut _ as isize),
        );
        let mon = output
            .as_deref()
            .and_then(|n| mons.iter().position(|m| m.name.contains(n)))
            .or_else(|| {
                let (sw, sh) = size?;
                mons.iter()
                    .position(|m| (m.rect.right - m.rect.left, m.rect.bottom - m.rect.top) == (sw, sh))
            })
            .or((!mons.is_empty()).then_some(0))
            .map(|i| mons.swap_remove(i))
            .ok_or("no monitors")?;
        let (w, h) = (
            mon.rect.right - mon.rect.left,
            mon.rect.bottom - mon.rect.top,
        );

        let inst = GetModuleHandleW(None).map_err(|e| e.to_string())?;
        let class = w!("arrowhead-overlay");
        RegisterClassW(&WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: inst.into(),
            lpszClassName: class,
            ..Default::default()
        });
        let hwnd = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOPMOST | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
            class,
            w!("arrowhead overlay"),
            WS_POPUP,
            mon.rect.left,
            mon.rect.top,
            w,
            h,
            None,
            None,
            Some(inst.into()),
            None,
        )
        .map_err(|e| e.to_string())?;

        let screen = GetDC(None);
        let mem = CreateCompatibleDC(Some(screen));
        ReleaseDC(None, screen);
        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                // Negative: top-down rows, like the pixmap.
                biHeight: -h,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits: *mut c_void = std::ptr::null_mut();
        let dib = CreateDIBSection(Some(mem), &bmi, DIB_RGB_COLORS, &mut bits, None, 0)
            .map_err(|e| e.to_string())?;
        SelectObject(mem, HGDIOBJ(dib.0));

        let inbox: Arc<Mutex<Vec<Msg>>> = Arc::default();
        WIN.set(Some(Win {
            hwnd,
            rect: mon.rect,
            dib,
            bits: bits as *mut u8,
            mem,
            pm: Pixmap::new(w as u32, h as u32).ok_or("bad monitor size")?,
            draw: Draw::default(),
            dirty: true,
            last: Instant::now() - Duration::from_secs(1),
            // Monitor refresh, capped at 30 Hz: each redraw copies a full-monitor buffer.
            min_gap: Duration::from_micros(1_000_000 / u64::from(refresh_hz(&mon.name)))
                .max(Duration::from_millis(33)),
            inbox: inbox.clone(),
        }));
        WIN.with_borrow_mut(|w| w.as_mut().unwrap().redraw());
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);

        // HWND is not Send; its value is.
        let raw = hwnd.0 as isize;
        std::thread::spawn(move || {
            crate::read_stdin(|m| {
                inbox.lock().unwrap().extend(m);
                let h = HWND(raw as *mut c_void);
                PostMessageW(Some(h), WM_MSGS, WPARAM(0), LPARAM(0)).is_ok()
            });
            let _ = PostMessageW(Some(HWND(raw as *mut c_void)), WM_EOF, WPARAM(0), LPARAM(0));
        });

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        WIN.with_borrow_mut(|w| {
            if let Some(w) = w.take() {
                let _ = DeleteObject(HGDIOBJ(w.dib.0));
                let _ = DeleteDC(w.mem);
            }
        });
    }
    Ok(())
}
