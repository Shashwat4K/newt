//! C ABI over `newt-core`.
//!
//! The surface is deliberately narrow and byte-oriented: plain data crosses the
//! boundary, there are no callbacks on the hot path, no object graphs, and no
//! platform types in any signature. `cbindgen` generates `newt.h` from this
//! file, so the header is never edited by hand.
//!
//! # Memory and lifetimes
//!
//! - A session handle is owned by the caller and must be released exactly once
//!   with [`newt_session_free`].
//! - Pointers inside [`NewtSnapshot`] borrow buffers owned by the session. They
//!   stay valid until the next [`newt_session_snapshot`] call on that session,
//!   or until the session is freed. Copy anything you need to keep.
//! - Strings returned by this API are NUL-terminated, owned by the session, and
//!   valid until the next call that can replace them.
//!
//! # Errors
//!
//! Fallible calls return `false` (or null) and record a message retrievable
//! with [`newt_last_error`] on the same thread.

use std::cell::RefCell;
use std::ffi::{c_char, CStr, CString};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::PathBuf;

use newt_core::input::{self, Key, KeyEvent, MouseEvent, MouseKind};
use newt_core::metadata::{AgentState, SessionMetadata};
use newt_core::snapshot::{cursor_shape, flags, Cell, Cursor, DamagedRow};
use newt_core::{
    SelectionMode, Session as CoreSession, SessionConfig, SizeInCells, Snapshot as CoreSnapshot,
};

// Cell attribute bits and cursor shapes, restated as literals so `cbindgen`
// can emit them into the header — it cannot evaluate cross-crate paths. The
// const assertions below make drift a compile error rather than a silent
// mismatch between the header and the core.
pub const NEWT_FLAG_BOLD: u16 = 1 << 0;
pub const NEWT_FLAG_ITALIC: u16 = 1 << 1;
pub const NEWT_FLAG_UNDERLINE: u16 = 1 << 2;
pub const NEWT_FLAG_DOUBLE_UNDERLINE: u16 = 1 << 3;
pub const NEWT_FLAG_UNDERCURL: u16 = 1 << 4;
pub const NEWT_FLAG_DOTTED_UNDERLINE: u16 = 1 << 5;
pub const NEWT_FLAG_DASHED_UNDERLINE: u16 = 1 << 6;
pub const NEWT_FLAG_STRIKEOUT: u16 = 1 << 7;
pub const NEWT_FLAG_DIM: u16 = 1 << 8;
pub const NEWT_FLAG_HIDDEN: u16 = 1 << 9;
pub const NEWT_FLAG_WIDE: u16 = 1 << 10;
pub const NEWT_FLAG_WIDE_SPACER: u16 = 1 << 11;
pub const NEWT_FLAG_WRAPLINE: u16 = 1 << 12;
/// Part of the current selection or search match.
pub const NEWT_FLAG_SELECTED: u16 = 1 << 13;

pub const NEWT_CURSOR_BLOCK: u8 = 0;
pub const NEWT_CURSOR_UNDERLINE: u8 = 1;
pub const NEWT_CURSOR_BEAM: u8 = 2;
pub const NEWT_CURSOR_HOLLOW_BLOCK: u8 = 3;
pub const NEWT_CURSOR_HIDDEN: u8 = 4;

// Key identifiers. Values below 0x110000 are Unicode scalars — the character
// the platform's layout produced. Named keys live above that range so the two
// can share one parameter without a discriminant.
pub const NEWT_KEY_ENTER: u32 = 0x1000_0001;
pub const NEWT_KEY_TAB: u32 = 0x1000_0002;
pub const NEWT_KEY_BACKSPACE: u32 = 0x1000_0003;
pub const NEWT_KEY_ESCAPE: u32 = 0x1000_0004;
pub const NEWT_KEY_DELETE: u32 = 0x1000_0005;
pub const NEWT_KEY_INSERT: u32 = 0x1000_0006;
pub const NEWT_KEY_UP: u32 = 0x1000_0007;
pub const NEWT_KEY_DOWN: u32 = 0x1000_0008;
pub const NEWT_KEY_LEFT: u32 = 0x1000_0009;
pub const NEWT_KEY_RIGHT: u32 = 0x1000_000a;
pub const NEWT_KEY_HOME: u32 = 0x1000_000b;
pub const NEWT_KEY_END: u32 = 0x1000_000c;
pub const NEWT_KEY_PAGE_UP: u32 = 0x1000_000d;
pub const NEWT_KEY_PAGE_DOWN: u32 = 0x1000_000e;
/// F1 is this value; Fn is `NEWT_KEY_F1 + (n - 1)`, up to F20.
pub const NEWT_KEY_F1: u32 = 0x1000_0100;

// Modifier bits.
pub const NEWT_MOD_SHIFT: u8 = 1 << 0;
pub const NEWT_MOD_ALT: u8 = 1 << 1;
pub const NEWT_MOD_CTRL: u8 = 1 << 2;
/// Command on macOS. Never reaches the child; it drives app shortcuts.
pub const NEWT_MOD_SUPER: u8 = 1 << 3;

// Mouse event kinds.
pub const NEWT_MOUSE_PRESS: u8 = 0;
pub const NEWT_MOUSE_RELEASE: u8 = 1;
pub const NEWT_MOUSE_MOTION: u8 = 2;
pub const NEWT_MOUSE_SCROLL_UP: u8 = 3;
pub const NEWT_MOUSE_SCROLL_DOWN: u8 = 4;

/// Button value meaning "no button held", for motion events.
pub const NEWT_MOUSE_NO_BUTTON: u8 = 255;

