//! OS pieces of a run: key sink, hotkeys, focus probe, permission checks (spec 05, 07).

use std::sync::Arc;
use std::sync::atomic::AtomicBool;
#[cfg(any(target_os = "linux", windows))]
use std::sync::atomic::Ordering;
use std::sync::mpsc;
#[cfg(any(target_os = "linux", windows))]
use std::time::Duration;

use arrowhead_core::config::Config;
use arrowhead_core::event::Level;
use arrowhead_core::input::{Hotkey, KeySink};
use serde::Serialize;

use crate::{Res, Tx, log};

#[cfg(target_os = "linux")]
pub fn live_sink(keys: [&str; 4]) -> Res<Box<dyn KeySink>> {
    Ok(Box::new(arrowhead_input::uinput::UinputSink::new(keys)?))
}

#[cfg(windows)]
pub fn live_sink(keys: [&str; 4]) -> Res<Box<dyn KeySink>> {
    Ok(Box::new(arrowhead_input::winsend::SendInputSink::new(
        keys,
    )?))
}

#[cfg(not(any(target_os = "linux", windows)))]
pub fn live_sink(_: [&str; 4]) -> Res<Box<dyn KeySink>> {
    Err("live input is not available on this OS".into())
}

#[cfg(target_os = "linux")]
pub type HotkeyGuard = Option<arrowhead_input::hotkeys::PortalHotkeys>;
#[cfg(windows)]
pub type HotkeyGuard = Option<arrowhead_input::winhotkeys::WinHotkeys>;
#[cfg(not(any(target_os = "linux", windows)))]
pub type HotkeyGuard = ();

/// Global hotkeys (spec 05). Returns the guard and a receiver for the one bind result.
#[cfg(target_os = "linux")]
pub fn start_hotkeys(
    cfg: &Config,
    tx: mpsc::Sender<Hotkey>,
    ev: &Tx,
) -> (HotkeyGuard, mpsc::Receiver<Result<String, String>>) {
    log(
        ev,
        Level::Info,
        format!(
            "binding hotkeys (toggle {}, panic {}, nudge {}/{}); the desktop may ask you to confirm",
            cfg.hotkeys.toggle, cfg.hotkeys.panic, cfg.hotkeys.nudge_up, cfg.hotkeys.nudge_down
        ),
    );
    match arrowhead_input::hotkeys::PortalHotkeys::start(&cfg.hotkeys, tx) {
        Ok((h, rx)) => (Some(h), rx),
        Err(e) => {
            let (etx, rx) = mpsc::channel();
            let _ = etx.send(Err(e.to_string()));
            (None, rx)
        }
    }
}

#[cfg(windows)]
pub fn start_hotkeys(
    cfg: &Config,
    tx: mpsc::Sender<Hotkey>,
    ev: &Tx,
) -> (HotkeyGuard, mpsc::Receiver<Result<String, String>>) {
    log(
        ev,
        Level::Info,
        format!(
            "registering hotkeys (toggle {}, panic {}, nudge {}/{})",
            cfg.hotkeys.toggle, cfg.hotkeys.panic, cfg.hotkeys.nudge_up, cfg.hotkeys.nudge_down
        ),
    );
    match arrowhead_input::winhotkeys::WinHotkeys::start(&cfg.hotkeys, tx) {
        Ok((h, rx)) => (Some(h), rx),
        Err(e) => {
            let (etx, rx) = mpsc::channel();
            let _ = etx.send(Err(e.to_string()));
            (None, rx)
        }
    }
}

#[cfg(not(any(target_os = "linux", windows)))]
pub fn start_hotkeys(
    _: &Config,
    _: mpsc::Sender<Hotkey>,
    _: &Tx,
) -> (HotkeyGuard, mpsc::Receiver<Result<String, String>>) {
    let (etx, rx) = mpsc::channel();
    let _ = etx.send(Err("no hotkeys on this OS yet".into()));
    ((), rx)
}

/// Polls `probe` at 10 Hz into `focus` until `stop`, or applies the require-guard policy when
/// the probe could not start. Returns whether a probe runs.
#[cfg(any(target_os = "linux", windows))]
fn run_focus<P: arrowhead_core::input::FocusProbe + 'static>(
    probe: Result<P, arrowhead_core::input::InputError>,
    cfg: &Config,
    focus: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    ev: &Tx,
) -> Res<bool> {
    match probe {
        Ok(mut probe) => {
            std::thread::Builder::new()
                .name("focus".into())
                .spawn(move || {
                    while !stop.load(Ordering::SeqCst) {
                        // No report yet counts as focused (fail open); a report decides.
                        focus.store(probe.game_focused().unwrap_or(true), Ordering::SeqCst);
                        std::thread::sleep(Duration::from_millis(100));
                    }
                })?;
            Ok(true)
        }
        Err(e) if cfg.safety.require_focus_guard => {
            Err(format!("{e} (safety.require_focus_guard is on)").into())
        }
        Err(e) => {
            log(
                ev,
                Level::Warn,
                format!("focus guard off ({e}); keys go to whatever window is focused"),
            );
            Ok(false)
        }
    }
}

/// Polls the focus probe at 10 Hz into `focus` until `stop`. Returns whether a probe runs.
#[cfg(target_os = "linux")]
pub fn start_focus(
    cfg: &Config,
    focus: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    ev: &Tx,
) -> Res<bool> {
    let probe = arrowhead_input::kwin::KwinProbe::start(cfg.safety.game_window_classes.clone());
    run_focus(probe, cfg, focus, stop, ev)
}

