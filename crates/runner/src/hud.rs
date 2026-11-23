//! HUD thread (spec 03 HUD reading, spec 04 readback): reads the game's stats, counts and our
//! ms readout from side copies of the screen (`AuxTap`), teaches the glyph atlas as the song
//! goes, matches each ms readout to the press that caused it and moves the offset, feeds the
//! accuracy controller, and writes the history row when a song ends. Normal priority, off the
//! hot path; it talks to the scan thread over channels only.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::time::{Duration, Instant, SystemTime};

use arrowhead_core::adapt::{Bias, Class, Judged, MaxNote};
use arrowhead_core::adapt::{PlayRecord, SongBook};
use arrowhead_core::config::{
    HistoryEntry, HitHist, Profile, Windows, append_history, data_dir, load_toml, save_toml,
};
use arrowhead_core::event::{EngineEvent, Level, Stats};
use arrowhead_core::frame::{AuxTap, OwnedFrame, Rect};
use arrowhead_core::hud::readback::{Readback, SentPress};
use arrowhead_core::hud::{
    Atlas, Counts, Face, HudReader, Layout, Locations, ResultsRead, StatsRead,
};
use arrowhead_core::time::Nanos;

use crate::{Tx, emit, log};

/// From the scan thread.
pub enum ToHud {
    /// A press the scheduler sent, with the error the style meant for it.
    Press {
        sent: Nanos,
        lane: u8,
        intended_ms: f32,
        /// Single, chord or jack, as the planner classed the note (spec 04, adaptation).
        class: Class,
    },
    /// Notes are approaching: a song started (counters read zero now).
    SongStart,
    /// No notes for 3 s: the song is over.
    SongOver {
        offset_ms: f32,
        engine_missed: u32,
        watchdog: u32,
    },
}

/// To the scan thread.
pub enum FromHud {
    /// New offset from readback, ms.
    Offset(f32),
    /// Accuracy the game shows, percent, for the controller.
    Accuracy(f32),
    /// Per-lane and per-class press corrections, ms (`Planner::set_corrections`).
    Corrections([f64; 4], [f64; 3]),
    /// The style's target accuracy for this song (its base plus the per-song adjust).
    Target(f64),
}

pub struct HudSetup {
    pub profile: String,
    pub prof_path: Option<PathBuf>,
    pub hud: arrowhead_core::config::HudRegions,
    pub stream: [u32; 2],
    pub receptors_x: [f32; 4],
    pub pitch: f32,
    pub max_sad: f32,
    pub offset_ms: f32,
    pub readback_sign: f32,
    pub windows: Windows,
    pub style: String,
    pub target: Option<f32>,
    pub seed: u64,
    /// `ui.ocr_model`: the OCR model set that teaches the glyph reader and reads song titles.
    pub ocr_model: String,
}

/// Every 8th frame: about 20 Hz at 165 Hz.
const EVERY: u32 = 8;
/// Full frames while something is still unknown, at the rate `locate` runs (about 2 Hz at
/// 165 Hz): each is a full-stream copy on the capture thread, so none is taken for nothing.
const EVERY_FULL: u32 = 80;
const HIST_FROM_MS: f32 = -240.0;
const HIST_BIN_MS: f32 = 10.0;
const HIST_BINS: usize = 48;

/// What the HUD learned and measured this song.
#[derive(Default)]
struct Song {
    started: Option<SystemTime>,
    t0: Option<Instant>,
    stats: Option<StatsRead>,
    counts: Option<Counts>,
    hit_errs: Vec<f32>,
    ms_taught: bool,
    zeros_taught: bool,
    /// Per-lane and per-class timing errors this song, and the corrections now applied.
    bias: Bias,
    applied: ([f64; 4], [f64; 3]),
    /// Non-Sick judgments by lane and class (kept for Max plays).
    max_note: MaxNote,
    /// Count-row label colours, learned from the counts panel (readouts use the same colours).
    label_rgb: Option<[[f32; 3]; 5]>,
    /// The title still has to be read (song start).
    title_wanted: bool,
    /// From the title: `SongBook` key, name and difficulty.
    key: Option<String>,
    name: String,
    difficulty: String,
}

/// How long after a song the results screen is waited for, and how often it is read.
const RESULTS_WAIT: Duration = Duration::from_secs(10);
const RESULTS_EVERY: Duration = Duration::from_secs(1);

/// A song that is over but whose history row waits for the results screen (spec 03).
struct PendingResults {
    song: Song,
    offset_ms: f32,
    engine_missed: u32,
    watchdog: u32,
    bias_ms: f32,
    ended: Instant,
    last_try: Instant,
}

/// A face knows every digit: OCR teaching can stop.
fn knows_digits(rd: &HudReader, face: Face) -> bool {
    let c = rd.atlas.chars(face);
    "0123456789".chars().all(|d| c.contains(d))
}

