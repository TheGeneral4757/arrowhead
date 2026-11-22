//! Global hotkeys through the xdg-desktop-portal GlobalShortcuts interface (spec 05).
//! The desktop may ask the user to confirm the bindings the first time.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::time::Duration;

use arrowhead_core::config::Hotkeys;
use arrowhead_core::input::{Hotkey, InputError};
use ashpd::desktop::CreateSessionOptions;
use ashpd::desktop::global_shortcuts::{BindShortcutsOptions, GlobalShortcuts, NewShortcut};
use futures_util::StreamExt;

pub const APP_ID: &str = "org.arrowhead.Arrowhead";

fn action(id: &str) -> Option<Hotkey> {
    Some(match id {
        "toggle" => Hotkey::Toggle,
        "panic" => Hotkey::Panic,
        "nudge-up" => Hotkey::NudgeUp,
        "nudge-down" => Hotkey::NudgeDown,
        "cycle-style" => Hotkey::CycleStyle,
        _ => return None,
    })
}

pub struct PortalHotkeys {
    stop: Arc<AtomicBool>,
}

impl PortalHotkeys {
    /// Binds the hotkeys in the background and sends each activation to `tx`. Bind results
    /// (or the error) arrive as one message on the returned receiver.
    pub fn start(
        keys: &Hotkeys,
        tx: mpsc::Sender<Hotkey>,
    ) -> Result<(Self, mpsc::Receiver<Result<String, String>>), InputError> {
        let shortcuts = [
            ("toggle", "Pause / resume", &keys.toggle),
            ("panic", "Panic: release all keys and stop", &keys.panic),
            ("nudge-up", "Accuracy nudge up", &keys.nudge_up),
            ("nudge-down", "Accuracy nudge down", &keys.nudge_down),
            ("cycle-style", "Cycle preset", &keys.cycle_style),
        ]
        .map(|(id, d, k)| NewShortcut::new(id, d).preferred_trigger(k.as_str()));
        let stop = Arc::new(AtomicBool::new(false));
        let s2 = stop.clone();
        let (btx, brx) = mpsc::channel();
        std::thread::Builder::new()
            .name("hotkeys".into())
            .spawn(move || {
                let rt = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(rt) => rt,
                    Err(e) => return drop(btx.send(Err(e.to_string()))),
                };
                rt.block_on(async move {
                    let setup = async {
                        let conn = zbus::Connection::session().await?;
                        // Host apps need an app id for the portal to store bindings; older portals lack the registry.
                        if let Ok(id) = APP_ID.try_into() {
                            let _ =
                                ashpd::register_host_app_with_connection(conn.clone(), id).await;
                        }
                        let gs = GlobalShortcuts::with_connection(conn).await?;
                        let session = gs.create_session(CreateSessionOptions::default()).await?;
                        let bound = gs
                            .bind_shortcuts(
                                &session,
                                &shortcuts,
                                None,
                                BindShortcutsOptions::default(),
                            )
                            .await?
                            .response()?;
                        let list = bound
                            .shortcuts()
                            .iter()
                            .map(|s| format!("{} = {}", s.id(), s.trigger_description()))
                            .collect::<Vec<_>>()
                            .join(", ");
                        ashpd::Result::Ok((gs, session, list))
                    };
                    let (gs, _session, list) = match setup.await {
                        Ok(v) => v,
                        Err(e) => return drop(btx.send(Err(e.to_string()))),
                    };
                    let mut events = match gs.receive_activated().await {
                        Ok(s) => s,
                        Err(e) => return drop(btx.send(Err(e.to_string()))),
                    };
                    let _ = btx.send(Ok(list));
                    while !s2.load(Ordering::SeqCst) {
                        if let Ok(Some(a)) =
                            tokio::time::timeout(Duration::from_millis(100), events.next()).await
                            && let Some(h) = action(a.shortcut_id())
                            && tx.send(h).is_err()
                        {
                            return;
                        }
                    }
                });
            })
            .map_err(|e| InputError(e.to_string()))?;
        Ok((Self { stop }, brx))
    }
}

impl Drop for PortalHotkeys {
    // Not joined: the thread may still be waiting on the desktop's confirm dialog.
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}
