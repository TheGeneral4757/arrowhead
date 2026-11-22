//! Synthetic frame generator (spec 09). Shapes and colours are random on purpose:
//! nothing about the real skin may be baked into detection.

use crate::config::Scroll;
use crate::detect::Opp;
use crate::frame::{OwnedFrame, Rect};
use crate::time::Nanos;

/// splitmix64.
pub struct Rng(pub u64);

impl Rng {
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    pub fn f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }
    pub fn range(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * self.f64()
    }
    pub fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n as u64) as usize
    }
}

fn hash(x: i32, y: i32, t: u64) -> u32 {
    let mut h =
        (x as u32).wrapping_mul(0x9E37_79B1) ^ (y as u32).wrapping_mul(0x85EB_CA77) ^ t as u32;
    h ^= h >> 15;
    h = h.wrapping_mul(0x2C1B_3C6D);
    h ^ (h >> 12)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Shape {
    Circle,
    Square,
    Arrow,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Bg {
    Black,
    Noise,
    Gradient,
}

#[derive(Clone, Copy, Debug)]
pub struct Note {
    pub lane: u8,
    pub t: f64,
    /// 0 for a tap.
    pub hold: f64,
}

#[derive(Clone, Debug)]
pub struct Scene {
    pub w: u32,
    pub h: u32,
    pub fps: f64,
    pub scroll: Scroll,
    /// px per ms.
    pub speed: f64,
    pub size: f32,
    pub pitch: f32,
    /// Our receptor row: x centre of the group, y of the receptors.
    pub centre: [f32; 2],
    pub shape: Shape,
    pub heads: [[u8; 3]; 4],
    pub trail: [u8; 3],
    /// Trail width as a fraction of `size`.
    pub trail_w: f32,
    pub receptor: [u8; 3],
    pub popup: [u8; 3],
    pub bg: Bg,
    pub opponent: bool,
    pub jitter_ms: f64,
    pub drop: f64,
}

fn opp(c: [u8; 3]) -> Opp {
    Opp::from_rgb(c[0], c[1], c[2])
}

impl Scene {
    /// A random scene: our group centred, a smaller opponent group on the right.
    pub fn random(rng: &mut Rng, w: u32, h: u32, scroll: Scroll, speed: f64) -> Scene {
        let size = h as f32 * rng.range(0.08, 0.11) as f32;
        let pitch = size * rng.range(1.3, 1.6) as f32;
        let ry = match scroll {
            Scroll::Up => h as f32 * rng.range(0.1, 0.16) as f32,
            Scroll::Down => h as f32 * rng.range(0.84, 0.9) as f32,
        };
        let shape = [Shape::Circle, Shape::Square, Shape::Arrow][rng.below(3)];
        let bg = [Bg::Black, Bg::Noise, Bg::Gradient][rng.below(3)];
        let receptor = {
            let v = rng.range(110.0, 200.0) as u8;
            [v, v, v]
        };
        let trail = if rng.f64() < 0.5 {
            let v = rng.range(80.0, 140.0) as u8;
            [v, v, v]
        } else {
            loop {
                let c = [0, 0, 0].map(|_| rng.range(40.0, 255.0) as u8);
                if (0..=50)
                    .step_by(10)
                    .all(|g| opp(c).dist(Opp(0, 0, g)) >= 40)
                {
                    break c;
                }
            }
        };
        let mut taken = vec![opp(receptor), opp(trail)];
        let pick = |rng: &mut Rng, taken: &mut Vec<Opp>| loop {
            let c = [0, 0, 0].map(|_| rng.below(256) as u8);
            let o = opp(c);
            if (0..=60).step_by(10).all(|g| o.dist(Opp(0, 0, g)) >= 70)
                && taken.iter().all(|t| t.dist(o) >= 70)
            {
                taken.push(o);
                break c;
            }
        };
        let heads = [0; 4].map(|_| pick(rng, &mut taken));
        let popup = pick(rng, &mut taken);
        Scene {
            w,
            h,
            fps: 60.0,
            scroll,
            speed,
            size,
            pitch,
            centre: [w as f32 / 2.0, ry],
            shape,
            heads,
            trail,
            trail_w: rng.range(0.35, 0.5) as f32,
            receptor,
            popup,
            bg,
            opponent: true,
            jitter_ms: 1.0,
            drop: 0.02,
        }
    }

    pub fn approach(&self) -> f32 {
        match self.scroll {
            Scroll::Up => 1.0,
            Scroll::Down => -1.0,
        }
    }

    pub fn receptor(&self, lane: usize) -> [f32; 2] {
        [
            self.centre[0] + (lane as f32 - 1.5) * self.pitch,
            self.centre[1],
        ]
    }

    /// Centre y of a note due at `t_note`, at time `t`.
    pub fn note_y(&self, t_note: f64, t: f64) -> f32 {
        self.centre[1] + self.approach() * ((t_note - t) * self.speed) as f32
    }

    /// Indices of notes possibly on screen at `t` (chart sorted by time).
    pub fn window_range(&self, chart: &[Note], t: f64) -> std::ops::Range<usize> {
        let lo = chart.partition_point(|n| n.t < t - 4100.0);
        let hi = chart.partition_point(|n| n.t <= t + self.h as f64 / self.speed + 100.0);
        lo..hi.max(lo)
    }

    pub fn window<'a>(&self, chart: &'a [Note], t: f64) -> &'a [Note] {
        &chart[self.window_range(chart, t)]
    }

    /// Frame times (true time ms, reported pts) with jitter and drops.
    pub fn frames(&self, rng: &mut Rng, from_ms: f64, to_ms: f64) -> Vec<(f64, Nanos)> {
        let dt = 1000.0 / self.fps;
        let mut out = Vec::new();
        let mut t = from_ms;
        while t < to_ms {
            if rng.f64() >= self.drop {
                let pts = t + rng.range(-self.jitter_ms, self.jitter_ms);
                out.push((t, Nanos(((pts + 10_000.0) * 1e6) as u64)));
            }
            t += dt;
        }
        out
    }

    fn inside(&self, lane: usize, dx: f32, dy: f32, r: f32) -> bool {
        let (x, y) = (dx / r, dy / r);
        match self.shape {
            Shape::Circle => x * x + y * y <= 1.0,
            Shape::Square => x.abs() <= 0.85 && y.abs() <= 0.85,
            Shape::Arrow => {
                // Up arrow in (u, v), tip at v = -1; rotated per lane: left, down, up, right.
                let (u, v) = match lane {
                    0 => (y, x),
                    1 => (x, -y),
                    2 => (x, y),
                    _ => (y, -x),
                };
                (-1.0..=0.05).contains(&v) && u.abs() <= (v + 1.0) * 0.95
                    || (0.05..=1.0).contains(&v) && u.abs() <= 0.4
            }
        }
    }

    /// Renders `region`. Background is painted only inside `paint` (all of `region` if `None`).
    pub fn render(
        &self,
        chart: &[Note],
        t: f64,
        region: Rect,
        paint: Option<&[Rect]>,
    ) -> OwnedFrame {
        let (rw, rh) = (region.w as usize, region.h as usize);
        let mut data = vec![0u8; rw * rh * 4];
        let tick = (t * 1000.0) as u64;
        let put = |data: &mut [u8], x: i32, y: i32, c: [u8; 3], noise: i32| {
            let i = ((y - region.y) as usize * rw + (x - region.x) as usize) * 4;
            let n = if noise > 0 {
                (hash(x, y, tick) % (2 * noise as u32 + 1)) as i32 - noise
            } else {
                0
            };
            for k in 0..3 {
                data[i + 2 - k] = (c[k] as i32 + n).clamp(0, 255) as u8;
            }
        };
        if self.bg != Bg::Black {
            for p in paint.unwrap_or(std::slice::from_ref(&region)) {
                let Some(p) = p.intersect(&region) else {
                    continue;
                };
                for y in p.y..p.y + p.h as i32 {
                    for x in p.x..p.x + p.w as i32 {
                        let c = match self.bg {
                            Bg::Noise => [20, 20, 26],
                            _ => {
                                let s = ((y as f64 / self.h as f64) * 3.0 + t / 1500.0).sin();
                                let v = (30.0 + 20.0 * s) as u8;
                                [v, v, v + 4]
                            }
                        };
                        put(
                            &mut data,
                            x,
                            y,
                            c,
                            if self.bg == Bg::Noise { 12 } else { 0 },
                        );
                    }
                }
            }
        }
        let blit = |data: &mut [u8],
                    cx: f32,
                    cy: f32,
                    r: f32,
                    f: &dyn Fn(f32, f32) -> bool,
                    c: [u8; 3]| {
            let b = Rect::new(
                (cx - r - 1.0) as i32,
                (cy - r - 1.0) as i32,
                (2.0 * r + 3.0) as u32,
                (2.0 * r + 3.0) as u32,
            );
            let Some(b) = b.intersect(&region) else {
                return;
            };
            for y in b.y..b.y + b.h as i32 {
                for x in b.x..b.x + b.w as i32 {
                    if f(x as f32 + 0.5 - cx, y as f32 + 0.5 - cy) {
                        put(data, x, y, c, 6);
                    }
                }
            }
        };
        let r = self.size / 2.0;
        let notes = self.window(chart, t);
        let lit = |lane: usize| {
            notes
                .iter()
                .any(|n| n.lane as usize == lane && t >= n.t && t < n.t + n.hold.max(80.0))
        };
        for lane in 0..4 {
            let [x, y] = self.receptor(lane);
            let (c, rr) = if lit(lane) {
                (self.heads[lane], r * 1.1)
            } else {
                (self.receptor, r)
            };
            blit(
                &mut data,
                x,
                y,
                rr,
                &|dx, dy| self.inside(lane, dx, dy, rr),
                c,
            );
        }
        if self.opponent {
            let (s, p) = (0.5, 0.5 * self.pitch);
            let ox = self.w as f32 * 0.82;
            for lane in 0..4 {
                let x = ox + (lane as f32 - 1.5) * p;
                let on = (t / 170.0 + lane as f64 * 1.7).sin() > 0.6;
                let c = if on { self.heads[lane] } else { self.receptor };
                blit(
                    &mut data,
                    x,
                    self.centre[1],
                    r * s,
                    &|dx, dy| self.inside(lane, dx, dy, r * s),
                    c,
                );
            }
        }
        // Popup between lanes 1 and 2 for 300 ms after a hit.
        if notes.iter().any(|n| t >= n.t && t < n.t + 300.0) {
            let (hw, hh) = (0.45 * self.pitch, 0.12 * self.size);
            let py = self.centre[1] + self.approach() * 0.35 * self.h as f32;
            blit(
                &mut data,
                self.centre[0],
                py,
                hw,
                &|dx, dy| dx.abs() <= hw && dy.abs() <= hh,
                self.popup,
            );
        }
        let tw = self.trail_w * self.size / 2.0;
        for n in notes {
            let lane = n.lane as usize;
            let end = n.t + n.hold;
            if n.hold <= 0.0 || t >= end {
                continue;
            }
            let x = self.receptor(lane)[0];
            let y0 = if t >= n.t {
                self.centre[1]
            } else {
                self.note_y(n.t, t)
            };
            let y1 = self.note_y(end, t);
            let (ya, yb) = (y0.min(y1), y0.max(y1));
            let half = (yb - ya) / 2.0;
            let my = (ya + yb) / 2.0;
            blit(
                &mut data,
                x,
                my,
                half.max(tw),
                &|dx, dy| dx.abs() <= tw && dy.abs() <= half,
                self.trail,
            );
        }
        // Farthest first, so nearer heads are on top.
        for n in notes.iter().rev() {
            let lane = n.lane as usize;
            if t >= n.t {
                continue;
            }
            let [x, _] = self.receptor(lane);
            let y = self.note_y(n.t, t);
            blit(
                &mut data,
                x,
                y,
                r,
                &|dx, dy| self.inside(lane, dx, dy, r),
                self.heads[lane],
            );
        }
        OwnedFrame {
            seq: tick,
            pts: Nanos(0),
            region,
            data,
        }
    }
}

