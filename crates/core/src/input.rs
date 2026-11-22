//! `KeySink` and the OS-free sinks (spec 05).

use std::sync::Arc;

use serde::Serialize;

use crate::time::{Clock, Nanos};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum KeyAction {
    Press(u8),
    Release(u8),
}

impl KeyAction {
    pub fn lane(self) -> u8 {
        match self {
            Self::Press(l) | Self::Release(l) => l,
        }
    }
}

#[derive(Debug)]
pub struct InputError(pub String);

impl std::fmt::Display for InputError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "input: {}", self.0)
    }
}

impl std::error::Error for InputError {}

pub trait KeySink: Send {
    /// One atomic batch (one SYN_REPORT / one SendInput call).
    fn send_batch(&mut self, actions: &[KeyAction]) -> Result<(), InputError>;
    fn release_all(&mut self) -> Result<(), InputError>;
    fn name(&self) -> &'static str;
}

/// Is the game window focused? Polled at 10 Hz by the focus thread (spec 05).
pub trait FocusProbe: Send {
    fn game_focused(&mut self) -> Result<bool, InputError>;
    fn name(&self) -> &'static str;
}

/// Global hotkey actions (spec 05).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hotkey {
    Toggle,
    Panic,
    NudgeUp,
    NudgeDown,
    CycleStyle,
}

/// Lane keys (left, down, up, right) for a key set (spec 05).
pub fn key_names(set: crate::config::KeySet, custom: &[String; 4]) -> [&str; 4] {
    use crate::config::KeySet;
    match set {
        KeySet::Wasd => ["a", "s", "w", "d"],
        KeySet::Arrows => ["left", "down", "up", "right"],
        KeySet::Custom => [0, 1, 2, 3].map(|i| custom[i].as_str()),
    }
}

/// Does the exe path (`C:\...\RobloxPlayerBeta.exe`) match one of `game_window_classes`?
/// Compared on the file name, case-insensitively, with or without `.exe`.
pub fn exe_matches(path: &str, classes: &[String]) -> bool {
    let file = path
        .rsplit(['\\', '/'])
        .next()
        .unwrap_or(path)
        .to_lowercase();
    if file.is_empty() {
        return false;
    }
    let stem = file.strip_suffix(".exe").unwrap_or(&file);
    classes.iter().any(|c| {
        let c = c.to_lowercase();
        c == file || c.strip_suffix(".exe").unwrap_or(&c) == stem
    })
}

/// Tests: records `(time, action)` against a clock.
pub struct RecordingSink {
    pub clock: Arc<dyn Clock>,
    pub log: Vec<(Nanos, KeyAction)>,
}

impl RecordingSink {
    pub fn new(clock: Arc<dyn Clock>) -> Self {
        Self {
            clock,
            log: Vec::new(),
        }
    }
}

impl KeySink for RecordingSink {
    fn send_batch(&mut self, actions: &[KeyAction]) -> Result<(), InputError> {
        let t = self.clock.now();
        self.log.extend(actions.iter().map(|&a| (t, a)));
        Ok(())
    }
    fn release_all(&mut self) -> Result<(), InputError> {
        Ok(())
    }
    fn name(&self) -> &'static str {
        "recording"
    }
}

/// Dry run: sends nothing. The scheduler's events still say what would have been pressed.
pub struct NullSink;

impl KeySink for NullSink {
    fn send_batch(&mut self, _: &[KeyAction]) -> Result<(), InputError> {
        Ok(())
    }
    fn release_all(&mut self) -> Result<(), InputError> {
        Ok(())
    }
    fn name(&self) -> &'static str {
        "dry-run"
    }
}
