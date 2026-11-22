//! Headless entry point: capture, record, replay, import, detect, run, bench, diag.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::time::{Duration, Instant};

use arrowhead_capture::{ClipHeader, ClipRecorder, ClipWriter, ReplaySource, clip::CLIP_VERSION};
use arrowhead_core::frame::{FrameSource, OwnedFrame, Rect};
use arrowhead_core::runtime::platform::monotonic_ns;
use arrowhead_core::time::Nanos;
use clap::{Parser, Subcommand};

mod detect;
mod latency;
mod run;

#[derive(Parser)]
#[command(name = "arrowhead-cli", version = arrowhead_core::VERSION, about = "arrowhead headless tools")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Stream frames from the screen and print capture stats
    Capture {
        #[arg(long, default_value_t = 10.0)]
        seconds: f32,
        /// Region to copy, x,y,w,h in stream pixels (default: whole screen)
        #[arg(long, value_parser = parse_rect)]
        region: Option<Rect>,
    },
    /// Record a region of the screen to an .ahclip
    Record {
        #[arg(short, long)]
        out: PathBuf,
        #[arg(long, default_value_t = 30.0)]
        seconds: f32,
        #[arg(long, value_parser = parse_rect)]
        region: Option<Rect>,
        #[arg(long, default_value = "")]
        notes: String,
    },
    /// Read an .ahclip through ReplaySource and print what it holds
    Replay {
        clip: PathBuf,
        /// Deliver frames at their recorded timing instead of as fast as possible
        #[arg(long)]
        paced: bool,
    },
    /// Convert a screen recording (any ffmpeg input) to an .ahclip
    Import {
        video: PathBuf,
        #[arg(short, long)]
        out: PathBuf,
        #[arg(long, value_parser = parse_rect)]
        region: Option<Rect>,
        /// Start time in seconds
        #[arg(long, default_value_t = 0.0)]
        start: f32,
        /// Length in seconds (default: to the end)
        #[arg(long)]
        seconds: Option<f32>,
    },
    /// Print environment and config locations
    Diag,
    /// Calibrate on a clip, then print layout, side, scroll and per-frame note observations
    Detect {
        clip: PathBuf,
        /// Seconds at the start of the clip used to find receptors and learn colours
        #[arg(long, default_value_t = 8.0)]
        learn: f32,
        /// Write an annotated PNG every `--every` frames into this directory
        #[arg(long)]
        debug_png: Option<PathBuf>,
        #[arg(long, default_value_t = 30)]
        every: usize,
        /// Summary only, no per-frame lines
        #[arg(short, long)]
        quiet: bool,
        /// Also run the tracker and planner and report what would be pressed (no keys)
        #[arg(long)]
        plan: bool,
    },
    /// Calibrate on the live screen, then play. Dry run (no keys) unless --live
    Run {
        /// Actually send keys (uinput). Without this nothing is pressed
        #[arg(long)]
        live: bool,
        #[arg(long, default_value_t = 60.0)]
        seconds: f32,
        /// Added to every press: negative presses earlier. Default: the profile's
        /// latency.game_bias_ms
        #[arg(long, allow_hyphen_values = true)]
        offset_ms: Option<f64>,
        /// Seconds of song used to learn colours before playing
        #[arg(long, default_value_t = 8.0)]
        learn: f32,
        /// Ignore the colours saved in the profile and learn them again (skin change)
        #[arg(long)]
        relearn: bool,
    },
    /// Show which window the focus guard thinks is active
    Focus {
        #[arg(long, default_value_t = 10.0)]
        seconds: f32,
    },
    /// Input latency test: this terminal flashes on a uinput key, the capture times it.
    /// Keep the terminal focused, large and on the captured monitor
    Latency {
        #[arg(long, default_value_t = 50)]
        trials: u32,
    },
    /// Measure scheduler lateness on the real clock (no keys sent)
    Bench {
        #[arg(long, default_value_t = 1000)]
        presses: u32,
    },
}

fn parse_rect(s: &str) -> Result<Rect, String> {
    let v: Vec<i64> = s
        .split(',')
        .map(|p| p.trim().parse::<i64>())
        .collect::<Result<_, _>>()
        .map_err(|e| e.to_string())?;
    match v[..] {
        [x, y, w, h] if w > 0 && h > 0 => Ok(Rect::new(x as i32, y as i32, w as u32, h as u32)),
        _ => Err("expected x,y,w,h with w, h > 0".into()),
    }
}