// How a selection expands as it is dragged.
pub const NEWT_SELECTION_SIMPLE: u8 = 0;
pub const NEWT_SELECTION_BLOCK: u8 = 1;
/// Whole words, as a double-click gives.
pub const NEWT_SELECTION_WORD: u8 = 2;
/// Whole lines, as a triple-click gives.
pub const NEWT_SELECTION_LINE: u8 = 3;

// What an agent driving a session is doing. `UNKNOWN` is deliberately distinct
// from `IDLE`: "no agent has reported" and "an agent is idle" are different
// things to show.
pub const NEWT_AGENT_UNKNOWN: u8 = 0;
pub const NEWT_AGENT_IDLE: u8 = 1;
pub const NEWT_AGENT_RUNNING: u8 = 2;
pub const NEWT_AGENT_WAITING: u8 = 3;
pub const NEWT_AGENT_ERROR: u8 = 4;

const _: () = {
    assert!(NEWT_FLAG_BOLD == flags::BOLD);
    assert!(NEWT_FLAG_ITALIC == flags::ITALIC);
    assert!(NEWT_FLAG_UNDERLINE == flags::UNDERLINE);
    assert!(NEWT_FLAG_DOUBLE_UNDERLINE == flags::DOUBLE_UNDERLINE);
    assert!(NEWT_FLAG_UNDERCURL == flags::UNDERCURL);
    assert!(NEWT_FLAG_DOTTED_UNDERLINE == flags::DOTTED_UNDERLINE);
    assert!(NEWT_FLAG_DASHED_UNDERLINE == flags::DASHED_UNDERLINE);
    assert!(NEWT_FLAG_STRIKEOUT == flags::STRIKEOUT);
    assert!(NEWT_FLAG_DIM == flags::DIM);
    assert!(NEWT_FLAG_HIDDEN == flags::HIDDEN);
    assert!(NEWT_FLAG_WIDE == flags::WIDE);
    assert!(NEWT_FLAG_WIDE_SPACER == flags::WIDE_SPACER);
    assert!(NEWT_FLAG_WRAPLINE == flags::WRAPLINE);
    assert!(NEWT_FLAG_SELECTED == flags::SELECTED);

    assert!(NEWT_CURSOR_BLOCK == cursor_shape::BLOCK);
    assert!(NEWT_CURSOR_UNDERLINE == cursor_shape::UNDERLINE);
    assert!(NEWT_CURSOR_BEAM == cursor_shape::BEAM);
    assert!(NEWT_CURSOR_HOLLOW_BLOCK == cursor_shape::HOLLOW_BLOCK);
    assert!(NEWT_CURSOR_HIDDEN == cursor_shape::HIDDEN);

    assert!(NEWT_MOD_SHIFT == input::modifiers::SHIFT);
    assert!(NEWT_MOD_ALT == input::modifiers::ALT);
    assert!(NEWT_MOD_CTRL == input::modifiers::CTRL);
    assert!(NEWT_MOD_SUPER == input::modifiers::SUPER);
    assert!(NEWT_MOUSE_NO_BUTTON == input::NO_BUTTON);
};

/// Translate an ABI key identifier into a core key.
fn decode_key(value: u32) -> Option<Key> {
    match value {
        NEWT_KEY_ENTER => Some(Key::Enter),
        NEWT_KEY_TAB => Some(Key::Tab),
        NEWT_KEY_BACKSPACE => Some(Key::Backspace),
        NEWT_KEY_ESCAPE => Some(Key::Escape),
        NEWT_KEY_DELETE => Some(Key::Delete),
        NEWT_KEY_INSERT => Some(Key::Insert),
        NEWT_KEY_UP => Some(Key::Up),
        NEWT_KEY_DOWN => Some(Key::Down),
        NEWT_KEY_LEFT => Some(Key::Left),
        NEWT_KEY_RIGHT => Some(Key::Right),
        NEWT_KEY_HOME => Some(Key::Home),
        NEWT_KEY_END => Some(Key::End),
        NEWT_KEY_PAGE_UP => Some(Key::PageUp),
        NEWT_KEY_PAGE_DOWN => Some(Key::PageDown),
        value if (NEWT_KEY_F1..NEWT_KEY_F1 + 20).contains(&value) => {
            Some(Key::Function((value - NEWT_KEY_F1 + 1) as u8))
        }
        value => char::from_u32(value).map(Key::Char),
    }
}

fn decode_selection_mode(value: u8) -> Option<SelectionMode> {
    match value {
        NEWT_SELECTION_SIMPLE => Some(SelectionMode::Simple),
        NEWT_SELECTION_BLOCK => Some(SelectionMode::Block),
        NEWT_SELECTION_WORD => Some(SelectionMode::Word),
        NEWT_SELECTION_LINE => Some(SelectionMode::Line),
        _ => None,
    }
}

fn decode_mouse_kind(value: u8) -> Option<MouseKind> {
    match value {
        NEWT_MOUSE_PRESS => Some(MouseKind::Press),
        NEWT_MOUSE_RELEASE => Some(MouseKind::Release),
        NEWT_MOUSE_MOTION => Some(MouseKind::Motion),
        NEWT_MOUSE_SCROLL_UP => Some(MouseKind::ScrollUp),
        NEWT_MOUSE_SCROLL_DOWN => Some(MouseKind::ScrollDown),
        _ => None,
    }
}

thread_local! {
    static LAST_ERROR: RefCell<Option<CString>> = const { RefCell::new(None) };
}

fn set_last_error(message: impl Into<Vec<u8>>) {
    let message = CString::new(message).unwrap_or_else(|_| {
        CString::new("error message contained an interior NUL").expect("static string is valid")
    });
    LAST_ERROR.with(|slot| *slot.borrow_mut() = Some(message));
}