/// Between OCR teaching passes, while a face still misses digits.
const TEACH_EVERY: Duration = Duration::from_secs(3);

/// Matched readbacks between correction updates sent to the planner.
const CORRECT_EVERY: u32 = 10;

fn bump(j: &mut Judged, judgment: &str) {
    match judgment {
        "good" => j.good += 1,
        "ok" => j.ok += 1,
        "bad" => j.bad += 1,
        "miss" => j.miss += 1,
        _ => {}
    }
}

/// The press that the counter tick between two HUD reads must come from: exactly one press
/// sent from 200 ms before the earlier read to the later read, so Hits went up by one (a
/// chord would add two). `None` when there were none or several.
fn single_press(presses: &VecDeque<SentPress>, from: Nanos, to: Nanos) -> bool {
    let lo = Nanos(from.0.saturating_sub(200_000_000));
    presses
        .iter()
        .filter(|p| p.sent >= lo && p.sent <= to)
        .count()
        == 1
}

/// Which count row went up by one since `prev`, when exactly one did.
fn judged(prev: &Counts, now: &Counts) -> Option<&'static str> {
    let a = [prev.sick, prev.good, prev.ok, prev.bad, prev.miss];
    let b = [now.sick, now.good, now.ok, now.bad, now.miss];
    let up: Vec<usize> = (0..5)
        .filter(|&i| matches!((a[i], b[i]), (Some(x), Some(y)) if y > x))
        .collect();
    (up.len() == 1).then(|| ["sick", "good", "ok", "bad", "miss"][up[0]])
}

fn by_windows(w: &Windows, hit: f32) -> &'static str {
    let a = hit.abs();
    match () {
        _ if a <= w.sick => "sick",
        _ if a <= w.good => "good",
        _ if a <= w.ok => "ok",
        _ if a <= w.bad => "bad",
        _ => "miss",
    }
}

fn mean_ur(e: &[f32]) -> (Option<f32>, Option<f32>) {
    if e.len() < 2 {
        return (e.first().copied(), None);
    }
    let n = e.len() as f32;
    let m = e.iter().sum::<f32>() / n;
    let sd = (e.iter().map(|x| (x - m).powi(2)).sum::<f32>() / (n - 1.0)).sqrt();
    (Some(m), Some(sd * 10.0))
}

fn hist(e: &[f32]) -> Option<HitHist> {
    if e.is_empty() {
        return None;
    }
    let mut counts = vec![0u32; HIST_BINS];
    for &x in e {
        let i = ((x - HIST_FROM_MS) / HIST_BIN_MS).floor() as i64;
        counts[i.clamp(0, HIST_BINS as i64 - 1) as usize] += 1;
    }
    Some(HitHist {
        from_ms: HIST_FROM_MS,
        bin_ms: HIST_BIN_MS,
        counts,
    })
}

/// RFC 3339 UTC, seconds (no date crate for one timestamp).
fn rfc3339(t: SystemTime) -> String {
    let s = t
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs()) as i64;
    let (days, rem) = (s.div_euclid(86_400), s.rem_euclid(86_400));
    // Howard Hinnant's civil_from_days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

/// The copy that holds `r`, as (frame, r).
fn holding(frames: &[OwnedFrame], r: Rect) -> Option<&OwnedFrame> {
    frames.iter().find(|f| f.region.intersect(&r) == Some(r))
}

pub fn spawn(
    tap: Option<Arc<AuxTap>>,
    setup: HudSetup,
    rx: Receiver<ToHud>,
    out: Sender<FromHud>,
    tx: Tx,
) -> std::io::Result<std::thread::JoinHandle<()>> {
    std::thread::Builder::new()
        .name("hud".into())
        .spawn(move || run(tap, setup, rx, out, tx))
}