#[cfg(windows)]
pub fn start_focus(
    cfg: &Config,
    focus: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    ev: &Tx,
) -> Res<bool> {
    let probe = Ok(arrowhead_input::winfocus::WindowsProbe::new(
        cfg.safety.game_window_classes.clone(),
    ));
    run_focus(probe, cfg, focus, stop, ev)
}

#[cfg(not(any(target_os = "linux", windows)))]
pub fn start_focus(cfg: &Config, _: Arc<AtomicBool>, _: Arc<AtomicBool>, ev: &Tx) -> Res<bool> {
    if cfg.safety.require_focus_guard {
        return Err("no focus probe on this OS yet (safety.require_focus_guard is on)".into());
    }
    log(ev, Level::Warn, "focus guard off (no probe on this OS yet)");
    Ok(false)
}

#[derive(Clone, Debug, Serialize)]
pub struct Check {
    pub ok: bool,
    pub detail: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct Permissions {
    pub uinput: Check,
    pub hotkeys: Check,
    pub screencast: Check,
    pub game: Check,
}

/// Portal interface version via busctl, or why it could not be read.
#[cfg(target_os = "linux")]
fn portal_version(iface: &str) -> Result<String, String> {
    let out = std::process::Command::new("busctl")
        .args([
            "--user",
            "get-property",
            "org.freedesktop.portal.Desktop",
            "/org/freedesktop/portal/desktop",
            &format!("org.freedesktop.portal.{iface}"),
            "version",
        ])
        .output()
        .map_err(|e| format!("could not run busctl: {e}"))?;
    if out.status.success() {
        // "u 5"
        Ok(String::from_utf8_lossy(&out.stdout)
            .trim()
            .trim_start_matches("u ")
            .to_string())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

/// Setup step 1 (spec 07): uinput, hotkey backend, ScreenCast portal, game window.
#[cfg(target_os = "linux")]
pub fn permissions(cfg: &Config) -> Permissions {
    let uinput = match std::fs::OpenOptions::new().write(true).open("/dev/uinput") {
        Ok(_) => Check {
            ok: true,
            detail: "/dev/uinput is writable".into(),
        },
        Err(e) => Check {
            ok: false,
            detail: format!(
                "/dev/uinput: {e}. Add a udev rule giving your user access (uaccess), or run: sudo modprobe uinput"
            ),
        },
    };
    let portal = |iface: &str, what: &str| match portal_version(iface) {
        Ok(v) => Check {
            ok: true,
            detail: format!("xdg-desktop-portal {iface} v{v} found"),
        },
        Err(e) => Check {
            ok: false,
            detail: format!("{what}: {iface} portal not found ({e})"),
        },
    };
    let screencast = portal(
        "ScreenCast",
        "Screen capture needs xdg-desktop-portal with a kde or hyprland backend",
    );
    let mut hotkeys = portal(
        "GlobalShortcuts",
        "Hotkeys are optional; use the app's buttons instead",
    );
    if hotkeys.ok {
        hotkeys
            .detail
            .push_str(". The desktop asks to confirm the keys on first play");
    }
    let classes: Vec<String> = cfg
        .safety
        .game_window_classes
        .iter()
        .map(|c| c.to_lowercase())
        .collect();
    let running = std::fs::read_dir("/proc")
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| std::fs::read_to_string(e.path().join("comm")).ok())
        .map(|c| c.trim().to_lowercase())
        .find(|c| {
            c.contains("sober")
                || c.contains("roblox")
                || classes.iter().any(|k| k.starts_with(c.as_str()))
        });
    let game = match running {
        Some(c) => Check {
            ok: true,
            detail: format!("game process running ({c})"),
        },
        None => Check {
            ok: false,
            detail: "Sober is not running. Start the game, then check again".into(),
        },
    };
    Permissions {
        uinput,
        hotkeys,
        screencast,
        game,
    }
}

/// Setup step 1 on Windows: nothing to grant. SendInput and RegisterHotKey need no setup, but an
/// elevated game ignores keys from a normal process, so the game check says so.
#[cfg(windows)]
pub fn permissions(cfg: &Config) -> Permissions {
    let yes = |d: &str| Check {
        ok: true,
        detail: d.into(),
    };
    let game = match arrowhead_input::winfocus::running_game(&cfg.safety.game_window_classes) {
        Some(n) => yes(&format!(
            "game process running ({n}). If keys do nothing, the game may be running as administrator: run arrowhead the same way"
        )),
        None => Check {
            ok: false,
            detail: "Roblox is not running. Start the game, then check again".into(),
        },
    };
    Permissions {
        uinput: yes("key output uses SendInput (nothing to set up)"),
        hotkeys: yes("hotkeys use RegisterHotKey (a key another program owns is skipped)"),
        screencast: yes(&format!(
            "capture backend: {:?} (DXGI first, Windows Graphics Capture as fallback)",
            cfg.capture.windows_backend
        )),
        game,
    }
}

#[cfg(not(any(target_os = "linux", windows)))]
pub fn permissions(_: &Config) -> Permissions {
    let no = |d: &str| Check {
        ok: false,
        detail: d.into(),
    };
    Permissions {
        uinput: no("key output on this OS arrives in v0.2"),
        hotkeys: no("hotkeys on this OS arrive in v0.2"),
        screencast: no("capture on this OS arrives in v0.2"),
        game: no("not checked on this OS yet"),
    }
}