fn clear_last_error() {
    LAST_ERROR.with(|slot| *slot.borrow_mut() = None);
}

/// A running terminal session. Opaque to C.
pub struct NewtSession {
    session: CoreSession,
    /// Reused between frames so steady-state rendering allocates nothing.
    snapshot: CoreSnapshot,
    /// Keeps the last returned title alive for the caller to read.
    title: Option<CString>,
    /// Likewise for selected text.
    selection: Option<CString>,
    /// Likewise for the model name in metadata.
    model: Option<CString>,
}

/// Per-session bookkeeping for the UI.
///
/// Present for the end goal — token, cost, and agent-state display alongside
/// the grid. Nothing here affects what is drawn.
#[repr(C)]
pub struct NewtSessionMetadata {
    pub input_tokens: u64,
    pub output_tokens: u64,
    /// Cost in millionths of a currency unit; an integer so long sessions do
    /// not accumulate floating-point drift.
    pub cost_micros: u64,
    /// One of the `NEWT_AGENT_*` values.
    pub agent_state: u8,
    /// Model name, or null. Valid until the next metadata call on this session.
    pub model: *const c_char,
}

/// Borrowed view of one frame. See the module docs for lifetimes.
#[repr(C)]
pub struct NewtSnapshot {
    pub cols: u16,
    pub rows: u16,
    /// `rows * cols` cells, row-major.
    pub cells: *const Cell,
    pub cell_count: usize,
    /// Combining marks, indexed by each cell's `combining_offset`.
    pub combining: *const u32,
    pub combining_count: usize,
    pub cursor: Cursor,
    /// Rows changed since the previous snapshot; ignore when `full_damage`.
    pub damage: *const DamagedRow,
    pub damage_count: usize,
    pub full_damage: bool,
    /// Lines the viewport is scrolled back.
    pub display_offset: u32,
    /// Lines currently held in scrollback.
    pub history_len: u32,
}

/// Version of the core, as a NUL-terminated static string.
///
/// The returned pointer is valid for the lifetime of the process and must not
/// be freed by the caller.
#[no_mangle]
pub extern "C" fn newt_version() -> *const c_char {
    // Built at compile time so there is nothing to allocate or free.
    concat!(env!("CARGO_PKG_VERSION"), "\0").as_ptr() as *const c_char
}

/// Message for the most recent failure on this thread, or null if none.
///
/// Valid until the next failing call on the same thread.
#[no_mangle]
pub extern "C" fn newt_last_error() -> *const c_char {
    LAST_ERROR.with(|slot| match slot.borrow().as_ref() {
        Some(message) => message.as_ptr(),
        None => std::ptr::null(),
    })
}

/// Start a session running `shell` (null for the user's login shell) in `cwd`
/// (null for the process working directory).
///
/// A borrowed byte slice.
///
/// Length-prefixed rather than NUL-terminated, matching
/// [`newt_session_write`] and [`newt_session_send_text`]: the boundary deals in
/// bytes with an explicit length everywhere else, and a shell argument may
/// legitimately contain anything but NUL.
///
/// An empty slice — null pointer or zero length — means "not supplied", and
/// each field below says what that defaults to.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct NewtBytes {
    pub ptr: *const u8,
    pub len: usize,
}

/// One environment variable.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct NewtEnvVar {
    pub key: NewtBytes,
    pub value: NewtBytes,
}

/// Everything needed to start a session.
///
/// A struct rather than a longer parameter list. `newt_session_new` already
/// took five positionals and needed four more; adding them one at a time is how
/// an ABI rots, and it makes every call site a row of unlabelled arguments.
/// One call taking plain data is narrower in spirit than a wide signature —
/// still no callbacks, no object graph, and no platform types.
#[repr(C)]
pub struct NewtSessionSpec {
    pub cols: u16,
    pub rows: u16,
    pub scrollback_lines: u32,
    /// Program to run. Empty means the user's login shell.
    pub program: NewtBytes,
    /// Arguments, excluding argv[0]. May be null when `arg_count` is zero.
    pub args: *const NewtBytes,
    pub arg_count: usize,
    /// Variables added to the inherited environment, overriding on collision.
    /// May be null when `env_count` is zero.
    pub env: *const NewtEnvVar,
    pub env_count: usize,
    /// Working directory. Empty means this process's.
    pub cwd: NewtBytes,
    /// Value advertised as `TERM`. Empty means `xterm-256color`.
    pub term: NewtBytes,
}

