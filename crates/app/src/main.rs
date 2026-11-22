// Hide the console window on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! Tauri shell (spec 07): commands, the event bus and the setup steps. The engine is
//! `arrowhead-runner`, the same one `arrowhead-cli run` drives.

mod bus;
mod overlay;

use std::sync::atomic::Ordering;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use arrowhead_core::config::{Config, Nudge, Profile, config_dir, data_dir, load_toml, save_toml};
use arrowhead_core::detect::Layout;
use arrowhead_core::event::{EngineEvent, Level};
use arrowhead_core::frame::{FrameSource, OwnedFrame, SourceInfo};
use arrowhead_core::input::{KeyAction, key_names};
use arrowhead_core::runtime::platform::{EXIT, install_exit_hooks};
use arrowhead_core::session::State;
use arrowhead_core::time::{Clock, Nanos, RealClock};
use arrowhead_runner::{Control, Opts, load_config, profile_path};
use serde::Serialize;
use tauri::ipc::Channel;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_updater::UpdaterExt;

use bus::{Live, LogFile};

type Src = Box<dyn FrameSource>;
type CmdRes<T> = Result<T, String>;

struct Engine {
    ctl: Arc<Control>,
    join: JoinHandle<()>,
}

#[derive(Clone, Serialize)]
struct UpdateInfo {
    version: String,
    notes: Option<String>,
    can_install: bool,
}

struct AppState {
    src: Arc<Mutex<Option<Src>>>,
    engine: Mutex<Option<Engine>>,
    live: Arc<Mutex<Live>>,
    logs: Arc<LogFile>,
    latency_tx: Mutex<Option<mpsc::Sender<Nanos>>>,
    /// PNG data URL of the last calibration snapshot.
    snapshot: Mutex<Option<String>>,
    update: Mutex<Option<UpdateInfo>>,
}

impl AppState {
    fn running(&self) -> bool {
        self.engine
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|e| !e.join.is_finished())
    }

    /// Stops the engine and waits for it: every key is released when this returns.
    fn stop_engine(&self) {
        if let Some(e) = self.engine.lock().unwrap().take() {
            e.ctl.stop.store(true, Ordering::SeqCst);
            let _ = e.join.join();
        }
    }
}

/// `aur` (or another packager) when installed by a package manager: updates are theirs.
fn packaged() -> Option<String> {
    option_env!("ARROWHEAD_PACKAGED")
        .map(String::from)
        .or_else(|| {
            std::fs::read_to_string("/usr/lib/arrowhead/packaged")
                .ok()
                .map(|s| s.trim().to_string())
        })
        .filter(|s| !s.is_empty())
}

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

#[cfg(target_os = "linux")]
fn open_source(fresh: bool) -> CmdRes<Src> {
    use arrowhead_capture::pipewire::{PipewireSource, default_token_file};
    let token = default_token_file().ok_or("no data dir for the restore token")?;
    if fresh {
        // Without the restore token the portal shows its picker again.
        let _ = std::fs::remove_file(&token);
    }
    Ok(Box::new(PipewireSource::open(&token).map_err(err)?))
}

#[cfg(not(target_os = "linux"))]
fn open_source(_: bool) -> CmdRes<Src> {
    Err("screen capture on this OS arrives in v0.2".into())
}

/// Runs `f` on a blocking thread with the capture source, opening it if needed.
async fn with_src<T: Send + 'static>(
    state: &AppState,
    f: impl FnOnce(&mut dyn FrameSource) -> CmdRes<T> + Send + 'static,
) -> CmdRes<T> {
    if state.running() {
        return Err("stop the engine first".into());
    }
    let src = state.src.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut g = src.lock().unwrap();
        if g.is_none() {
            *g = Some(open_source(false)?);
        }
        f(g.as_mut().unwrap().as_mut())
    })
    .await
    .map_err(err)?
}

/// Log-only event sink for setup steps: their messages go to the log file.
fn log_tx(logs: &Arc<LogFile>) -> arrowhead_runner::Tx {
    let (tx, rx) = mpsc::sync_channel(1024);
    let logs = logs.clone();
    std::thread::spawn(move || bus::run(rx, None, Arc::default(), logs, None));
    tx
}

#[derive(Serialize)]
struct AppInfo {
    version: &'static str,
    os: &'static str,
    packaged: Option<String>,
}