/// Generated chart (spec 09): singles, streams up to 20 nps, 50 ms jacks, 2-4 note chords,
/// holds 100 ms-4 s with taps in other lanes. Sorted by time.
pub fn chart(rng: &mut Rng, notes: usize, start_ms: f64) -> Vec<Note> {
    let mut out: Vec<Note> = Vec::with_capacity(notes + 8);
    // Earliest time each lane is free again.
    let mut free = [start_ms; 4];
    let mut t = start_ms;
    let add = |out: &mut Vec<Note>, free: &mut [f64; 4], lane: usize, t: f64, hold: f64| {
        if t < free[lane] {
            return;
        }
        out.push(Note {
            lane: lane as u8,
            t,
            hold,
        });
        free[lane] = t + hold + if hold > 0.0 { 150.0 } else { 50.0 };
    };
    while out.len() < notes {
        match rng.below(6) {
            0 => {
                for _ in 0..8 {
                    t += rng.range(150.0, 400.0);
                    add(&mut out, &mut free, rng.below(4), t, 0.0);
                }
            }
            1 => {
                let dt = rng.range(50.0, 100.0);
                let mut lane = rng.below(4);
                for _ in 0..16 {
                    t += dt;
                    lane = (lane + 1 + rng.below(3)) % 4;
                    add(&mut out, &mut free, lane, t, 0.0);
                }
            }
            2 => {
                let lane = rng.below(4);
                t = t.max(free[lane]);
                for _ in 0..4 {
                    add(&mut out, &mut free, lane, t, 0.0);
                    t += 50.0;
                }
            }
            3 => {
                t += rng.range(150.0, 300.0);
                let k = 2 + rng.below(3);
                let first = rng.below(4);
                for i in 0..k {
                    add(&mut out, &mut free, (first + i) % 4, t, 0.0);
                }
            }
            _ => {
                t += rng.range(200.0, 400.0);
                let lane = rng.below(4);
                let hold = rng.range(100.0, 4000.0);
                add(&mut out, &mut free, lane, t, hold);
                let mut u = t;
                while u + 120.0 < t + hold {
                    u += rng.range(120.0, 300.0);
                    let other = (lane + 1 + rng.below(3)) % 4;
                    add(&mut out, &mut free, other, u, 0.0);
                }
                t = u;
            }
        }
        t += 200.0;
    }
    out.sort_by(|a, b| a.t.total_cmp(&b.t));
    out
}