type Res = Result<(), Box<dyn std::error::Error>>;

fn main() -> ExitCode {
    let res = match Cli::parse().cmd {
        Cmd::Capture { seconds, region } => capture(seconds, region),
        Cmd::Record {
            out,
            seconds,
            region,
            notes,
        } => record(&out, seconds, region, notes),
        Cmd::Replay { clip, paced } => replay(&clip, paced),
        Cmd::Import {
            video,
            out,
            region,
            start,
            seconds,
        } => import(&video, &out, region, start, seconds),
        Cmd::Diag => diag(),
        Cmd::Detect {
            clip,
            learn,
            debug_png,
            every,
            quiet,
            plan,
        } => {
            if let Some(d) = &debug_png
                && let Err(e) = std::fs::create_dir_all(d)
            {
                eprintln!("error: {e}");
                return ExitCode::FAILURE;
            }
            detect::run(detect::Opts {
                clip: &clip,
                learn_s: learn,
                debug_png: debug_png.as_deref(),
                every,
                quiet,
                plan,
            })
        }
        Cmd::Run {
            live,
            seconds,
            offset_ms,
            learn,
            relearn,
        } => open_screen().and_then(|mut src| {
            run::run(
                &mut src,
                run::Opts {
                    live,
                    seconds,
                    offset_ms,
                    learn_s: learn,
                    relearn,
                },
            )
        }),
        Cmd::Focus { seconds } => run::focus(seconds),
        Cmd::Bench { presses } => run::bench(presses),
        Cmd::Latency { trials } => {
            open_screen().and_then(|mut src| latency::latency(&mut src, trials))
        }
    };
    match res {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

/// p-th percentile (0-100) of an unsorted slice.
fn pct(v: &mut [f64], p: f64) -> f64 {
    if v.is_empty() {
        return f64::NAN;
    }
    v.sort_by(|a, b| a.total_cmp(b));
    v[((v.len() - 1) as f64 * p / 100.0).round() as usize]
}

#[cfg(target_os = "linux")]
fn open_screen() -> Result<arrowhead_capture::pipewire::PipewireSource, Box<dyn std::error::Error>>
{
    use arrowhead_capture::pipewire::{PipewireSource, default_token_file};
    let token = default_token_file().ok_or("no data dir for the restore token")?;
    eprintln!("Opening screen capture (pick the game's monitor if asked)...");
    Ok(PipewireSource::open(&token)?)
}

#[cfg(not(target_os = "linux"))]
fn open_screen() -> Result<ReplaySource, Box<dyn std::error::Error>> {
    Err("screen capture on this OS arrives in v0.2".into())
}

fn capture(seconds: f32, region: Option<Rect>) -> Res {
    let mut src = open_screen()?;
    if let Some(r) = region {
        src.set_region(r);
    }
    let info = src.info();
    let n = (seconds * 1000.0) as usize;
    let (mut intervals, mut ages) = (Vec::with_capacity(n), Vec::with_capacity(n));
    let mut last: Option<Nanos> = None;
    let mut frames = 0u64;
    let mut out_region = Rect::default();
    let t0 = Instant::now();
    while t0.elapsed().as_secs_f32() < seconds {
        let Some(f) = src.next_frame(Duration::from_millis(500))? else {
            continue;
        };
        let now = monotonic_ns();
        ages.push(now.saturating_sub(f.pts.0) as f64 / 1e6);
        if let Some(p) = last {
            intervals.push(f.pts.saturating_sub(p).as_secs_f64() * 1e3);
        }
        last = Some(f.pts);
        out_region = f.region;
        frames += 1;
    }
    let secs = t0.elapsed().as_secs_f64();
    #[cfg(target_os = "linux")]
    let stats = src.stats();
    println!(
        "backend      {} ({}), stream {}x{} @ {:.1} Hz",
        info.backend, info.monitor, info.width, info.height, info.refresh_hz
    );
    println!(
        "region       {},{},{},{}",
        out_region.x, out_region.y, out_region.w, out_region.h
    );
    println!(
        "frames       {frames} in {secs:.2} s = {:.1} fps",
        frames as f64 / secs
    );
    println!(
        "interval ms  p50 {:.2}  p99 {:.2}",
        pct(&mut intervals, 50.0),
        pct(&mut intervals, 99.0)
    );
    println!(
        "pts age ms   p50 {:.2}  p99 {:.2}{}",
        pct(&mut ages, 50.0),
        pct(&mut ages, 99.0),
        if src.info().pts_is_receive_time {
            "  (no pts from compositor: receive time)"
        } else {
            ""
        }
    );
    #[cfg(target_os = "linux")]
    {
        let mut copy: Vec<f64> = stats.copy_ns.iter().map(|&c| c as f64 / 1e3).collect();
        println!(
            "copy us      p50 {:.0}  p99 {:.0}",
            pct(&mut copy, 50.0),
            pct(&mut copy, 99.0)
        );
        println!("drops        {}", stats.drops);
    }
    Ok(())
}

fn now_utc() -> String {
    // shortcut: seconds since epoch instead of RFC 3339; add a date crate when history needs real dates (M5).
    let s = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("unix:{s}")
}

fn record(out: &Path, seconds: f32, region: Option<Rect>, notes: String) -> Res {
    let mut src = open_screen()?;
    if let Some(r) = region {
        src.set_region(r);
    }
    // The first frame fixes the clip region (the requested one, clamped to the screen).
    let first = loop {
        if let Some(f) = src.next_frame(Duration::from_secs(1))? {
            break f.to_owned();
        }
    };
    let info = src.info();
    let header = ClipHeader {
        version: CLIP_VERSION,
        width: info.width,
        height: info.height,
        region: first.region,
        refresh_hz: info.refresh_hz,
        backend: info.backend.clone(),
        recorded_at: now_utc(),
        profile_snapshot: None,
        notes,
    };
    let mut rec = ClipRecorder::start(ClipWriter::create(out, &header)?, 256);
    rec.push(&first.view());
    let t0 = Instant::now();
    while t0.elapsed().as_secs_f32() < seconds {
        if let Some(f) = src.next_frame(Duration::from_millis(500))? {
            rec.push(&f);
        }
    }
    let drops = rec.drops;
    let written = rec.finish()?;
    let r = header.region;
    println!(
        "wrote {} frames ({drops} dropped by the writer queue), region {},{},{},{} to {}",
        written,
        r.x,
        r.y,
        r.w,
        r.h,
        out.display()
    );
    Ok(())
}

/// FNV-1a over all frame bytes, so two reads of a clip can be compared.
fn fnv(h: &mut u64, bytes: &[u8]) {
    for b in bytes {
        *h = (*h ^ *b as u64).wrapping_mul(0x100000001b3);
    }
}

fn replay(clip: &Path, paced: bool) -> Res {
    let mut src = ReplaySource::open(clip, paced)?;
    let h = src.header().clone();
    let (mut first, mut last, mut frames, mut hash) = (None, Nanos(0), 0u64, 0xcbf29ce484222325u64);
    let t0 = Instant::now();
    loop {
        match src.next_frame(Duration::from_secs(1)) {
            Ok(Some(f)) => {
                first.get_or_insert(f.pts);
                last = f.pts;
                frames += 1;
                let row = f.region.w as usize * 4;
                for y in 0..f.region.h as usize {
                    fnv(&mut hash, &f.data[y * f.stride..y * f.stride + row]);
                }
            }
            Ok(None) => continue,
            Err(arrowhead_core::frame::CaptureError::SourceLost) => break,
            Err(e) => return Err(e.into()),
        }
    }
    let span = first
        .map(|p| last.saturating_sub(p).as_secs_f64())
        .unwrap_or(0.0);
    let r = h.region;
    println!(
        "clip      {} (v{}, {}, {})",
        clip.display(),
        h.version,
        h.backend,
        h.recorded_at
    );
    println!(
        "stream    {}x{} @ {:.1} Hz, region {},{},{},{}",
        h.width, h.height, h.refresh_hz, r.x, r.y, r.w, r.h
    );
    println!(
        "frames    {frames} over {span:.2} s = {:.1} fps (read in {:.2} s)",
        if span > 0.0 {
            (frames - 1) as f64 / span
        } else {
            0.0
        },
        t0.elapsed().as_secs_f64()
    );
    println!("checksum  {hash:016x}");
    if !h.notes.is_empty() {
        println!("notes     {}", h.notes);
    }
    Ok(())
}

fn probe(video: &Path) -> Result<(u32, u32, f32), Box<dyn std::error::Error>> {
    let out = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=width,height,r_frame_rate",
            "-of",
            "csv=p=0",
        ])
        .arg(video)
        .output()
        .map_err(|e| format!("ffprobe: {e} (is ffmpeg installed?)"))?;
    let s = String::from_utf8(out.stdout)?;
    let p: Vec<&str> = s.trim().split(',').collect();
    let [w, h, r] = p[..] else {
        return Err(format!("ffprobe gave {s:?}").into());
    };
    let fps = match r.split_once('/') {
        Some((n, d)) => n.parse::<f32>()? / d.parse::<f32>()?,
        None => r.parse()?,
    };
    Ok((w.parse()?, h.parse()?, fps))
}