fn run(tap: Option<Arc<AuxTap>>, s: HudSetup, rx: Receiver<ToHud>, out: Sender<FromHud>, tx: Tx) {
    let atlas_path = Atlas::path(&s.profile);
    let mut rd = HudReader::new(
        atlas_path.as_deref().map(Atlas::load).unwrap_or_default(),
        s.max_sad,
    );
    rd.set_playfield(s.receptors_x, s.pitch);
    rd.locs = Locations::from_regions(&s.hud);
    let full = Rect::new(0, 0, s.stream[0], s.stream[1]);
    let mut rb = Readback::new(s.offset_ms);
    rb.sign = s.readback_sign;
    let mut presses: VecDeque<SentPress> = VecDeque::with_capacity(64);
    // The class of each press in `presses`, same order.
    let mut classes: VecDeque<Class> = VecDeque::with_capacity(64);
    let mut song = Song::default();
    let mut pending: Option<PendingResults> = None;
    let (mut seq, mut last_pts, mut last_stats_at) = (0u64, Nanos(0), Instant::now());
    let mut requested: Vec<Rect> = vec![];
    let mut last_locate = Instant::now() - Duration::from_secs(10);
    let mut dirty_atlas = false;
    let mut title_rect: Option<Rect> = s
        .hud
        .title
        .map(|a| Rect::new(a[0], a[1], a[2].max(0) as u32, a[3].max(0) as u32));
    let book_path = data_dir().map(|d| d.join("songs.json"));
    let mut book = book_path.as_deref().map(SongBook::load).unwrap_or_default();
    let style_id = s.style.to_lowercase();
    let mut next_teach = Instant::now();
    // OCR teaches the glyph reader and reads song titles; without its models the HUD still
    // teaches itself, more slowly, and songs are not told apart.
    let mut ocr = if tap.is_some() {
        match crate::ocrwork::Worker::start(s.ocr_model.clone()) {
            Ok(w) => Some(w),
            Err(e) => {
                log(&tx, Level::Warn, format!("OCR off: {e}"));
                None
            }
        }
    } else {
        None
    };
    if tap.is_none() {
        log(
            &tx,
            Level::Info,
            "HUD reader off: this capture gives no side copies",
        );
    }

    loop {
        // Messages from the scan thread.
        loop {
            match rx.try_recv() {
                Ok(ToHud::Press {
                    sent,
                    lane,
                    intended_ms,
                    class,
                }) => {
                    if presses.len() == presses.capacity() {
                        presses.pop_front();
                        classes.pop_front();
                    }
                    classes.push_back(class);
                    presses.push_back(SentPress {
                        sent,
                        lane,
                        intended_err_ms: intended_ms,
                    });
                }
                Ok(ToHud::SongStart) => {
                    if let Some(p) = pending.take() {
                        log(
                            &tx,
                            Level::Info,
                            "results screen: next song began, using the last live reading",
                        );
                        finish_song(&mut book, book_path.as_deref(), &s, &style_id, p, None, &tx);
                    }
                    song = Song {
                        started: Some(SystemTime::now()),
                        t0: Some(Instant::now()),
                        title_wanted: ocr.is_some(),
                        ..Song::default()
                    };
                    // Each song starts with a fast average from the offset it ended on.
                    let sign = rb.sign;
                    rb = Readback::new(rb.bias_ms);
                    rb.sign = sign;
                    let _ = out.send(FromHud::Corrections([0.0; 4], [0.0; 3]));
                }
                Ok(ToHud::SongOver {
                    offset_ms,
                    engine_missed,
                    watchdog,
                }) => {
                    if !song.hit_errs.is_empty() {
                        save_bias(&s, rb.bias_ms, &tx);
                    }
                    let done = PendingResults {
                        song: std::mem::take(&mut song),
                        offset_ms,
                        engine_missed,
                        watchdog,
                        bias_ms: rb.bias_ms,
                        ended: Instant::now(),
                        last_try: Instant::now() - RESULTS_EVERY,
                    };
                    if ocr.is_some() {
                        // the row waits for the results screen's exact numbers
                        if let Some(old) = pending.replace(done) {
                            finish_song(
                                &mut book,
                                book_path.as_deref(),
                                &s,
                                &style_id,
                                old,
                                None,
                                &tx,
                            );
                        }
                    } else {
                        finish_song(
                            &mut book,
                            book_path.as_deref(),
                            &s,
                            &style_id,
                            done,
                            None,
                            &tx,
                        );
                    }
                    if dirty_atlas
                        && let Some(p) = &atlas_path
                        && rd.atlas.save(p).is_ok()
                    {
                        dirty_atlas = false;
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    if let Some(p) = pending.take() {
                        finish_song(&mut book, book_path.as_deref(), &s, &style_id, p, None, &tx);
                    }
                    if dirty_atlas && let Some(p) = &atlas_path {
                        let _ = rd.atlas.save(p);
                    }
                    return;
                }
            }
        }
        if pending
            .as_ref()
            .is_some_and(|p| p.ended.elapsed() >= RESULTS_WAIT)
        {
            let p = pending.take().expect("checked");
            log(
                &tx,
                Level::Info,
                "results screen not read: using the last live reading",
            );
            finish_song(&mut book, book_path.as_deref(), &s, &style_id, p, None, &tx);
        }
        let Some(tap) = &tap else {
            // No HUD: still write history rows (without the game's numbers).
            match rx.recv_timeout(Duration::from_millis(200)) {
                Ok(m) => handle_late(m, &s, &mut song, &tx),
                Err(RecvTimeoutError::Disconnected) => return,
                Err(RecvTimeoutError::Timeout) => {}
            }
            continue;
        };

        // Ask for what we need: full frames until the stats, counts and our ms readout are
        // found, then just their rects.
        let l = rd.locs;
        let known: Vec<Rect> = [l.stats, l.counts, l.ms, title_rect]
            .into_iter()
            .flatten()
            .collect();
        let need_full = l.stats.is_none()
            || l.counts.is_none()
            || l.ms.is_none()
            || (ocr.is_some() && title_rect.is_none())
            || pending.is_some();
        let want = if need_full { vec![full] } else { known };
        if want != requested {
            tap.request(want.clone(), if need_full { EVERY_FULL } else { EVERY });
            requested = want;
        }
        let Some((n, frames)) = tap.wait(seq, Duration::from_millis(200)) else {
            continue;
        };
        seq = n;
        let Some(pts) = frames.first().map(|f| f.pts) else {
            continue;
        };

        if need_full
            && let Some(w) = ocr.as_mut()
            && !w.busy()
            && last_locate.elapsed() >= Duration::from_secs(2)
            && let Some(f) = holding(&frames, full)
        {
            last_locate = Instant::now();
            let column = rd
                .playfield
                .map(|(a, b)| Rect::new(a, 0, (b - a).max(0) as u32, s.stream[1]));
            w.offer(crate::ocrwork::Job::Locate {
                frame: f.clone(),
                column,
            });
        } else if need_full && ocr.is_none() && last_locate.elapsed() >= Duration::from_millis(400)
        {
            last_locate = Instant::now();
            let before = rd.locs;
            let found = rd.locate(&frames[0].view());
            // keep what was known when this frame did not show it
            rd.locs = Locations {
                stats: found.stats.or(before.stats),
                counts: found.counts.or(before.counts),
                ms: found.ms.or(before.ms),
                score_left: found.score_left.or(before.score_left),
                score_right: found.score_right.or(before.score_right),
                ..found
            };
            if rd.locs != before {
                save_regions(&s, &rd.locs, &tx);
            }
        }
        // OCR results.
        if let Some(d) = ocr.as_mut().and_then(|w| w.poll()) {
            use crate::ocrwork::Done;
            match d {
                Done::Lines(lines) => {
                    let found = crate::ocrteach::locate(&lines, rd.playfield, s.stream[0]);
                    let before = (rd.locs, title_rect);
                    rd.locs.stats = rd.locs.stats.or(found.stats);
                    rd.locs.counts = rd.locs.counts.or(found.counts);
                    rd.locs.ms = rd.locs.ms.or(found.ms);
                    title_rect = title_rect.or(found.title);
                    if (rd.locs, title_rect) != before {
                        save_regions_with_title(&s, &rd.locs, title_rect, &tx);
                        log(
                            &tx,
                            Level::Info,
                            format!(
                                "HUD: found by OCR: stats {}, counts {}, our ms readout {}, title {}",
                                yes(rd.locs.stats),
                                yes(rd.locs.counts),
                                yes(rd.locs.ms),
                                yes(title_rect)
                            ),
                        );
                    }
                }
                Done::Title(Some(t)) => {
                    let (name, difficulty) = crate::ocrteach::title(&t);
                    let key = SongBook::key(&t);
                    let start =
                        book.start_values(&key, &style_id, f64::from(s.target.unwrap_or(100.0)));
                    if let Some(o) = start.offset_ms {
                        let sign = rb.sign;
                        rb = Readback::new(o);
                        rb.sign = sign;
                        let _ = out.send(FromHud::Offset(o));
                    }
                    song.applied = (start.lane_ms, start.class_ms);
                    let _ = out.send(FromHud::Corrections(start.lane_ms, start.class_ms));
                    if let Some(t) = s.target {
                        let _ = out.send(FromHud::Target(f64::from(t) + start.target_adjust));
                    }
                    let plays = book.get(&key).map_or(0, |g| g.plays);
                    log(
                        &tx,
                        Level::Info,
                        match start.offset_ms {
                            Some(o) if plays > 0 => format!(
                                "song: {name} ({difficulty}), played {plays}x, starting at offset {o:+.1} ms"
                            ),
                            _ => format!("song: {name} ({difficulty}), first time"),
                        },
                    );
                    emit(
                        &tx,
                        EngineEvent::Song {
                            title: name.clone(),
                            difficulty: difficulty.clone(),
                        },
                    );
                    (song.key, song.name, song.difficulty) = (Some(key), name, difficulty);
                }
                Done::Title(None) => {}
                Done::Teach {
                    stats,
                    counts,
                    stats_frame,
                    counts_frame,
                } => {
                    let (st, cn) = crate::ocrteach::trusted(&stats, &counts);
                    let mut taught = 0;
                    if let Some(r) = rd.locs.stats {
                        let rows: [Option<String>; 7] = [
                            st.accuracy.map(|v| format!("{v:.2}%")),
                            st.highest_combo.map(|v| v.to_string()),
                            st.combo.map(|v| v.to_string()),
                            st.hits.map(|v| v.to_string()),
                            st.misses.map(|v| v.to_string()),
                            st.mean_ms.map(|v| format!("{v:.2}ms")),
                            st.kps.map(|v| v.to_string()),
                        ];
                        for (i, v) in rows.iter().enumerate() {
                            if let Some(v) = v {
                                taught += rd.teach_row(&stats_frame.view(), Layout::Stats, r, i, v)
                                    as u32;
                            }
                        }
                    }
                    if let Some(r) = rd.locs.counts {
                        for (i, v) in [cn.sick, cn.good, cn.ok, cn.bad, cn.miss]
                            .iter()
                            .enumerate()
                        {
                            if let Some(v) = v {
                                taught += rd.teach_row(
                                    &counts_frame.view(),
                                    Layout::Counts,
                                    r,
                                    i,
                                    &v.to_string(),
                                ) as u32;
                            }
                        }
                    }
                    if taught > 0 {
                        dirty_atlas = true;
                        log(
                            &tx,
                            Level::Info,
                            format!(
                                "HUD: OCR taught {taught} rows; stats font knows '{}'",
                                rd.atlas.chars(Face::Stats)
                            ),
                        );
                    }
                }
                Done::Ms {
                    value: Some(v),
                    frame,
                    rect,
                } => {
                    if rd.teach_ms_at(&frame.view(), rect, v) {
                        dirty_atlas = true;
                        log(
                            &tx,
                            Level::Info,
                            format!(
                                "HUD: learned ms digits from {v:.2} ms; readout font knows '{}'",
                                rd.atlas.chars(Face::Ms)
                            ),
                        );
                    }
                }
                Done::Ms { value: None, .. } => {}
                Done::Results(Some(r)) => {
                    if let Some(p) = pending.take() {
                        log(
                            &tx,
                            Level::Info,
                            "results screen read: history row uses its numbers",
                        );
                        finish_song(
                            &mut book,
                            book_path.as_deref(),
                            &s,
                            &style_id,
                            p,
                            Some(r),
                            &tx,
                        );
                    }
                }
                Done::Results(None) => {}
            }
        }
        if let Some(p) = pending.as_mut()
            && let Some(w) = ocr.as_mut()
            && !w.busy()
            && p.last_try.elapsed() >= RESULTS_EVERY
            && let Some(f) = holding(&frames, full)
        {
            p.last_try = Instant::now();
            w.offer(crate::ocrwork::Job::Results(f.clone()));
        }
        if let Some(w) = ocr.as_mut()
            && !w.busy()
        {
            if song.title_wanted
                && let Some(tr) = title_rect
                && let Some(f) = holding(&frames, tr)
            {
                song.title_wanted = false;
                w.offer(crate::ocrwork::Job::Title(f.view().crop(tr)));
            } else if song.t0.is_some()
                && !knows_digits(&rd, Face::Stats)
                && next_teach <= Instant::now()
                && let (Some(sr), Some(cr)) = (rd.locs.stats, rd.locs.counts)
                && let (Some(sf), Some(cf)) = (holding(&frames, sr), holding(&frames, cr))
            {
                next_teach = Instant::now() + TEACH_EVERY;
                w.offer(crate::ocrwork::Job::Teach {
                    stats: sf.view().crop(sr),
                    counts: cf.view().crop(cr),
                });
            }
        }
        let l = rd.locs;

        // Song start: every counter reads zero, which teaches 0 . % m s and the row labels.
        if let (Some(_), false) = (song.t0, song.zeros_taught) {
            song.zeros_taught = true;
            if rd.atlas.chars(arrowhead_core::hud::Face::Stats).is_empty() {
                let mut ok = true;
                if let Some(r) = l.stats
                    && let Some(f) = holding(&frames, r)
                {
                    ok &= rd
                        .teach_stack(
                            &f.view(),
                            Layout::Stats,
                            &["0.00%", "0", "0", "0", "0", "0.00ms", "0"],
                        )
                        .is_ok();
                }
                if let Some(r) = l.counts
                    && let Some(f) = holding(&frames, r)
                {
                    ok &= rd.teach_stack(&f.view(), Layout::Counts, &["0"; 5]).is_ok();
                }
                dirty_atlas |= ok;
                log(
                    &tx,
                    if ok { Level::Info } else { Level::Warn },
                    if ok {
                        "HUD: learned the stats font from the zeros at song start"
                    } else {
                        "HUD: could not read the stats panel at song start"
                    },
                );
            }
        }

        // Stats and counts.
        let prev = (song.stats, song.counts);
        if let Some(r) = l.stats
            && let Some(f) = holding(&frames, r)
        {
            let mut st = rd.read_stats(&f.view(), r);
            // Hits did not read, it was known before, and exactly one press went out since:
            // it shows one more, which teaches the new digit(s).
            if st.hits.is_none()
                && let Some(h) = prev.0.and_then(|p| p.hits)
                && single_press(&presses, last_pts, pts)
                && rd.teach_row(&f.view(), Layout::Stats, r, 3, &(h + 1).to_string())
            {
                dirty_atlas = true;
                st = rd.read_stats(&f.view(), r);
            }
            // First hit: Mean MS is that hit's ms, which teaches the ms readout's font (G14).
            if !song.ms_taught
                && st.hits == Some(1)
                && let Some(m) = st.mean_ms
            {
                song.ms_taught = true;
                // at the readout found by OCR, else searched for in our playfield
                let kept = match rd.locs.ms.and_then(|r| Some((r, holding(&frames, r)?))) {
                    Some((r, f)) => rd.teach_ms_at(&f.view(), r, m),
                    None => {
                        holding(&frames, full).is_some_and(|f| rd.teach_ms_first_hit(&f.view(), m))
                    }
                };
                if kept {
                    dirty_atlas = true;
                    save_regions(&s, &rd.locs, &tx);
                    log(
                        &tx,
                        Level::Info,
                        format!("HUD: learned the ms readout from the first hit ({m:.2} ms)"),
                    );
                }
            }
            if let Some(a) = st.accuracy.filter(|_| st.hits.is_some_and(|h| h > 0)) {
                let _ = out.send(FromHud::Accuracy(a));
            }
            song.stats = Some(st);
        }
        if let Some(r) = l.counts
            && let Some(f) = holding(&frames, r)
        {
            song.counts = Some(rd.read_counts(&f.view(), r));
            if song.label_rgb.is_none() {
                song.label_rgb = crate::ocrteach::label_colours(&f.view(), r);
            }
        }

        // Readback.
        if let Some(r) = rd.locs.ms
            && let Some(f) = holding(&frames, r)
        {
            let read = rd.read_ms(&f.view(), r);
            // The readout font is missing digits and Hits just went up by one: Mean MS says what
            // the readout shows (to its rounding), and OCR's reading of it can be checked.
            if read.0.is_none()
                && !read.1
                && !knows_digits(&rd, Face::Ms)
                && let (Some(p), Some(n)) = (prev.0, song.stats)
                && let (Some(pm), Some(ph), Some(nm), Some(nh)) =
                    (p.mean_ms, p.hits, n.mean_ms, n.hits)
                && let Some((expected, tol)) = crate::ocrteach::hit_ms_from_mean(pm, ph, nm, nh)
                && let Some(w) = ocr.as_mut()
            {
                w.offer(crate::ocrwork::Job::Ms {
                    frame: f.view().crop(r),
                    rect: r,
                    expected,
                    tol,
                });
            }
            match read {
                (_, true) => rb.on_blank(),
                (Some(raw), _) if (raw * rb.sign).abs() <= s.windows.bad + 20.0 => {
                    if let Some(m) = rb.on_read(pts, raw, None, presses.make_contiguous()) {
                        song.hit_errs.push(m.hit_error_ms);
                        // The readout is drawn in its judgment's colour; else the count that went
                        // up; else the (unverified) windows.
                        let by_colour = song.label_rgb.and_then(|labels| {
                            let c = crate::ocrteach::text_colour(&f.view(), r)?;
                            crate::ocrteach::judgment_by_colour(c, &labels)
                        });
                        let judgment = by_colour
                            .map(|i| ["sick", "good", "ok", "bad", "miss"][i])
                            .or_else(|| match (prev.1, song.counts) {
                                (Some(a), Some(b)) => judged(&a, &b),
                                _ => None,
                            })
                            .unwrap_or_else(|| by_windows(&s.windows, m.hit_error_ms));
                        // Per-lane and per-pattern bias, from the error as it would have been
                        // without the corrections in force (else they cancel themselves).
                        let lane = presses[m.press].lane;
                        let class = classes.get(m.press).copied().unwrap_or(Class::Single);
                        let (al, ac) = song.applied;
                        song.bias.add(
                            lane,
                            class,
                            f64::from(m.hit_error_ms - m.intended_err_ms)
                                + al[lane as usize & 3]
                                + ac[class as usize],
                        );
                        if song.bias.samples().is_multiple_of(CORRECT_EVERY) {
                            song.applied = (song.bias.lanes_ms(), song.bias.classes_ms());
                            let _ = out.send(FromHud::Corrections(song.applied.0, song.applied.1));
                        }
                        if judgment != "sick" {
                            bump(&mut song.max_note.lane[lane as usize & 3], judgment);
                            bump(&mut song.max_note.class[class as usize], judgment);
                        }
                        let _ = out.send(FromHud::Offset(m.bias_ms));
                        emit(
                            &tx,
                            EngineEvent::Readback {
                                ms: m.hit_error_ms,
                                judgment,
                                offset_ms: m.bias_ms,
                            },
                        );
                    }
                }
                // Unreadable, or outside every window (a misread): ignored.
                _ => {}
            }
        }
        last_pts = pts;

        if song.t0.is_some() && last_stats_at.elapsed() >= Duration::from_millis(500) {
            last_stats_at = Instant::now();
            emit(&tx, EngineEvent::Stats(stats_of(&song, s.target)));
        }
    }
}

fn stats_of(song: &Song, target: Option<f32>) -> Stats {
    let c = song.counts.unwrap_or_default();
    let (mean, ur) = mean_ur(&song.hit_errs);
    Stats {
        accuracy: song.stats.and_then(|s| s.accuracy),
        counts: [c.sick, c.good, c.ok, c.bad, c.miss],
        mean_err_ms: mean,
        ur,
        readbacks: song.hit_errs.len() as u32,
        target,
    }
}

/// Without a HUD: only song start and end matter.
fn handle_late(m: ToHud, s: &HudSetup, song: &mut Song, tx: &Tx) {
    match m {
        ToHud::SongStart => {
            *song = Song {
                started: Some(SystemTime::now()),
                t0: Some(Instant::now()),
                ..Song::default()
            }
        }
        ToHud::SongOver {
            offset_ms,
            engine_missed,
            watchdog,
            ..
        } => {
            write_history(
                s,
                song,
                offset_ms,
                engine_missed,
                watchdog,
                Instant::now(),
                None,
                tx,
            );
            *song = Song::default();
        }
        ToHud::Press { .. } => {}
    }
}

fn yes(r: Option<Rect>) -> &'static str {
    if r.is_some() { "yes" } else { "no" }
}