/// Start a session.
///
/// Returns null on failure; see [`newt_last_error`].
///
/// # Safety
///
/// `spec` must be non-null and point to a fully initialised
/// [`NewtSessionSpec`]. Every byte slice it names must either be empty or point
/// to `len` readable bytes, and `args`/`env` must point to `arg_count`/
/// `env_count` elements. Nothing is retained after this call returns.
#[no_mangle]
pub unsafe extern "C" fn newt_session_open(spec: *const NewtSessionSpec) -> *mut NewtSession {
    clear_last_error();

    let result = catch_unwind(AssertUnwindSafe(|| {
        if spec.is_null() {
            set_last_error("session spec must not be null");
            return std::ptr::null_mut();
        }
        let spec = &*spec;

        if spec.cols == 0 || spec.rows == 0 {
            set_last_error("terminal size must be at least 1x1");
            return std::ptr::null_mut();
        }

        macro_rules! text {
            ($slice:expr, $what:literal) => {
                match bytes_to_string($slice) {
                    Ok(value) => value,
                    Err(_) => {
                        set_last_error(concat!($what, " was not valid UTF-8"));
                        return std::ptr::null_mut();
                    }
                }
            };
        }

        let program = text!(spec.program, "program");
        let cwd = text!(spec.cwd, "cwd");
        let term = text!(spec.term, "term");

        let mut args = Vec::with_capacity(spec.arg_count);
        for index in 0..spec.arg_count {
            if spec.args.is_null() {
                set_last_error("arg_count is non-zero but args is null");
                return std::ptr::null_mut();
            }
            args.push(match bytes_to_string(*spec.args.add(index)) {
                // An argument may be deliberately empty, so a missing value is
                // still an argument — unlike the optional fields above.
                Ok(value) => value.unwrap_or_default(),
                Err(_) => {
                    set_last_error("argument was not valid UTF-8");
                    return std::ptr::null_mut();
                }
            });
        }

        let mut env = Vec::with_capacity(spec.env_count);
        for index in 0..spec.env_count {
            if spec.env.is_null() {
                set_last_error("env_count is non-zero but env is null");
                return std::ptr::null_mut();
            }
            let entry = *spec.env.add(index);
            let key = match bytes_to_string(entry.key) {
                Ok(Some(key)) => key,
                Ok(None) => {
                    set_last_error("environment variable name must not be empty");
                    return std::ptr::null_mut();
                }
                Err(_) => {
                    set_last_error("environment variable name was not valid UTF-8");
                    return std::ptr::null_mut();
                }
            };
            let value = match bytes_to_string(entry.value) {
                Ok(value) => value.unwrap_or_default(),
                Err(_) => {
                    set_last_error("environment variable value was not valid UTF-8");
                    return std::ptr::null_mut();
                }
            };
            env.push((key, value));
        }

        let mut config = SessionConfig {
            size: SizeInCells::new(spec.cols, spec.rows),
            shell: program,
            args,
            env,
            cwd: cwd.map(PathBuf::from),
            scrollback_lines: spec.scrollback_lines as usize,
            ..SessionConfig::default()
        };
        if let Some(term) = term {
            config.term = term;
        }

        match CoreSession::spawn(config) {
            Ok(session) => Box::into_raw(Box::new(NewtSession {
                session,
                snapshot: CoreSnapshot::default(),
                title: None,
                selection: None,
                model: None,
            })),
            Err(e) => {
                set_last_error(e.to_string());
                std::ptr::null_mut()
            }
        }
    }));

    result.unwrap_or_else(|_| {
        set_last_error("panic while starting the session");
        std::ptr::null_mut()
    })
}

/// Start a session with the login shell, or a named program and no arguments.
///
/// A convenience over [`newt_session_open`], kept because most callers want a
/// plain shell and should not have to build a spec to say so.
///
/// Returns null on failure; see [`newt_last_error`].
///
/// # Safety
///
/// `shell` and `cwd`, when non-null, must point to NUL-terminated strings.
#[no_mangle]
pub unsafe extern "C" fn newt_session_new(
    cols: u16,
    rows: u16,
    shell: *const c_char,
    cwd: *const c_char,
    scrollback_lines: u32,
) -> *mut NewtSession {
    fn slice_of(text: &Option<String>) -> NewtBytes {
        match text {
            Some(text) => NewtBytes {
                ptr: text.as_ptr(),
                len: text.len(),
            },
            None => NewtBytes {
                ptr: std::ptr::null(),
                len: 0,
            },
        }
    }

    clear_last_error();

    let shell = match optional_string(shell) {
        Ok(value) => value,
        Err(message) => {
            set_last_error(message);
            return std::ptr::null_mut();
        }
    };
    let cwd = match optional_string(cwd) {
        Ok(value) => value,
        Err(message) => {
            set_last_error(message);
            return std::ptr::null_mut();
        }
    };

    let spec = NewtSessionSpec {
        cols,
        rows,
        scrollback_lines,
        program: slice_of(&shell),
        args: std::ptr::null(),
        arg_count: 0,
        env: std::ptr::null(),
        env_count: 0,
        cwd: slice_of(&cwd),
        term: NewtBytes {
            ptr: std::ptr::null(),
            len: 0,
        },
    };
    newt_session_open(&spec)
}

/// Release a session. Passing null is a no-op; passing the same handle twice is
/// undefined behavior.
///
/// # Safety
///
/// `handle` must come from [`newt_session_new`] and not have been freed.
#[no_mangle]
pub unsafe extern "C" fn newt_session_free(handle: *mut NewtSession) {
    if handle.is_null() {
        return;
    }
    // Dropping the session stops the child and joins the reader thread.
    drop(Box::from_raw(handle));
}

/// Send input to the child process.
///
/// # Safety
///
/// `handle` must be live, and `bytes` must point to at least `len` bytes.
#[no_mangle]
pub unsafe extern "C" fn newt_session_write(
    handle: *mut NewtSession,
    bytes: *const u8,
    len: usize,
) -> bool {
    with_session(handle, |session| {
        if bytes.is_null() && len != 0 {
            set_last_error("write called with a null buffer");
            return false;
        }
        let data = if len == 0 {
            &[][..]
        } else {
            std::slice::from_raw_parts(bytes, len)
        };

        match session.session.write(data) {
            Ok(()) => true,
            Err(e) => {
                set_last_error(e.to_string());
                false
            }
        }
    })
}

/// Resize the terminal and inform the child.
///
/// # Safety
///
/// `handle` must be live.
#[no_mangle]
pub unsafe extern "C" fn newt_session_resize(
    handle: *mut NewtSession,
    cols: u16,
    rows: u16,
) -> bool {
    with_session(handle, |session| {
        if cols == 0 || rows == 0 {
            set_last_error("terminal size must be at least 1x1");
            return false;
        }
        match session.session.resize(SizeInCells::new(cols, rows)) {
            Ok(()) => true,
            Err(e) => {
                set_last_error(e.to_string());
                false
            }
        }
    })
}

