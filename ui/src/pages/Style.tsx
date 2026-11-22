import { useEffect, useState } from "react";
import { api, type Config, type Style as StyleT, type StylePreview } from "../api";
import { useEngine } from "../engine";

const BUILTINS = ["Max", "Sweaty", "Casual friend", "Rusty"];
const BLURB: Record<string, string> = {
  Max: "Every press on the centre",
  Sweaty: "Tight, rare slips",
  "Casual friend": "Loose, misses jacks",
  Rusty: "Late bias, tires",
};
const JUDGE = ["sick", "good", "ok", "bad", "miss"] as const;
const WINDOW_KEYS = ["bad", "ok", "good", "sick"] as const;

/** Mirrors core `style::custom_name`. */
const customName = (base: string) => `Custom (based on ${base.replace(/^Custom \(based on (.*)\)$/, "$1")})`;
const same = (a: StyleT, b: StyleT) => (Object.keys(a) as (keyof StyleT)[]).every((k) => k === "name" || a[k] === b[k]);
const pct = (v: number) => `${(v * 100).toFixed(1)}%`;
const sign = (v: number, unit = "") => `${v >= 0 ? "+" : ""}${v}${unit}`;

type Field = { key: keyof StyleT; label: string; min: number; max: number; step: number; show: (v: number) => string; note: string };
const FIELDS: Field[] = [
  { key: "bias", label: "Timing bias", min: -25, max: 25, step: 1, show: (v) => sign(v, " ms"), note: "Positive presses late, like most people." },
  { key: "spread", label: "Spread", min: 0.5, max: 2, step: 0.05, show: (v) => `${v.toFixed(2)}x`, note: "Scales the error the target implies." },
  { key: "chord", label: "Chord difficulty", min: 0, max: 1, step: 0.05, show: (v) => v.toFixed(2), note: "Extra error on two and three note chords." },
  { key: "jack", label: "Jack difficulty", min: 0, max: 1, step: 0.05, show: (v) => v.toFixed(2), note: "Extra error on fast repeats in one lane." },
  { key: "stream", label: "Stream difficulty", min: 0, max: 1, step: 0.05, show: (v) => v.toFixed(2), note: "Extra error in dense runs." },
  { key: "fatigue", label: "Fatigue", min: 0, max: 1, step: 0.05, show: (v) => v.toFixed(2), note: "Error grows over a long song." },
  { key: "focus_drift", label: "Focus drift", min: 0, max: 1, step: 0.05, show: (v) => v.toFixed(2), note: "Slow wander of the bias, like losing focus." },
  { key: "miss_floor", label: "Miss floor", min: 0, max: 0.05, step: 0.001, show: (v) => pct(v), note: "Chance of a clean miss, whatever the timing." },
  { key: "miss_k", label: "Hard-pattern misses", min: 0, max: 0.1, step: 0.005, show: (v) => pct(v), note: "Extra misses on chords, jacks and streams." },
  { key: "hold_release_ms", label: "Hold release", min: 0, max: 60, step: 1, show: (v) => `${v} ms`, note: "Spread on when a hold is let go." },
];