fn save_regions_with_title(s: &HudSetup, locs: &Locations, title: Option<Rect>, tx: &Tx) {
    save_regions(s, locs, tx);
    let Some(p) = &s.prof_path else { return };
    if let Ok(mut prof) = load_toml::<Profile>(p) {
        prof.hud.title = title.map(|r| [r.x, r.y, r.w as i32, r.h as i32]);
        let _ = save_toml(p, &prof);
    }
}

/// Writes the history row and the song's memory for a finished song. With the results screen
/// read, its counts and accuracy replace the last live ones.
fn finish_song(
    book: &mut SongBook,
    book_path: Option<&std::path::Path>,
    s: &HudSetup,
    style_id: &str,
    mut p: PendingResults,
    results: Option<ResultsRead>,
    tx: &Tx,
) {
    if let Some(r) = &results {
        p.song.counts = Some(r.counts);
        if let Some(a) = r.accuracy {
            p.song.stats.get_or_insert_with(Default::default).accuracy = Some(a);
        }
    }
    write_history(
        s,
        &p.song,
        p.offset_ms,
        p.engine_missed,
        p.watchdog,
        p.ended,
        results.as_ref(),
        tx,
    );
    record_song(book, book_path, s, style_id, &p.song, p.bias_ms, tx);
}