#[tauri::command]
fn app_info() -> AppInfo {
    AppInfo {
        version: arrowhead_core::VERSION,
        os: std::env::consts::OS,
        packaged: packaged(),
    }
}

#[derive(Serialize)]
struct Status {
    running: bool,
    state: State,
    dry_run: bool,
    paused: bool,
    capture: Option<SourceInfo>,
    profile: String,
    focus: Option<bool>,
    update: Option<UpdateInfo>,
}

#[tauri::command]
fn get_status(state: tauri::State<AppState>) -> Status {
    let running = state.running();
    let live = state.live.lock().unwrap().clone();
    let paused = state
        .engine
        .lock()
        .unwrap()
        .as_ref()
        .is_some_and(|e| e.ctl.paused.load(Ordering::SeqCst));
    // The engine holds the source while it runs.
    let capture = state
        .src
        .try_lock()
        .ok()
        .and_then(|g| g.as_ref().map(|s| s.info()));
    Status {
        running,
        state: if running { live.state } else { State::Idle },
        dry_run: live.dry_run,
        paused,
        capture,
        profile: load_config().map(|c| c.profile).unwrap_or_default(),
        focus: if running { live.focus } else { None },
        update: state.update.lock().unwrap().clone(),
    }
}

#[tauri::command]
fn start_engine(
    app: AppHandle,
    state: tauri::State<AppState>,
    on_event: Channel<Vec<EngineEvent>>,
    dry_run: bool,
) -> CmdRes<()> {
    if state.running() {
        return Err("the engine is already running".into());
    }
    let ctl = Arc::new(Control::default());
    ctl.dry_run.store(dry_run, Ordering::SeqCst);
    *state.live.lock().unwrap() = Live {
        state: State::Searching,
        dry_run,
        focus: None,
    };
    let (tx, rx) = mpsc::sync_channel::<EngineEvent>(4096);
    let (live, logs) = (state.live.clone(), state.logs.clone());
    let cfg = load_config().unwrap_or_default();
    let prof = load_profile().unwrap_or_default();
    let (overlay, overlay_err) = overlay::Overlay::start(
        &app,
        cfg.ui.overlay,
        cfg.ui.minimise_while_playing,
        &prof.monitor,
        prof.stream_size,
    );
    if let Some(e) = overlay_err {
        logs.write(Level::Warn, &format!("overlay not started: {e}"));
    }
    std::thread::Builder::new()
        .name("bus".into())
        .spawn(move || bus::run(rx, Some(on_event), live, logs, Some(overlay)))
        .map_err(err)?;
    let (src, c2, live) = (state.src.clone(), ctl.clone(), state.live.clone());
    let join = std::thread::Builder::new()
        .name("engine".into())
        .spawn(move || {
            let res = (|| -> CmdRes<()> {
                let mut g = src.lock().unwrap();
                if g.is_none() {
                    *g = Some(open_source(false)?);
                }
                // Without a writable uinput only dry runs are possible.
                let live_keys = std::fs::OpenOptions::new()
                    .write(true)
                    .open("/dev/uinput")
                    .is_ok();
                if !live_keys {
                    arrowhead_runner::log(
                        &tx,
                        Level::Warn,
                        "/dev/uinput is not writable: dry run only (see Setup, Permissions)",
                    );
                }
                let o = Opts {
                    live: live_keys,
                    seconds: None,
                    offset_ms: None,
                    learn_s: 8.0,
                    relearn: false,
                };
                let r = arrowhead_runner::run(g.as_mut().unwrap().as_mut(), o, c2, tx.clone())
                    .map(drop)
                    .map_err(err);
                if r.is_err() {
                    // A lost stream is rebuilt on the next start.
                    *g = None;
                }
                r
            })();
            live.lock().unwrap().state = State::Idle;
            arrowhead_runner::emit(
                &tx,
                EngineEvent::State {
                    state: State::Idle,
                    dry_run,
                    reason: "stopped",
                },
            );
            arrowhead_runner::emit(&tx, EngineEvent::Stopped { error: res.err() });
        })
        .map_err(err)?;
    *state.engine.lock().unwrap() = Some(Engine { ctl, join });
    Ok(())
}

#[tauri::command]
async fn stop_engine(state: tauri::State<'_, AppState>) -> CmdRes<()> {
    let e = state.engine.lock().unwrap().take();
    if let Some(e) = e {
        e.ctl.stop.store(true, Ordering::SeqCst);
        tauri::async_runtime::spawn_blocking(move || e.join.join())
            .await
            .map_err(err)?
            .map_err(|_| "engine thread panicked")?;
    }
    Ok(())
}

