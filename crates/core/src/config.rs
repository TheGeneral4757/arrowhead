//! Config, profile and history files (spec 08). Writes are atomic and keep keys
//! this version does not know about, so hand edits survive a save.

use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

pub const CONFIG_VERSION: u32 = 1;

/// `~/.config/arrowhead` (Linux), `%APPDATA%\arrowhead` (Windows).
pub fn config_dir() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("arrowhead"))
}

/// `~/.local/share/arrowhead` (Linux), `%LOCALAPPDATA%\arrowhead` (Windows).
pub fn data_dir() -> Option<PathBuf> {
    dirs::data_local_dir().map(|d| d.join("arrowhead"))
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub version: u32,
    pub profile: String,
    pub style: String,
    pub keys: Keys,
    pub hotkeys: Hotkeys,
    pub safety: Safety,
    pub capture: Capture,
    pub timing: Timing,
    pub game: Game,
    pub ui: Ui,
    pub updates: Updates,
    pub log: Log,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            version: CONFIG_VERSION,
            profile: "default".into(),
            style: "Max".into(),
            keys: Keys::default(),
            hotkeys: Hotkeys::default(),
            safety: Safety::default(),
            capture: Capture::default(),
            timing: Timing::default(),
            game: Game::default(),
            ui: Ui::default(),
            updates: Updates::default(),
            log: Log::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum KeySet {
    #[default]
    Wasd,
    Arrows,
    Custom,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Keys {
    pub set: KeySet,
    pub custom: [String; 4],
}

impl Default for Keys {
    fn default() -> Self {
        Self {
            set: KeySet::Wasd,
            custom: ["a", "s", "w", "d"].map(String::from),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Hotkeys {
    pub toggle: String,
    pub panic: String,
    pub nudge_up: String,
    pub nudge_down: String,
    pub cycle_style: String,
}

impl Default for Hotkeys {
    fn default() -> Self {
        Self {
            toggle: "F8".into(),
            panic: "F12".into(),
            nudge_up: "F9".into(),
            nudge_down: "F10".into(),
            cycle_style: "F11".into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Safety {
    pub dry_run: bool,
    pub require_focus_guard: bool,
    pub max_hold_ms: u32,
    pub game_window_classes: Vec<String>,
}

impl Default for Safety {
    fn default() -> Self {
        Self {
            dry_run: false,
            require_focus_guard: false,
            max_hold_ms: 15000,
            game_window_classes: ["org.vinegarhq.Sober", "sober", "RobloxPlayerBeta.exe"]
                .map(String::from)
                .to_vec(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LinuxSource {
    #[default]
    Monitor,
    Window,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WindowsBackend {
    #[default]
    Auto,
    Dxgi,
    WgcMonitor,
    WgcWindow,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Capture {
    pub linux_source: LinuxSource,
    pub windows_backend: WindowsBackend,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Timing {
    pub commit_ms: f32,
    pub late_limit_ms: f32,
    pub tap_hold_ms: f32,
    pub min_gap_ms: f32,
}

impl Default for Timing {
    fn default() -> Self {
        Self {
            commit_ms: 8.0,
            late_limit_ms: 40.0,
            tap_hold_ms: 35.0,
            min_gap_ms: 10.0,
        }
    }
}

/// Per-judgment values, Sick to Bad (and Miss where it applies).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Windows {
    pub sick: f32,
    pub good: f32,
    pub ok: f32,
    pub bad: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AccuracyWeights {
    pub sick: f32,
    pub good: f32,
    pub ok: f32,
    pub bad: f32,
    pub miss: f32,
}

/// Game facts, some still unverified (spec 12).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Game {
    pub windows_ms: Windows,
    pub accuracy_weights: AccuracyWeights,
    pub readback_sign: i8,
}

impl Default for Game {
    fn default() -> Self {
        Self {
            windows_ms: Windows {
                sick: 45.0,
                good: 80.0,
                ok: 130.0,
                bad: 230.0,
            },
            accuracy_weights: AccuracyWeights {
                sick: 1.0,
                good: 0.75,
                ok: 0.5,
                bad: 0.25,
                miss: 0.0,
            },
            readback_sign: -1,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AutoOnOff {
    #[default]
    Auto,
    On,
    Off,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Ui {
    pub overlay: AutoOnOff,
    pub minimise_while_playing: AutoOnOff,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Updates {
    pub check: bool,
}

impl Default for Updates {
    fn default() -> Self {
        Self { check: true }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Log {
    pub level: String,
    pub per_note: bool,
}

impl Default for Log {
    fn default() -> Self {
        Self {
            level: "info".into(),
            per_note: false,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    #[default]
    Centre,
    Left,
    Right,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Scroll {
    #[default]
    Up,
    Down,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Nudge {
    pub dx: f32,
    pub dy: f32,
    pub dpitch: f32,
    pub dwidth: f32,
}

impl Nudge {
    /// The hand correction from Setup on top of a detected layout: shift all receptors by
    /// (dx, dy), spread them by dpitch per lane around the centre, grow the size by dwidth.
    pub fn apply(&self, l: &crate::detect::Layout) -> crate::detect::Layout {
        let mut out = l.clone();
        for (i, r) in out.receptors.iter_mut().enumerate() {
            r[0] += self.dx + (i as f32 - 1.5) * self.dpitch;
            r[1] += self.dy;
        }
        out.size = (out.size + self.dwidth).max(4.0);
        out
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Latency {
    pub capture_ms: f32,
    pub input_ms: f32,
    pub game_bias_ms: f32,
}

/// HUD regions `[x, y, w, h]` found by search; absent = not readable.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct HudRegions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ms_readout: Option<[i32; 4]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub accuracy: Option<[i32; 4]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub score_self: Option<[i32; 4]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub score_opponent: Option<[i32; 4]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<[i32; 4]>,
}

/// Calibration for one display setup. `[colours]` and `[detect]` arrive in M3;
/// until then they are kept as unknown keys.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Profile {
    pub version: u32,
    pub monitor: String,
    pub stream_size: [u32; 2],
    pub refresh_hz: f32,
    pub scale: f32,
    pub side: Side,
    pub scroll: Scroll,
    pub receptors: [[f32; 2]; 4],
    pub radius_px: f32,
    pub nudge: Nudge,
    pub latency: Latency,
    pub hud: HudRegions,
    /// Learned head and trail colours; absent until calibration ran.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub colours: Option<crate::detect::Colours>,
    /// Full receptor layout from the last calibration; lets a run skip the receptor search.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub layout: Option<crate::detect::Layout>,
    pub detect: crate::detect::DetectConfig,
}

impl Default for Profile {
    fn default() -> Self {
        Self {
            version: CONFIG_VERSION,
            monitor: String::new(),
            stream_size: [0, 0],
            refresh_hz: 0.0,
            scale: 1.0,
            side: Side::Centre,
            scroll: Scroll::Up,
            receptors: [[0.0; 2]; 4],
            radius_px: 0.0,
            nudge: Nudge::default(),
            latency: Latency::default(),
            hud: HudRegions::default(),
            colours: None,
            layout: None,
            detect: Default::default(),
        }
    }
}

impl Profile {
    /// Profiles are matched by `(monitor, stream_size, scale)`.
    pub fn matches(&self, monitor: &str, stream_size: [u32; 2], scale: f32) -> bool {
        self.monitor == monitor
            && self.stream_size == stream_size
            && (self.scale - scale).abs() < 1e-3
    }
}

/// One line of `history.jsonl`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub v: u32,
    pub started: String,
    pub duration_s: f32,
    pub song: String,
    pub difficulty: String,
    pub profile: String,
    pub style: String,
    pub target: Option<f32>,
    pub accuracy_hud: Option<f32>,
    pub counts: BTreeMap<String, u32>,
    pub mean_err_ms: Option<f32>,
    pub ur: Option<f32>,
    pub offset_ms: f32,
    pub engine_missed: u32,
    pub watchdog_releases: u32,
    pub seed: u64,
}

#[derive(Debug)]
pub enum ConfigError {
    Io(io::Error),
    Parse(String),
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "{e}"),
            Self::Parse(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for ConfigError {}

impl From<io::Error> for ConfigError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

/// Reads a TOML file. A missing file gives the defaults.
pub fn load_toml<T: DeserializeOwned + Default>(path: &Path) -> Result<T, ConfigError> {
    match fs::read_to_string(path) {
        Ok(s) => {
            toml::from_str(&s).map_err(|e| ConfigError::Parse(format!("{}: {e}", path.display())))
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(T::default()),
        Err(e) => Err(e.into()),
    }
}

/// Writes `value` over the existing file, keeping keys and comments the struct does not know.
pub fn save_toml<T: Serialize>(path: &Path, value: &T) -> Result<(), ConfigError> {
    let fresh: toml_edit::DocumentMut = toml::to_string(value)
        .map_err(|e| ConfigError::Parse(e.to_string()))?
        .parse()
        .map_err(|e: toml_edit::TomlError| ConfigError::Parse(e.to_string()))?;
    let mut doc = match fs::read_to_string(path) {
        Ok(s) => s.parse::<toml_edit::DocumentMut>().unwrap_or_default(),
        Err(e) if e.kind() == io::ErrorKind::NotFound => toml_edit::DocumentMut::new(),
        Err(e) => return Err(e.into()),
    };
    merge(doc.as_table_mut(), fresh.as_table());
    write_atomic(path, doc.to_string().as_bytes())?;
    Ok(())
}

// shortcut: keys removed from the struct (None fields) stay in the file; fine until a field is meant to be deletable.
fn merge(old: &mut toml_edit::Table, new: &toml_edit::Table) {
    for (key, item) in new.iter() {
        match (
            old.get_mut(key).and_then(|i| i.as_table_mut()),
            item.as_table(),
        ) {
            (Some(o), Some(n)) => merge(o, n),
            _ => match (
                old.get_mut(key).and_then(|i| i.as_value_mut()),
                item.as_value(),
            ) {
                // Keep the old value's comments and spacing.
                (Some(o), Some(n)) => {
                    let decor = o.decor().clone();
                    *o = n.clone();
                    *o.decor_mut() = decor;
                }
                _ => old[key] = item.clone(),
            },
        }
    }
}

/// Write `*.tmp`, fsync, rename over `path`.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    let mut f = fs::File::create(&tmp)?;
    f.write_all(bytes)?;
    f.sync_all()?;
    drop(f);
    fs::rename(&tmp, path)
}

/// Appends one JSON line with a single write, so a crash never leaves half a line.
pub fn append_history(path: &Path, entry: &HistoryEntry) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let mut line = serde_json::to_vec(entry).map_err(io::Error::other)?;
    line.push(b'\n');
    let mut f = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    f.write_all(&line)?;
    f.sync_data()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_gives_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let c: Config = load_toml(&dir.path().join("config.toml")).unwrap();
        assert_eq!(c, Config::default());
        assert_eq!(c.game.readback_sign, -1);
    }

    #[test]
    fn save_keeps_unknown_keys_and_comments() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("profile.toml");
        fs::write(
            &p,
            "monitor = \"DP-1\" # my screen\nfuture = 3\n\n[future_table]\nheads = [1, 2]\n",
        )
        .unwrap();
        let mut prof: Profile = load_toml(&p).unwrap();
        assert_eq!(prof.monitor, "DP-1");
        prof.monitor = "DP-2".into();
        prof.stream_size = [2560, 1440];
        save_toml(&p, &prof).unwrap();
        let text = fs::read_to_string(&p).unwrap();
        assert!(text.contains("monitor = \"DP-2\" # my screen"), "{text}");
        assert!(text.contains("future = 3"));
        assert!(text.contains("heads = [1, 2]"));
        let back: Profile = load_toml(&p).unwrap();
        assert_eq!(back, prof);
        assert!(!dir.path().join("profile.toml.tmp").exists());
    }

    #[test]
    fn spec_example_config_parses() {
        let c: Config = toml::from_str(
            "version = 1\nprofile = \"default\"\n[keys]\nset = \"arrows\"\n[capture]\nwindows_backend = \"wgc-monitor\"\n[ui]\noverlay = \"off\"\n",
        )
        .unwrap();
        assert_eq!(c.keys.set, KeySet::Arrows);
        assert_eq!(c.capture.windows_backend, WindowsBackend::WgcMonitor);
        assert_eq!(c.ui.overlay, AutoOnOff::Off);
    }

    #[test]
    fn history_appends_lines() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("history.jsonl");
        let e = HistoryEntry {
            v: 1,
            started: "2026-11-21T21:42:03Z".into(),
            duration_s: 142.5,
            song: "Gospel".into(),
            difficulty: "Hard".into(),
            profile: "default".into(),
            style: "Max".into(),
            target: None,
            accuracy_hud: Some(84.48),
            counts: [("sick", 243), ("good", 177)]
                .map(|(k, v)| (k.to_string(), v))
                .into(),
            mean_err_ms: None,
            ur: None,
            offset_ms: 0.0,
            engine_missed: 0,
            watchdog_releases: 0,
            seed: 1,
        };
        append_history(&p, &e).unwrap();
        append_history(&p, &e).unwrap();
        let text = fs::read_to_string(&p).unwrap();
        let lines: Vec<_> = text.lines().collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(serde_json::from_str::<HistoryEntry>(lines[1]).unwrap(), e);
    }
}