/// The play goes into the song's memory (`songs.json`): offset and corrections (when enough
/// readbacks), the final accuracy for this style, and for Max the non-Sick judgments.
fn record_song(
    book: &mut SongBook,
    path: Option<&std::path::Path>,
    s: &HudSetup,
    style_id: &str,
    song: &Song,
    bias_ms: f32,
    tx: &Tx,
) {
    let (Some(key), Some(started)) = (&song.key, song.started) else {
        return;
    };
    let when = rfc3339(started);
    book.record_play(
        key,
        &PlayRecord {
            name: &song.name,
            difficulty: &song.difficulty,
            when: &when,
            readbacks: song.hit_errs.len() as u32,
            offset_ms: bias_ms,
            lane_ms: song.bias.lanes_ms(),
            class_ms: song.bias.classes_ms(),
            style: style_id,
            target: f64::from(s.target.unwrap_or(100.0)),
            final_accuracy: song.stats.and_then(|st| st.accuracy).map(f64::from),
            max_note: (style_id == "max").then_some(song.max_note),
        },
    );
    if let Some(p) = path
        && let Err(e) = book.save(p)
    {
        log(tx, Level::Warn, format!("song memory not saved: {e}"));
    }
}

/// Regions go into the profile's `[hud]`. The profile is reloaded first: the trail learner
/// saves it too, and a stale copy would undo its colours.
fn save_regions(s: &HudSetup, locs: &Locations, tx: &Tx) {
    let Some(p) = &s.prof_path else { return };
    let mut prof: Profile = match load_toml(p) {
        Ok(pr) => pr,
        Err(e) => return log(tx, Level::Warn, format!("HUD regions not saved: {e}")),
    };
    prof.hud = locs.to_regions();
    if let Err(e) = save_toml(p, &prof) {
        log(tx, Level::Warn, format!("HUD regions not saved: {e}"));
    }
}