fn with_ctl(state: &AppState, f: impl FnOnce(&Control)) -> CmdRes<()> {
    match state.engine.lock().unwrap().as_ref() {
        Some(e) => {
            f(&e.ctl);
            Ok(())
        }
        None => Err("the engine is not running".into()),
    }
}

#[tauri::command]
fn set_paused(state: tauri::State<AppState>, paused: bool) -> CmdRes<()> {
    with_ctl(&state, |c| c.paused.store(paused, Ordering::SeqCst))
}

/// Releases every key and stops (spec 05). Never fails: with no engine there is nothing held.
#[tauri::command]
fn panic(state: tauri::State<AppState>) {
    let _ = with_ctl(&state, |c| c.stop.store(true, Ordering::SeqCst));
}

#[tauri::command]
fn set_dry_run(state: tauri::State<AppState>, dry_run: bool) -> CmdRes<()> {
    state.live.lock().unwrap().dry_run = dry_run;
    with_ctl(&state, |c| c.dry_run.store(dry_run, Ordering::SeqCst))
}

fn save_config(cfg: &Config) -> CmdRes<()> {
    let dir = config_dir().ok_or("no config dir")?;
    save_toml(&dir.join("config.toml"), cfg).map_err(err)
}

#[tauri::command]
fn get_config() -> CmdRes<Config> {
    load_config().map_err(err)
}

fn merge(a: &mut serde_json::Value, b: serde_json::Value) {
    match (a, b) {
        (serde_json::Value::Object(a), serde_json::Value::Object(b)) => {
            for (k, v) in b {
                merge(a.entry(k).or_insert(serde_json::Value::Null), v);
            }
        }
        (a, b) => *a = b,
    }
}

#[tauri::command]
fn set_config(state: tauri::State<AppState>, patch: serde_json::Value) -> CmdRes<Config> {
    let mut v = serde_json::to_value(load_config().map_err(err)?).map_err(err)?;
    merge(&mut v, patch);
    let cfg: Config = serde_json::from_value(v).map_err(err)?;
    save_config(&cfg)?;
    *state.logs.min.lock().unwrap() = bus::parse_level(&cfg.log.level);
    Ok(cfg)
}

#[tauri::command]
fn list_profiles() -> Vec<String> {
    let mut v: Vec<String> = config_dir()
        .and_then(|d| std::fs::read_dir(d.join("profiles")).ok())
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            e.path()
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
        })
        .collect();
    v.sort();
    v
}

#[tauri::command]
fn select_profile(name: String) -> CmdRes<()> {
    if name.is_empty() || name.contains(['/', '\\', '.']) {
        return Err("profile names are plain words".into());
    }
    let mut cfg = load_config().map_err(err)?;
    cfg.profile = name;
    save_config(&cfg)
}

fn load_profile() -> CmdRes<Profile> {
    let cfg = load_config().map_err(err)?;
    profile_path(&cfg)
        .map(|p| load_toml(&p))
        .transpose()
        .map_err(err)
        .map(Option::unwrap_or_default)
}

#[tauri::command]
fn get_profile() -> CmdRes<Profile> {
    load_profile()
}

#[derive(Serialize)]
struct Calibration {
    layout: Layout,
    nudge: Nudge,
    png: String,
    colours_learned: bool,
    trail_learned: bool,
}

fn b64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::with_capacity(data.len().div_ceil(3) * 4);
    for c in data.chunks(3) {
        let n = (c[0] as u32) << 16
            | (*c.get(1).unwrap_or(&0) as u32) << 8
            | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            s.push(if i <= c.len() {
                T[(n >> (18 - 6 * i) & 63) as usize] as char
            } else {
                '='
            });
        }
    }
    s
}

/// Half-size PNG of a full-stream BGRx frame, as a data URL (the UI draws the layout over it).
fn png_url(f: &OwnedFrame) -> CmdRes<String> {
    let (w, h) = (f.region.w as usize / 2, f.region.h as usize / 2);
    let stride = f.region.w as usize * 4;
    let mut rgb = Vec::with_capacity(w * h * 3);
    for y in 0..h {
        for x in 0..w {
            let i = 2 * y * stride + 2 * x * 4;
            rgb.extend_from_slice(&[f.data[i + 2], f.data[i + 1], f.data[i]]);
        }
    }
    let mut out = Vec::new();
    let mut enc = png::Encoder::new(&mut out, w as u32, h as u32);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    let mut wr = enc.write_header().map_err(err)?;
    wr.write_image_data(&rgb).map_err(err)?;
    wr.finish().map_err(err)?;
    Ok(format!("data:image/png;base64,{}", b64(&out)))
}