/// Scroll the viewport by `delta` lines; positive scrolls back into history.
///
/// # Safety
///
/// `handle` must be live.
#[no_mangle]
pub unsafe extern "C" fn newt_session_scroll(handle: *mut NewtSession, delta: i32) -> bool {
    with_session(handle, |session| {
        session.session.with_emulator(|e| e.scroll(delta));
        true
    })
}

/// Report cell metrics so the terminal can answer pixel-size queries.
///
/// # Safety
///
/// `handle` must be live.
#[no_mangle]
pub unsafe extern "C" fn newt_session_set_cell_size(
    handle: *mut NewtSession,
    width: u16,
    height: u16,
) -> bool {
    with_session(handle, |session| {
        session
            .session
            .with_emulator(|e| e.set_cell_size(width, height));
        true
    })
}

/// Fill `out` with the current screen.
///
/// The pointers written into `out` borrow session-owned buffers and are valid
/// until the next snapshot call on this session, or until it is freed.
///
/// # Safety
///
/// `handle` must be live and `out` must point to a writable `NewtSnapshot`.
#[no_mangle]
pub unsafe extern "C" fn newt_session_snapshot(
    handle: *mut NewtSession,
    out: *mut NewtSnapshot,
) -> bool {
    with_session(handle, |session| {
        if out.is_null() {
            set_last_error("snapshot called with a null output pointer");
            return false;
        }

        session
            .session
            .with_emulator(|e| e.snapshot_into(&mut session.snapshot));

        let snapshot = &session.snapshot;
        out.write(NewtSnapshot {
            cols: snapshot.cols,
            rows: snapshot.rows,
            cells: snapshot.cells.as_ptr(),
            cell_count: snapshot.cells.len(),
            combining: snapshot.combining.as_ptr(),
            combining_count: snapshot.combining.len(),
            cursor: snapshot.cursor,
            damage: snapshot.damage.as_ptr(),
            damage_count: snapshot.damage.len(),
            full_damage: snapshot.full_damage,
            display_offset: snapshot.display_offset,
            history_len: snapshot.history_len,
        });
        true
    })
}

/// Whether the child process has closed the terminal.
///
/// # Safety
///
/// `handle` must be live.
#[no_mangle]
pub unsafe extern "C" fn newt_session_has_exited(handle: *mut NewtSession) -> bool {
    with_session(handle, |session| session.session.has_exited())
}

/// Window title most recently set by the child, or null if none.
///
/// Valid until the next call to this function on the same session.
///
/// # Safety
///
/// `handle` must be live.
#[no_mangle]
pub unsafe extern "C" fn newt_session_title(handle: *mut NewtSession) -> *const c_char {
    if handle.is_null() {
        set_last_error("null session handle");
        return std::ptr::null();
    }

    let session = &mut *handle;
    let title = session.session.with_emulator(|e| e.title());

    session.title = title.and_then(|title| CString::new(title).ok());
    match session.title.as_ref() {
        Some(title) => title.as_ptr(),
        None => std::ptr::null(),
    }
}

/// Send a key press.
///
/// `key` is a Unicode scalar for character keys, or one of the `NEWT_KEY_*`
/// constants. Encoding depends on the terminal's current modes, so it happens
/// in the core rather than in the caller.
///
/// # Safety
///
/// `handle` must be live.
#[no_mangle]
pub unsafe extern "C" fn newt_session_send_key(
    handle: *mut NewtSession,
    key: u32,
    mods: u8,
) -> bool {
    with_session(handle, |session| {
        let Some(key) = decode_key(key) else {
            set_last_error("unknown key identifier");
            return false;
        };

        match session.session.send_key(KeyEvent::new(key, mods)) {
            Ok(()) => true,
            Err(e) => {
                set_last_error(e.to_string());
                false
            }
        }
    })
}

/// Send text produced by the platform, such as an IME commit.
///
/// # Safety
///
/// `handle` must be live, and `text` must point to `len` bytes of UTF-8.
#[no_mangle]
pub unsafe extern "C" fn newt_session_send_text(
    handle: *mut NewtSession,
    text: *const u8,
    len: usize,
) -> bool {
    with_session(handle, |session| match borrow_str(text, len) {
        Ok(text) => match session.session.send_text(text) {
            Ok(()) => true,
            Err(e) => {
                set_last_error(e.to_string());
                false
            }
        },
        Err(message) => {
            set_last_error(message);
            false
        }
    })
}

/// Send a mouse event.
///
/// `handled` is set to whether the terminal wanted the event; when false the
/// caller should apply its own behavior, such as scrolling the viewport.
///
/// # Safety
///
/// `handle` must be live; `handled` may be null.
#[no_mangle]
pub unsafe extern "C" fn newt_session_send_mouse(
    handle: *mut NewtSession,
    kind: u8,
    button: u8,
    col: u16,
    row: u16,
    mods: u8,
    handled: *mut bool,
) -> bool {
    with_session(handle, |session| {
        let Some(kind) = decode_mouse_kind(kind) else {
            set_last_error("unknown mouse event kind");
            return false;
        };

        let event = MouseEvent {
            kind,
            button,
            col,
            row,
            mods,
        };
        match session.session.send_mouse(event) {
            Ok(was_handled) => {
                if !handled.is_null() {
                    handled.write(was_handled);
                }
                true
            }
            Err(e) => {
                set_last_error(e.to_string());
                false
            }
        }
    })
}

