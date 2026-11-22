import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

const PAGES = [
  { id: "live", label: "Live", note: "Lane visualiser, judgments and pipeline metrics arrive in M5." },
  { id: "style", label: "Style", note: "v0.1 plays the Max preset only. Presets and the player model arrive in v0.2." },
  { id: "setup", label: "Setup", note: "Permissions, Screen, Calibrate, Latency and Keys steps arrive in M5." },
  { id: "history", label: "History", note: "Per-song history arrives in v0.2." },
  { id: "settings", label: "Settings", note: "Hotkeys, key set, safety, capture backend and logs arrive in M5." },
] as const;

type AppInfo = { version: string; os: string };

export default function App() {
  const [page, setPage] = useState(0);
  const [info, setInfo] = useState<AppInfo | null>(null);

  useEffect(() => {
    invoke<AppInfo>("app_info").then(setInfo).catch(() => setInfo(null));
  }, []);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.target instanceof HTMLInputElement || e.ctrlKey || e.altKey || e.metaKey) return;
      const n = Number(e.key);
      if (n >= 1 && n <= PAGES.length) setPage(n - 1);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  const current = PAGES[page];

  return (
    <div className="app">
      <header className="app-top">
        <span className="brand">
          <svg viewBox="0 0 24 24" aria-hidden="true">
            <path d="M12 3 20 11h-5v9H9v-9H4Z" fill="none" strokeWidth="1.8" strokeLinejoin="round" className="ink-stroke" />
            <path d="M2 14.5h20" strokeWidth="1.8" className="acc-stroke" />
          </svg>
          arrowhead
        </span>
        <span className="spacer" />
        <div className="status">
          <span className="pill">Idle</span>
          <span className="pill">No capture</span>
          <span className="pill">Focus unknown</span>
        </div>
      </header>
      <div className="app-body">
        <nav className="side" aria-label="App pages">
          {PAGES.map((p, i) => (
            <button key={p.id} aria-current={i === page ? "page" : undefined} onClick={() => setPage(i)}>
              {p.label} <span className="key">{i + 1}</span>
            </button>
          ))}
          <div className="side-foot xs muted">
            <span>{info ? `v${info.version}, ${info.os}` : "version unknown"}</span>
            <span>No profile</span>
          </div>
        </nav>
        <main className="main">
          <section className="screen-head" aria-labelledby="page-title">
            <h2 id="page-title">{current.label}</h2>
            <p>{current.note}</p>
          </section>
        </main>
      </div>
    </div>
  );
}
