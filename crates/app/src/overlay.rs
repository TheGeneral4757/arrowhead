//! Overlay process control (spec 07, overlay): spawns `arrowhead-overlay`, feeds it the filtered
//! events as JSON lines on stdin, and minimises the main window while a song plays.
//! Dropping the `Overlay` closes the queue, which closes the pipe and kills the child.
//! `shown` (the Live page's toggle) starts or stops the child while a run goes on.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{SyncSender, sync_channel};

use arrowhead_core::config::AutoOnOff;
use arrowhead_core::event::EngineEvent;
use arrowhead_core::session::State;
use serde_json::{Value, json};
use tauri::{AppHandle, Manager, WebviewWindow};

pub struct Overlay {
    /// None when the overlay is off, missing or failed to start.
    tx: Option<SyncSender<String>>,
    shown: Arc<AtomicBool>,
    /// `--output`, `--size`, `--hz` for a restart.
    args: (String, [u32; 2], f32),
    /// The last State, Layout and Hud, replayed to a restarted child (it draws nothing without
    /// the first two, and needs the HUD regions to keep clear of them).
    replay: [Option<Value>; 3],
    win: Option<WebviewWindow>,
    minimise: AutoOnOff,
    /// One monitor: the overlay covers the main window, so Auto minimises it.
    single: bool,
    minimised: bool,
    batch: Vec<Value>,
    last_notes_ms: f64,
}

fn binary() -> Option<PathBuf> {
    let name = if cfg!(windows) {
        "arrowhead-overlay.exe"
    } else {
        "arrowhead-overlay"
    };
    let sibling = std::env::current_exe().ok()?.parent()?.join(name);
    [sibling, PathBuf::from("/usr/lib/arrowhead").join(name)]
        .into_iter()
        .find(|p| p.is_file())
}

/// Whether the overlay should run: `on`, or `auto` with a single monitor.
pub fn wanted(mode: AutoOnOff, monitors: usize) -> bool {
    match mode {
        AutoOnOff::On => true,
        AutoOnOff::Auto => monitors == 1,
        AutoOnOff::Off => false,
    }
}

