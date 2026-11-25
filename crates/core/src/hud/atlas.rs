//! Glyph atlas: normalised 12x16 cells per (face, char), whole-word templates for row
//! labels, SAD matching, atomic binary save (`glyphs/<profile>.bin`, spec 08).

use std::io;
use std::path::{Path, PathBuf};

use super::text::{CH, CW, Cell, WW};

/// A font style on screen. Each face has its own glyph shapes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Face {
    /// Left stats panel and the judgment counts (same font).
    Stats = 0,
    /// Yellow "-143.26 ms" readout.
    Ms = 1,
    /// Big score numbers in the bottom corners.
    Score = 2,
    /// "Stats from last song" panel.
    Results = 3,
}

impl Face {
    fn from_u8(v: u8) -> Option<Face> {
        [Face::Stats, Face::Ms, Face::Score, Face::Results]
            .get(v as usize)
            .copied()
    }
}

#[derive(Clone, Debug)]
struct Glyph {
    face: Face,
    ch: char,
    ar: f32,
    bits: [u8; CW * CH],
}

#[derive(Clone, Debug)]
struct Word {
    face: Face,
    name: String,
    ncells: u8,
    ar: f32,
    bits: [u8; WW * CH],
}

/// Example glyphs kept per (face, char); a new one is added only if it differs.
/// Best and runner-up (other character) closer than this: the read is rejected.
const AMBIGUOUS: f32 = 0.02;
const MAX_EXAMPLES: usize = 6;
const MAGIC: &[u8; 4] = b"AHGL";
/// 2: atlases saved before the Mean MS sign fix (spec 13, 1.4) may hold mis-taught digits.
const VERSION: u8 = 2;

#[derive(Clone, Debug, Default)]
pub struct Atlas {
    glyphs: Vec<Glyph>,
    words: Vec<Word>,
}

fn sad(a: &[u8], b: &[u8]) -> f32 {
    a.iter()
        .zip(b)
        .map(|(&x, &y)| x.abs_diff(y) as u32)
        .sum::<u32>() as f32
        / (a.len() as f32 * 255.0)
}

/// Aspect mismatch counts as much as 0.4 of a full SAD per unit.
fn dist(sad_: f32, ar_a: f32, ar_b: f32) -> f32 {
    sad_ + 0.4 * (ar_a - ar_b).abs()
}

impl Atlas {
    pub fn is_empty(&self) -> bool {
        self.glyphs.is_empty() && self.words.is_empty()
    }

    pub fn chars(&self, face: Face) -> String {
        let mut v: Vec<char> = self
            .glyphs
            .iter()
            .filter(|g| g.face == face)
            .map(|g| g.ch)
            .collect();
        v.sort_unstable();
        v.dedup();
        v.into_iter().collect()
    }

    /// Replace `face`'s glyphs with `from`'s (an empty atlas forgets them); labels stay.
    pub fn set_face(&mut self, face: Face, from: &Atlas) {
        self.glyphs.retain(|g| g.face != face);
        self.glyphs
            .extend(from.glyphs.iter().filter(|g| g.face == face).cloned());
    }

    pub fn add_glyph(&mut self, face: Face, ch: char, c: &Cell) {
        let same: Vec<&Glyph> = self
            .glyphs
            .iter()
            .filter(|g| g.face == face && g.ch == ch)
            .collect();
        if same.len() >= MAX_EXAMPLES || same.iter().any(|g| sad(&g.bits, &c.bits) < 0.02) {
            return;
        }
        self.glyphs.push(Glyph {
            face,
            ch,
            ar: c.ar,
            bits: c.bits,
        });
    }

    pub fn add_word(
        &mut self,
        face: Face,
        name: &str,
        ncells: usize,
        bits: [u8; WW * CH],
        ar: f32,
    ) {
        self.words.retain(|w| !(w.face == face && w.name == name));
        self.words.push(Word {
            face,
            name: name.into(),
            ncells: ncells as u8,
            ar,
            bits,
        });
    }