/// Paste text, bracketed when the program has asked for it.
///
/// # Safety
///
/// `handle` must be live, and `text` must point to `len` bytes of UTF-8.
#[no_mangle]
pub unsafe extern "C" fn newt_session_send_paste(
    handle: *mut NewtSession,
    text: *const u8,
    len: usize,
) -> bool {
    with_session(handle, |session| match borrow_str(text, len) {
        Ok(text) => match session.session.send_paste(text) {
            Ok(()) => true,
            Err(e) => {
                set_last_error(e.to_string());
                false
            }
        },
        Err(message) => {
            set_last_error(message);
            false
        }
    })
}

/// Read a session's metadata.
///
/// `model` in the result borrows session-owned memory and is valid until the
/// next call to this function on the same session.
///
/// # Safety
///
/// `handle` must be live and `out` must point to a writable struct.
#[no_mangle]
pub unsafe extern "C" fn newt_session_metadata(
    handle: *mut NewtSession,
    out: *mut NewtSessionMetadata,
) -> bool {
    with_session(handle, |session| {
        if out.is_null() {
            set_last_error("metadata called with a null output pointer");
            return false;
        }

        let metadata = session.session.metadata();
        session.model = metadata
            .model
            .as_ref()
            .and_then(|model| CString::new(model.as_str()).ok());

        out.write(NewtSessionMetadata {
            input_tokens: metadata.input_tokens,
            output_tokens: metadata.output_tokens,
            cost_micros: metadata.cost_micros,
            agent_state: metadata.agent_state.as_u8(),
            model: session
                .model
                .as_ref()
                .map_or(std::ptr::null(), |model| model.as_ptr()),
        });
        true
    })
}

/// Replace a session's metadata.
///
/// # Safety
///
/// `handle` must be live; `model` may be null, otherwise it must point to
/// `model_len` bytes of UTF-8.
#[no_mangle]
pub unsafe extern "C" fn newt_session_set_metadata(
    handle: *mut NewtSession,
    input_tokens: u64,
    output_tokens: u64,
    cost_micros: u64,
    agent_state: u8,
    model: *const u8,
    model_len: usize,
) -> bool {
    with_session(handle, |session| {
        let Some(agent_state) = AgentState::from_u8(agent_state) else {
            set_last_error("unknown agent state");
            return false;
        };

        let model = if model.is_null() {
            None
        } else {
            match borrow_str(model, model_len) {
                Ok(model) => Some(model.to_string()),
                Err(message) => {
                    set_last_error(message);
                    return false;
                }
            }
        };

        session.session.set_metadata(SessionMetadata {
            input_tokens,
            output_tokens,
            cost_micros,
            agent_state,
            model,
        });
        true
    })
}

/// Begin a selection at a viewport cell.
///
/// `side_right` says which half of the cell the pointer is in, which decides
/// whether that cell is included.
///
/// # Safety
///
/// `handle` must be live.
#[no_mangle]
pub unsafe extern "C" fn newt_session_selection_start(
    handle: *mut NewtSession,
    col: u16,
    row: u16,
    side_right: bool,
    mode: u8,
) -> bool {
    with_session(handle, |session| {
        let Some(mode) = decode_selection_mode(mode) else {
            set_last_error("unknown selection mode");
            return false;
        };
        session.session.start_selection(col, row, side_right, mode);
        true
    })
}

/// Extend the active selection to a viewport cell.
///
/// # Safety
///
/// `handle` must be live.
#[no_mangle]
pub unsafe extern "C" fn newt_session_selection_update(
    handle: *mut NewtSession,
    col: u16,
    row: u16,
    side_right: bool,
) -> bool {
    with_session(handle, |session| {
        session.session.update_selection(col, row, side_right);
        true
    })
}

/// Clear any selection.
///
/// # Safety
///
/// `handle` must be live.
#[no_mangle]
pub unsafe extern "C" fn newt_session_selection_clear(handle: *mut NewtSession) -> bool {
    with_session(handle, |session| {
        session.session.clear_selection();
        true
    })
}

/// The selected text, or null when nothing is selected.
///
/// Valid until the next call to this function on the same session.
///
/// # Safety
///
/// `handle` must be live.
#[no_mangle]
pub unsafe extern "C" fn newt_session_selected_text(handle: *mut NewtSession) -> *const c_char {
    if handle.is_null() {
        set_last_error("null session handle");
        return std::ptr::null();
    }

    let session = &mut *handle;
    let text = session.session.selected_text();

    session.selection = text.and_then(|text| CString::new(text).ok());
    match session.selection.as_ref() {
        Some(text) => text.as_ptr(),
        None => std::ptr::null(),
    }
}

/// Find `pattern`, selecting and scrolling to the match.
///
/// The search is literal, not a regular expression. `found` receives whether a
/// match was located.
///
/// # Safety
///
/// `handle` must be live, `pattern` must point to `len` bytes of UTF-8, and
/// `found` may be null.
#[no_mangle]
pub unsafe extern "C" fn newt_session_find(
    handle: *mut NewtSession,
    pattern: *const u8,
    len: usize,
    forward: bool,
    found: *mut bool,
) -> bool {
    with_session(handle, |session| match borrow_str(pattern, len) {
        Ok(pattern) => {
            let matched = session.session.find(pattern, forward);
            if !found.is_null() {
                found.write(matched);
            }
            true
        }
        Err(message) => {
            set_last_error(message);
            false
        }
    })
}

/// Jump the viewport back to the live edge.
///
/// # Safety
///
/// `handle` must be live.
#[no_mangle]
pub unsafe extern "C" fn newt_session_scroll_to_bottom(handle: *mut NewtSession) -> bool {
    with_session(handle, |session| {
        session.session.scroll_to_bottom();
        true
    })
}

