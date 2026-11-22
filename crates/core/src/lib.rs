//! Engine: frame types, detection, tracking, timing, scheduler, player model,
//! session state and config. OS-free except `runtime::platform`.

pub mod config;
pub mod detect;
pub mod frame;
pub mod runtime;
pub mod session;
#[cfg(any(test, feature = "testkit"))]
pub mod testkit;
pub mod time;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