fn spawn(output: &str, size: [u32; 2], hz: f32) -> Result<SyncSender<String>, String> {
    let exe = binary().ok_or("arrowhead-overlay not found next to the app")?;
    let mut cmd = Command::new(exe);
    if !output.is_empty() {
        cmd.args(["--output", output]);
    }
    if size[0] > 0 {
        cmd.args(["--size", &format!("{}x{}", size[0], size[1])]);
    }
    if hz > 0.0 {
        cmd.args(["--hz", &hz.to_string()]);
    }
    let mut child = cmd
        .stdin(Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;
    let mut stdin = child.stdin.take().ok_or("no overlay stdin")?;
    // A writer thread so a stuck overlay can never stall the bus; a full queue drops batches.
    let (tx, rx) = sync_channel::<String>(64);
    std::thread::Builder::new()
        .name("overlay-pipe".into())
        .spawn(move || {
            for line in rx {
                if writeln!(stdin, "{line}")
                    .and_then(|()| stdin.flush())
                    .is_err()
                {
                    break;
                }
            }
            drop(stdin);
            let _ = child.kill();
            let _ = child.wait();
        })
        .map_err(|e| e.to_string())?;
    Ok(tx)
}

impl Overlay {
    /// `output`, `size` and `hz` (the profile's monitor name, stream size and rate, or the
    /// `ui.overlay_output` override) pick the overlay's output. Sets `shown` to whether it
    /// runs. Returns the overlay and, when it could not start, why (for the log).
    pub fn start(
        app: &AppHandle,
        overlay: AutoOnOff,
        minimise: AutoOnOff,
        (output, size, hz): (&str, [u32; 2], f32),
        shown: Arc<AtomicBool>,
    ) -> (Self, Option<String>) {
        let monitors = app.available_monitors().map_or(1, |m| m.len());
        let want = wanted(overlay, monitors);
        shown.store(want, Ordering::SeqCst);
        let (tx, err) = if want {
            match spawn(output, size, hz) {
                Ok(tx) => (Some(tx), None),
                Err(e) => (None, Some(e)),
            }
        } else {
            let why = match overlay {
                AutoOnOff::Off => "set to Off in Settings".to_string(),
                _ => format!(
                    "Auto shows it only with one monitor ({monitors} found); set it to On in Settings or show it from Live"
                ),
            };
            (None, Some(why))
        };
        if err.is_some() {
            shown.store(false, Ordering::SeqCst);
        }
        (
            Self {
                tx,
                shown,
                args: (output.to_string(), size, hz),
                replay: [None, None, None],
                win: app.get_webview_window("main"),
                minimise,
                single: monitors == 1,
                minimised: false,
                batch: Vec::new(),
                last_notes_ms: f64::MIN,
            },
            err,
        )
    }

    /// Takes one engine event: keeps what the overlay draws, and minimises or restores the
    /// window on state changes.
    pub fn push(&mut self, e: &EngineEvent) {
        match e {
            EngineEvent::State { state, .. } => self.window_for(*state),
            EngineEvent::Notes { t_ms, .. } => {
                // The overlay only needs the clock, to drop stale ticks.
                if self.tx.is_some() && t_ms - self.last_notes_ms >= 100.0 {
                    self.last_notes_ms = *t_ms;
                    self.batch.push(json!({"type": "Notes", "t_ms": t_ms}));
                }
                return;
            }
            _ => {}
        }
        let slot = match e {
            EngineEvent::State { .. } => Some(0),
            EngineEvent::Layout { .. } => Some(1),
            EngineEvent::Hud(_) => Some(2),
            _ => None,
        };
        if let Some(i) = slot {
            self.replay[i] = serde_json::to_value(e).ok();
        }
        if self.tx.is_some()
            && matches!(
                e,
                EngineEvent::State { .. }
                    | EngineEvent::Layout { .. }
                    | EngineEvent::Planned { .. }
                    | EngineEvent::Pressed { .. }
                    | EngineEvent::Readback { .. }
                    | EngineEvent::Hud(_)
            )
            && let Ok(v) = serde_json::to_value(e)
        {
            self.batch.push(v);
        }
    }

    /// Sends what `push` collected as one line.
    pub fn flush(&mut self) {
        let shown = self.shown.load(Ordering::Relaxed);
        if shown != self.tx.is_some() {
            self.tx = None;
            if shown {
                let (o, s, hz) = &self.args;
                self.tx = spawn(o, *s, *hz).ok();
                if self.tx.is_none() {
                    self.shown.store(false, Ordering::SeqCst);
                }
                let replay: Vec<Value> = self.replay.iter().flatten().cloned().collect();
                self.batch.splice(0..0, replay);
            }
        }
        if self.batch.is_empty() {
            return;
        }
        let line = serde_json::to_string(&self.batch).unwrap_or_default();
        self.batch.clear();
        if let Some(tx) = &self.tx {
            let _ = tx.try_send(line);
        }
    }

    /// Minimise on Playing; restore when the engine stops or the results screen shows. Between
    /// songs the session drops to Armed for a moment, which keeps the window down.
    fn window_for(&mut self, state: State) {
        let Some(win) = &self.win else { return };
        let up = match self.minimise {
            AutoOnOff::On => true,
            AutoOnOff::Auto => self.tx.is_some() && self.single,
            AutoOnOff::Off => false,
        };
        match state {
            State::Playing if up && !self.minimised => {
                self.minimised = win.minimize().is_ok();
            }
            State::Idle | State::Results if self.minimised => {
                let _ = win.unminimize();
                self.minimised = false;
            }
            _ => {}
        }
    }
}

impl Drop for Overlay {
    fn drop(&mut self) {
        if self.minimised
            && let Some(w) = &self.win
        {
            let _ = w.unminimize();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_means_single_monitor() {
        assert!(wanted(AutoOnOff::Auto, 1));
        assert!(!wanted(AutoOnOff::Auto, 2));
        assert!(wanted(AutoOnOff::On, 2));
        assert!(!wanted(AutoOnOff::Off, 1));
    }
}