fn import(video: &Path, out: &Path, region: Option<Rect>, start: f32, seconds: Option<f32>) -> Res {
    let (w, h, fps) = probe(video)?;
    let full = Rect::new(0, 0, w, h);
    let r = match region {
        Some(r) => r.intersect(&full).ok_or("region is outside the video")?,
        None => full,
    };
    let mut cmd = Command::new("ffmpeg");
    cmd.args(["-v", "error", "-ss", &start.to_string()])
        .arg("-i")
        .arg(video);
    if let Some(s) = seconds {
        cmd.args(["-t", &s.to_string()]);
    }
    cmd.args([
        "-vf",
        &format!("crop={}:{}:{}:{}", r.w, r.h, r.x, r.y),
        "-fps_mode",
        "passthrough",
        "-pix_fmt",
        "bgr0",
        "-f",
        "rawvideo",
        "-",
    ]);
    let mut child = cmd
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|e| format!("ffmpeg: {e}"))?;
    let mut pipe = child.stdout.take().unwrap();

    let header = ClipHeader {
        version: CLIP_VERSION,
        width: w,
        height: h,
        region: r,
        refresh_hz: fps,
        backend: "import".into(),
        recorded_at: now_utc(),
        profile_snapshot: None,
        notes: format!(
            "imported from {} at {start} s",
            video.file_name().unwrap_or_default().to_string_lossy()
        ),
    };
    let mut writer = ClipWriter::create(out, &header)?;
    let mut frame = OwnedFrame {
        seq: 0,
        pts: Nanos(0),
        region: r,
        data: vec![0; r.w as usize * r.h as usize * 4],
    };
    // Constant-rate recordings: pts comes from the frame number.
    let period = 1e9 / fps as f64;
    let base = (start as f64 * 1e9) as u64;
    loop {
        match pipe.read_exact(&mut frame.data) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(e.into()),
        }
        frame.pts = Nanos(base + (frame.seq as f64 * period) as u64);
        frame.seq += 1;
        writer.push(&frame.view())?;
    }
    if !child.wait()?.success() {
        return Err("ffmpeg failed".into());
    }
    let n = writer.frames();
    writer.finish()?;
    println!(
        "wrote {n} frames ({}x{} region at {fps:.2} fps) to {}",
        r.w,
        r.h,
        out.display()
    );
    Ok(())
}

