//! Event bus (spec 07): engine events to the UI in batches (lane visualiser at 60 Hz, the rest
//! at 30 Hz), to the shared status and to the log file.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use arrowhead_core::config::data_dir;
use arrowhead_core::event::{EngineEvent, Level};
use arrowhead_core::session::State;
use tauri::ipc::Channel;

/// What the top bar shows, kept current by the bus.
#[derive(Clone, Debug)]
pub struct Live {
    pub state: State,
    pub dry_run: bool,
    pub focus: Option<bool>,
}

impl Default for Live {
    fn default() -> Self {
        Self {
            state: State::Idle,
            dry_run: false,
            focus: None,
        }
    }
}

/// Civil date (UTC) from days since 1970-01-01 (H. Hinnant's algorithm).
fn civil(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// Daily log files `logs/arrowhead.<date>.log` in the data dir, keeping 7 (spec 08).
pub struct LogFile {
    pub dir: Option<PathBuf>,
    pub min: Mutex<Level>,
}

impl LogFile {
    pub fn new(level: &str) -> Self {
        let dir = data_dir().map(|d| d.join("logs"));
        if let Some(d) = &dir {
            let _ = fs::create_dir_all(d);
            // Names sort by date: keep the newest 7.
            if let Ok(rd) = fs::read_dir(d) {
                let mut logs: Vec<PathBuf> = rd
                    .flatten()
                    .map(|e| e.path())
                    .filter(|p| p.extension().is_some_and(|x| x == "log"))
                    .collect();
                logs.sort();
                let n = logs.len().saturating_sub(7);
                for p in &logs[..n] {
                    let _ = fs::remove_file(p);
                }
            }
        }
        Self {
            dir,
            min: Mutex::new(parse_level(level)),
        }
    }

    pub fn write(&self, level: Level, text: &str) {
        let rank = |l: Level| l as u8;
        if rank(level) < rank(*self.min.lock().unwrap()) {
            return;
        }
        let Some(dir) = &self.dir else { return };
        let secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs()) as i64;
        let (y, m, d) = civil(secs.div_euclid(86_400));
        let t = secs.rem_euclid(86_400);
        let path = dir.join(format!("arrowhead.{y:04}-{m:02}-{d:02}.log"));
        if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(path) {
            let lv = match level {
                Level::Info => "INFO",
                Level::Warn => "WARN",
                Level::Error => "ERROR",
            };
            let _ = writeln!(
                f,
                "{:02}:{:02}:{:02}Z {lv} {text}",
                t / 3600,
                t / 60 % 60,
                t % 60
            );
        }
    }

    /// Last `n` lines at warn or above, oldest first, across the kept files.
    pub fn recent_warnings(&self, n: usize) -> Vec<String> {
        let Some(dir) = &self.dir else { return vec![] };
        let mut files: Vec<PathBuf> = fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "log"))
            .collect();
        files.sort();
        let mut out: Vec<String> = files
            .iter()
            .filter_map(|p| fs::read_to_string(p).ok())
            .flat_map(|s| {
                s.lines()
                    .filter(|l| l.contains(" WARN ") || l.contains(" ERROR "))
                    .map(String::from)
                    .collect::<Vec<_>>()
            })
            .collect();
        let skip = out.len().saturating_sub(n);
        out.drain(..skip);
        out
    }
}

pub fn parse_level(s: &str) -> Level {
    match s {
        "warn" => Level::Warn,
        "error" => Level::Error,
        _ => Level::Info,
    }
}

/// Runs until every sender of `rx` is gone. `ui` is None for setup steps (log only).
pub fn run(
    rx: Receiver<EngineEvent>,
    ui: Option<Channel<Vec<EngineEvent>>>,
    live: Arc<Mutex<Live>>,
    logs: Arc<LogFile>,
) {
    let tick = Duration::from_millis(16);
    let mut next = Instant::now() + tick;
    let mut n = 0u64;
    let mut pending = Vec::with_capacity(256);
    let mut notes = None;
    loop {
        let done = match rx.recv_timeout(next.saturating_duration_since(Instant::now())) {
            Ok(e) => {
                match &e {
                    EngineEvent::State { state, dry_run, .. } => {
                        let mut l = live.lock().unwrap();
                        l.state = *state;
                        l.dry_run = *dry_run;
                    }
                    EngineEvent::Focus { focused } => live.lock().unwrap().focus = *focused,
                    EngineEvent::Log { level, text } => logs.write(*level, text),
                    EngineEvent::Stopped { error: Some(e) } => logs.write(Level::Error, e),
                    _ => {}
                }
                if matches!(e, EngineEvent::Notes { .. }) {
                    notes = Some(e);
                } else if pending.len() < 4096 {
                    pending.push(e);
                }
                false
            }
            Err(RecvTimeoutError::Timeout) => false,
            Err(RecvTimeoutError::Disconnected) => true,
        };
        if Instant::now() >= next || done {
            next += tick;
            n += 1;
            let mut batch = if n.is_multiple_of(2) || done {
                std::mem::take(&mut pending)
            } else {
                Vec::new()
            };
            batch.extend(notes.take());
            if let Some(ch) = &ui
                && !batch.is_empty()
            {
                let _ = ch.send(batch);
            }
        }
        if done {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_dates() {
        assert_eq!(civil(0), (1970, 1, 1));
        assert_eq!(civil(20_779), (2026, 11, 22));
        assert_eq!(civil(11_016), (2000, 2, 29));
    }
}
