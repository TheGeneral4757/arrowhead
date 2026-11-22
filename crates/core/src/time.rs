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

pub trait Clock: Send + Sync {
    fn now(&self) -> Nanos;
}

pub struct RealClock;

impl Clock for RealClock {
    fn now(&self) -> Nanos {
        Nanos(crate::runtime::platform::monotonic_ns())
    }
}
