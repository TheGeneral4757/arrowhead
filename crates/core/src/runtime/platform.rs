//! The only OS-specific code in core: the monotonic clock.

/// `CLOCK_MONOTONIC` in ns, the same clock PipeWire stamps frames with.
#[cfg(unix)]
pub fn monotonic_ns() -> u64 {
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: valid pointer to a timespec; CLOCK_MONOTONIC always exists on Linux.
    unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) };
    ts.tv_sec as u64 * 1_000_000_000 + ts.tv_nsec as u64
}

/// QPC ticks to ns without overflowing `ticks * 1e9` (QPC runs for days at 10 MHz).
pub fn qpc_to_ns(ticks: i64, freq: i64) -> u64 {
    let (t, f) = (ticks.max(0) as u128, freq.max(1) as u128);
    (t * 1_000_000_000 / f) as u64
}

#[cfg(windows)]
fn qpc_freq() -> i64 {
    use std::sync::OnceLock;
    static F: OnceLock<i64> = OnceLock::new();
    *F.get_or_init(|| {
        let mut f = 0;
        // SAFETY: valid pointer; cannot fail on Windows XP and later.
        unsafe { windows::Win32::System::Performance::QueryPerformanceFrequency(&mut f).ok() };
        f
    })
}

/// A raw QPC reading (for example DXGI `LastPresentTime`) on the `monotonic_ns` clock.
#[cfg(windows)]
pub fn qpc_ticks_to_ns(ticks: i64) -> u64 {
    qpc_to_ns(ticks, qpc_freq())
}

/// QPC in ns, the clock DXGI stamps frames with.
#[cfg(windows)]
pub fn monotonic_ns() -> u64 {
    let mut t = 0;
    // SAFETY: valid pointer; cannot fail on Windows XP and later.
    unsafe { windows::Win32::System::Performance::QueryPerformanceCounter(&mut t).ok() };
    qpc_ticks_to_ns(t)
}

#[cfg(not(any(unix, windows)))]
pub fn monotonic_ns() -> u64 {
    use std::sync::OnceLock;
    use std::time::Instant;
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_nanos() as u64
}

/// Sleep until `CLOCK_MONOTONIC` reaches `t` ns (`clock_nanosleep`, `TIMER_ABSTIME`).
#[cfg(unix)]
pub fn sleep_abs_ns(t: u64) {
    let ts = libc::timespec {
        tv_sec: (t / 1_000_000_000) as libc::time_t,
        tv_nsec: (t % 1_000_000_000) as libc::c_long,
    };
    // SAFETY: valid timespec; EINTR just returns early and the caller spins the rest.
    unsafe {
        libc::clock_nanosleep(
            libc::CLOCK_MONOTONIC,
            libc::TIMER_ABSTIME,
            &ts,
            std::ptr::null_mut(),
        )
    };
}

/// High-resolution waitable timer (Windows 10 1803+). It can wake up to ~0.5 ms late, so it
/// stops 700 us short and the caller's spin loop covers the rest.
#[cfg(windows)]
pub fn sleep_abs_ns(t: u64) {
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::System::Threading::{
        CREATE_WAITABLE_TIMER_HIGH_RESOLUTION, CreateWaitableTimerExW, INFINITE, SetWaitableTimer,
        TIMER_ALL_ACCESS, WaitForSingleObject,
    };
    thread_local! {
        static TIMER: Option<HANDLE> = {
            // SAFETY: no pointers; the handle lives as long as the thread (never closed).
            unsafe {
                CreateWaitableTimerExW(None, None, CREATE_WAITABLE_TIMER_HIGH_RESOLUTION, TIMER_ALL_ACCESS.0).ok()
            }
        };
    }
    let target = t.saturating_sub(700_000);
    let now = monotonic_ns();
    if target <= now {
        return;
    }
    let due = -(((target - now) / 100) as i64); // negative = relative, 100 ns units
    TIMER.with(|h| match h {
        // SAFETY: valid timer handle and due-time pointer.
        Some(h) => unsafe {
            if SetWaitableTimer(*h, &due, 0, None, None, false).is_ok() {
                WaitForSingleObject(*h, INFINITE);
            }
        },
        None => std::thread::sleep(std::time::Duration::from_nanos(target - now)),
    });
}

#[cfg(not(any(unix, windows)))]
pub fn sleep_abs_ns(t: u64) {
    let now = monotonic_ns();
    if t > now {
        std::thread::sleep(std::time::Duration::from_nanos(t - now));
    }
}

/// Ask for `SCHED_FIFO` (needs rtprio, e.g. the `realtime` group); falls back to nice -10.
/// Returns what was applied.
#[cfg(unix)]
pub fn make_realtime() -> &'static str {
    let p = libc::sched_param { sched_priority: 20 };
    // SAFETY: plain syscalls on the calling thread.
    unsafe {
        if libc::sched_setscheduler(0, libc::SCHED_FIFO, &p) == 0 {
            return "SCHED_FIFO";
        }
        if libc::setpriority(libc::PRIO_PROCESS, 0, -10) == 0 {
            return "nice -10";
        }
    }
    "normal"
}

/// `THREAD_PRIORITY_HIGHEST` for the calling thread (spec 04). Returns what was applied.
#[cfg(windows)]
pub fn make_realtime() -> &'static str {
    use windows::Win32::System::Threading::{
        GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_HIGHEST,
    };
    // SAFETY: the pseudo handle of the calling thread is always valid.
    match unsafe { SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_HIGHEST) } {
        Ok(()) => "THREAD_PRIORITY_HIGHEST",
        Err(_) => "normal",
    }
}

