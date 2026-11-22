//! KWin (Plasma 6) focus probe (spec 05): a generated KWin script reports every window
//! activation to a DBus object we serve, so no polling of KWin is needed.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use arrowhead_core::input::{FocusProbe, InputError};

const SERVICE: &str = "org.arrowhead.Focus";
const SCRIPT_NAME: &str = "arrowhead-focus";

#[derive(Default)]
struct Shared {
    focused: AtomicBool,
    seen: AtomicBool,
    stop: AtomicBool,
    class: Mutex<String>,
    classes: Vec<String>,
}

struct Focus(Arc<Shared>);

#[zbus::interface(name = "org.arrowhead.Focus")]
impl Focus {
    fn active(&self, class: String, _caption: String) {
        let hit = self
            .0
            .classes
            .iter()
            .any(|c| c.eq_ignore_ascii_case(&class));
        self.0.focused.store(hit, Ordering::SeqCst);
        self.0.seen.store(true, Ordering::SeqCst);
        *self.0.class.lock().unwrap() = class;
    }
}

fn script() -> String {
    format!(
        "function report(w) {{\n  callDBus(\"{SERVICE}\", \"/Focus\", \"{SERVICE}\", \"Active\", \
         w ? String(w.resourceClass) : \"\", w ? String(w.caption) : \"\");\n}}\n\
         workspace.windowActivated.connect(report);\nreport(workspace.activeWindow);\n"
    )
}

async fn scripting(
    conn: &zbus::Connection,
    method: &str,
    body: &(impl serde::Serialize + zbus::zvariant::DynamicType),
) -> zbus::Result<zbus::Message> {
    conn.call_method(
        Some("org.kde.KWin"),
        "/Scripting",
        Some("org.kde.kwin.Scripting"),
        method,
        body,
    )
    .await
}

pub struct KwinProbe {
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
}

impl KwinProbe {
    /// Serves the callback object, loads and starts the script. `classes`: game window classes.
    pub fn start(classes: Vec<String>) -> Result<Self, InputError> {
        let shared = Arc::new(Shared {
            classes,
            ..Default::default()
        });
        let path = std::env::temp_dir().join("arrowhead-kwin-focus.js");
        std::fs::write(&path, script()).map_err(|e| InputError(format!("kwin script: {e}")))?;
        let (tx, rx) = std::sync::mpsc::channel::<Result<(), String>>();
        let sh = shared.clone();
        let thread = std::thread::Builder::new()
            .name("focus-kwin".into())
            .spawn(move || {
                let rt = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(rt) => rt,
                    Err(e) => return drop(tx.send(Err(e.to_string()))),
                };
                rt.block_on(async move {
                    let setup = async {
                        let conn = zbus::connection::Builder::session()
                            .name(SERVICE)
                            .serve_at("/Focus", Focus(sh.clone()))
                            .build()
                            .await?;
                        // A stale copy from a crashed run would keep its old callback.
                        let _ = scripting(&conn, "unloadScript", &(SCRIPT_NAME,)).await;
                        scripting(
                            &conn,
                            "loadScript",
                            &(path.to_string_lossy().as_ref(), SCRIPT_NAME),
                        )
                        .await?;
                        scripting(&conn, "start", &()).await?;
                        zbus::Result::Ok(conn)
                    };
                    let conn = match setup.await {
                        Ok(c) => {
                            let _ = tx.send(Ok(()));
                            c
                        }
                        Err(e) => return drop(tx.send(Err(e.to_string()))),
                    };
                    while !sh.stop.load(Ordering::SeqCst) {
                        tokio::time::sleep(Duration::from_millis(100)).await;
                    }
                    let _ = scripting(&conn, "unloadScript", &(SCRIPT_NAME,)).await;
                });
            })
            .map_err(|e| InputError(e.to_string()))?;
        match rx.recv_timeout(Duration::from_secs(3)) {
            Ok(Ok(())) => Ok(Self {
                shared,
                thread: Some(thread),
            }),
            Ok(Err(e)) => Err(InputError(format!("kwin focus probe: {e}"))),
            Err(_) => Err(InputError(
                "kwin focus probe: no answer from the session bus".into(),
            )),
        }
    }

    /// Class of the active window as KWin reported it (empty until the first report).
    pub fn active_class(&self) -> String {
        self.shared.class.lock().unwrap().clone()
    }
}

impl FocusProbe for KwinProbe {
    fn game_focused(&mut self) -> Result<bool, InputError> {
        if !self.shared.seen.load(Ordering::SeqCst) {
            return Err(InputError("kwin focus probe: no report yet".into()));
        }
        Ok(self.shared.focused.load(Ordering::SeqCst))
    }

    fn name(&self) -> &'static str {
        "kwin-script"
    }
}

impl Drop for KwinProbe {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}
