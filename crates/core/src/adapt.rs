//! Adaptation (specs 04, 06, 08): per-lane and per-pattern timing bias, and per-song memory.
//! Pure; the runner feeds `Bias` from readback and stores the `SongBook`.

use std::collections::BTreeMap;
use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};

/// How a note sits among its neighbours (spec 04, per-pattern bias).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class {
    Single = 0,
    /// Another lane has a note within 5 ms.
    Chord = 1,
    /// The previous note in the same lane is less than 150 ms earlier.
    Jack = 2,
}

pub const CHORD_MS: f64 = 5.0;
pub const JACK_MS: f64 = 150.0;
/// Shrinkage: a mean over `n` samples counts `n / (n + SHRINK_N)`.
const SHRINK_N: f64 = 10.0;
/// A lane or class gets no correction before this many samples of its own (spec 13, 4.5).
pub const MIN_SAMPLES: u32 = 20;

#[derive(Clone, Copy, Debug, Default)]
struct Acc {
    n: u32,
    sum: f64,
}

impl Acc {
    fn add(&mut self, x: f64) {
        self.n += 1;
        self.sum += x;
    }
    /// Shrunk mean relative to `overall`.
    fn rel(&self, overall: f64) -> f64 {
        if self.n < MIN_SAMPLES {
            return 0.0;
        }
        let n = self.n as f64;
        (self.sum / n - overall) * n / (n + SHRINK_N)
    }
}

/// Running per-lane and per-class mean of `hit_error - intended` (ms, positive = late),
/// relative to the overall mean, so the overall offset stays readback's job.
///
/// Feed the error as it would have been without the corrections now applied: add
/// `lane_ms(lane) + class_ms(class)` as sent to the measured error, or the corrections
/// cancel themselves as they work.
#[derive(Clone, Debug, Default)]
pub struct Bias {
    all: Acc,
    lane: [Acc; 4],
    class: [Acc; 3],
}

impl Bias {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(&mut self, lane: u8, class: Class, err_ms: f64) {
        if !err_ms.is_finite() {
            return;
        }
        self.all.add(err_ms);
        self.lane[lane as usize & 3].add(err_ms);
        self.class[class as usize].add(err_ms);
    }

    pub fn samples(&self) -> u32 {
        self.all.n
    }

    fn overall(&self) -> f64 {
        if self.all.n == 0 {
            0.0
        } else {
            self.all.sum / self.all.n as f64
        }
    }

    /// Correction to add to a lane's press time, ms (a late lane gets a negative one).
    pub fn lane_ms(&self, lane: u8) -> f64 {
        -self.lane[lane as usize & 3].rel(self.overall())
    }

    pub fn class_ms(&self, class: Class) -> f64 {
        -self.class[class as usize].rel(self.overall())
    }

    pub fn lanes_ms(&self) -> [f64; 4] {
        std::array::from_fn(|i| self.lane_ms(i as u8))
    }

    pub fn classes_ms(&self) -> [f64; 3] {
        [Class::Single, Class::Chord, Class::Jack].map(|c| self.class_ms(c))
    }
}

// ---- song memory (spec 08 songs.json) ----

/// Readbacks a play needs before its offset and corrections are kept.
pub const MIN_READBACKS: u32 = 20;
/// `target_adjust` bound, accuracy points.
pub const TARGET_ADJUST_MAX: f64 = 2.0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Judged {
    pub good: u32,
    pub ok: u32,
    pub bad: u32,
    pub miss: u32,
}

