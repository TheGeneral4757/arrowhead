import { useEffect, useRef } from "react";
import type { Pipeline } from "../api";
import { engine, frame, useEngine } from "../engine";

const css = (v: string) => getComputedStyle(document.documentElement).getPropertyValue(v).trim();

function roundRect(g: CanvasRenderingContext2D, x: number, y: number, w: number, h: number, r: number) {
  g.beginPath();
  g.roundRect(x, y, w, h, r);
}

/** Four lanes as the game draws them: receptors, tracked notes, planned presses, actual presses. */
function Lanes({ active }: { active: boolean }) {
  const ref = useRef<HTMLCanvasElement>(null);
  const { layout, pipeline, dryRun } = useEngine();
  const speed = pipeline.at(-1)?.speed_px_ms ?? 0;

  useEffect(() => {
    const c = ref.current;
    if (!c) return;
    const size = () => {
      const d = devicePixelRatio || 1;
      c.width = c.clientWidth * d;
      c.height = c.clientHeight * d;
    };
    size();
    const ro = new ResizeObserver(() => {
      size();
      if (!active) draw();
    });
    ro.observe(c);
    const cols = ["--lane-1", "--lane-2", "--lane-3", "--lane-4"].map(css);
    const [paper, rule, muted, accent] = ["--color-paper-3", "--color-rule", "--color-muted", "--color-accent"].map(css);
    // Canvas text follows the UI scale (0.6875rem, 11px at the base size).
    const rem = parseFloat(getComputedStyle(document.documentElement).fontSize) || 16;
    const font = `${0.6875 * rem}px ${css("--font-mono")}`;
    let raf = 0;
    const draw = () => {
      const g = c.getContext("2d");
      if (g && c.width) {
        const d = devicePixelRatio || 1;
        const w = c.width / d;
        const h = c.height / d;
        g.setTransform(d, 0, 0, d, 0, 0);
        g.fillStyle = paper;
        g.fillRect(0, 0, w, h);
        const lw = w / 4;
        const up = layout?.scroll !== "down";
        const ry = up ? 3.25 * rem : h - 3.25 * rem;
        const dir = up ? 1 : -1;
        // About 0.7 of the stream height is approach strip (detection's approach_len_frac).
        const scale = layout ? (h - 4 * rem) / (0.7 * layout.stream[1]) : 1;
        const sz = Math.min(lw * 0.62, layout ? Math.max(layout.size * scale, 0.75 * rem) : 2.75 * rem);
        g.strokeStyle = rule;
        g.lineWidth = 1;
        for (let i = 1; i < 4; i++) {
          g.beginPath();
          g.moveTo(i * lw + 0.5, 0);
          g.lineTo(i * lw + 0.5, h);
          g.stroke();
        }
        const now = performance.now();
        for (let i = 0; i < 4; i++) {
          const cx = i * lw + lw / 2;
          g.globalAlpha = now - frame.pressedAt[i] < 90 ? 0.95 : 0.35;
          g.strokeStyle = cols[i];
          g.lineWidth = 2;
          roundRect(g, cx - sz / 2, ry - sz / 2, sz, sz, 0.5 * rem);
          g.stroke();
        }
        g.globalAlpha = 1;
        if (layout) {
          for (const n of frame.dots) {
            const dist = (n.y - layout.receptors[n.lane][1]) * (up ? 1 : -1);
            const y = ry + dir * dist * scale;
            const cx = n.lane * lw + lw / 2;
            g.fillStyle = cols[n.lane];
            if (n.tail) {
              g.fillRect(cx - sz * 0.3, y - 2, sz * 0.6, 4);
            } else {
              roundRect(g, cx - sz / 2, y - sz / 2, sz, sz, 0.5 * rem);
              g.fill();
            }
          }
          if (speed > 0) {
            g.strokeStyle = accent;
            g.lineWidth = 2;
            for (const p of frame.planned) {
              const y = ry + dir * (p.at_ms - frame.t_ms) * speed * scale;
              const cx = p.lane * lw + lw / 2;
              g.beginPath();
              g.moveTo(cx - sz * 0.6, y);
              g.lineTo(cx + sz * 0.6, y);
              g.stroke();
            }
          }
        }
        g.fillStyle = muted;
        g.font = font;
        g.fillText(
          !layout ? "calibrating: start a song" : dryRun ? "dry run, no keys sent" : "tracked notes, amber ticks are planned presses",
          0.625 * rem,
          up ? h - 0.75 * rem : 1.125 * rem,
        );
      }
      if (active) raf = requestAnimationFrame(draw);
    };
    draw();
    return () => {
      cancelAnimationFrame(raf);
      ro.disconnect();
    };
  }, [active, layout, speed, dryRun]);

  return (
    <div className="lanes-wrap">
      <canvas ref={ref} aria-label="Lane visualiser: tracked notes moving toward the receptors, planned and actual presses" />
      <div className="lane-legend">
        <span>left</span>
        <span>down</span>
        <span>up</span>
        <span>right</span>
      </div>
    </div>
  );
}

function Spark({ values }: { values: number[] }) {
  if (values.length < 2) return <svg viewBox="0 0 100 24" aria-hidden="true" />;
  const lo = Math.min(...values);
  const hi = Math.max(...values);
  const pts = values.map((v, i) => `${(i / (values.length - 1)) * 100},${22 - ((v - lo) / (hi - lo || 1)) * 20}`).join(" ");
  return (
    <svg viewBox="0 0 100 24" preserveAspectRatio="none" aria-hidden="true">
      <polyline points={pts} fill="none" strokeWidth="1.5" className="cool-line" />
    </svg>
  );
}

