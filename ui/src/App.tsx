import { useEffect, useState } from "react";
import { api, type AppInfo, type Status } from "./api";
import { engine, useEngine } from "./engine";
import History from "./pages/History";
import Live from "./pages/Live";
import Settings from "./pages/Settings";
import Setup from "./pages/Setup";
import Style from "./pages/Style";

const PAGES = ["Live", "Style", "Setup", "History", "Settings"] as const;

const STATE_PILL: Record<string, string> = { Playing: "live", Armed: "armed", Searching: "cool", Paused: "armed" };

export default function App() {
  const [page, setPage] = useState(0);
  const [info, setInfo] = useState<AppInfo | null>(null);
  const [status, setStatus] = useState<Status | null>(null);
  const s = useEngine();

  useEffect(() => {
    api.appInfo().then(setInfo, () => setInfo(null));
    api.getConfig().then((c) => engine.setDryRun(c.safety.dry_run), () => {});
  }, []);

  useEffect(() => {
    const poll = () =>
      api.getStatus().then((st) => {
        setStatus(st);
        engine.sync(st.running, st.paused);
      }, () => {});
    void poll();
    const id = setInterval(poll, 2000);
    return () => clearInterval(id);
  }, []);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.target instanceof HTMLInputElement || e.target instanceof HTMLSelectElement || e.ctrlKey || e.altKey || e.metaKey) return;
      const n = Number(e.key);
      if (n >= 1 && n <= PAGES.length) setPage(n - 1);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  const shownState = s.running ? (s.paused ? "Paused" : s.state) : "Idle";
  const focus = s.running ? s.focus : null;
  const cap = status?.capture;

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
        <div className="status" role="status">
          {status?.update && (
            <button className="pill armed pill-btn" onClick={() => setPage(4)}>
              Update {status.update.version}
            </button>
          )}
          <span className={`pill ${STATE_PILL[shownState] ?? ""}`}>{shownState}</span>
          {s.dryRun && <span className="pill armed">Dry run</span>}
          <span className={`pill ${cap ? "cool" : ""}`}>{cap ? `${cap.backend}, ${cap.monitor || "monitor"}` : s.running ? "Capturing" : "No capture"}</span>
          <span className={`pill ${focus === false ? "bad" : ""}`}>{focus === null ? "Focus unknown" : focus ? "Game focused" : "Game not focused"}</span>
        </div>
      </header>
      <div className="app-body">
        <nav className="side" aria-label="App pages">
          {PAGES.map((p, i) => (
            <button key={p} aria-current={i === page ? "page" : undefined} onClick={() => setPage(i)}>
              {p} <span className="key">{i + 1}</span>
            </button>
          ))}
          <div className="side-foot xs muted">
            <span>{info ? `v${info.version}, ${info.os}` : "version unknown"}</span>
            <span>Profile: {status?.profile || "none"}</span>
          </div>
        </nav>
        <main className="main">
          {page === 0 && <Live active />}
          {page === 1 && <Style />}
          {page === 2 && <Setup />}
          {page === 3 && <History />}
          {page === 4 && <Settings update={status?.update ?? null} />}
        </main>
      </div>
    </div>
  );
}
