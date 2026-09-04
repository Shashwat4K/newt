//! The listener every hook process reports to.
//!
//! # One bridge, not one per session
//!
//! A single socket and a single accept thread serve every tab. The alternative
//! — a socket per session — costs a thread and a lifetime per tab and buys
//! nothing, because the envelope already carries a token saying which session
//! a payload belongs to.
//!
//! That token is why `NEWT_SESSION_TOKEN` exists. Claude Code's own
//! `session_id` cannot do the job: it is not known until `SessionStart` fires,
//! and `--fork-session` deliberately mints a new one, so a child tab's id is
//! not knowable in advance by anybody.
//!
//! # No callbacks
//!
//! Payloads land in a per-session mailbox that the owner drains when it reads
//! its metadata. Nothing calls into the shell, which keeps the `CLAUDE.md`
//! rule about the hot path intact and means the bridge never blocks on a
//! consumer that is busy drawing.

use std::collections::HashMap;
use std::io;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use crate::ipc;
use crate::update::MetadataUpdate;

/// Where a session's pending updates collect until it reads them.
pub type Mailbox = Arc<Mutex<MetadataUpdate>>;

/// How often the accept loop checks whether it has been asked to stop.
const ACCEPT_POLL: Duration = Duration::from_millis(100);

/// The process-wide hook listener.
pub struct AgentBridge {
    socket_path: PathBuf,
    sessions: Arc<Mutex<HashMap<String, Mailbox>>>,
    stop: Arc<AtomicBool>,
    listener: Mutex<Option<std::thread::JoinHandle<()>>>,
}

static SHARED: OnceLock<Option<AgentBridge>> = OnceLock::new();

impl AgentBridge {
    /// The shared bridge, started on first use.
    ///
    /// `None` when the listener could not be created — a session then runs
    /// with no state reporting rather than failing to start. A terminal that
    /// refuses to open a shell because a sidebar indicator is unavailable
    /// would be a poor trade.
    pub fn shared() -> Option<&'static AgentBridge> {
        SHARED.get_or_init(|| AgentBridge::start().ok()).as_ref()
    }

    /// Bind a socket and begin accepting hook connections.
    pub fn start() -> io::Result<Self> {
        Self::start_in(ipc::ensure_socket_dir()?)
    }

    /// Start in a specific directory.
    ///
    /// Separated so tests get a directory of their own. Sharing one is not
    /// merely untidy: `start` sweeps stale sockets by *connecting* to them,
    /// and a sweep that reaches into a directory another bridge is serving
    /// disturbs a live listener — which is how this surfaced, as different
    /// tests failing on each run rather than one failing consistently.
    pub fn start_in(directory: PathBuf) -> io::Result<Self> {
        std::fs::create_dir_all(&directory)?;
        sweep_stale_sockets(&directory);

        let socket_path = directory.join(format!("{}.sock", nonce()));
        ipc::check_socket_path(&socket_path)?;

        let sessions: Arc<Mutex<HashMap<String, Mailbox>>> = Arc::new(Mutex::new(HashMap::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let listener = spawn_listener(&socket_path, Arc::clone(&sessions), Arc::clone(&stop))?;

        Ok(Self {
            socket_path,
            sessions,
            stop,
            listener: Mutex::new(Some(listener)),
        })
    }

    pub fn socket_path(&self) -> &std::path::Path {
        &self.socket_path
    }

    /// Register a session and return the mailbox its reports will land in.
    pub fn register(&self, token: impl Into<String>) -> Mailbox {
        let mailbox: Mailbox = Arc::new(Mutex::new(MetadataUpdate::default()));
        if let Ok(mut sessions) = self.sessions.lock() {
            sessions.insert(token.into(), Arc::clone(&mailbox));
        }
        mailbox
    }

    pub fn unregister(&self, token: &str) {
        if let Ok(mut sessions) = self.sessions.lock() {
            sessions.remove(token);
        }
    }

    /// A token no other live session is using.
    pub fn new_token() -> String {
        nonce()
    }

    pub fn stop(&self) {
        self.stop.store(true, Ordering::Release);
        if let Ok(mut handle) = self.listener.lock() {
            if let Some(handle) = handle.take() {
                let _ = handle.join();
            }
        }
        let _ = std::fs::remove_file(&self.socket_path);
    }
}

impl Drop for AgentBridge {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Ok(mut handle) = self.listener.lock() {
            if let Some(handle) = handle.take() {
                let _ = handle.join();
            }
        }
        let _ = std::fs::remove_file(&self.socket_path);
    }
}

