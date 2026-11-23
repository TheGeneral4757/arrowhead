// Engine state for the whole app: one store fed by the event channel, so pages can come and go.
import { useSyncExternalStore } from "react";
import { api, type Dot, type EngineEvent, type GameStats, type Judgment, type Pipeline, type Scroll, type SessionState } from "./api";

export type LogLine = { level: "info" | "warn" | "error"; text: string; at: number };
export type Planned = { id: number; lane: number; at_ms: number };
export type Hit = { ms: number; judgment: Judgment };

type Snapshot = {
  running: boolean;
  state: SessionState;
  dryRun: boolean;
  paused: boolean;
  focus: boolean | null;
  layout: { receptors: [number, number][]; size: number; scroll: Scroll; stream: [number, number]; profile: string } | null;
  pipeline: Pipeline[];
  logs: LogLine[];
  hotkeys: { bound: string | null; error: string | null } | null;
  error: string | null;
  stats: GameStats | null;
  /** Last 40 hits read back from the game, oldest first. */
  hits: Hit[];
  offset: number | null;
  song: { title: string; difficulty: string } | null;
};

// Fast-changing data the canvas reads every frame without re-rendering React.
export const frame = {
  t_ms: 0,
  dots: [] as Dot[],
  planned: [] as Planned[],
  /** performance.now() of the last press per lane. */
  pressedAt: [0, 0, 0, 0],
};

let snap: Snapshot = {
  running: false,
  state: "Idle",
  dryRun: false,
  paused: false,
  focus: null,
  layout: null,
  pipeline: [],
  logs: [],
  hotkeys: null,
  error: null,
  stats: null,
  hits: [],
  offset: null,
  song: null,
};
const subs = new Set<() => void>();
function set(p: Partial<Snapshot>) {
  snap = { ...snap, ...p };
  subs.forEach((f) => f());
}

function onBatch(batch: EngineEvent[]) {
  const p: Partial<Snapshot> = {};
  let logs: LogLine[] | null = null;
  let pipeline: Pipeline[] | null = null;
  let hits: Hit[] | null = null;
  for (const e of batch) {
    switch (e.type) {
      case "State":
        p.state = e.state;
        p.dryRun = e.dry_run;
        break;
      case "Layout":
        p.layout = { receptors: e.receptors, size: e.size, scroll: e.scroll, stream: e.stream, profile: e.profile };
        break;
      case "Notes":
        frame.t_ms = e.t_ms;
        frame.dots = e.dots;
        frame.planned = frame.planned.filter((x) => x.at_ms > e.t_ms - 150);
        break;
      case "Planned":
        frame.planned.push({ id: e.id, lane: e.lane, at_ms: e.at_ms });
        break;
      case "Pressed":
        frame.pressedAt[e.lane] = performance.now();
        break;
      case "Pipeline": {
        const { type: _t, ...m } = e;
        void _t;
        pipeline = [...(pipeline ?? snap.pipeline), m].slice(-60);
        break;
      }
      case "Focus":
        p.focus = e.focused;
        break;
      case "Readback":
        hits = [...(hits ?? snap.hits), { ms: e.ms, judgment: e.judgment }].slice(-40);
        p.offset = e.offset_ms;
        break;
      case "Song":
        p.song = { title: e.title, difficulty: e.difficulty };
        break;
      case "Stats": {
        const { type: _t, ...st } = e;
        void _t;
        p.stats = st;
        break;
      }
      case "Hotkeys":
        p.hotkeys = { bound: e.bound, error: e.error };
        break;
      case "Log":
        logs = [...(logs ?? snap.logs), { level: e.level, text: e.text, at: Date.now() }].slice(-50);
        break;
      case "Stopped":
        p.running = false;
        p.state = "Idle";
        p.paused = false;
        p.focus = null;
        p.error = e.error;
        break;
    }
  }
  if (logs) p.logs = logs;
  if (pipeline) p.pipeline = pipeline;
  if (hits) p.hits = hits;
  if (Object.keys(p).length) set(p);
}

export const engine = {
  get: () => snap,
  subscribe(f: () => void) {
    subs.add(f);
    return () => void subs.delete(f);
  },
  async start(dryRun: boolean) {
    frame.dots = [];
    frame.planned = [];
    set({ running: true, state: "Searching", dryRun, paused: false, error: null, pipeline: [], logs: [], hotkeys: null, layout: null, stats: null, hits: [], offset: null, song: null });
    try {
      await api.startEngine(onBatch, dryRun);
    } catch (e) {
      set({ running: false, state: "Idle", error: String(e) });
    }
  },
  async stop() {
    await api.stopEngine();
    set({ running: false, state: "Idle", paused: false });
  },
  async setPaused(paused: boolean) {
    await api.setPaused(paused);
    set({ paused });
  },
  async setDryRun(dryRun: boolean) {
    if (snap.running) await api.setDryRun(dryRun);
    set({ dryRun });
  },
  panic: () => api.panic(),
  /** Status polling fills in what the event stream cannot (an engine started before a reload). */
  sync(running: boolean, paused: boolean) {
    if (running !== snap.running || (running && paused !== snap.paused)) set({ running, paused });
  },
};

export function useEngine() {
  return useSyncExternalStore(engine.subscribe, engine.get);
}
