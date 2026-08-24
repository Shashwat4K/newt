//! Headless driver for `newt-core`.
//!
//! Proves the emulator against a real shell with no UI in the loop: spawn a
//! session, run a command, dump the resulting grid as text.
//!
//!     cargo run -p newt-cli -- "ls -1 | head -3"
//!     cargo run -p newt-cli -- --cols 100 --rows 30 "echo hi"

use std::time::{Duration, Instant};

use newt_core::input::{Key, KeyEvent};
use newt_core::{Direction, SessionConfig, SizeInCells};

/// Default time output must stay unchanged before the screen is considered
/// settled. Slow-starting full-screen programs need more; see `--settle`.
const DEFAULT_QUIET_PERIOD_MS: u64 = 300;
/// Upper bound on waiting, so a command that never settles still returns.
const MAX_WAIT: Duration = Duration::from_secs(10);
const POLL_INTERVAL: Duration = Duration::from_millis(20);

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (size, command, trace_enabled, quiet_period, keys) = parse_args(&args);

    let config = SessionConfig {
        size,
        ..SessionConfig::default()
    };

    let trace: Option<newt_core::Trace> = if trace_enabled {
        Some(Box::new(|direction, bytes| {
            let arrow = match direction {
                Direction::Output => "<--",
                Direction::Input => "-->",
            };
            eprintln!("{arrow} {}", escape(bytes));
        }))
    } else {
        None
    };

    let session = match newt_core::Session::spawn_with_trace(config, trace) {
        Ok(session) => session,
        Err(e) => {
            eprintln!("newt: {e}");
            std::process::exit(1);
        }
    };

    // Wait for the prompt before typing at it. A heavy startup (oh-my-zsh,
    // powerlevel10k) can take seconds to emit its first byte, and an empty
    // screen must not be mistaken for a settled one.
    settle(&session, WaitFor::FirstOutput, quiet_period);

    if let Err(e) = session.write(format!("{command}\n").as_bytes()) {
        eprintln!("newt: {e}");
        std::process::exit(1);
    }
    settle(&session, WaitFor::Quiet, quiet_period);

    // Typed steps go through the key-encoding path rather than raw writes, so
    // `--trace` shows exactly what a keystroke puts on the wire.
    for step in &keys {
        if let Err(e) = send_step(&session, step) {
            eprintln!("newt: {e}");
            std::process::exit(1);
        }
        settle(&session, WaitFor::Quiet, quiet_period);
    }

    let (text, cursor) = session.with_emulator(|e| (e.visible_string(), e.cursor()));
    println!("{text}");
    eprintln!(
        "\n[{}x{} cursor {},{} exited={}]",
        size.cols,
        size.rows,
        cursor.0,
        cursor.1,
        session.has_exited()
    );
}

#[derive(PartialEq)]
enum WaitFor {
    /// Settle only after the screen has produced something.
    FirstOutput,
    /// Settle as soon as output stops changing, even if nothing arrived.
    Quiet,
}

/// Wait until the screen stops changing, or the deadline passes.
fn settle(session: &newt_core::Session, wait_for: WaitFor, quiet_period: Duration) {
    let deadline = Instant::now() + MAX_WAIT;
    let mut last = session.with_emulator(|e| e.visible_string());
    let mut seen_output = !last.is_empty();
    let mut unchanged_since = Instant::now();

    while Instant::now() < deadline {
        std::thread::sleep(POLL_INTERVAL);
        let current = session.with_emulator(|e| e.visible_string());

        if current != last {
            last = current;
            seen_output = true;
            unchanged_since = Instant::now();
            continue;
        }

        let waiting_on_first_output = wait_for == WaitFor::FirstOutput && !seen_output;
        if !waiting_on_first_output && unchanged_since.elapsed() >= quiet_period {
            return;
        }
    }
}

/// Send one step as key events. `<name>` is a named key; anything else is
/// typed character by character.
fn send_step(session: &newt_core::Session, step: &str) -> newt_core::Result<()> {
    if let Some(name) = step.strip_prefix('<').and_then(|s| s.strip_suffix('>')) {
        let key = match name.to_ascii_lowercase().as_str() {
            "enter" | "return" => Key::Enter,
            "escape" | "esc" => Key::Escape,
            "tab" => Key::Tab,
            "backspace" => Key::Backspace,
            "up" => Key::Up,
            "down" => Key::Down,
            "left" => Key::Left,
            "right" => Key::Right,
            "home" => Key::Home,
            "end" => Key::End,
            _ => return Ok(()),
        };
        return session.send_key(KeyEvent::new(key, 0));
    }

    for character in step.chars() {
        session.send_key(KeyEvent::new(Key::Char(character), 0))?;
    }
    Ok(())
}

/// Render bytes readably: printable ASCII as-is, everything else escaped.
fn escape(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        match b {
            0x1b => out.push_str("\\e"),
            b'\n' => out.push_str("\\n"),
            b'\r' => out.push_str("\\r"),
            0x07 => out.push_str("\\a"),
            0x20..=0x7e => out.push(b as char),
            _ => out.push_str(&format!("\\x{b:02x}")),
        }
    }
    out
}

fn parse_args(args: &[String]) -> (SizeInCells, String, bool, Duration, Vec<String>) {
    let mut cols = 80u16;
    let mut rows = 24u16;
    let mut trace = false;
    let mut quiet_period_ms = DEFAULT_QUIET_PERIOD_MS;
    let mut command = String::from("echo newt is alive");
    let mut keys: Vec<String> = Vec::new();

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--cols" => {
                cols = args.get(i + 1).and_then(|v| v.parse().ok()).unwrap_or(cols);
                i += 2;
            }
            "--rows" => {
                rows = args.get(i + 1).and_then(|v| v.parse().ok()).unwrap_or(rows);
                i += 2;
            }
            "--settle" => {
                quiet_period_ms = args
                    .get(i + 1)
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(quiet_period_ms);
                i += 2;
            }
            "--trace" => {
                trace = true;
                i += 1;
            }
            // Everything after --keys is typed, one step per argument.
            "--keys" => {
                keys = args[(i + 1)..].to_vec();
                i = args.len();
            }
            other => {
                command = other.to_string();
                i += 1;
            }
        }
    }

    (
        SizeInCells::new(cols, rows),
        command,
        trace,
        Duration::from_millis(quiet_period_ms),
        keys,
    )
}