function Curve({ p, windows }: { p: StylePreview; windows: Config["game"]["windows_ms"] }) {
  const W = 300;
  const H = 120;
  const R = windows.bad + 20;
  const x = (ms: number) => ((ms + R) / (2 * R)) * W;
  const pts: string[] = [];
  if (p.sigma_ms > 0) {
    for (let i = 0; i <= 120; i++) {
      const ms = -R + (i * 2 * R) / 120;
      const y = Math.exp(-0.5 * ((ms - p.mean_ms) / p.sigma_ms) ** 2);
      pts.push(`${x(ms).toFixed(1)},${(H - 6 - y * (H - 16)).toFixed(1)}`);
    }
  }
  return (
    <div className="curve">
      <svg viewBox={`0 0 ${W} ${H}`} preserveAspectRatio="none" role="img" aria-label="Expected timing error against the judgment windows">
        {WINDOW_KEYS.map((k) => (
          <rect key={k} x={x(-windows[k])} y={0} width={x(windows[k]) - x(-windows[k])} height={H} className={`band band-${k}`} />
        ))}
        <line x1={x(0)} x2={x(0)} y1={0} y2={H} className="zero" />
        {pts.length > 0 ? (
          <>
            <polygon points={`${x(-R)},${H} ${pts.join(" ")} ${x(R)},${H}`} className="model" />
            <polyline points={pts.join(" ")} className="model-line" />
          </>
        ) : (
          <line x1={x(0)} x2={x(0)} y1={6} y2={H} className="model-line" />
        )}
      </svg>
      <div className="hit-scale mono xs muted">
        <span>-{R} ms</span>
        <span>0</span>
        <span>+{R} ms</span>
      </div>
    </div>
  );
}