/// Borrow caller-owned bytes as UTF-8, rejecting anything malformed rather
/// than sending replacement characters to the child.
unsafe fn borrow_str<'a>(pointer: *const u8, len: usize) -> Result<&'a str, &'static str> {
    if len == 0 {
        return Ok("");
    }
    if pointer.is_null() {
        return Err("text pointer was null");
    }
    std::str::from_utf8(std::slice::from_raw_parts(pointer, len))
        .map_err(|_| "text was not valid UTF-8")
}

/// Run `f` with a live session, translating null handles and panics into a
/// `false` return so neither can cross the ABI.
unsafe fn with_session(handle: *mut NewtSession, f: impl FnOnce(&mut NewtSession) -> bool) -> bool {
    clear_last_error();

    if handle.is_null() {
        set_last_error("null session handle");
        return false;
    }

    catch_unwind(AssertUnwindSafe(|| f(&mut *handle))).unwrap_or_else(|_| {
        set_last_error("panic crossing the ffi boundary");
        false
    })
}

/// Read a borrowed byte slice, rejecting invalid UTF-8 rather than guessing.
///
/// An empty slice reads as `None` — the boundary's way of saying "not
/// supplied", so a caller can zero the field it does not care about.
unsafe fn bytes_to_string(bytes: NewtBytes) -> Result<Option<String>, &'static str> {
    if bytes.ptr.is_null() || bytes.len == 0 {
        return Ok(None);
    }
    match std::str::from_utf8(std::slice::from_raw_parts(bytes.ptr, bytes.len)) {
        Ok(value) => Ok(Some(value.to_string())),
        Err(_) => Err("value was not valid UTF-8"),
    }
}

