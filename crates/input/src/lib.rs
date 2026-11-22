//! `KeySink` backends (uinput, SendInput, null), `FocusProbe` backends, global hotkeys.

#[cfg(target_os = "linux")]
pub mod hotkeys;
#[cfg(target_os = "linux")]
pub mod kwin;
#[cfg(target_os = "linux")]
pub mod uinput;

#[cfg(windows)]
pub mod winfocus;
#[cfg(windows)]
pub mod winhotkeys;
#[cfg(windows)]
pub mod winsend;
pub mod winutil;
