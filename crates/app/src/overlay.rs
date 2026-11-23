//! Overlay process control (spec 07, overlay): spawns `arrowhead-overlay`, feeds it the filtered
//! events as JSON lines on stdin, and minimises the main window while a song plays.
//! Dropping the `Overlay` closes the queue, which closes the pipe and kills the child.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc::{SyncSender, sync_channel};

use arrowhead_core::config::AutoOnOff;
use arrowhead_core::event::EngineEvent;
use arrowhead_core::session::State;
use serde_json::{Value, json};
use tauri::{AppHandle, Manager, WebviewWindow};

pub struct Overlay {
    /// None when the overlay is off, missing or failed to start.
    tx: Option<SyncSender<String>>,
    win: Option<WebviewWindow>,
    minimise: AutoOnOff,
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
fn wanted(mode: AutoOnOff, monitors: usize) -> bool {
    match mode {
        AutoOnOff::On => true,
        AutoOnOff::Auto => monitors == 1,
        AutoOnOff::Off => false,
    }
}

fn spawn(output: &str, size: [u32; 2]) -> Result<SyncSender<String>, String> {
    let exe = binary().ok_or("arrowhead-overlay not found next to the app")?;
    let mut cmd = Command::new(exe);
    if !output.is_empty() {
        cmd.args(["--output", output]);
    }
    if size[0] > 0 {
        cmd.args(["--size", &format!("{}x{}", size[0], size[1])]);
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
    /// `output` and `size` are the profile's monitor name and stream size, which pick the
    /// overlay's output. Returns the overlay and, when it could not start, why (for the log).
    pub fn start(
        app: &AppHandle,
        overlay: AutoOnOff,
        minimise: AutoOnOff,
        output: &str,
        size: [u32; 2],
    ) -> (Self, Option<String>) {
        let monitors = app.available_monitors().map_or(1, |m| m.len());
        let (tx, err) = if wanted(overlay, monitors) {
            match spawn(output, size) {
                Ok(tx) => (Some(tx), None),
                Err(e) => (None, Some(e)),
            }
        } else {
            let why = match overlay {
                AutoOnOff::Off => "set to Off in Settings".to_string(),
                _ => format!("Auto shows it only with one monitor ({monitors} found); set it to On in Settings"),
            };
            (None, Some(why))
        };
        (
            Self {
                tx,
                win: app.get_webview_window("main"),
                minimise,
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
        if self.tx.is_some()
            && matches!(
                e,
                EngineEvent::State { .. }
                    | EngineEvent::Layout { .. }
                    | EngineEvent::Planned { .. }
                    | EngineEvent::Pressed { .. }
                    | EngineEvent::Readback { .. }
            )
            && let Ok(v) = serde_json::to_value(e)
        {
            self.batch.push(v);
        }
    }

    /// Sends what `push` collected as one line.
    pub fn flush(&mut self) {
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
            AutoOnOff::Auto => self.tx.is_some(),
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