#[cfg(unix)]
fn spawn_listener(
    socket_path: &std::path::Path,
    sessions: Arc<Mutex<HashMap<String, Mailbox>>>,
    stop: Arc<AtomicBool>,
) -> io::Result<std::thread::JoinHandle<()>> {
    use std::os::unix::net::UnixListener;

    let _ = std::fs::remove_file(socket_path);
    let listener = UnixListener::bind(socket_path)?;
    listener.set_nonblocking(true)?;

    Ok(std::thread::spawn(move || {
        while !stop.load(Ordering::Acquire) {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    // Served on this thread rather than a spawned one, so
                    // hooks are processed in the order they connected. Order
                    // matters: a `Stop` overtaking a `PreToolUse` would leave a
                    // tab spinning forever.
                    // Bounded so a connection that never sends anything —
                    // a stale-socket probe, or any local process poking at
                    // it — costs the loop a quarter second rather than
                    // blocking hook delivery indefinitely.
                    let _ = stream.set_read_timeout(Some(Duration::from_millis(250)));
                    if let Ok((token, payload)) = ipc::read_frame(&mut stream) {
                        deliver(&sessions, &token, &payload);
                    }
                }
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                    std::thread::sleep(ACCEPT_POLL);
                }
                Err(_) => break,
            }
        }
    }))
}

#[cfg(not(unix))]
fn spawn_listener(
    _socket_path: &std::path::Path,
    _sessions: Arc<Mutex<HashMap<String, Mailbox>>>,
    _stop: Arc<AtomicBool>,
) -> io::Result<std::thread::JoinHandle<()>> {
    // Windows wants a named pipe. Absorbed here, never exposed through the ABI.
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "the agent bridge needs a named pipe implementation on this platform",
    ))
}

/// Parse a payload and fold it into the session's mailbox.
fn deliver(sessions: &Arc<Mutex<HashMap<String, Mailbox>>>, token: &[u8], payload: &[u8]) {
    let Ok(token) = std::str::from_utf8(token) else {
        return;
    };
    let Some(outcome) = crate::hooks::parse(payload) else {
        return;
    };

    let mailbox = match sessions.lock() {
        // Cloned out so the map is not held while touching the mailbox, which
        // the owning session may be reading at the same moment.
        Ok(sessions) => sessions.get(token).map(Arc::clone),
        Err(_) => None,
    };

    if let Some(mailbox) = mailbox {
        if let Ok(mut pending) = mailbox.lock() {
            pending.merge(outcome.update);
        }
    }
}

/// Remove sockets left behind by a newt that crashed.
///
/// A stale socket file refuses connections; a live one accepts. Connecting is
/// the only reliable way to tell them apart, and leaving them would let the
/// directory grow without bound across crashes.
#[cfg(unix)]
fn sweep_stale_sockets(directory: &std::path::Path) {
    use std::os::unix::net::UnixStream;

    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("sock") {
            continue;
        }
        if UnixStream::connect(&path).is_err() {
            let _ = std::fs::remove_file(&path);
        }
    }
}

#[cfg(not(unix))]
fn sweep_stale_sockets(_directory: &std::path::Path) {}