const METRICS: { k: string; f: (p: Pipeline) => number; fmt: (v: number) => string }[] = [
  { k: "Capture", f: (p) => p.fps, fmt: (v) => `${v.toFixed(0)} fps` },
  { k: "Frame interval p50 / p99", f: (p) => p.interval_p99_ms, fmt: (v) => `${v.toFixed(1)} ms` },
  { k: "Frame age p50", f: (p) => p.pts_age_ms, fmt: (v) => `${v.toFixed(1)} ms` },
  { k: "Scan p99", f: (p) => p.scan_us_p99, fmt: (v) => `${v.toFixed(0)} us` },
  { k: "Scheduler late p99", f: (p) => p.late_us_p99, fmt: (v) => `${v.toFixed(0)} us` },
  { k: "Offset applied", f: (p) => p.offset_ms, fmt: (v) => `${v >= 0 ? "+" : ""}${v.toFixed(1)} ms` },
];

export default function Live({ active }: { active: boolean }) {
  const s = useEngine();
  const last = s.pipeline.at(-1);
  const busy = s.running;
  const scroll = s.layout ? (s.layout.scroll === "up" ? "upscroll" : "downscroll") : null;

  return (
    <section className="screen" aria-labelledby="h-live">
      <div className="screen-head">
        <div>
          <h2 id="h-live">Live</h2>
          <p>
            {scroll
              ? `Centre side, ${scroll}, scroll speed ${last && last.speed_px_ms > 0 ? `${last.speed_px_ms.toFixed(2)} px/ms` : "not fitted yet"}.`
              : busy
                ? "Calibrating. Keep a song playing and the game visible."
                : "Start a song in the game, then press Start."}{" "}
            Song titles arrive with the HUD reader in v0.2.
          </p>
        </div>
        <div className="live-controls">
          <span className="pill armed">Preset: Max</span>
          <button className="btn" aria-pressed={s.dryRun} onClick={() => engine.setDryRun(!s.dryRun)}>
            Dry run {s.dryRun ? "on" : "off"}
          </button>
          {busy ? (
            <>
              <button className="btn" onClick={() => engine.setPaused(!s.paused)}>
                {s.paused ? "Resume" : "Pause"}
              </button>
              <button className="btn" onClick={() => engine.stop()}>
                Stop
              </button>
            </>
          ) : (
            <button className="btn primary" onClick={() => engine.start(s.dryRun)}>
              Start
            </button>
          )}
          <button className="btn danger" onClick={() => engine.panic()} disabled={!busy}>
            Panic, release all
          </button>
        </div>
      </div>
      {s.error && <p className="error-line" role="alert">Stopped: {s.error}</p>}
      <div className="live-grid">
        <Lanes active={active && busy} />
        <div className="stack">
          <div className="panel">
            <div className="panel-head">
              <h3>Accuracy and judgments</h3>
            </div>
            <div className="panel-body">
              <p className="small muted">
                Accuracy, the five judgment counts and the hit error bar are read from the game's HUD. That reader arrives in
                v0.2, so nothing is shown here yet.
              </p>
            </div>
          </div>
          <div className="panel">
            <div className="panel-head">
              <h3>Pipeline</h3>
              <span className="note">{last ? "measured, last minute" : "appears once playing"}</span>
            </div>
            <div className="panel-body">
              {last ? (
                <>
                  <div className="metrics">
                    {METRICS.map((m) => (
                      <div className="metric" key={m.k}>
                        <span className="k">{m.k}</span>
                        <span className="v">
                          {m.k.startsWith("Frame interval")
                            ? `${last.interval_p50_ms.toFixed(1)} / ${last.interval_p99_ms.toFixed(1)} ms`
                            : m.fmt(m.f(last))}
                        </span>
                        <Spark values={s.pipeline.map(m.f)} />
                      </div>
                    ))}
                  </div>
                  <table className="counts">
                    <tbody>
                      <tr>
                        <td>Presses</td>
                        <td className="num">{last.presses}</td>
                        <td>Found too late</td>
                        <td className="num">{last.missed_by_engine}</td>
                      </tr>
                      <tr>
                        <td>Suppressed</td>
                        <td className="num">{last.suppressed}</td>
                        <td>Watchdog releases</td>
                        <td className="num">{last.watchdog}</td>
                      </tr>
                    </tbody>
                  </table>
                </>
              ) : (
                <p className="small muted">Capture rate, frame timing, scan time and scheduler lateness show here while the engine runs.</p>
              )}
            </div>
          </div>
          <div className="panel">
            <div className="panel-head">
              <h3>Log</h3>
              <span className="note">
                {s.hotkeys?.bound ? `hotkeys: ${s.hotkeys.bound}` : s.hotkeys?.error ? `hotkeys off: ${s.hotkeys.error}` : ""}
              </span>
            </div>
            <ol className="log" aria-live="polite">
              {s.logs.length === 0 && <li className="muted">Engine messages show here.</li>}
              {s.logs.map((l, i) => (
                <li key={i} className={l.level}>
                  {l.text}
                </li>
              ))}
            </ol>
          </div>
        </div>
      </div>
    </section>
  );
}
