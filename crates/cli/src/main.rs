//! Headless entry point: capture, record, replay, detect, run, bench, diag.

use std::process::ExitCode;

// shortcut: hand-rolled arg match, move to clap when commands take real options (M2).
const COMMANDS: [(&str, &str); 7] = [
    ("capture", "Stream frames and print capture stats (M2)"),
    ("record", "Record the lane region to an .ahclip (M2)"),
    ("replay", "Play an .ahclip through the engine (M2)"),
    (
        "detect",
        "Print layout, side, scroll and note observations (M3)",
    ),
    ("run", "Play the game (M4)"),
    ("bench", "Benchmark the scan path (M3)"),
    ("diag", "Print diagnostics (M2)"),
];

fn usage() -> String {
    let mut s = format!(
        "arrowhead-cli {}\n\nUsage: arrowhead-cli <command>\n\nCommands:\n",
        arrowhead_core::VERSION
    );
    for (name, help) in COMMANDS {
        s += &format!("  {name:<8} {help}\n");
    }
    s + "\nOptions:\n  -h, --help     Print help\n  -V, --version  Print version\n"
}

fn main() -> ExitCode {
    let arg = std::env::args().nth(1);
    match arg.as_deref() {
        Some("-V" | "--version") => println!("arrowhead-cli {}", arrowhead_core::VERSION),
        None | Some("-h" | "--help") => print!("{}", usage()),
        Some(cmd) if COMMANDS.iter().any(|(n, _)| *n == cmd) => {
            eprintln!("{cmd}: not implemented yet");
            return ExitCode::from(2);
        }
        Some(other) => {
            eprint!("unknown command: {other}\n\n{}", usage());
            return ExitCode::from(2);
        }
    }
    ExitCode::SUCCESS
}
