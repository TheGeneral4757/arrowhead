// Front end <-> Rust contract (spec 07, Tauri commands and events). Mirrors crates/app/src/main.rs
// and crates/core/src/event.rs; change both sides together.
import { Channel, invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export type SessionState = "Idle" | "Searching" | "Armed" | "Playing" | "Results" | "Paused";
export type Scroll = "up" | "down";
export type KeySet = "wasd" | "arrows" | "custom";
export type AutoOnOff = "auto" | "on" | "off";

export type Dot = { id: number; lane: number; y: number; tail: boolean };
export type Pipeline = {
  fps: number;
  interval_p50_ms: number;
  interval_p99_ms: number;
  pts_age_ms: number;
  scan_us_p99: number;
  late_us_p99: number;
  offset_ms: number;
  speed_px_ms: number;
  presses: number;
  missed_by_engine: number;
  suppressed: number;
  watchdog: number;
};

// All *_ms times are monotonic milliseconds on one clock (t_ms of Notes is "now").
export type EngineEvent =
  | { type: "State"; state: SessionState; dry_run: boolean; reason: string }
  | { type: "Layout"; receptors: [number, number][]; size: number; scroll: Scroll; stream: [number, number]; profile: string }
  | { type: "Notes"; t_ms: number; dots: Dot[] }
  | { type: "Planned"; id: number; lane: number; at_ms: number; release_ms: number }
  | { type: "Pressed"; id: number; lane: number; planned_ms: number; actual_ms: number }
  | { type: "Released"; id: number; lane: number; planned_ms: number; actual_ms: number }
  | { type: "Suppressed"; id: number; lane: number; reason: "focus" | "paused" | "dry_run" }
  | ({ type: "Pipeline" } & Pipeline)
  | { type: "Focus"; focused: boolean | null }
  | { type: "Hotkeys"; bound: string | null; error: string | null }
  | { type: "Log"; level: "info" | "warn" | "error"; text: string }
  | { type: "Readback"; ms: number; judgment: Judgment; offset_ms: number }
  | ({ type: "Stats" } & GameStats)
  | { type: "Song"; title: string; difficulty: string }
  | { type: "Stopped"; error: string | null };

/** HUD totals for the song (spec 07, Stats). `counts`: Sick, Good, Ok, Bad, Miss. `null` = not readable yet. */
export type GameStats = {
  accuracy: number | null;
  counts: (number | null)[];
  mean_err_ms: number | null;
  ur: number | null;
  readbacks: number;
  target: number | null;
  score_self: number | null;
  score_opponent: number | null;
};

export type SourceInfo = {
  width: number;
  height: number;
  refresh_hz: number;
  backend: string;
  monitor: string;
  scale: number;
  pts_is_receive_time: boolean;
};

export type AppInfo = { version: string; os: string; packaged: string | null };

export type Status = {
  running: boolean;
  state: SessionState;
  dry_run: boolean;
  paused: boolean;
  capture: SourceInfo | null;
  profile: string;
  focus: boolean | null;
  update: UpdateInfo | null;
};

export type Config = {
  version: number;
  profile: string;
  style: string;
  keys: { set: KeySet; custom: [string, string, string, string] };
  hotkeys: { toggle: string; panic: string; nudge_up: string; nudge_down: string; cycle_style: string };
  safety: { dry_run: boolean; require_focus_guard: boolean; max_hold_ms: number; game_window_classes: string[] };
  capture: { linux_source: "monitor" | "window"; windows_backend: "auto" | "dxgi" | "wgc-monitor" | "wgc-window" };
  timing: { commit_ms: number; late_limit_ms: number; tap_hold_ms: number; min_gap_ms: number };
  game: {
    windows_ms: { sick: number; good: number; ok: number; bad: number };
    accuracy_weights: { sick: number; good: number; ok: number; bad: number; miss: number };
    readback_sign: number;
    seat: "auto" | "left" | "right";
  };
  match_friend: { on: boolean; margin_pct: number };
  ui: { overlay: AutoOnOff; minimise_while_playing: AutoOnOff; hud_reader: boolean; ocr_model: string };
  updates: { check: boolean };
  log: { level: string; per_note: boolean };
};
type DeepPartial<T> = { [K in keyof T]?: T[K] extends object ? DeepPartial<T[K]> : T[K] };

export type Layout = {
  receptors: [number, number][];
  size: number;
  run_len: number[];
  run_shape: number[][];
  scroll: Scroll;
  stream: [number, number];
};
export type Nudge = { dx: number; dy: number; dpitch: number; dwidth: number };
export type Profile = {
  monitor: string;
  stream_size: [number, number];
  refresh_hz: number;
  scale: number;
  scroll: Scroll;
  receptors: [number, number][];
  radius_px: number;
  nudge: Nudge;
  latency: { capture_ms: number; input_ms: number; game_bias_ms: number };
  layout?: Layout;
  colours?: unknown;
};

export type Check = { ok: boolean; detail: string };
export type Permissions = { uinput: Check; hotkeys: Check; screencast: Check; game: Check };
/** `png` is a data: URL of the full-stream snapshot; the layout is drawn over it by the UI. */
export type Calibration = { layout: Layout; nudge: Nudge; png: string; colours_learned: boolean; trail_learned: boolean };
export type LatencyResult = { trials: number; input_ms_p50: number; input_ms_p90: number; includes_capture: boolean };
/** Player style (spec 06, core::style::Style). Built-ins: Max, Sweaty, Casual friend, Rusty.
 * `miss_floor` and `miss_k` are fractions, `target` a percentage. */
export type Style = {
  name: string;
  target: number;
  sigma0: number;
  bias: number;
  spread: number;
  chord: number;
  jack: number;
  stream: number;
  fatigue: number;
  focus_drift: number;
  miss_floor: number;
  miss_k: number;
  p_slip: number;
  hold_release_ms: number;
  release_bias: number;
  ur_min: number;
};
/** `probs`: Sick, Good, Ok, Bad, Miss. */
export type StylePreview = { k: number; sigma_ms: number; mean_ms: number; probs: number[]; accuracy: number };
export type Judgment = "sick" | "good" | "ok" | "bad" | "miss";
/** One line of history.jsonl (spec 08). `hit_hist.counts[i]` covers [from_ms + i * bin_ms, + bin_ms). */
export type HistoryEntry = {
  v: number;
  started: string;
  duration_s: number;
  song: string;
  difficulty: string;
  profile: string;
  style: string;
  target: number | null;
  accuracy_hud: number | null;
  counts: Partial<Record<Judgment, number>>;
  mean_err_ms: number | null;
  ur: number | null;
  offset_ms: number;
  engine_missed: number;
  watchdog_releases: number;
  seed: number;
  hit_hist?: { from_ms: number; bin_ms: number; counts: number[] };
  /** From the results screen; absent when it was not read. */
  total_notes?: number;
  highest_combo?: number;
};
export type UpdateInfo = { version: string; notes: string | null; can_install: boolean };

export const api = {
  appInfo: () => invoke<AppInfo>("app_info"),
  getStatus: () => invoke<Status>("get_status"),
  /** Opens the capture (portal picker on first use), calibrates and plays until stopped. */
  startEngine: (onEvents: (batch: EngineEvent[]) => void, dryRun: boolean) => {
    const ch = new Channel<EngineEvent[]>();
    ch.onmessage = onEvents;
    return invoke<void>("start_engine", { onEvent: ch, dryRun });
  },
  stopEngine: () => invoke<void>("stop_engine"),
  setPaused: (paused: boolean) => invoke<void>("set_paused", { paused }),
  panic: () => invoke<void>("panic"),
  setDryRun: (dryRun: boolean) => invoke<void>("set_dry_run", { dryRun }),
  getConfig: () => invoke<Config>("get_config"),
  /** Deep-merges `patch` into the config, saves it and returns the result. */
  setConfig: (patch: DeepPartial<Config>) => invoke<Config>("set_config", { patch }),
  listProfiles: () => invoke<string[]>("list_profiles"),
  selectProfile: (name: string) => invoke<void>("select_profile", { name }),
  getProfile: () => invoke<Profile>("get_profile"),
  setupPermissions: () => invoke<Permissions>("run_setup_step", { step: "permissions" }),
  /** Opens (or re-opens) the portal picker. */
  setupScreen: () => invoke<SourceInfo>("run_setup_step", { step: "screen" }),
  /** Needs a song playing. Finds receptors, learns colours, saves the profile. Takes 5-40 s. */
  setupCalibrate: () => invoke<Calibration>("run_setup_step", { step: "calibrate" }),
  /** Latency test: the UI shows a full-window panel, paints it on "latency-paint" events,
   * turns it white on any keydown and calls latencyKey(). Resolves when the trials are done. */
  setupLatency: () => invoke<LatencyResult>("run_setup_step", { step: "latency" }),
  latencyKey: () => invoke<void>("latency_key"),
  onLatencyPaint: (f: (white: boolean) => void): Promise<UnlistenFn> =>
    listen<boolean>("latency-paint", (e) => f(e.payload)),
  /** Sends one tap of `lane` (0-3) after a 2 s countdown, for the Keys step. */
  testLane: (lane: number) => invoke<void>("test_lane", { lane }),
  listStyles: () => invoke<Style[]>("list_styles"),
  selectStyle: (name: string) => invoke<void>("select_style", { name }),
  /** Saves a user style and makes it active. Built-in names are refused. */
  saveStyle: (style: Style) => invoke<void>("save_style", { style }),
  deleteStyle: (name: string) => invoke<void>("delete_style", { name }),
  stylePreview: (style: Style) => invoke<StylePreview>("style_preview", { style }),
  /** Newest first. */
  history: (limit: number, offset: number) => invoke<HistoryEntry[]>("history", { limit, offset }),
  calibrationSnapshot: () => invoke<Calibration>("calibration_snapshot"),
  nudgeLayout: (n: Nudge) => invoke<Calibration>("nudge_layout", n),
  copyDiagnostics: () => invoke<string>("copy_diagnostics"),
  openLogsFolder: () => invoke<void>("open_logs_folder"),
  checkUpdate: () => invoke<UpdateInfo | null>("check_update"),
  installUpdate: () => invoke<void>("install_update"),
};