fn calibration_from_profile(png: String) -> CmdRes<Calibration> {
    let prof = load_profile()?;
    let raw = prof
        .layout
        .clone()
        .ok_or("not calibrated yet: run Calibrate with a song playing")?;
    Ok(Calibration {
        layout: prof.nudge.apply(&raw),
        nudge: prof.nudge.clone(),
        png,
        colours_learned: prof.colours.is_some(),
        trail_learned: prof.colours.as_ref().is_some_and(|c| c.trail.is_some()),
    })
}

#[derive(Serialize)]
#[serde(untagged)]
enum StepResult {
    Permissions(arrowhead_runner::platform::Permissions),
    Screen(SourceInfo),
    Calibrate(Calibration),
    Latency(arrowhead_runner::latency::LatencyResult),
}

#[tauri::command]
async fn run_setup_step(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    step: String,
) -> CmdRes<StepResult> {
    match step.as_str() {
        "permissions" => Ok(StepResult::Permissions(
            arrowhead_runner::platform::permissions(&load_config().map_err(err)?),
        )),
        "screen" => {
            if state.running() {
                return Err("stop the engine first".into());
            }
            let src = state.src.clone();
            let info = tauri::async_runtime::spawn_blocking(move || -> CmdRes<SourceInfo> {
                let mut g = src.lock().unwrap();
                *g = None;
                let s = open_source(true)?;
                let info = s.info();
                *g = Some(s);
                Ok(info)
            })
            .await
            .map_err(err)??;
            Ok(StepResult::Screen(info))
        }
        "calibrate" => {
            let tx = log_tx(&state.logs);
            let frame = with_src(&state, move |src| {
                let cal = arrowhead_runner::calibrate(src, 8.0, true, &tx).map_err(err)?;
                match cal.snapshot {
                    Some(f) => Ok(f),
                    None => src.snapshot().map_err(err),
                }
            })
            .await?;
            let png = png_url(&frame)?;
            *state.snapshot.lock().unwrap() = Some(png.clone());
            Ok(StepResult::Calibrate(calibration_from_profile(png)?))
        }
        "latency" => {
            let (ktx, krx) = mpsc::channel();
            *state.latency_tx.lock().unwrap() = Some(ktx);
            let res = with_src(&state, move |src| {
                let paint = |white: bool| {
                    let _ = app.emit("latency-paint", white);
                };
                arrowhead_runner::latency::run(src, 50, &paint, &krx).map_err(err)
            })
            .await;
            *state.latency_tx.lock().unwrap() = None;
            Ok(StepResult::Latency(res?))
        }
        s => Err(format!("unknown setup step {s}")),
    }
}

/// The latency panel saw a key (its keydown handler calls this at once).
#[tauri::command]
fn latency_key(state: tauri::State<AppState>) {
    if let Some(tx) = state.latency_tx.lock().unwrap().as_ref() {
        let _ = tx.send(RealClock.now());
    }
}

/// Keys step: one tap of `lane` after 2 s, so the user can switch to the game first.
#[tauri::command]
async fn test_lane(state: tauri::State<'_, AppState>, lane: u8) -> CmdRes<()> {
    if lane > 3 {
        return Err("lane must be 0-3".into());
    }
    if state.running() {
        return Err("stop the engine first".into());
    }
    tauri::async_runtime::spawn_blocking(move || -> CmdRes<()> {
        let cfg = load_config().map_err(err)?;
        let mut sink =
            arrowhead_runner::live_sink(key_names(cfg.keys.set, &cfg.keys.custom)).map_err(err)?;
        std::thread::sleep(Duration::from_secs(2));
        sink.send_batch(&[KeyAction::Press(lane)]).map_err(err)?;
        std::thread::sleep(Duration::from_millis(cfg.timing.tap_hold_ms as u64));
        let r = sink.send_batch(&[KeyAction::Release(lane)]);
        sink.release_all().map_err(err)?;
        r.map_err(err)
    })
    .await
    .map_err(err)?
}

