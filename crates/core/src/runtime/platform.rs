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

// shortcut: Instant-based on Windows, switch to QPC when the DXGI backend lands (v0.2).
#[cfg(not(unix))]
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

// shortcut: plain sleep on Windows, switch to a high-resolution waitable timer in v0.2.
#[cfg(not(unix))]
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

#[cfg(not(unix))]
pub fn make_realtime() -> &'static str {
    "normal"
}