#[cfg(not(any(unix, windows)))]
pub fn make_realtime() -> &'static str {
    "normal"
}

/// Set by SIGINT/SIGTERM/SIGHUP and the panic hook (S2): run loops stop and release all keys.
pub static EXIT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[cfg(unix)]
extern "C" fn on_exit_signal(_: libc::c_int) {
    EXIT.store(true, std::sync::atomic::Ordering::SeqCst);
}

/// Windows (S2): console control events and `WM_ENDSESSION` set `EXIT`. A console handler
/// runs on its own thread; for close, logoff and shutdown Windows ends the process when it
/// returns, so it waits a moment for the scheduler to release the keys.
#[cfg(windows)]
unsafe extern "system" fn on_console_event(kind: u32) -> windows::core::BOOL {
    use windows::Win32::System::Console::CTRL_C_EVENT;
    EXIT.store(true, std::sync::atomic::Ordering::SeqCst);
    if kind != CTRL_C_EVENT {
        std::thread::sleep(std::time::Duration::from_millis(300));
    }
    true.into()
}

#[cfg(windows)]
unsafe extern "system" fn exit_wndproc(
    hwnd: windows::Win32::Foundation::HWND,
    msg: u32,
    wp: windows::Win32::Foundation::WPARAM,
    lp: windows::Win32::Foundation::LPARAM,
) -> windows::Win32::Foundation::LRESULT {
    use windows::Win32::Foundation::LRESULT;
    use windows::Win32::UI::WindowsAndMessaging::{
        DefWindowProcW, WM_ENDSESSION, WM_QUERYENDSESSION,
    };
    match msg {
        WM_QUERYENDSESSION => LRESULT(1),
        WM_ENDSESSION => {
            if wp.0 != 0 {
                EXIT.store(true, std::sync::atomic::Ordering::SeqCst);
                std::thread::sleep(std::time::Duration::from_millis(300));
            }
            LRESULT(0)
        }
        // SAFETY: forwarding the window procedure's own arguments.
        _ => unsafe { DefWindowProcW(hwnd, msg, wp, lp) },
    }
}

/// A hidden top-level window on its own thread. Message-only windows do not get
/// `WM_ENDSESSION`, so this one is a plain, never shown window.
#[cfg(windows)]
fn spawn_endsession_window() {
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DispatchMessageW, GetMessageW, MSG, RegisterClassW, TranslateMessage,
        WINDOW_EX_STYLE, WNDCLASSW, WS_OVERLAPPED,
    };
    use windows::core::w;
    let _ = std::thread::Builder::new()
        .name("exit-window".into())
        .spawn(|| {
            // SAFETY: standard window class and message loop on this thread.
            unsafe {
                let Ok(inst) = GetModuleHandleW(None) else {
                    return;
                };
                let class = w!("arrowhead_exit");
                let wc = WNDCLASSW {
                    lpfnWndProc: Some(exit_wndproc),
                    hInstance: inst.into(),
                    lpszClassName: class,
                    ..Default::default()
                };
                RegisterClassW(&wc);
                let Ok(_hwnd) = CreateWindowExW(
                    WINDOW_EX_STYLE(0),
                    class,
                    w!("arrowhead"),
                    WS_OVERLAPPED,
                    0,
                    0,
                    0,
                    0,
                    None,
                    None,
                    Some(inst.into()),
                    None,
                ) else {
                    return;
                };
                let mut msg = MSG::default();
                while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
        });
}

/// Installs the exit signal handlers and a panic hook that sets `EXIT` before the default hook.
pub fn install_exit_hooks() {
    #[cfg(windows)]
    {
        use windows::Win32::System::Console::SetConsoleCtrlHandler;
        // SAFETY: the handler only stores to an atomic and sleeps.
        unsafe {
            let _ = SetConsoleCtrlHandler(Some(on_console_event), true);
        }
        spawn_endsession_window();
    }
    #[cfg(unix)]
    // SAFETY: the handler only stores to an atomic, which is async-signal-safe.
    unsafe {
        for s in [libc::SIGINT, libc::SIGTERM, libc::SIGHUP] {
            libc::signal(s, on_exit_signal as *const () as libc::sighandler_t);
        }
    }
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        EXIT.store(true, std::sync::atomic::Ordering::SeqCst);
        default(info);
    }));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;

    #[test]
    fn qpc_conversion() {
        assert_eq!(qpc_to_ns(10_000_000, 10_000_000), 1_000_000_000);
        assert_eq!(qpc_to_ns(15, 10_000_000), 1_500);
        // 10 MHz for 30 days: ticks * 1e9 would overflow u64.
        let ticks = 10_000_000i64 * 86_400 * 30;
        assert_eq!(qpc_to_ns(ticks, 10_000_000), 86_400u64 * 30 * 1_000_000_000);
        assert_eq!(qpc_to_ns(-5, 10_000_000), 0);
        assert_eq!(qpc_to_ns(5, 0), 5_000_000_000);
    }

    #[test]
    fn panic_sets_exit() {
        install_exit_hooks();
        assert!(std::panic::catch_unwind(|| panic!("boom")).is_err());
        assert!(EXIT.load(Ordering::SeqCst));
    }
}
