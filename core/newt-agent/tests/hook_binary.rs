//! The `newt-hook` binary, run for real.
//!
//! These are the guarantee that newt cannot break someone's agent session.
//! Claude Code runs this command on every hook event, and a non-zero exit from
//! a `PreToolUse` hook *blocks the tool call* — so "exits 0 and says nothing,
//! whatever happens" is not a nicety, it is the contract. Asserting it against
//! the actual compiled binary, rather than against a function that resembles
//! it, is the point of testing it this way.

#![cfg(unix)]

use std::io::Write;
use std::os::unix::net::UnixListener;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

use newt_agent::ipc;
use newt_agent::launch::{SOCKET_ENV, TOKEN_ENV};

const HELPER: &str = env!("CARGO_BIN_EXE_newt-hook");

/// A short-lived directory under `/tmp`, for `sun_path`'s sake.
struct Scratch {
    root: PathBuf,
}

/// Distinguishes scratch directories within this process.
///
/// A timestamp is not enough: these tests run in parallel threads sharing one
/// pid, and two that start close together can read the same nanosecond — at
/// which point one test's cleanup deletes the other's socket, and the failure
/// appears in whichever test happened to lose.
static SCRATCH_SEQUENCE: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

impl Scratch {
    fn new() -> Self {
        let sequence = SCRATCH_SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let root = PathBuf::from(format!(
            "/tmp/newt-hooktest-{}-{sequence}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("scratch");
        Self { root }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// Run the helper with the given environment and stdin, returning
/// (exit code, stdout, stderr).
fn run_helper(env: &[(&str, &str)], stdin: &[u8]) -> (i32, String, String) {
    let mut command = Command::new(HELPER);
    command
        .env_remove(SOCKET_ENV)
        .env_remove(TOKEN_ENV)
        .env_remove("NEWT_HOOK_DEBUG")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (key, value) in env {
        command.env(key, value);
    }

    let mut child = command.spawn().expect("spawn newt-hook");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(stdin)
        .expect("write stdin");
    let output = child.wait_with_output().expect("wait");

    (
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

const PAYLOAD: &str = r#"{"hook_event_name":"Stop","session_id":"s-1"}"#;

#[test]
fn the_payload_reaches_a_listening_bridge() {
    let scratch = Scratch::new();
    let socket = scratch.path("bridge.sock");
    let listener = UnixListener::bind(&socket).expect("bind");

    let accepted = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept");
        // Tolerated rather than asserted: the helper writes and exits, so by
        // the time this accepts, the peer may already be gone and macOS
        // refuses the sockopt with EINVAL. Losing the timeout costs nothing —
        // a closed peer gives EOF immediately, so the read cannot hang.
        let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
        ipc::read_frame(&mut stream).expect("frame")
    });

    let (code, stdout, _) = run_helper(
        &[
            (SOCKET_ENV, socket.to_str().unwrap()),
            (TOKEN_ENV, "token-xyz"),
        ],
        PAYLOAD.as_bytes(),
    );

    let (token, payload) = accepted.join().expect("listener thread");
    assert_eq!(token, b"token-xyz");
    assert_eq!(payload, PAYLOAD.as_bytes());
    assert_eq!(code, 0);
    assert!(stdout.is_empty());
}

#[test]
fn it_exits_cleanly_when_newt_is_not_listening_at_all() {
    // The ordinary case for anyone running `claude` outside newt while a
    // settings file from a previous session is still around.
    let (code, stdout, stderr) = run_helper(&[], PAYLOAD.as_bytes());

    assert_eq!(code, 0, "a non-zero exit here would block tool calls");
    assert!(stdout.is_empty(), "stdout is interpreted by Claude Code");
    assert!(stderr.is_empty());
}

#[test]
fn it_exits_cleanly_when_the_socket_path_does_not_exist() {
    let scratch = Scratch::new();
    let missing = scratch.path("never-created.sock");

    let (code, stdout, stderr) = run_helper(
        &[(SOCKET_ENV, missing.to_str().unwrap()), (TOKEN_ENV, "t")],
        PAYLOAD.as_bytes(),
    );

    assert_eq!(code, 0);
    assert!(stdout.is_empty());
    assert!(stderr.is_empty());
}

#[test]
fn it_exits_cleanly_when_the_socket_is_a_stale_file() {
    // What a crashed newt leaves behind: the file exists, nothing accepts.
    let scratch = Scratch::new();
    let stale = scratch.path("stale.sock");
    std::fs::write(&stale, b"not a socket").expect("write");

    let (code, stdout, stderr) = run_helper(
        &[(SOCKET_ENV, stale.to_str().unwrap()), (TOKEN_ENV, "t")],
        PAYLOAD.as_bytes(),
    );

    assert_eq!(code, 0);
    assert!(stdout.is_empty());
    assert!(stderr.is_empty());
}

#[test]
fn it_exits_cleanly_when_the_bridge_binds_but_never_accepts() {
    // A wedged newt. The helper must not wait on it.
    let scratch = Scratch::new();
    let socket = scratch.path("deaf.sock");
    let _listener = UnixListener::bind(&socket).expect("bind");

    let started = std::time::Instant::now();
    let (code, stdout, _) = run_helper(
        &[(SOCKET_ENV, socket.to_str().unwrap()), (TOKEN_ENV, "t")],
        PAYLOAD.as_bytes(),
    );

    assert_eq!(code, 0);
    assert!(stdout.is_empty());
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "helper took {:?}; every hook event pays this latency",
        started.elapsed()
    );
}

#[test]
fn an_empty_payload_is_forwarded_rather_than_treated_as_an_error() {
    // The helper deliberately does not parse. Deciding what an empty payload
    // means is the bridge's job, and a helper that judged its input would be a
    // helper that can reject valid input.
    let scratch = Scratch::new();
    let socket = scratch.path("empty.sock");
    let listener = UnixListener::bind(&socket).expect("bind");

    let accepted = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept");
        // Tolerated rather than asserted: the helper writes and exits, so by
        // the time this accepts, the peer may already be gone and macOS
        // refuses the sockopt with EINVAL. Losing the timeout costs nothing —
        // a closed peer gives EOF immediately, so the read cannot hang.
        let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
        ipc::read_frame(&mut stream).expect("frame")
    });

    let (code, _, _) = run_helper(
        &[(SOCKET_ENV, socket.to_str().unwrap()), (TOKEN_ENV, "t")],
        b"",
    );

    let (_, payload) = accepted.join().expect("listener thread");
    assert!(payload.is_empty());
    assert_eq!(code, 0);
}

#[test]
fn diagnostics_are_opt_in_and_never_touch_stdout() {
    let (code, stdout, stderr) = run_helper(&[("NEWT_HOOK_DEBUG", "1")], PAYLOAD.as_bytes());

    assert_eq!(code, 0);
    assert!(
        stdout.is_empty(),
        "stdout must stay clean even when debugging"
    );
    assert!(
        stderr.contains(SOCKET_ENV),
        "expected a reason on stderr, got {stderr:?}"
    );
}
