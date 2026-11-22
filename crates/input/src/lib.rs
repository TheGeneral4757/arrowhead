//! `KeySink` backends (uinput, SendInput, null), `FocusProbe` backends, global hotkeys.

#[cfg(target_os = "linux")]
pub mod kwin;
#[cfg(target_os = "linux")]
pub mod uinput;
