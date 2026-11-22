//! Click-through overlay process (v0.2, spec 07). The app writes one JSON array of `EngineEvent`s
//! per line on stdin; the overlay draws them over the game and exits when stdin closes.
//!
//! `arrowhead-overlay [--output NAME]`: NAME is the output (Wayland connector, e.g. DP-2, or a
//! Windows display name) the game runs on; the first output when missing or unknown.

mod model;
mod render;
#[cfg(target_os = "linux")]
mod wayland;
#[cfg(windows)]
mod win;

use std::io::BufRead;

use model::{Msg, parse_batch};

/// Feeds parsed batches to `send` until stdin closes or `send` returns false.
pub fn read_stdin(mut send: impl FnMut(Vec<Msg>) -> bool) {
    for line in std::io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        let msgs = parse_batch(&line);
        if !msgs.is_empty() && !send(msgs) {
            break;
        }
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let mut output = None;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--output" => output = args.next(),
            "-h" | "--help" => {
                eprintln!(
                    "usage: arrowhead-overlay [--output NAME]  (JSON event batches on stdin)"
                );
                return;
            }
            _ => {
                eprintln!("arrowhead-overlay: unknown argument {a}");
                std::process::exit(2);
            }
        }
    }
    #[cfg(target_os = "linux")]
    let res = wayland::run(output);
    #[cfg(windows)]
    let res = win::run(output);
    #[cfg(not(any(target_os = "linux", windows)))]
    let res: Result<(), String> = {
        let _ = output;
        Err("no overlay backend on this OS".into())
    };
    if let Err(e) = res {
        eprintln!("arrowhead-overlay: {e}");
        std::process::exit(1);
    }
}
