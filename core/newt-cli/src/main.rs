//! Headless driver for `newt-core`.
//!
//! Proves the emulator against a real shell with no UI in the loop: spawn a
//! session, run a command, dump the resulting grid as text.
//!
//!     cargo run -p newt-cli -- "ls -1 | head -3"
//!     cargo run -p newt-cli -- --cols 100 --rows 30 "echo hi"
//!     cargo run -p newt-cli -- --shell /bin/sh "echo hi"
//!
//! Everything after a bare `--` is a program and its arguments, run directly
//! instead of typed at a shell prompt:
//!
//!     cargo run -p newt-cli -- --env FOO=bar -- /bin/sh -c 'echo $FOO'
//!
//! `--agents` lists the agent CLIs installed here; `--agent claude` starts one
//! in the grid, which is how the launch recipe is exercised without a window:
//!
//!     cargo run -p newt-cli -- --agents
//!     cargo run -p newt-cli -- --agent claude --settle 3000

use std::time::{Duration, Instant};

use newt_agent::{detect, AgentBridge, AgentKind};
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
    let Args {
        size,
        command,
        trace_enabled,
        quiet_period,
        keys,
        resize_to,
        shell,
        program_args,
        env,
        exec_mode,
        agent,
        list_agents,
        watch_seconds,
    } = parse_args(&args);

    if list_agents {
        print_installed_agents();
        return;
    }

    // An agent replaces the program, its arguments, and its environment, so it
    // is resolved before the config rather than merged into one.
    let (shell, program_args, env, env_remove, exec_mode, agent_cwd, mailbox) = match &agent {
        Some(name) => match resolve_agent(name) {
            Ok((plan, mailbox)) => (
                Some(plan.program),
                plan.args,
                plan.env,
                plan.env_remove,
                true,
                plan.cwd,
                mailbox,
            ),
            Err(message) => {
                eprintln!("newt: {message}");
                std::process::exit(1);
            }
        },
        None => (shell, program_args, env, Vec::new(), exec_mode, None, None),
    };

    let config = SessionConfig {
        size,
        shell,
        args: program_args,
        env,
        env_remove,
        cwd: agent_cwd,
        agent_mailbox: mailbox,
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

    // Wait for the prompt before typing at it. A login shell with a heavy
    // startup can take seconds to emit its first byte, and an empty screen
    // must not be mistaken for a settled one. `--shell /bin/sh` sidesteps that
    // entirely when the point is to test newt rather than someone's dotfiles.
    settle(&session, WaitFor::FirstOutput, quiet_period);

    // With an explicit program there is no prompt to type at — it is already
    // running the thing under test, and typing would send the command into
    // whatever it is doing.
    if !exec_mode {
        if let Err(e) = session.write(format!("{command}\n").as_bytes()) {
            eprintln!("newt: {e}");
            std::process::exit(1);
        }
        settle(&session, WaitFor::Quiet, quiet_period);
    }

    // Typed steps go through the key-encoding path rather than raw writes, so
    // `--trace` shows exactly what a keystroke puts on the wire.
    for step in &keys {
        if let Err(e) = send_step(&session, step) {
            eprintln!("newt: {e}");
            std::process::exit(1);
        }
        match watch_seconds {
            // Settling waits for the screen to go *quiet*, which for an agent
            // means waiting for the whole turn to finish — by which time every
            // transition worth watching has already happened. A short fixed
            // pause instead, just enough for the TUI to take the keystroke.
            Some(_) => std::thread::sleep(Duration::from_millis(400)),
            None => settle(&session, WaitFor::Quiet, quiet_period),
        }
    }

    if let Some(seconds) = watch_seconds {
        watch_agent(&session, seconds);
    }

    if let Some(new_size) = resize_to {
        if let Err(e) = session.resize(new_size) {
            eprintln!("newt: {e}");
            std::process::exit(1);
        }
        settle(&session, WaitFor::Quiet, quiet_period);
    }

    let (text, cursor) = session.with_emulator(|e| (e.visible_string(), e.cursor()));
    println!("{text}");
    let final_size = session.with_emulator(|e| e.size());
    eprintln!(
        "\n[{}x{} cursor {},{} exited={}]",
        final_size.cols,
        final_size.rows,
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

#[allow(clippy::type_complexity)]
/// Parsed command line.
///
/// A struct rather than a tuple: this had grown to six positional fields, and
/// the seventh is the one that decides *which shell runs*, which is far too
/// important to identify by position.
struct Args {
    size: SizeInCells,
    command: String,
    trace_enabled: bool,
    quiet_period: Duration,
    keys: Vec<String>,
    resize_to: Option<SizeInCells>,
    /// Program to run. `None` uses the login shell, which is the right default
    /// for a driver of a real terminal; pass `--shell /bin/sh` for a run that
    /// must not depend on the invoking user's configuration.
    shell: Option<String>,
    /// Arguments for the program, excluding argv[0].
    program_args: Vec<String>,
    /// Variables added to the child's inherited environment.
    env: Vec<(String, String)>,
    /// True when a bare `--` named the program, so nothing is typed at it.
    exec_mode: bool,
    /// Agent CLI to run instead of a shell, by name.
    agent: Option<String>,
    /// Print the installed agents and exit.
    list_agents: bool,
    /// Seconds to report agent state changes for, before dumping the grid.
    watch_seconds: Option<u64>,
}

/// Print every agent newt can find, with the absolute path it resolved to.
///
/// The path is the interesting half: a bare name would resolve against
/// launchd's minimal `PATH` once newt runs as a bundle, so seeing where
/// detection actually landed is what makes a bundle-only failure diagnosable.
fn print_installed_agents() {
    let installed = detect::installed();
    if installed.is_empty() {
        println!("no agent CLIs found");
        println!("searched:");
        for directory in detect::search_path() {
            println!("  {}", directory.display());
        }
        return;
    }

    for kind in installed {
        match detect::find(kind) {
            Some(path) => println!("{} -> {}", kind.display_name(), path.display()),
            None => println!("{} -> (vanished between calls)", kind.display_name()),
        }
    }
}

/// The `newt-hook` helper, as a sibling of this executable.
///
/// The same rule the app bundle follows: whoever built the binary put the
/// helper next to it, so nothing has to search. `None` when it is absent,
/// which registers no hooks rather than pointing Claude Code at a command that
/// does not exist — every tool call would then run a missing program.
fn hook_helper() -> Option<std::path::PathBuf> {
    let candidate = std::env::current_exe().ok()?.parent()?.join("newt-hook");
    candidate.is_file().then_some(candidate)
}

/// Resolve an agent by name into a launch plan and a mailbox for its reports.
fn resolve_agent(
    name: &str,
) -> Result<(newt_agent::LaunchPlan, Option<newt_agent::Mailbox>), String> {
    let kind = match name.to_ascii_lowercase().as_str() {
        "claude" => AgentKind::Claude,
        other => return Err(format!("unknown agent {other:?}")),
    };

    let program =
        detect::find(kind).ok_or_else(|| format!("{} is not installed", kind.display_name()))?;

    // A missing bridge is not fatal: the agent runs and simply reports nothing.
    let bridge = AgentBridge::shared();
    let token = bridge.map(|_| AgentBridge::new_token());
    let mailbox = match (bridge, &token) {
        (Some(bridge), Some(token)) => Some(bridge.register(token.clone())),
        _ => None,
    };

    let helper = hook_helper();
    if helper.is_none() {
        eprintln!("newt: newt-hook is not beside this binary; no state will be reported");
    }

    let runtime_dir = std::env::temp_dir()
        .join("newt-cli")
        .join(token.clone().unwrap_or_else(|| "no-bridge".to_string()));

    let plan = newt_agent::plan(&newt_agent::LaunchRequest {
        kind,
        program,
        cwd: std::env::current_dir().ok(),
        fork_from: None,
        hook_helper: helper.filter(|_| bridge.is_some()),
        runtime_dir,
        socket_path: bridge.map(|b| b.socket_path().to_path_buf()),
        session_token: token,
    })
    .map_err(|e| format!("could not prepare the agent launch: {e}"))?;

    Ok((plan, mailbox))
}

/// Print the session's agent metadata whenever it changes.
///
/// The headless equivalent of watching the sidebar, and what proves the hook
/// path end to end without a window: the state table can be checked against a
/// real Claude Code session rather than against fixtures alone.
fn watch_agent(session: &newt_core::Session, seconds: u64) {
    let deadline = Instant::now() + Duration::from_secs(seconds);
    let mut last = String::new();

    while Instant::now() < deadline && !session.has_exited() {
        let metadata = session.metadata();
        let line = format!(
            "state={:?} model={} in={} out={} cost_micros={} title={} id={}",
            metadata.agent_state,
            metadata.model.as_deref().unwrap_or("-"),
            metadata.input_tokens,
            metadata.output_tokens,
            metadata.cost_micros,
            metadata.agent_title.as_deref().unwrap_or("-"),
            metadata.agent_session_id.as_deref().unwrap_or("-"),
        );
        if line != last {
            eprintln!("[agent] {line}");
            last = line;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

fn parse_args(args: &[String]) -> Args {
    let mut cols = 80u16;
    let mut rows = 24u16;
    let mut trace = false;
    let mut quiet_period_ms = DEFAULT_QUIET_PERIOD_MS;
    let mut command = String::from("echo newt is alive");
    let mut keys: Vec<String> = Vec::new();
    let mut resize_to: Option<SizeInCells> = None;
    let mut shell: Option<String> = None;
    let mut program_args: Vec<String> = Vec::new();
    let mut env: Vec<(String, String)> = Vec::new();
    let mut exec_mode = false;
    let mut agent: Option<String> = None;
    let mut list_agents = false;
    let mut watch_seconds: Option<u64> = None;

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
            "--shell" => {
                shell = args.get(i + 1).cloned();
                i += 2;
            }
            "--env" => {
                // KEY=VALUE, split at the first `=` so a value may contain one.
                if let Some((key, value)) = args.get(i + 1).and_then(|v| v.split_once('=')) {
                    env.push((key.to_string(), value.to_string()));
                }
                i += 2;
            }
            // Everything after a bare `--` is the program and its arguments.
            // Consuming the rest unconditionally is safe here precisely because
            // this marker says so — unlike `--keys`, which must stop at the
            // next flag.
            "--" => {
                let rest = &args[(i + 1)..];
                if let Some((first, remainder)) = rest.split_first() {
                    shell = Some(first.clone());
                    program_args = remainder.to_vec();
                    exec_mode = true;
                }
                i = args.len();
            }
            "--agent" => {
                agent = args.get(i + 1).cloned();
                i += 2;
            }
            "--agents" => {
                list_agents = true;
                i += 1;
            }
            "--watch" => {
                watch_seconds = args.get(i + 1).and_then(|v| v.parse().ok()).or(Some(30));
                i += 2;
            }
            "--trace" => {
                trace = true;
                i += 1;
            }
            // Resize after the typed steps, to exercise SIGWINCH and reflow.
            "--resize" => {
                let cols = args.get(i + 1).and_then(|v| v.parse().ok());
                let rows = args.get(i + 2).and_then(|v| v.parse().ok());
                if let (Some(cols), Some(rows)) = (cols, rows) {
                    resize_to = Some(SizeInCells::new(cols, rows));
                }
                i += 3;
            }
            // Steps after --keys are typed, one per argument, stopping at the
            // next flag. Stopping matters: consuming the rest unconditionally
            // silently types later flags into the program under test.
            "--keys" => {
                let mut end = i + 1;
                while end < args.len() && !args[end].starts_with("--") {
                    end += 1;
                }
                keys = args[(i + 1)..end].to_vec();
                i = end;
            }
            other => {
                command = other.to_string();
                i += 1;
            }
        }
    }

    Args {
        size: SizeInCells::new(cols, rows),
        command,
        trace_enabled: trace,
        quiet_period: Duration::from_millis(quiet_period_ms),
        keys,
        resize_to,
        shell,
        program_args,
        env,
        exec_mode,
        agent,
        list_agents,
        watch_seconds,
    }
}