/// Read an optional C string, rejecting invalid UTF-8 rather than guessing.
unsafe fn optional_string(pointer: *const c_char) -> Result<Option<String>, &'static str> {
    if pointer.is_null() {
        return Ok(None);
    }
    match CStr::from_ptr(pointer).to_str() {
        Ok(value) => Ok(Some(value.to_string())),
        Err(_) => Err("argument was not valid UTF-8"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_is_valid_c_string() {
        let ptr = newt_version();
        let s = unsafe { CStr::from_ptr(ptr) }.to_str().unwrap();
        assert_eq!(s, env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn key_identifiers_decode_to_the_right_keys() {
        assert_eq!(decode_key('a' as u32), Some(Key::Char('a')));
        assert_eq!(decode_key('中' as u32), Some(Key::Char('中')));
        assert_eq!(decode_key(NEWT_KEY_ENTER), Some(Key::Enter));
        assert_eq!(decode_key(NEWT_KEY_PAGE_DOWN), Some(Key::PageDown));
        assert_eq!(decode_key(NEWT_KEY_F1), Some(Key::Function(1)));
        assert_eq!(decode_key(NEWT_KEY_F1 + 11), Some(Key::Function(12)));
        // Past F20 there is no key, and the value is not a valid scalar.
        assert_eq!(decode_key(NEWT_KEY_F1 + 25), None);
    }

    #[test]
    fn typing_reaches_the_child() {
        unsafe {
            let shell = CString::new("/bin/sh").unwrap();
            let handle = newt_session_new(40, 8, shell.as_ptr(), std::ptr::null(), 100);
            assert!(!handle.is_null());

            for character in "printf 'typed'".chars() {
                assert!(newt_session_send_key(handle, character as u32, 0));
            }
            assert!(newt_session_send_key(handle, NEWT_KEY_ENTER, 0));

            let mut snapshot = std::mem::zeroed::<NewtSnapshot>();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            let mut found = false;
            while std::time::Instant::now() < deadline && !found {
                std::thread::sleep(std::time::Duration::from_millis(20));
                assert!(newt_session_snapshot(handle, &mut snapshot));
                let cells = std::slice::from_raw_parts(snapshot.cells, snapshot.cell_count);
                let text: String = cells
                    .iter()
                    .filter_map(|cell| char::from_u32(cell.codepoint))
                    .collect();
                found = text.contains("typed");
            }

            assert!(found, "typed keys never reached the child");
            newt_session_free(handle);
        }
    }

    /// Build a `NewtBytes` borrowing `text`.
    ///
    /// Only valid while `text` outlives the spec, which is what every caller
    /// of this ABI has to arrange anyway.
    fn slice(text: &str) -> NewtBytes {
        NewtBytes {
            ptr: text.as_ptr(),
            len: text.len(),
        }
    }

    fn empty() -> NewtBytes {
        NewtBytes {
            ptr: std::ptr::null(),
            len: 0,
        }
    }

    /// Read the grid until it contains `needle`, or give up.
    unsafe fn wait_for_text(handle: *mut NewtSession, needle: &str) -> bool {
        let mut snapshot = std::mem::zeroed::<NewtSnapshot>();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(20));
            assert!(newt_session_snapshot(handle, &mut snapshot));
            let cells = std::slice::from_raw_parts(snapshot.cells, snapshot.cell_count);
            let text: String = cells
                .iter()
                .filter_map(|cell| char::from_u32(cell.codepoint))
                .collect();
            if text.contains(needle) {
                return true;
            }
        }
        false
    }

    #[test]
    fn a_spec_carries_arguments_and_environment_across_the_boundary() {
        unsafe {
            let program = "/bin/sh";
            let (dash_c, script) = ("-c", "printf %s \"$NEWT_ABI_VAR\"");
            let (key, value) = ("NEWT_ABI_VAR", "crossed");

            let args = [slice(dash_c), slice(script)];
            let env = [NewtEnvVar {
                key: slice(key),
                value: slice(value),
            }];

            let spec = NewtSessionSpec {
                cols: 40,
                rows: 8,
                scrollback_lines: 100,
                program: slice(program),
                args: args.as_ptr(),
                arg_count: args.len(),
                env: env.as_ptr(),
                env_count: env.len(),
                cwd: empty(),
                term: empty(),
            };

            let handle = newt_session_open(&spec);
            assert!(!handle.is_null(), "spec was rejected");
            assert!(wait_for_text(handle, "crossed"), "argv or env was dropped");
            newt_session_free(handle);
        }
    }

    #[test]
    fn a_spec_can_set_term() {
        unsafe {
            let args = [slice("-c"), slice("printf %s \"$TERM\"")];
            let spec = NewtSessionSpec {
                cols: 40,
                rows: 8,
                scrollback_lines: 100,
                program: slice("/bin/sh"),
                args: args.as_ptr(),
                arg_count: args.len(),
                env: std::ptr::null(),
                env_count: 0,
                cwd: empty(),
                term: slice("newt-test-term"),
            };

            let handle = newt_session_open(&spec);
            assert!(!handle.is_null());
            assert!(wait_for_text(handle, "newt-test-term"));
            newt_session_free(handle);
        }
    }

    #[test]
    fn malformed_specs_are_rejected_rather_than_dereferenced() {
        unsafe {
            assert!(newt_session_open(std::ptr::null()).is_null());

            let base = NewtSessionSpec {
                cols: 40,
                rows: 8,
                scrollback_lines: 100,
                program: slice("/bin/sh"),
                args: std::ptr::null(),
                arg_count: 0,
                env: std::ptr::null(),
                env_count: 0,
                cwd: empty(),
                term: empty(),
            };

            // A count without the array behind it is the mistake most likely to
            // reach this boundary from a hand-written caller.
            let dangling = NewtSessionSpec {
                arg_count: 2,
                ..base
            };
            assert!(newt_session_open(&dangling).is_null());
            assert!(!newt_last_error().is_null());

            let no_env_array = NewtSessionSpec {
                env_count: 1,
                ..base
            };
            assert!(newt_session_open(&no_env_array).is_null());

            let zero_size = NewtSessionSpec { cols: 0, ..base };
            assert!(newt_session_open(&zero_size).is_null());

            // An unnamed variable cannot be set, and silently dropping it would
            // leave the child missing something the caller believes it has.
            let nameless = [NewtEnvVar {
                key: empty(),
                value: slice("orphan"),
            }];
            let bad_env = NewtSessionSpec {
                env: nameless.as_ptr(),
                env_count: 1,
                ..base
            };
            assert!(newt_session_open(&bad_env).is_null());
        }
    }

    #[test]
    fn invalid_utf8_in_a_spec_is_rejected() {
        unsafe {
            let raw = [0x66u8, 0xff, 0x66];
            let spec = NewtSessionSpec {
                cols: 40,
                rows: 8,
                scrollback_lines: 100,
                program: NewtBytes {
                    ptr: raw.as_ptr(),
                    len: raw.len(),
                },
                args: std::ptr::null(),
                arg_count: 0,
                env: std::ptr::null(),
                env_count: 0,
                cwd: empty(),
                term: empty(),
            };
            assert!(newt_session_open(&spec).is_null());
        }
    }

    #[test]
    fn null_handles_are_rejected_without_crashing() {
        unsafe {
            assert!(!newt_session_write(std::ptr::null_mut(), b"x".as_ptr(), 1));
            assert!(!newt_session_resize(std::ptr::null_mut(), 80, 24));
            assert!(!newt_session_has_exited(std::ptr::null_mut()));
            assert!(newt_session_title(std::ptr::null_mut()).is_null());
            assert!(!newt_last_error().is_null(), "no error was recorded");
            // Freeing null is explicitly a no-op.
            newt_session_free(std::ptr::null_mut());
        }
    }

    #[test]
    fn zero_size_is_rejected() {
        unsafe {
            let handle = newt_session_new(0, 24, std::ptr::null(), std::ptr::null(), 100);
            assert!(handle.is_null());
            let message = CStr::from_ptr(newt_last_error()).to_str().unwrap();
            assert!(message.contains("size"), "unexpected message: {message}");
        }
    }

    /// The full round trip a shell performs every frame: start, write, read the
    /// grid back through the ABI, and release.
    #[test]
    fn session_round_trip_through_the_abi() {
        unsafe {
            let shell = CString::new("/bin/sh").unwrap();
            let handle = newt_session_new(40, 8, shell.as_ptr(), std::ptr::null(), 100);
            assert!(!handle.is_null(), "session did not start");

            let input = b"printf 'abc'\n";
            assert!(newt_session_write(handle, input.as_ptr(), input.len()));

            // Poll until the output appears rather than sleeping a fixed time.
            let mut snapshot = std::mem::zeroed::<NewtSnapshot>();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            let mut found = false;
            while std::time::Instant::now() < deadline && !found {
                std::thread::sleep(std::time::Duration::from_millis(20));
                assert!(newt_session_snapshot(handle, &mut snapshot));

                let cells = std::slice::from_raw_parts(snapshot.cells, snapshot.cell_count);
                let text: String = cells
                    .iter()
                    .filter_map(|cell| char::from_u32(cell.codepoint))
                    .collect();
                found = text.contains("abc");
            }

            assert!(found, "output never reached the snapshot");
            assert_eq!(snapshot.cols, 40);
            assert_eq!(snapshot.rows, 8);
            assert_eq!(snapshot.cell_count, 40 * 8);

            assert!(newt_session_resize(handle, 60, 12));
            assert!(newt_session_snapshot(handle, &mut snapshot));
            assert_eq!((snapshot.cols, snapshot.rows), (60, 12));

            newt_session_free(handle);
        }
    }
}
