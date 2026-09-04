//! The command Claude Code runs on every hook event.
//!
//! Reads the payload on stdin, forwards it to newt's bridge, exits.
//!
//! # This runs inside someone else's agent session
//!
//! That constraint decides everything here:
//!
//! - **It always exits 0.** A non-zero exit from a `PreToolUse` hook *blocks
//!   the tool call*. A bug in newt must never be able to stop the user's agent
//!   from working.
//! - **It never writes to stdout.** Claude Code interprets hook stdout for
//!   several events. Diagnostics go to stderr, and only when
//!   `NEWT_HOOK_DEBUG` is set.
//! - **It gives up quickly.** Every socket operation has a short timeout, so a
//!   dead or wedged bridge costs milliseconds rather than hanging a tool call.
//! - **It does not parse the payload.** Copying bytes through is the whole job;
//!   anything it does not do cannot go wrong.
//!
//! If newt is not listening — not running, crashed, or never started a bridge —
//! this exits silently and the agent session continues with no state reporting.

use std::io::Read;
use std::time::Duration;

use newt_agent::ipc;
use newt_agent::launch::{SOCKET_ENV, TOKEN_ENV};

/// Beyond this, give up rather than delay a tool call.
const TIMEOUT: Duration = Duration::from_millis(200);
/// Refuse to buffer more than the bridge would accept anyway.
const MAX_PAYLOAD: usize = ipc::MAX_FRAME_BYTES;

fn main() {
    // Every path ends here. `report` is a no-op unless debugging is on.
    if let Err(reason) = forward() {
        report(&reason);
    }
    std::process::exit(0);
}

fn forward() -> Result<(), String> {
    let socket = std::env::var(SOCKET_ENV).map_err(|_| format!("{SOCKET_ENV} is not set"))?;
    let token = std::env::var(TOKEN_ENV).unwrap_or_default();

    let mut payload = Vec::new();
    std::io::stdin()
        .take(MAX_PAYLOAD as u64)
        .read_to_end(&mut payload)
        .map_err(|e| format!("reading stdin: {e}"))?;

    send(&socket, token.as_bytes(), &payload)
}

#[cfg(unix)]
fn send(socket: &str, token: &[u8], payload: &[u8]) -> Result<(), String> {
    use std::os::unix::net::UnixStream;

    let mut stream = UnixStream::connect(socket).map_err(|e| format!("connect: {e}"))?;
    stream
        .set_write_timeout(Some(TIMEOUT))
        .map_err(|e| format!("timeout: {e}"))?;

    ipc::write_frame(&mut stream, token, payload).map_err(|e| format!("write: {e}"))?;
    // Tell the reader the payload is complete, rather than leaving it waiting
    // on a connection this process is about to drop anyway.
    let _ = stream.shutdown(std::net::Shutdown::Write);
    Ok(())
}

#[cfg(not(unix))]
fn send(_socket: &str, _token: &[u8], _payload: &[u8]) -> Result<(), String> {
    Err("no transport on this platform".to_string())
}

/// Diagnostics, on stderr, only when asked for.
fn report(reason: &str) {
    if std::env::var("NEWT_HOOK_DEBUG").is_ok() {
        eprintln!("newt-hook: {reason}");
    }
}
