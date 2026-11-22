//! Engine: frame types, detection, tracking, timing, scheduler, player model,
//! session state and config. OS-free except `runtime::platform`.

pub mod config;
pub mod detect;
pub mod engine;
pub mod frame;
pub mod input;
pub mod plan;
pub mod runtime;
pub mod sched;
pub mod session;
#[cfg(any(test, feature = "testkit"))]
pub mod testkit;
pub mod time;
pub mod track;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