    /// Best glyph for a cell: (char, distance). A near tie with a different character
    /// (3 against 5) adds a penalty, so ambiguous cells are rejected rather than guessed.
    pub fn classify(&self, face: Face, c: &Cell) -> Option<(char, f32)> {
        let (mut best, mut second): (Option<(char, f32)>, f32) = (None, f32::INFINITY);
        for g in self.glyphs.iter().filter(|g| g.face == face) {
            let d = dist(sad(&g.bits, &c.bits), g.ar, c.ar);
            match best {
                Some((ch, bd)) if d < bd => {
                    if ch != g.ch {
                        second = second.min(bd);
                    }
                    best = Some((g.ch, d));
                }
                Some((ch, _)) if ch != g.ch => second = second.min(d),
                Some(_) => {}
                None => best = Some((g.ch, d)),
            }
        }
        best.map(|(ch, d)| (ch, if second - d < AMBIGUOUS { d + 0.5 } else { d }))
    }

    /// Label template for `name`: (number of cells it spans, its bitmap, aspect).
    pub fn word(&self, face: Face, name: &str) -> Option<(usize, &[u8; WW * CH], f32)> {
        self.words
            .iter()
            .find(|w| w.face == face && w.name == name)
            .map(|w| (w.ncells as usize, &w.bits, w.ar))
    }

    pub fn word_dist(&self, bits: &[u8; WW * CH], ar: f32, tpl: &([u8; WW * CH], f32)) -> f32 {
        dist(sad(bits, &tpl.0), ar, tpl.1)
    }

    pub fn path(profile: &str) -> Option<PathBuf> {
        crate::config::data_dir().map(|d| d.join("glyphs").join(format!("{profile}.bin")))
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut o = MAGIC.to_vec();
        o.push(VERSION);
        o.extend((self.glyphs.len() as u32).to_le_bytes());
        for g in &self.glyphs {
            o.push(g.face as u8);
            o.extend((g.ch as u32).to_le_bytes());
            o.extend(g.ar.to_le_bytes());
            o.extend(g.bits);
        }
        o.extend((self.words.len() as u32).to_le_bytes());
        for w in &self.words {
            o.push(w.face as u8);
            o.push(w.name.len() as u8);
            o.extend(w.name.as_bytes());
            o.push(w.ncells);
            o.extend(w.ar.to_le_bytes());
            o.extend(w.bits);
        }
        o
    }

    pub fn from_bytes(b: &[u8]) -> Option<Atlas> {
        struct R<'a>(&'a [u8]);
        impl<'a> R<'a> {
            fn take(&mut self, n: usize) -> Option<&'a [u8]> {
                let (a, b) = self.0.split_at_checked(n)?;
                self.0 = b;
                Some(a)
            }
            fn u32(&mut self) -> Option<u32> {
                Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
            }
            fn f32(&mut self) -> Option<f32> {
                Some(f32::from_bits(self.u32()?))
            }
            fn u8(&mut self) -> Option<u8> {
                Some(self.take(1)?[0])
            }
        }
        let mut r = R(b);
        if r.take(4)? != MAGIC || r.u8()? != VERSION {
            return None;
        }
        let mut a = Atlas::default();
        for _ in 0..r.u32()? {
            let face = Face::from_u8(r.u8()?)?;
            let ch = char::from_u32(r.u32()?)?;
            let ar = r.f32()?;
            let bits = r.take(CW * CH)?.try_into().ok()?;
            a.glyphs.push(Glyph { face, ch, ar, bits });
        }
        for _ in 0..r.u32()? {
            let face = Face::from_u8(r.u8()?)?;
            let n = r.u8()? as usize;
            let name = String::from_utf8(r.take(n)?.to_vec()).ok()?;
            let ncells = r.u8()?;
            let ar = r.f32()?;
            let bits = r.take(WW * CH)?.try_into().ok()?;
            a.words.push(Word {
                face,
                name,
                ncells,
                ar,
                bits,
            });
        }
        Some(a)
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        crate::config::write_atomic(path, &self.to_bytes())
    }

    /// Missing or unreadable file gives an empty atlas (the HUD is then not readable).
    pub fn load(path: &Path) -> Atlas {
        std::fs::read(path)
            .ok()
            .and_then(|b| Atlas::from_bytes(&b))
            .unwrap_or_default()
    }
}
