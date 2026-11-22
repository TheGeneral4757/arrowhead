import { useEffect, useState } from "react";
import { api, type Config, type HistoryEntry, type Judgment } from "../api";

const PAGE = 50;
const JUDGMENTS: Judgment[] = ["sick", "good", "ok", "bad", "miss"];
const WINDOWS: Exclude<Judgment, "miss">[] = ["bad", "ok", "good", "sick"];

const when = (iso: string) => {
  const d = new Date(iso);
  return Number.isNaN(d.getTime())
    ? iso
    : d.toLocaleString(undefined, { day: "numeric", month: "short", hour: "2-digit", minute: "2-digit", hour12: false });
};
const fixed = (v: number | null, n: number, unit = "") => (v === null ? "-" : `${v.toFixed(n)}${unit}`);
const signed = (v: number | null) => (v === null ? "-" : `${v >= 0 ? "+" : ""}${v.toFixed(1)}`);
const song = (e: HistoryEntry) => (e.difficulty ? `${e.song || "Unknown song"}, ${e.difficulty}` : e.song || "Unknown song");

/** Hit-error histogram over the judgment windows, like the Style page's expected curve. */
function Histogram({ e, windows }: { e: HistoryEntry; windows: Config["game"]["windows_ms"] | null }) {
  const h = e.hit_hist;
  if (!h || h.counts.every((c) => c === 0)) {
    return <p className="small muted">This song has no hit errors: the game's ms readout was not readable while it played.</p>;
  }
  const W = 300;
  const H = 120;
  const span = h.counts.length * h.bin_ms;
  const x = (ms: number) => ((ms - h.from_ms) / span) * W;
  const peak = Math.max(...h.counts);
  const bw = (h.bin_ms / span) * W;
  return (
    <div className="curve">
      <svg viewBox={`0 0 ${W} ${H}`} preserveAspectRatio="none" role="img" aria-label={`Hit error histogram for ${song(e)}`}>
        {windows &&
          WINDOWS.map((k) => (
            <rect key={k} x={x(-windows[k])} y={0} width={x(windows[k]) - x(-windows[k])} height={H} className={`band band-${k}`} />
          ))}
        <line x1={x(0)} x2={x(0)} y1={0} y2={H} className="zero" />
        {h.counts.map((c, i) =>
          c ? <rect key={i} x={i * bw + 0.5} width={Math.max(bw - 1, 0.5)} y={H - (c / peak) * (H - 8)} height={(c / peak) * (H - 8)} className="bar" /> : null,
        )}
      </svg>
      <div className="hit-scale mono xs muted">
        <span>{h.from_ms} ms</span>
        <span>0</span>
        <span>+{h.from_ms + span} ms</span>
      </div>
    </div>
  );
}

export default function History() {
  const [rows, setRows] = useState<HistoryEntry[] | null>(null);
  const [more, setMore] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const [sel, setSel] = useState<number | null>(null);
  const [windows, setWindows] = useState<Config["game"]["windows_ms"] | null>(null);

  const load = (offset: number) =>
    api.history(PAGE, offset).then(
      (page) => {
        setRows((r) => (offset ? [...(r ?? []), ...page] : page));
        setMore(page.length === PAGE);
        setErr(null);
      },
      (e) => setErr(String(e)),
    );
  useEffect(() => {
    void load(0);
    api.getConfig().then((c) => setWindows(c.game.windows_ms), () => {});
  }, []);

  const picked = rows && sel !== null ? rows[sel] : null;

  return (
    <section className="screen" aria-labelledby="h-history">
      <div className="screen-head">
        <div>
          <h2 id="h-history">History</h2>
          <p>One row per song, with accuracy and judgments read from the game. Pick a row for its hit errors.</p>
        </div>
        {rows && rows.length > 0 && (
          <button className="btn" onClick={() => void load(0)}>
            Refresh
          </button>
        )}
      </div>

      {err && (
        <div className="row">
          <p className="error-line">History could not be read: {err}</p>
          <button className="btn" onClick={() => void load(0)}>
            Try again
          </button>
        </div>
      )}

      {!rows && !err && (
        <div className="table-wrap" aria-busy="true">
          <div className="skeleton" />
          <div className="skeleton" />
          <div className="skeleton" />
        </div>
      )}

      {rows && rows.length === 0 && (
        <div className="empty">
          <h3>No songs yet</h3>
          <p className="small muted">Each song you play while arrowhead is armed lands here with its judgments, mean error and UR.</p>
        </div>
      )}

      {rows && rows.length > 0 && (
        <>
          <div className="table-wrap">
            <table>
              <thead>
                <tr>
                  <th>When</th>
                  <th>Song</th>
                  <th>Preset</th>
                  <th className="num">Accuracy</th>
                  {JUDGMENTS.map((j) => (
                    <th key={j} className="num">
                      {j[0].toUpperCase() + j.slice(1)}
                    </th>
                  ))}
                  <th className="num">Mean error</th>
                  <th className="num">UR</th>
                </tr>
              </thead>
              <tbody>
                {rows.map((e, i) => (
                  <tr key={`${e.started}-${e.seed}-${i}`} aria-selected={i === sel}>
                    <td className="mono">
                      <button className="row-pick" aria-expanded={i === sel} onClick={() => setSel(i === sel ? null : i)}>
                        {when(e.started)}
                      </button>
                    </td>
                    <td>{song(e)}</td>
                    <td>{e.style}</td>
                    <td className="num measured">{fixed(e.accuracy_hud, 2, "%")}</td>
                    {JUDGMENTS.map((j) => (
                      <td key={j} className="num">
                        {e.counts[j] ?? "-"}
                      </td>
                    ))}
                    <td className="num">{signed(e.mean_err_ms)}</td>
                    <td className="num">{fixed(e.ur, 0)}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
          {more && (
            <button className="btn" onClick={() => void load(rows.length)}>
              Load older songs
            </button>
          )}
        </>
      )}

      {picked && (
        <div className="panel">
          <div className="panel-head">
            <h3>{song(picked)}</h3>
            <span className="note">
              {picked.style}
              {picked.target !== null ? `, target ${picked.target.toFixed(1)}%` : ""}, offset {signed(picked.offset_ms)} ms
            </span>
          </div>
          <div className="panel-body">
            <Histogram e={picked} windows={windows} />
            {(picked.engine_missed > 0 || picked.watchdog_releases > 0) && (
              <p className="small muted">
                Missed by the engine: {picked.engine_missed}. Watchdog releases: {picked.watchdog_releases}.
              </p>
            )}
          </div>
        </div>
      )}
    </section>
  );
}
