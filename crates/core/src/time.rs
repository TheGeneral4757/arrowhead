use std::time::Duration;

/// Monotonic time in nanoseconds (Linux `CLOCK_MONOTONIC`, Windows QPC).
#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    serde::Serialize,
    serde::Deserialize,
)]
pub struct Nanos(pub u64);

impl Nanos {
    pub fn saturating_sub(self, other: Nanos) -> Duration {
        Duration::from_nanos(self.0.saturating_sub(other.0))
    }
}

impl Nanos {
    pub fn from_ms(ms: f64) -> Nanos {
        Nanos((ms * 1e6).max(0.0) as u64)
    }

    pub fn ms(self) -> f64 {
        self.0 as f64 / 1e6
    }

    /// `self - o` in ms, signed.
    pub fn ms_since(self, o: Nanos) -> f64 {
        (self.0 as i64 - o.0 as i64) as f64 / 1e6
    }

    pub fn add_ms(self, ms: f64) -> Nanos {
        Nanos((self.0 as i64 + (ms * 1e6) as i64).max(0) as u64)
    }
}

pub trait Clock: Send + Sync {
    fn now(&self) -> Nanos;
    /// Returns at `t` or later.
    fn sleep_until(&self, t: Nanos);
}

pub struct RealClock;

impl Clock for RealClock {
    fn now(&self) -> Nanos {
        Nanos(crate::runtime::platform::monotonic_ns())
    }

    /// Absolute sleep to `t - 300 us`, then spin (spec 04).
    fn sleep_until(&self, t: Nanos) {
        crate::runtime::platform::sleep_abs_ns(t.0.saturating_sub(300_000));
        while self.now() < t {
            std::hint::spin_loop();
        }
    }
}

/// Test clock: `sleep_until` jumps time forward instantly.
#[derive(Default)]
pub struct VirtualClock(std::sync::atomic::AtomicU64);

impl VirtualClock {
    pub fn set(&self, t: Nanos) {
        self.0.fetch_max(t.0, std::sync::atomic::Ordering::SeqCst);
    }
}

impl Clock for VirtualClock {
    fn now(&self) -> Nanos {
        Nanos(self.0.load(std::sync::atomic::Ordering::SeqCst))
    }

    fn sleep_until(&self, t: Nanos) {
        self.set(t);
    }
}
