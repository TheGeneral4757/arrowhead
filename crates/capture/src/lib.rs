//! `FrameSource` backends: PipeWire (Linux), DXGI + WGC (Windows, v0.2), clip replay and recording.

pub mod clip;
pub mod ring;

#[cfg(target_os = "linux")]
pub mod pipewire;

pub use clip::{ClipHeader, ClipReader, ClipRecorder, ClipWriter, ReplaySource};