#[tauri::command]
async fn calibration_snapshot(state: tauri::State<'_, AppState>) -> CmdRes<Calibration> {
    let cached = state.snapshot.lock().unwrap().clone();
    let png = match cached {
        Some(p) => p,
        None => {
            let f = with_src(&state, |src| src.snapshot().map_err(err)).await?;
            let p = png_url(&f)?;
            *state.snapshot.lock().unwrap() = Some(p.clone());
            p
        }
    };
    calibration_from_profile(png)
}

/// Sets the profile's hand correction (absolute, not a delta) and returns the corrected layout.
#[tauri::command]
fn nudge_layout(
    state: tauri::State<AppState>,
    dx: f32,
    dy: f32,
    dpitch: f32,
    dwidth: f32,
) -> CmdRes<Calibration> {
    let cfg = load_config().map_err(err)?;
    let path = profile_path(&cfg).ok_or("no config dir")?;
    let mut prof: Profile = load_toml(&path).map_err(err)?;
    prof.nudge = Nudge {
        dx,
        dy,
        dpitch,
        dwidth,
    };
    save_toml(&path, &prof).map_err(err)?;
    calibration_from_profile(state.snapshot.lock().unwrap().clone().unwrap_or_default())
}

#[tauri::command]
fn copy_diagnostics(state: tauri::State<AppState>) -> String {
    let env = |k: &str| std::env::var(k).unwrap_or_else(|_| "-".into());
    let mut s = format!(
        "arrowhead {} ({})\nos {} {}\ndesktop {} ({})\n",
        arrowhead_core::VERSION,
        packaged().unwrap_or_else(|| "unpackaged".into()),
        std::env::consts::OS,
        std::env::consts::ARCH,
        env("XDG_CURRENT_DESKTOP"),
        env("XDG_SESSION_TYPE"),
    );
    let st = get_status(state.clone());
    s += &match &st.capture {
        Some(c) => format!(
            "capture {} {} {}x{} @ {:.0} Hz scale {}\n",
            c.backend, c.monitor, c.width, c.height, c.refresh_hz, c.scale
        ),
        None if st.running => "capture held by the running engine\n".into(),
        None => "capture not open\n".into(),
    };
    let cfg = load_config().unwrap_or_default();
    let p = arrowhead_runner::platform::permissions(&cfg);
    for (k, c) in [
        ("input", &p.uinput),
        ("hotkeys", &p.hotkeys),
        ("screencast", &p.screencast),
        ("game", &p.game),
    ] {
        s += &format!(
            "{k} {}: {}\n",
            if c.ok { "ok" } else { "missing" },
            c.detail
        );
    }
    s += &format!(
        "focus probe {}\nengine {:?} dry_run {}\n",
        if env("XDG_CURRENT_DESKTOP").contains("KDE") {
            "kwin"
        } else {
            "none (fail open)"
        },
        st.state,
        st.dry_run
    );
    match load_profile() {
        Ok(pr) => {
            s += &format!(
                "profile {}: monitor {} stream {:?} {} Hz scale {} scroll {:?} radius {:.1} nudge {:?} latency {:?} colours {} trail {}\n",
                cfg.profile,
                pr.monitor,
                pr.stream_size,
                pr.refresh_hz,
                pr.scale,
                pr.scroll,
                pr.radius_px,
                pr.nudge,
                pr.latency,
                pr.colours.is_some(),
                pr.colours.as_ref().is_some_and(|c| c.trail.is_some())
            )
        }
        Err(e) => s += &format!("profile {}: {e}\n", cfg.profile),
    }
    s += "\nlast warnings:\n";
    for l in state.logs.recent_warnings(200) {
        s += &l;
        s.push('\n');
    }
    if let Some(h) = data_dir().and_then(|d| std::fs::read_to_string(d.join("history.jsonl")).ok())
    {
        s += "\nlast history:\n";
        let lines: Vec<&str> = h.lines().collect();
        for l in &lines[lines.len().saturating_sub(5)..] {
            s += l;
            s.push('\n');
        }
    }
    s
}

#[tauri::command]
fn open_logs_folder(state: tauri::State<AppState>) -> CmdRes<()> {
    let dir = state.logs.dir.clone().ok_or("no data dir")?;
    std::fs::create_dir_all(&dir).map_err(err)?;
    let opener = if cfg!(windows) {
        "explorer"
    } else {
        "xdg-open"
    };
    std::process::Command::new(opener)
        .arg(&dir)
        .spawn()
        .map_err(err)?;
    Ok(())
}

