import { useEffect, useRef, useState } from "react";
import { api, type Calibration, type Check, type Config, type KeySet, type LatencyResult, type Nudge, type Permissions, type SourceInfo } from "../api";
import { useEngine } from "../engine";

const STEPS = ["Permissions", "Screen", "Calibrate", "Latency", "Keys"] as const;
type Done = Partial<Record<(typeof STEPS)[number], boolean>>;

/** Runs an async step with busy and error state. */
function useStep<T>() {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [data, setData] = useState<T | null>(null);
  const run = async (f: () => Promise<T>) => {
    setBusy(true);
    setError(null);
    try {
      const r = await f();
      setData(r);
      return r;
    } catch (e) {
      setError(String(e));
      return null;
    } finally {
      setBusy(false);
    }
  };
  return { busy, error, data, setData, run };
}

function Status({ ok }: { ok: boolean }) {
  return <span className={`pill ${ok ? "live" : "bad"}`}>{ok ? "Ready" : "Missing"}</span>;
}

function Err({ e }: { e: string | null }) {
  return e ? (
    <p className="error-line" role="alert">
      {e}
    </p>
  ) : null;
}

function PermissionsStep({ onDone }: { onDone: (ok: boolean) => void }) {
  const s = useStep<Permissions>();
  const check = () => s.run(api.setupPermissions).then((p) => p && onDone(p.uinput.ok && p.screencast.ok));
  useEffect(() => {
    void check();
    // Runs once when the step opens.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  const rows: [string, Check | undefined][] = [
    ["Key output", s.data?.uinput],
    ["Screen capture", s.data?.screencast],
    ["Global hotkeys (optional)", s.data?.hotkeys],
    ["Game window", s.data?.game],
  ];
  return (
    <>
      {s.data && (
        <div className="check-list">
          {rows.map(([k, c]) => (
            <div className="check" key={k}>
              <div>
                <b className="small">{k}</b>
                <p>{c?.detail}</p>
              </div>
              <Status ok={!!c?.ok} />
            </div>
          ))}
        </div>
      )}
      <Err e={s.error} />
      <div className="row">
        <span className="note">Passes when key output and screen capture are ready. Without hotkeys, use the buttons on Live.</span>
        <button className="btn" onClick={check} disabled={s.busy}>
          {s.busy ? "Checking" : "Check again"}
        </button>
      </div>
    </>
  );
}

function ScreenStep({ onDone, running }: { onDone: (ok: boolean) => void; running: boolean }) {
  const s = useStep<SourceInfo>();
  useEffect(() => {
    api.getStatus().then((st) => st.capture && s.setData(st.capture));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  const i = s.data;
  return (
    <>
      <p className="small muted">Pick the monitor the game is on. Your desktop's share dialog opens; the choice is remembered.</p>
      {i && (
        <div className="check-list">
          <div className="check">
            <div>
              <b className="small">{i.monitor || "Monitor"}</b>
              <p className="mono">
                {i.width}x{i.height}, {i.refresh_hz.toFixed(0)} Hz, scale {i.scale}, {i.backend}
              </p>
            </div>
            <span className="pill live">Streaming</span>
          </div>
        </div>
      )}
      <Err e={s.error} />
      <div className="row">
        <span className="note">{running ? "Stop the engine first." : ""}</span>
        <button className="btn primary" disabled={s.busy || running} onClick={() => s.run(api.setupScreen).then((r) => onDone(!!r))}>
          {s.busy ? "Waiting for the share dialog" : i ? "Pick again" : "Open share dialog"}
        </button>
      </div>
    </>
  );
}

function CalibView({ cal }: { cal: Calibration }) {
  const ref = useRef<HTMLCanvasElement>(null);
  useEffect(() => {
    const c = ref.current;
    if (!c || !cal.png) return;
    const img = new Image();
    img.onload = () => {
      c.width = img.naturalWidth;
      c.height = img.naturalHeight;
      const g = c.getContext("2d");
      if (!g) return;
      g.drawImage(img, 0, 0);
      const k = img.naturalWidth / cal.layout.stream[0];
      const css = (v: string) => getComputedStyle(document.documentElement).getPropertyValue(v).trim();
      const r = (cal.layout.size / 2) * k;
      const up = cal.layout.scroll === "up";
      const len = 0.7 * cal.layout.stream[1] * k;
      cal.layout.receptors.forEach(([x, y], i) => {
        const cx = x * k;
        const cy = y * k;
        g.strokeStyle = css(`--lane-${i + 1}`);
        g.lineWidth = 2;
        g.beginPath();
        g.arc(cx, cy, r, 0, Math.PI * 2);
        g.stroke();
        g.strokeStyle = css("--color-accent");
        g.setLineDash([4, 3]);
        g.lineWidth = 1;
        g.strokeRect(cx - r * 0.35, up ? cy : cy - len, r * 0.7, len);
        g.setLineDash([]);
      });
    };
    img.src = cal.png;
  }, [cal]);
  return (
    <div className="calib">
      {cal.png ? (
        <canvas ref={ref} role="img" aria-label="Captured frame with the detected receptors circled and the lane strips boxed" />
      ) : (
        <p className="small muted calib-empty">No snapshot this session. Re-detect to take one.</p>
      )}
    </div>
  );
}

const NUDGES: { k: keyof Nudge; label: string; step: number }[] = [
  { k: "dx", label: "Move left or right", step: 1 },
  { k: "dy", label: "Move up or down", step: 1 },
  { k: "dpitch", label: "Lane spacing", step: 0.5 },
  { k: "dwidth", label: "Receptor size", step: 1 },
];

function CalibrateStep({ onDone, running }: { onDone: (ok: boolean) => void; running: boolean }) {
  const s = useStep<Calibration>();
  useEffect(() => {
    api.calibrationSnapshot().then(s.setData, () => {});
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  const nudge = async (k: keyof Nudge, d: number) => {
    if (!s.data) return;
    const n = { ...s.data.nudge, [k]: s.data.nudge[k] + d };
    await s.run(() => api.nudgeLayout(n));
  };
  const c = s.data;
  return (
    <>
      <p className="small muted">
        Start any song, keep the game visible, then press Detect. arrowhead finds our receptors, measures the lane strips, checks the scroll
        direction and learns the note colours. It takes 10 to 40 seconds.
      </p>
      {c && <CalibView cal={c} />}
      {c && (
        <>
          <p className="small">
            Detected: {c.layout.scroll === "up" ? "upscroll" : "downscroll"}, receptor size <span className="mono">{c.layout.size.toFixed(0)} px</span>, lane
            pitch <span className="mono">{((c.layout.receptors[3][0] - c.layout.receptors[0][0]) / 3).toFixed(1)} px</span>. Colours{" "}
            {c.colours_learned ? "learned" : "not learned"}, hold trail {c.trail_learned ? "learned" : "learned while playing"}.
          </p>
          <div className="nudges">
            {NUDGES.map((n) => (
              <div className="nudge" key={n.k}>
                <span className="small">{n.label}</span>
                <span className="mono num">{c.nudge[n.k].toFixed(1)}</span>
                <button className="btn icon" aria-label={`${n.label}: less`} disabled={s.busy} onClick={() => nudge(n.k, -n.step)}>
                  -
                </button>
                <button className="btn icon" aria-label={`${n.label}: more`} disabled={s.busy} onClick={() => nudge(n.k, n.step)}>
                  +
                </button>
              </div>
            ))}
          </div>
        </>
      )}
      <Err e={s.error} />
      <div className="row">
        <span className="note">{running ? "Stop the engine first." : "Detecting again replaces the saved layout and colours."}</span>
        <button className="btn primary" disabled={s.busy || running} onClick={() => s.run(api.setupCalibrate).then((r) => onDone(!!r))}>
          {s.busy ? "Detecting, keep the song playing" : c ? "Re-detect" : "Detect"}
        </button>
      </div>
    </>
  );
}

function LatencyStep({ onDone, running }: { onDone: (ok: boolean) => void; running: boolean }) {
  const s = useStep<LatencyResult>();
  const [white, setWhite] = useState(false);
  useEffect(() => {
    if (!s.busy) return;
    const un = api.onLatencyPaint(setWhite);
    const onKey = (e: KeyboardEvent) => {
      e.preventDefault();
      setWhite(true);
      void api.latencyKey();
    };
    window.addEventListener("keydown", onKey, true);
    return () => {
      window.removeEventListener("keydown", onKey, true);
      void un.then((f) => f());
    };
  }, [s.busy]);
  const r = s.data;
  return (
    <>
      <p className="small muted">
        arrowhead sends a key 50 times; this window turns white each time the key arrives and the screen capture times it. Keep this window
        focused and on the captured monitor, and do not touch the keyboard. The result is for reference: play timing uses the profile's game
        bias.
      </p>
      {r && (
        <div className="check-list">
          <div className="check">
            <div>
              <b className="small">Key sent to seen on screen</b>
              <p>
                {r.trials} trials, p90 <span className="mono">{r.input_ms_p90.toFixed(1)} ms</span>
                {r.includes_capture ? ", includes the capture delay" : ""}
              </p>
            </div>
            <span className="mono measured">{r.input_ms_p50.toFixed(1)} ms</span>
          </div>
        </div>
      )}
      <Err e={s.error} />
      <div className="row">
        <span className="note">{running ? "Stop the engine first." : ""}</span>
        <button className="btn primary" disabled={s.busy || running} onClick={() => s.run(api.setupLatency).then((x) => onDone(!!x))}>
          {r ? "Run again" : "Start test"}
        </button>
      </div>
      {s.busy && <div className={`latency-panel${white ? " white" : ""}`} role="status" aria-label="Latency test running" />}
    </>
  );
}

const SETS: { v: KeySet; label: string; keys: string[] }[] = [
  { v: "wasd", label: "A S W D", keys: ["A", "S", "W", "D"] },
  { v: "arrows", label: "Arrow keys", keys: ["Left", "Down", "Up", "Right"] },
  { v: "custom", label: "Custom", keys: [] },
];
const LANES = ["Left", "Down", "Up", "Right"];

function KeysStep({ onDone, running }: { onDone: (ok: boolean) => void; running: boolean }) {
  const [cfg, setCfg] = useState<Config | null>(null);
  const [lane, setLane] = useState<("ok" | "skip" | null)[]>([null, null, null, null]);
  const s = useStep<void>();
  useEffect(() => {
    api.getConfig().then(setCfg, () => {});
  }, []);
  useEffect(() => {
    if (lane.every((x) => x)) onDone(true);
  }, [lane, onDone]);
  if (!cfg) return null;
  const save = async (keys: Config["keys"]) => setCfg(await api.setConfig({ keys }));
  const mark = (i: number, v: "ok" | "skip") => setLane((l) => l.map((x, j) => (j === i ? v : x)));
  const shown = cfg.keys.set === "custom" ? cfg.keys.custom : SETS.find((x) => x.v === cfg.keys.set)!.keys;
  return (
    <>
      <p className="small muted">Funky Friday accepts both sets by default. arrowhead presses the set you pick.</p>
      <div className="seg" role="group" aria-label="Key set">
        {SETS.map((x) => (
          <button key={x.v} aria-pressed={cfg.keys.set === x.v} onClick={() => save({ ...cfg.keys, set: x.v })}>
            {x.label}
          </button>
        ))}
      </div>
      {cfg.keys.set === "custom" && (
        <div className="custom-keys">
          {LANES.map((l, i) => (
            <label key={l} className="field">
              <span className="small">{l}</span>
              <input
                className="input mono"
                defaultValue={cfg.keys.custom[i]}
                onBlur={(e) => {
                  const custom = [...cfg.keys.custom] as Config["keys"]["custom"];
                  custom[i] = e.target.value.trim().toLowerCase();
                  void save({ ...cfg.keys, custom });
                }}
              />
            </label>
          ))}
        </div>
      )}
      <p className="small muted">
        Lane test: start a song, press Test, switch to the game within 2 seconds, and watch that lane get one press.
      </p>
      <div className="check-list">
        {LANES.map((l, i) => (
          <div className="check" key={l}>
            <div className="keys">
              <kbd>{shown[i]}</kbd>
              <span className="small">{l}</span>
            </div>
            <div className="live-controls">
              <button className="btn" disabled={s.busy || running} onClick={() => s.run(() => api.testLane(i))}>
                Test
              </button>
              <button className="btn" aria-pressed={lane[i] === "ok"} onClick={() => mark(i, "ok")}>
                It pressed
              </button>
              <button className="btn" aria-pressed={lane[i] === "skip"} onClick={() => mark(i, "skip")}>
                Skip
              </button>
            </div>
          </div>
        ))}
      </div>
      <Err e={s.error} />
    </>
  );
}

export default function Setup() {
  const [step, setStep] = useState(0);
  const [done, setDone] = useState<Done>({});
  const { running } = useEngine();
  const name = STEPS[step];
  const mark = (k: (typeof STEPS)[number]) => (ok: boolean) => setDone((d) => (d[k] === ok ? d : { ...d, [k]: ok }));
  return (
    <section className="screen" aria-labelledby="h-setup">
      <div className="screen-head">
        <div>
          <h2 id="h-setup">Setup</h2>
          <p>Runs on first launch. Re-run any step from here.</p>
        </div>
      </div>
      <ol className="steps">
        {STEPS.map((k, i) => (
          <li key={k} className={i === step ? "current" : done[k] ? "done" : undefined}>
            <button onClick={() => setStep(i)} aria-current={i === step ? "step" : undefined}>
              {k}
              {done[k] ? ", passed" : ""}
            </button>
          </li>
        ))}
      </ol>
      <div className="wiz">
        {name === "Permissions" && <PermissionsStep onDone={mark("Permissions")} />}
        {name === "Screen" && <ScreenStep onDone={mark("Screen")} running={running} />}
        {name === "Calibrate" && <CalibrateStep onDone={mark("Calibrate")} running={running} />}
        {name === "Latency" && <LatencyStep onDone={mark("Latency")} running={running} />}
        {name === "Keys" && <KeysStep onDone={mark("Keys")} running={running} />}
      </div>
      <div className="wizard-foot">
        <button className="btn" disabled={step === 0} onClick={() => setStep(step - 1)}>
          Back
        </button>
        <button className="btn primary" disabled={step === STEPS.length - 1} onClick={() => setStep(step + 1)}>
          Next
        </button>
      </div>
    </section>
  );
}