/// Readback's offset becomes the profile's `game_bias_ms`, the next run's start (spec 04).
fn save_bias(s: &HudSetup, bias_ms: f32, tx: &Tx) {
    let Some(p) = &s.prof_path else { return };
    let saved = load_toml::<Profile>(p).and_then(|mut prof| {
        prof.latency.game_bias_ms = bias_ms;
        save_toml(p, &prof)
    });
    match saved {
        Ok(()) => log(
            tx,
            Level::Info,
            format!("offset {bias_ms:+.1} ms saved to the profile"),
        ),
        Err(e) => log(tx, Level::Warn, format!("offset not saved: {e}")),
    }
}

#[allow(clippy::too_many_arguments)]
fn write_history(
    s: &HudSetup,
    song: &Song,
    offset_ms: f32,
    engine_missed: u32,
    watchdog: u32,
    ended: Instant,
    results: Option<&ResultsRead>,
    tx: &Tx,
) {
    let (Some(started), Some(t0)) = (song.started, song.t0) else {
        return;
    };
    let c = song.counts.unwrap_or_default();
    let counts = [
        ("sick", c.sick),
        ("good", c.good),
        ("ok", c.ok),
        ("bad", c.bad),
        ("miss", c.miss),
    ]
    .into_iter()
    .filter_map(|(k, v)| v.map(|v| (k.to_string(), v)))
    .collect();
    let (mean, ur) = mean_ur(&song.hit_errs);
    let e = HistoryEntry {
        v: 1,
        started: rfc3339(started),
        // the 3 s without notes that ended it are not part of the song
        duration_s: (ended.duration_since(t0).as_secs_f32() - 3.0).max(0.0),
        song: song.name.clone(),
        difficulty: song.difficulty.clone(),
        profile: s.profile.clone(),
        style: s.style.clone(),
        target: s.target,
        accuracy_hud: song.stats.and_then(|st| st.accuracy),
        counts,
        mean_err_ms: mean,
        ur,
        offset_ms,
        engine_missed,
        watchdog_releases: watchdog,
        seed: s.seed,
        hit_hist: hist(&song.hit_errs),
        total_notes: results.and_then(|r| r.total_notes),
        highest_combo: results.and_then(|r| r.highest_combo),
    };
    let Some(p) = data_dir().map(|d| d.join("history.jsonl")) else {
        return;
    };
    match append_history(&p, &e) {
        Ok(()) => log(tx, Level::Info, "history: song saved"),
        Err(err) => log(tx, Level::Warn, format!("history not saved: {err}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(ms: f64) -> SentPress {
        SentPress {
            sent: Nanos::from_ms(ms),
            lane: 0,
            intended_err_ms: 0.0,
        }
    }

    #[test]
    fn single_press_only_when_one() {
        let ps: VecDeque<SentPress> = [press(1000.0), press(1500.0), press(1510.0)].into();
        assert!(single_press(
            &ps,
            Nanos::from_ms(1100.0),
            Nanos::from_ms(1150.0)
        ));
        // the 200 ms before the earlier read count too: the press may not be counted yet
        assert!(single_press(
            &ps,
            Nanos::from_ms(1150.0),
            Nanos::from_ms(1200.0)
        ));
        assert!(
            !single_press(&ps, Nanos::from_ms(1300.0), Nanos::from_ms(1600.0)),
            "chord"
        );
        assert!(
            !single_press(&ps, Nanos::from_ms(2000.0), Nanos::from_ms(2050.0)),
            "none"
        );
    }

    #[test]
    fn judged_needs_exactly_one_row() {
        let a = Counts {
            sick: Some(3),
            good: Some(1),
            ok: Some(0),
            bad: Some(0),
            miss: Some(0),
        };
        let mut b = a;
        b.good = Some(2);
        assert_eq!(judged(&a, &b), Some("good"));
        b.sick = Some(4);
        assert_eq!(judged(&a, &b), None);
    }

    #[test]
    fn stats_and_hist() {
        let (m, ur) = mean_ur(&[-10.0, 10.0]);
        assert_eq!(m, Some(0.0));
        assert!((ur.unwrap() - 141.42).abs() < 0.1);
        let h = hist(&[-240.0, 0.0, 5.0, 300.0]).unwrap();
        assert_eq!(h.counts[0], 1);
        assert_eq!(h.counts[24], 2);
        assert_eq!(h.counts[HIST_BINS - 1], 1, "outliers land in the end bins");
    }

    #[test]
    fn rfc3339_dates() {
        let t = |s: u64| SystemTime::UNIX_EPOCH + Duration::from_secs(s);
        assert_eq!(rfc3339(t(0)), "1970-01-01T00:00:00Z");
        assert_eq!(rfc3339(t(951_782_400)), "2000-02-29T00:00:00Z");
        assert_eq!(rfc3339(t(1_795_381_323)), "2026-11-22T21:02:03Z");
    }
}