fn diag() -> Res {
    use arrowhead_core::config::{config_dir, data_dir};
    let env = |k: &str| std::env::var(k).unwrap_or_else(|_| "-".into());
    println!("arrowhead-cli {}", arrowhead_core::VERSION);
    println!(
        "os            {} {}",
        std::env::consts::OS,
        std::env::consts::ARCH
    );
    println!(
        "desktop       {} ({})",
        env("XDG_CURRENT_DESKTOP"),
        env("XDG_SESSION_TYPE")
    );
    let show = |name: &str, p: Option<PathBuf>| match p {
        Some(p) => println!(
            "{name:<13} {} ({})",
            p.display(),
            if p.exists() { "exists" } else { "missing" }
        ),
        None => println!("{name:<13} unknown"),
    };
    show("config dir", config_dir());
    show("data dir", data_dir());
    show(
        "restore token",
        data_dir().map(|d| d.join("screencast_token")),
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rect_parsing() {
        assert_eq!(parse_rect("1,2,3,4"), Ok(Rect::new(1, 2, 3, 4)));
        assert!(parse_rect("1,2,0,4").is_err());
        assert!(parse_rect("1,2,3").is_err());
    }

    #[test]
    fn percentile() {
        let mut v = vec![5.0, 1.0, 3.0, 2.0, 4.0];
        assert_eq!(pct(&mut v, 50.0), 3.0);
        assert_eq!(pct(&mut v, 100.0), 5.0);
    }
}
