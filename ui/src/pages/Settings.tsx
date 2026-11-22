import { useEffect, useState } from "react";
import { api, type Config, type UpdateInfo } from "../api";
import { useEngine } from "../engine";

type Patch = Parameters<typeof api.setConfig>[0];

function Switch({ label, checked, onChange }: { label: string; checked: boolean; onChange: (v: boolean) => void }) {
  return (
    <label className="switch">
      <input type="checkbox" checked={checked} aria-label={label} onChange={(e) => onChange(e.target.checked)} />
      <span />
    </label>
  );
}

function Row({ title, note, children }: { title: string; note?: string; children: React.ReactNode }) {
  return (
    <div className="row">
      <div className="row-text">
        <span className="small">{title}</span>
        {note && <p>{note}</p>}
      </div>
      {children}
    </div>
  );
}

/** Text or number input that saves on blur or Enter. */
function Field({ label, value, onSave, type = "text", mono }: { label: string; value: string | number; onSave: (v: string) => void; type?: string; mono?: boolean }) {
  return (
    <input
      className={`input${mono ? " mono" : ""}`}
      aria-label={label}
      type={type}
      defaultValue={value}
      key={String(value)}
      onBlur={(e) => e.target.value !== String(value) && onSave(e.target.value)}
      onKeyDown={(e) => e.key === "Enter" && (e.target as HTMLInputElement).blur()}
    />
  );
}

const HOTKEYS: [keyof Config["hotkeys"], string][] = [
  ["toggle", "Pause or resume"],
  ["panic", "Panic, release all keys and stop"],
  ["nudge_up", "Offset +1 ms"],
  ["nudge_down", "Offset -1 ms"],
  ["cycle_style", "Next preset (v0.2)"],
];