/// Sixteen hex characters of randomness.
///
/// Not the process id: two newt instances, or one that crashed and restarted
/// with the same pid recycled, would collide.
fn nonce() -> String {
    let mut bytes = [0u8; 8];
    if let Ok(mut file) = std::fs::File::open("/dev/urandom") {
        use std::io::Read;
        let _ = file.read_exact(&mut bytes);
    }
    // Mixed with the clock so a urandom failure still yields distinct values.
    let clock = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as u64)
        .unwrap_or_default();
    let value = u64::from_le_bytes(bytes) ^ clock.rotate_left(17);
    format!("{value:016x}")
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::net::UnixStream;

    /// A bridge in a directory of its own, cleaned up on drop.
    ///
    /// Under `/tmp`, not `std::env::temp_dir()`. On macOS that resolves to
    /// `/var/folders/…`, which is long enough on its own to blow `sun_path`'s
    /// 104 bytes — the very reason the production socket directory is
    /// `/tmp/newt-<uid>`. The length check caught this when the test helper
    /// first tried it, which is the check doing its job.
    struct TestBridge {
        bridge: AgentBridge,
        directory: PathBuf,
    }

    impl std::ops::Deref for TestBridge {
        type Target = AgentBridge;
        fn deref(&self) -> &AgentBridge {
            &self.bridge
        }
    }

    impl Drop for TestBridge {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.directory);
        }
    }

    /// Each test gets its own directory: `start` sweeps stale sockets by
    /// *connecting* to them, so a shared directory means tests disturbing each
    /// other's live listeners.
    fn bridge() -> TestBridge {
        let directory = PathBuf::from(format!("/tmp/newt-t-{}", AgentBridge::new_token()));
        let bridge = AgentBridge::start_in(directory.clone()).expect("start");
        TestBridge { bridge, directory }
    }

    fn send(socket: &std::path::Path, token: &str, payload: &str) {
        let mut stream = UnixStream::connect(socket).expect("connect");
        ipc::write_frame(&mut stream, token.as_bytes(), payload.as_bytes()).expect("write");
    }

    fn wait_for(mailbox: &Mailbox, predicate: impl Fn(&MetadataUpdate) -> bool) -> bool {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while std::time::Instant::now() < deadline {
            if let Ok(pending) = mailbox.lock() {
                if predicate(&pending) {
                    return true;
                }
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        false
    }

    #[test]
    fn a_hook_payload_reaches_the_registered_session() {
        let bridge = bridge();
        let mailbox = bridge.register("token-a");

        send(
            bridge.socket_path(),
            "token-a",
            r#"{"hook_event_name":"UserPromptSubmit","session_id":"abc"}"#,
        );

        assert!(wait_for(&mailbox, |update| {
            update.agent_state == Some(crate::update::AgentStateHint::Running)
        }));
        assert!(wait_for(&mailbox, |u| u.agent_session_id.as_deref() == Some("abc")));
    }

    #[test]
    fn payloads_are_routed_by_token_and_never_to_the_wrong_tab() {
        let bridge = bridge();
        let first = bridge.register("one");
        let second = bridge.register("two");

        send(
            bridge.socket_path(),
            "one",
            r#"{"hook_event_name":"Notification"}"#,
        );

        assert!(wait_for(&first, |u| {
            u.agent_state == Some(crate::update::AgentStateHint::Waiting)
        }));
        assert!(
            second.lock().unwrap().is_empty(),
            "delivered to the wrong session"
        );
    }

    #[test]
    fn several_payloads_fold_into_one_pending_update() {
        let bridge = bridge();
        let mailbox = bridge.register("folding");
        let socket = bridge.socket_path().to_path_buf();

        send(
            &socket,
            "folding",
            r#"{"hook_event_name":"SessionStart","session_id":"s1","transcript_path":"/tmp/t.jsonl"}"#,
        );
        assert!(wait_for(&mailbox, |u| u.agent_session_id.is_some()));
        send(
            &socket,
            "folding",
            r#"{"hook_event_name":"UserPromptSubmit"}"#,
        );

        // The later state wins; the earlier identifiers survive. This is what
        // makes it safe to read the mailbox at whatever rate the UI polls.
        assert!(wait_for(&mailbox, |u| {
            u.agent_state == Some(crate::update::AgentStateHint::Running)
        }));
        let pending = mailbox.lock().unwrap();
        assert_eq!(pending.agent_session_id.as_deref(), Some("s1"));
        assert_eq!(pending.transcript_path, Some(PathBuf::from("/tmp/t.jsonl")));
    }

    #[test]
    fn an_unknown_token_is_dropped_rather_than_creating_a_session() {
        let bridge = bridge();
        let mailbox = bridge.register("known");

        send(
            bridge.socket_path(),
            "stranger",
            r#"{"hook_event_name":"Stop"}"#,
        );
        std::thread::sleep(Duration::from_millis(200));

        assert!(mailbox.lock().unwrap().is_empty());
    }

    #[test]
    fn garbage_on_the_socket_does_not_stop_the_listener() {
        // Any process running as this user can connect. One that writes
        // nonsense must not take the bridge down for every tab.
        let bridge = AgentBridge::start().expect("start");
        let mailbox = bridge.register("survivor");

        {
            let mut stream = UnixStream::connect(bridge.socket_path()).unwrap();
            use std::io::Write;
            let _ = stream.write_all(b"\xff\xff\xff\xff garbage");
        }
        let _ = UnixStream::connect(bridge.socket_path());

        send(
            bridge.socket_path(),
            "survivor",
            r#"{"hook_event_name":"Stop"}"#,
        );
        assert!(wait_for(&mailbox, |u| {
            u.agent_state == Some(crate::update::AgentStateHint::Idle)
        }));
    }

    #[test]
    fn unregistering_stops_delivery() {
        let bridge = bridge();
        let mailbox = bridge.register("leaving");
        bridge.unregister("leaving");

        send(
            bridge.socket_path(),
            "leaving",
            r#"{"hook_event_name":"Stop"}"#,
        );
        std::thread::sleep(Duration::from_millis(200));

        assert!(mailbox.lock().unwrap().is_empty());
    }

    #[test]
    fn stopping_the_bridge_removes_its_socket() {
        let bridge = bridge();
        let path = bridge.socket_path().to_path_buf();
        assert!(path.exists());

        bridge.stop();
        assert!(!path.exists());
    }

    #[test]
    fn tokens_do_not_repeat() {
        let tokens: std::collections::HashSet<String> =
            (0..64).map(|_| AgentBridge::new_token()).collect();
        assert_eq!(tokens.len(), 64);
    }
}