export default function Style() {
  const { running } = useEngine();
  const [styles, setStyles] = useState<StyleT[] | null>(null);
  const [active, setActive] = useState<string>("Max");
  const [draft, setDraft] = useState<StyleT | null>(null);
  const [preview, setPreview] = useState<StylePreview | null>(null);
  const [windows, setWindows] = useState<Config["game"]["windows_ms"] | null>(null);
  const [saveAs, setSaveAs] = useState("");
  const [msg, setMsg] = useState<string | null>(null);

  const reload = () =>
    Promise.all([api.listStyles(), api.getConfig()]).then(
      ([list, cfg]) => {
        const cur = list.find((s) => s.name === cfg.style) ?? list[0];
        setStyles(list);
        setActive(cur.name);
        setDraft(cur);
        setSaveAs(customName(cur.name));
        setWindows(cfg.game.windows_ms);
      },
      (e) => setMsg(String(e)),
    );
  useEffect(() => {
    void reload();
  }, []);
  useEffect(() => {
    if (draft) api.stylePreview(draft).then(setPreview, (e) => setMsg(String(e)));
  }, [draft]);

  if (!styles || !draft) return msg ? <p className="error-line">{msg}</p> : null;

  const saved = styles.find((s) => s.name === active) ?? styles[0];
  const edited = !same(draft, saved);
  const isMax = draft.name === "Max" && !edited;
  const user = !BUILTINS.includes(active);

  const act = async (f: () => Promise<unknown>, done: string) => {
    try {
      await f();
      await reload();
      setMsg(running ? `${done} It applies from the next start.` : done);
    } catch (e) {
      setMsg(String(e));
    }
  };
  const pick = (s: StyleT) => act(() => api.selectStyle(s.name), `Playing as ${s.name}.`);
  const edit = (patch: Partial<StyleT>) => setDraft({ ...draft, ...patch });
  const save = () => {
    const name = saveAs.trim();
    if (BUILTINS.includes(name)) return setMsg(`${name} is a built-in preset; pick another name.`);
    void act(() => api.saveStyle({ ...draft, name }), `Saved ${name}.`);
  };

  return (
    <section className="screen" aria-labelledby="h-style">
      <div className="screen-head">
        <div>
          <h2 id="h-style">Style</h2>
          <p>Pick a preset or set a target. A player model makes the timing errors; a controller keeps the song near the target.</p>
        </div>
        <span className="pill armed">{edited ? "Unsaved changes" : active}</span>
      </div>
      {msg && <p className="small muted" role="status">{msg}</p>}

      <div className="style-grid">
        <div className="panel">
          <div className="panel-head">
            <h3>How it plays</h3>
          </div>
          <div className="panel-body">
            <div className="presets" role="group" aria-label="Styles">
              {styles.map((s) => (
                <button key={s.name} className="preset" aria-pressed={s.name === active && !edited} onClick={() => void pick(s)}>
                  <b>{s.name}</b>
                  <span>{BLURB[s.name] ?? `Target ${s.target.toFixed(1)}%`}</span>
                </button>
              ))}
            </div>
            <div className="target-row">
              <div className="readout">
                <label htmlFor="target">Target accuracy</label>
                <span className="num">{isMax ? "100%" : `${draft.target.toFixed(1)}%`}</span>
              </div>
              <input
                type="range"
                id="target"
                min={70}
                max={100}
                step={0.5}
                value={isMax ? 100 : draft.target}
                disabled={isMax}
                onChange={(e) => edit({ target: Number(e.target.value) })}
              />
              {isMax && <p className="note">Max aims every press at the centre of the window. Pick another preset to set a target.</p>}
            </div>
          </div>
        </div>

        <div className="panel">
          <div className="panel-head">
            <h3>Expected result</h3>
            <span className="note">model estimate, not measured</span>
          </div>
          <div className="panel-body">
            {preview && windows && (
              <>
                <div className="dist">
                  <div className="dist-bar" aria-hidden="true">
                    {JUDGE.map((k, i) => (
                      <i key={k} className={`judge-${k}`} style={{ flexGrow: Math.max(preview.probs[i], 0) }} />
                    ))}
                  </div>
                  <div className="dist-rows">
                    {JUDGE.map((k, i) => (
                      <div key={k} className="dist-row">
                        <span className={`sw judge-${k}`} />
                        <span>
                          {k[0].toUpperCase() + k.slice(1)}
                          {k !== "miss" ? `, within ${windows[k]} ms` : ""}
                        </span>
                        <span className="num mono">{pct(preview.probs[i])}</span>
                      </div>
                    ))}
                  </div>
                </div>
                <Curve p={preview} windows={windows} />
                <p className="note">
                  Spread {preview.sigma_ms.toFixed(1)} ms, mean {sign(Math.round(preview.mean_ms * 10) / 10, " ms")}. The game does not weight accuracy by
                  judgment counts, so while a song plays the controller steers by the accuracy the game shows.
                </p>
              </>
            )}
          </div>
        </div>
      </div>

      {!isMax && (
        <details className="advanced">
          <summary>Advanced player model</summary>
          <div className="adv-grid">
            {FIELDS.map((f) => (
              <div className="field" key={f.key}>
                <label htmlFor={`a-${f.key}`}>
                  {f.label} <span className="num">{f.show(draft[f.key] as number)}</span>
                </label>
                <input
                  type="range"
                  id={`a-${f.key}`}
                  min={f.min}
                  max={f.max}
                  step={f.step}
                  value={draft[f.key] as number}
                  onChange={(e) => {
                    const v = Number(e.target.value);
                    // The presets release holds with the same bias they press with.
                    edit(f.key === "bias" ? { bias: v, release_bias: v } : { [f.key]: v });
                  }}
                />
                <p>{f.note}</p>
              </div>
            ))}
          </div>
        </details>
      )}

      {(edited || user) && (
        <div className="panel">
          <div className="panel-body">
            <div className="row">
              <div className="row-text">
                {edited ? (
                  <label className="small" htmlFor="save-as">
                    Save as a style
                  </label>
                ) : (
                  <span className="small">Your style</span>
                )}
                <p>Saved to styles/ in the config folder. Built-in presets stay as they are.</p>
              </div>
              {edited && <input id="save-as" className="input" value={saveAs} onChange={(e) => setSaveAs(e.target.value)} />}
              {edited && (
                <button className="btn primary" onClick={save} disabled={!saveAs.trim()}>
                  Save and use
                </button>
              )}
              {edited && (
                <button
                  className="btn"
                  onClick={() => {
                    setDraft(saved);
                    setSaveAs(customName(saved.name));
                  }}
                >
                  Discard
                </button>
              )}
              {!edited && user && (
                <button className="btn danger" onClick={() => void act(() => api.deleteStyle(active), `Deleted ${active}; playing as Max.`)}>
                  Delete {active}
                </button>
              )}
            </div>
          </div>
        </div>
      )}
    </section>
  );
}