export default function Settings({ update }: { update: UpdateInfo | null }) {
  const [cfg, setCfg] = useState<Config | null>(null);
  const [msg, setMsg] = useState<string | null>(null);
  const [upd, setUpd] = useState<UpdateInfo | null | undefined>(update ?? undefined);
  const [busy, setBusy] = useState(false);
  const { state } = useEngine();
  useEffect(() => {
    api.getConfig().then(setCfg, (e) => setMsg(String(e)));
  }, []);
  if (!cfg) return msg ? <p className="error-line">{msg}</p> : null;

  const save = async (p: Patch) => {
    try {
      setCfg(await api.setConfig(p));
      setMsg("Saved");
    } catch (e) {
      setMsg(String(e));
    }
  };
  const num = (v: string) => Number(v) || 0;
  const act = async (f: () => Promise<unknown>) => {
    setBusy(true);
    try {
      await f();
    } catch (e) {
      setMsg(String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <section className="screen" aria-labelledby="h-settings">
      <div className="screen-head">
        <div>
          <h2 id="h-settings">Settings</h2>
          <p>
            Saved to <span className="mono">~/.config/arrowhead/config.toml</span> as you change them. Hotkey and key changes apply on the next
            Start.
          </p>
        </div>
        <span className="note" role="status">
          {msg}
        </span>
      </div>
      <div className="settings-grid">
        <div className="panel">
          <div className="panel-head">
            <h3>Play</h3>
          </div>
          <div className="panel-body set-list">
            <Row title="Start in dry run" note="Detects and times everything, presses nothing.">
              <Switch label="Start in dry run" checked={cfg.safety.dry_run} onChange={(v) => save({ safety: { dry_run: v } })} />
            </Row>
            <Row title="Require the focus guard" note="Refuse to play when the focused window cannot be checked.">
              <Switch label="Require the focus guard" checked={cfg.safety.require_focus_guard} onChange={(v) => save({ safety: { require_focus_guard: v } })} />
            </Row>
            <Row title="Longest hold, ms" note="The watchdog releases any key held longer.">
              <Field label="Longest hold in ms" type="number" mono value={cfg.safety.max_hold_ms} onSave={(v) => save({ safety: { max_hold_ms: num(v) } })} />
            </Row>
            <Row title="Game window classes" note="Comma separated. Keys are only sent while one of these is focused.">
              <Field
                label="Game window classes"
                mono
                value={cfg.safety.game_window_classes.join(", ")}
                onSave={(v) => save({ safety: { game_window_classes: v.split(",").map((x) => x.trim()).filter(Boolean) } })}
              />
            </Row>
          </div>
        </div>
        <div className="panel">
          <div className="panel-head">
            <h3>Hotkeys</h3>
          </div>
          <div className="panel-body hotkeys">
            {HOTKEYS.map(([k, label]) => (
              <label key={k} className="hotkey">
                <span className="small">{label}</span>
                <Field label={label} mono value={cfg.hotkeys[k]} onSave={(v) => save({ hotkeys: { [k]: v.trim() } })} />
              </label>
            ))}
          </div>
        </div>
        <div className="panel">
          <div className="panel-head">
            <h3>Keys and timing</h3>
          </div>
          <div className="panel-body set-list">
            <Row title="Key set" note="Custom keys are set in Setup, Keys.">
              <select aria-label="Key set" value={cfg.keys.set} onChange={(e) => save({ keys: { set: e.target.value as Config["keys"]["set"] } })}>
                <option value="wasd">A S W D</option>
                <option value="arrows">Arrow keys</option>
                <option value="custom">Custom</option>
              </select>
            </Row>
            {(
              [
                ["commit_ms", "Commit ahead, ms", "How far ahead a press is fixed."],
                ["late_limit_ms", "Late limit, ms", "Notes found later than this are left."],
                ["tap_hold_ms", "Tap length, ms", "How long a tap holds its key."],
                ["min_gap_ms", "Gap in a lane, ms", "Up time before the next press in a lane."],
              ] as const
            ).map(([k, t, n]) => (
              <Row key={k} title={t} note={n}>
                <Field label={t} type="number" mono value={cfg.timing[k]} onSave={(v) => save({ timing: { [k]: num(v) } })} />
              </Row>
            ))}
          </div>
        </div>
        <div className="panel">
          <div className="panel-head">
            <h3>App</h3>
          </div>
          <div className="panel-body set-list">
            <Row title="Log level" note="Daily files in the data folder, kept for 7 days.">
              <select aria-label="Log level" value={cfg.log.level} onChange={(e) => save({ log: { level: e.target.value } })}>
                <option value="info">info</option>
                <option value="warn">warn</option>
                <option value="error">error</option>
              </select>
            </Row>
            <Row title="Logs" note="Opens the folder in your file manager.">
              <button className="btn" onClick={() => act(api.openLogsFolder)}>
                Open logs folder
              </button>
            </Row>
            <Row title="Diagnostics" note="Versions, backends, profile and recent warnings. Copied to the clipboard only.">
              <button
                className="btn"
                onClick={() =>
                  act(async () => {
                    await navigator.clipboard.writeText(await api.copyDiagnostics());
                    setMsg("Diagnostics copied");
                  })
                }
              >
                Copy diagnostics
              </button>
            </Row>
          </div>
        </div>
        <div className="panel">
          <div className="panel-head">
            <h3>Updates</h3>
          </div>
          <div className="panel-body set-list">
            <Row title="Check on launch and daily" note="The only network request arrowhead makes.">
              <Switch label="Check for updates" checked={cfg.updates.check} onChange={(v) => save({ updates: { check: v } })} />
            </Row>
            <Row
              title={upd ? `Version ${upd.version} is out` : upd === null ? "Up to date" : "Not checked yet"}
              note={upd && !upd.can_install ? "Update with your package manager, or download the new release." : undefined}
            >
              <div className="live-controls">
                <button className="btn" disabled={busy} onClick={() => act(async () => setUpd(await api.checkUpdate()))}>
                  Check now
                </button>
                {upd?.can_install && (
                  <button className="btn primary" disabled={busy || state === "Playing"} onClick={() => act(api.installUpdate)}>
                    Update and restart
                  </button>
                )}
              </div>
            </Row>
            {upd?.notes && <pre className="notes">{upd.notes}</pre>}
          </div>
        </div>
      </div>
    </section>
  );
}