/// Non-Sick judgments of a Max play, by lane and by class (Single, Chord, Jack).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct MaxNote {
    pub lane: [Judged; 4],
    pub class: [Judged; 3],
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct StyleMem {
    pub last_accuracy: f64,
    pub target: f64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Song {
    pub name: String,
    pub difficulty: String,
    pub plays: u32,
    /// RFC 3339, given by the caller.
    pub last_played: String,
    pub offset_ms: Option<f32>,
    pub lane_ms: [f64; 4],
    pub class_ms: [f64; 3],
    pub styles: BTreeMap<String, StyleMem>,
    pub max_note: Option<MaxNote>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SongBook {
    pub version: u32,
    pub songs: BTreeMap<String, Song>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

impl Default for SongBook {
    fn default() -> Self {
        Self {
            version: 1,
            songs: BTreeMap::new(),
            extra: BTreeMap::new(),
        }
    }
}

/// One finished play, as the runner reports it.
#[derive(Clone, Debug)]
pub struct PlayRecord<'a> {
    pub name: &'a str,
    pub difficulty: &'a str,
    /// RFC 3339.
    pub when: &'a str,
    /// Readbacks matched during the play; below `MIN_READBACKS` offset and corrections are not kept.
    pub readbacks: u32,
    pub offset_ms: f32,
    pub lane_ms: [f64; 4],
    pub class_ms: [f64; 3],
    /// Style id ("max", "casual", ...).
    pub style: &'a str,
    pub target: f64,
    pub final_accuracy: Option<f64>,
    /// Non-Sick counts; kept when `style == "max"`.
    pub max_note: Option<MaxNote>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct StartValues {
    pub offset_ms: Option<f32>,
    pub lane_ms: [f64; 4],
    pub class_ms: [f64; 3],
    /// Add to the style's target accuracy, points.
    pub target_adjust: f64,
    /// Last Max play's non-Sick counts, for the UI.
    pub max_note: Option<MaxNote>,
}

impl SongBook {
    /// Lowercase, punctuation to spaces, whitespace collapsed.
    pub fn key(title: &str) -> String {
        let mut out = String::new();
        for w in title
            .split(|c: char| !c.is_alphanumeric())
            .filter(|w| !w.is_empty())
        {
            if !out.is_empty() {
                out.push(' ');
            }
            out.extend(w.chars().flat_map(char::to_lowercase));
        }
        out
    }

    pub fn get(&self, key: &str) -> Option<&Song> {
        self.songs.get(key)
    }

    /// The key to use for a song read as `key`: an entry whose title is the same or within two
    /// OCR slips of it (letters and digits only, look-alike digits folded; 8+ chars), else
    /// `key` itself. Other entries that match are merged into it, so one song played twice has
    /// one entry even when OCR spelled it differently (spec 13, 5.4). "Ghost (VIP)" and "Ghost"
    /// differ by three letters and stay apart.
    pub fn resolve(&mut self, key: &str) -> String {
        let c = canon(key);
        let near: Vec<String> = self
            .songs
            .keys()
            .filter(|k| {
                let o = canon(k);
                o == c || (c.len() >= 8 && edits(o.as_bytes(), c.as_bytes()) <= 2)
            })
            .cloned()
            .collect();
        let Some(target) = near
            .iter()
            .find(|k| *k == key)
            .or_else(|| near.iter().max_by_key(|k| self.songs[*k].plays))
            .cloned()
        else {
            return key.to_string();
        };
        for k in near.iter().filter(|k| **k != target) {
            let o = self.songs.remove(k).expect("listed");
            let t = self.songs.get_mut(&target).expect("listed");
            t.plays += o.plays;
            if o.last_played > t.last_played {
                t.last_played = o.last_played;
                if o.offset_ms.is_some() {
                    (t.offset_ms, t.lane_ms, t.class_ms) = (o.offset_ms, o.lane_ms, o.class_ms);
                }
                t.max_note = o.max_note.or(t.max_note);
                t.styles.extend(o.styles);
            } else {
                for (st, m) in o.styles {
                    t.styles.entry(st).or_insert(m);
                }
                t.offset_ms = t.offset_ms.or(o.offset_ms);
            }
        }
        target
    }

    pub fn record_play(&mut self, key: &str, r: &PlayRecord) {
        let s = self.songs.entry(key.to_string()).or_default();
        s.name = r.name.to_string();
        s.difficulty = r.difficulty.to_string();
        s.last_played = r.when.to_string();
        s.plays += 1;
        if r.readbacks >= MIN_READBACKS {
            s.offset_ms = Some(r.offset_ms);
            s.lane_ms = r.lane_ms;
            s.class_ms = r.class_ms;
        }
        if let Some(a) = r.final_accuracy {
            s.styles.insert(
                r.style.to_string(),
                StyleMem {
                    last_accuracy: a,
                    target: r.target,
                },
            );
        }
        if r.style == "max" && r.max_note.is_some() {
            s.max_note = r.max_note;
        }
    }

    /// `target`: the style's current target accuracy, percent.
    pub fn start_values(&self, key: &str, style: &str, target: f64) -> StartValues {
        let Some(s) = self.songs.get(key) else {
            return StartValues::default();
        };
        let target_adjust = s.styles.get(style).map_or(0.0, |m| {
            ((target - m.last_accuracy) / 2.0).clamp(-TARGET_ADJUST_MAX, TARGET_ADJUST_MAX)
        });
        StartValues {
            offset_ms: s.offset_ms,
            lane_ms: s.lane_ms,
            class_ms: s.class_ms,
            target_adjust,
            max_note: s.max_note,
        }
    }

    /// Missing file: empty book. Unreadable file: empty book, the file kept as `<path>.bad`.
    pub fn load(path: &Path) -> Self {
        let Ok(bytes) = std::fs::read(path) else {
            return Self::default();
        };
        serde_json::from_slice(&bytes).unwrap_or_else(|_| {
            let mut bad = path.as_os_str().to_owned();
            bad.push(".bad");
            let _ = std::fs::rename(path, bad);
            Self::default()
        })
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        let bytes = serde_json::to_vec_pretty(self).map_err(io::Error::other)?;
        crate::config::write_atomic(path, &bytes)
    }
}

/// A title as OCR could have read it: letters and digits only, upper case, digit look-alikes
/// folded to letters.
fn canon(t: &str) -> String {
    t.chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| match c.to_ascii_uppercase() {
            '0' => 'O',
            '1' => 'I',
            '5' => 'S',
            '8' => 'B',
            u => u,
        })
        .collect()
}

fn edits(a: &[u8], b: &[u8]) -> usize {
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, &x) in a.iter().enumerate() {
        let mut cur = vec![i + 1];
        for (j, &y) in b.iter().enumerate() {
            cur.push(
                (prev[j] + usize::from(x != y))
                    .min(prev[j + 1] + 1)
                    .min(cur[j] + 1),
            );
        }
        prev = cur;
    }
    prev[b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_song_one_entry_despite_ocr() {
        let mut b = SongBook::default();
        let play = |when| PlayRecord {
            name: "Deathmatch",
            difficulty: "Hard",
            when,
            readbacks: 30,
            offset_ms: -2.0,
            lane_ms: [0.0; 4],
            class_ms: [0.0; 3],
            style: "max",
            target: 100.0,
            final_accuracy: Some(100.0),
            max_note: None,
        };
        let k = b.resolve(&SongBook::key("VS Matt WIIK 3 Deathmatch Hard"));
        b.record_play(&k, &play("2026-11-25T01:00:00Z"));
        // read again with a 3 as S and a missing space: the same entry
        let k2 = b.resolve(&SongBook::key("VS MATT WIIK S DEATHMATCH HARD"));
        assert_eq!(k2, k);
        b.record_play(&k2, &play("2026-11-25T02:00:00Z"));
        assert_eq!((b.songs.len(), b.songs[&k].plays), (1, 2));
        // a different song stays apart
        assert_ne!(
            b.resolve(&SongBook::key("VS Matt WIIK 3 Deathmatch Hard VIP")),
            k
        );
        // two entries saved apart earlier are merged into the one with more plays
        b.songs.insert(
            "vs matt wiik3 deathmatch hard".into(),
            Song {
                plays: 1,
                ..Song::default()
            },
        );
        let k3 = b.resolve(&SongBook::key("vs matt wiik 3 deathmatch hard"));
        assert_eq!((b.songs.len(), b.songs[&k3].plays), (1, 3));
    }

    #[test]
    fn bias_zero_without_data_and_with_uniform_data() {
        let mut b = Bias::new();
        assert_eq!(b.lanes_ms(), [0.0; 4]);
        assert_eq!(b.classes_ms(), [0.0; 3]);
        for l in 0..4 {
            for _ in 0..30 {
                b.add(l, Class::Single, 7.0);
            }
        }
        assert!(b.lanes_ms().iter().all(|x| x.abs() < 1e-12));
    }

    #[test]
    fn bias_lane_relative_and_shrunk() {
        let mut b = Bias::new();
        for _ in 0..100 {
            b.add(0, Class::Single, 10.0); // late lane
            b.add(1, Class::Single, 0.0);
            b.add(2, Class::Single, 0.0);
            b.add(3, Class::Single, 0.0);
        }
        // overall 2.5; lane 0 rel 7.5 * 100/110
        let c = b.lanes_ms();
        assert!((c[0] + 7.5 * 100.0 / 110.0).abs() < 1e-9);
        assert!((c[1] - 2.5 * 100.0 / 110.0).abs() < 1e-9);
        // the corrections cancel up to shrinkage
        assert!(c.iter().sum::<f64>().abs() < 0.5);
        // under MIN_SAMPLES of its own a lane or class is not corrected at all
        let mut f = Bias::new();
        for _ in 0..MIN_SAMPLES - 1 {
            f.add(0, Class::Jack, 20.0);
            f.add(1, Class::Single, 0.0);
        }
        assert_eq!(f.lane_ms(0), 0.0);
        assert_eq!(f.class_ms(Class::Jack), 0.0);
        f.add(0, Class::Jack, 20.0);
        assert!(f.lane_ms(0) < -5.0);
    }

    #[test]
    fn bias_class() {
        let mut b = Bias::new();
        for _ in 0..50 {
            b.add(0, Class::Chord, 6.0);
            b.add(1, Class::Single, 0.0);
        }
        assert!(b.class_ms(Class::Chord) < -2.0);
        assert!(b.class_ms(Class::Single) > 2.0);
        assert_eq!(b.class_ms(Class::Jack), 0.0);
        b.add(0, Class::Single, f64::NAN);
        assert_eq!(b.samples(), 100);
    }

    #[test]
    fn key_normalises() {
        assert_eq!(
            SongBook::key("  Hello,  WORLD!! (Remix) "),
            "hello world remix"
        );
        assert_eq!(SongBook::key("Hello World  Remix"), "hello world remix");
        assert_eq!(SongBook::key("!!!"), "");
    }

    fn rec<'a>(readbacks: u32, off: f32, acc: Option<f64>) -> PlayRecord<'a> {
        PlayRecord {
            name: "Song",
            difficulty: "Hard",
            when: "2026-11-23T10:00:00Z",
            readbacks,
            offset_ms: off,
            lane_ms: [1.0, 0.0, 0.0, -1.0],
            class_ms: [0.0, -2.0, 0.5],
            style: "casual",
            target: 90.0,
            final_accuracy: acc,
            max_note: None,
        }
    }

    #[test]
    fn record_and_start_values() {
        let mut bk = SongBook::default();
        assert_eq!(bk.start_values("a", "casual", 90.0), StartValues::default());
        bk.record_play("a", &rec(5, -3.0, Some(87.0)));
        let v = bk.start_values("a", "casual", 90.0);
        assert_eq!(v.offset_ms, None); // too few readbacks
        assert_eq!(v.lane_ms, [0.0; 4]);
        assert!((v.target_adjust - 1.5).abs() < 1e-9);
        bk.record_play("a", &rec(40, -3.0, Some(80.0)));
        let v = bk.start_values("a", "casual", 90.0);
        assert_eq!(v.offset_ms, Some(-3.0));
        assert_eq!(v.lane_ms[0], 1.0);
        assert_eq!(v.target_adjust, 2.0); // clamped
        assert_eq!(bk.get("a").unwrap().plays, 2);
        assert_eq!(bk.start_values("a", "casual", 70.0).target_adjust, -2.0);
        assert_eq!(bk.start_values("a", "other", 90.0).target_adjust, 0.0);
    }

    #[test]
    fn max_note_kept_for_max_only() {
        let mut bk = SongBook::default();
        let mut m = MaxNote::default();
        m.lane[2].good = 4;
        let mut r = rec(30, 0.0, None);
        r.max_note = Some(m);
        bk.record_play("a", &r);
        assert_eq!(bk.start_values("a", "max", 100.0).max_note, None);
        r.style = "max";
        bk.record_play("a", &r);
        let got = bk.start_values("a", "max", 100.0).max_note.unwrap();
        assert_eq!(got.lane[2].good, 4);
    }

    #[test]
    fn file_roundtrip_unknown_fields_and_bad_file() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("songs.json");
        assert_eq!(SongBook::load(&p), SongBook::default());
        std::fs::write(
            &p,
            r#"{"version":1,"future":{"x":1},"songs":{"a":{"name":"A","plays":3,"later":true}}}"#,
        )
        .unwrap();
        let mut bk = SongBook::load(&p);
        assert_eq!(bk.get("a").unwrap().plays, 3);
        bk.record_play("a", &rec(30, 1.0, Some(95.0)));
        bk.save(&p).unwrap();
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(text.contains("future") && text.contains("later"));
        assert_eq!(SongBook::load(&p), bk);
        std::fs::write(&p, "{ not json").unwrap();
        assert_eq!(SongBook::load(&p), SongBook::default());
        assert!(d.path().join("songs.json.bad").exists());
    }
}