fn can_install() -> bool {
    // The updater replaces AppImages (and Windows installers, v0.2); the Linux tarball and
    // package-manager installs only get the notice.
    packaged().is_none() && (cfg!(windows) || std::env::var_os("APPIMAGE").is_some())
}

async fn check(app: &AppHandle) -> CmdRes<Option<UpdateInfo>> {
    let u = app.updater().map_err(err)?.check().await.map_err(err)?;
    let info = u.map(|u| UpdateInfo {
        version: u.version,
        notes: u.body,
        can_install: can_install(),
    });
    *app.state::<AppState>().update.lock().unwrap() = info.clone();
    Ok(info)
}

#[tauri::command]
async fn check_update(app: AppHandle) -> CmdRes<Option<UpdateInfo>> {
    check(&app).await
}

#[tauri::command]
async fn install_update(app: AppHandle, state: tauri::State<'_, AppState>) -> CmdRes<()> {
    if let Some(p) = packaged() {
        return Err(format!(
            "installed by {p}: update with your package manager"
        ));
    }
    if !can_install() {
        return Err(
            "this install cannot update itself: download the new release from GitHub".into(),
        );
    }
    if state.running() && state.live.lock().unwrap().state == State::Playing {
        return Err("not while playing".into());
    }
    let u = app
        .updater()
        .map_err(err)?
        .check()
        .await
        .map_err(err)?
        .ok_or("already up to date")?;
    state.stop_engine();
    u.download_and_install(|_, _| {}, || {})
        .await
        .map_err(err)?;
    app.restart();
}

fn main() {
    // A panic anywhere sets EXIT, which stops the engine and releases keys (S2); so do signals.
    install_exit_hooks();
    let cfg = load_config().unwrap_or_default();
    let logs = Arc::new(LogFile::new(&cfg.log.level));
    logs.write(
        Level::Info,
        &format!("arrowhead {} starting", arrowhead_core::VERSION),
    );
    tauri::Builder::default()
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(AppState {
            src: Arc::default(),
            engine: Mutex::default(),
            live: Arc::default(),
            logs,
            latency_tx: Mutex::default(),
            snapshot: Mutex::default(),
            update: Mutex::default(),
        })
        .setup(move |app| {
            let h = app.handle().clone();
            std::thread::spawn(move || {
                loop {
                    if EXIT.load(Ordering::SeqCst) {
                        h.state::<AppState>().stop_engine();
                        std::process::exit(130);
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
            });
            if cfg.updates.check {
                // The only network call (spec 10): on launch and every 24 h.
                let h = app.handle().clone();
                std::thread::spawn(move || {
                    loop {
                        if let Err(e) = tauri::async_runtime::block_on(check(&h)) {
                            h.state::<AppState>()
                                .logs
                                .write(Level::Warn, &format!("update check failed: {e}"));
                        }
                        std::thread::sleep(Duration::from_secs(24 * 3600));
                    }
                });
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            app_info,
            get_status,
            start_engine,
            stop_engine,
            set_paused,
            panic,
            set_dry_run,
            get_config,
            set_config,
            list_profiles,
            select_profile,
            get_profile,
            run_setup_step,
            latency_key,
            test_lane,
            calibration_snapshot,
            nudge_layout,
            copy_diagnostics,
            open_logs_folder,
            check_update,
            install_update,
        ])
        .build(tauri::generate_context!())
        .expect("failed to start arrowhead")
        .run(|app, ev| {
            if let tauri::RunEvent::Exit = ev {
                // Every press has a release, also when the window closes mid-song.
                app.state::<AppState>().stop_engine();
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64() {
        assert_eq!(b64(b""), "");
        assert_eq!(b64(b"f"), "Zg==");
        assert_eq!(b64(b"fo"), "Zm8=");
        assert_eq!(b64(b"foo"), "Zm9v");
        assert_eq!(b64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn config_merge() {
        let mut a = serde_json::json!({"keys": {"set": "wasd", "custom": ["a"]}, "profile": "x"});
        merge(&mut a, serde_json::json!({"keys": {"set": "arrows"}}));
        assert_eq!(
            a,
            serde_json::json!({"keys": {"set": "arrows", "custom": ["a"]}, "profile": "x"})
        );
    }
}
