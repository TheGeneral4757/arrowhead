//! `FrameSource` backends: PipeWire (Linux), DXGI + WGC (Windows, v0.2), clip replay and recording.

pub mod clip;
pub mod ring;
pub mod winpolicy;

#[cfg(target_os = "linux")]
pub mod pipewire;

#[cfg(windows)]
pub mod dxgi;
#[cfg(windows)]
pub mod wgc;
#[cfg(windows)]
pub mod winsrc;

pub use clip::{ClipHeader, ClipReader, ClipRecorder, ClipWriter, ReplaySource};
#[cfg(windows)]
pub use winsrc::WindowsSource;
