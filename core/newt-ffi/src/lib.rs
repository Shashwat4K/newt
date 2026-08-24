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

use newt_core::snapshot::{cursor_shape, flags, Cell, Cursor, DamagedRow};
use newt_core::{Session as CoreSession, SessionConfig, SizeInCells, Snapshot as CoreSnapshot};

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

pub const NEWT_CURSOR_BLOCK: u8 = 0;
pub const NEWT_CURSOR_UNDERLINE: u8 = 1;
pub const NEWT_CURSOR_BEAM: u8 = 2;
pub const NEWT_CURSOR_HOLLOW_BLOCK: u8 = 3;
pub const NEWT_CURSOR_HIDDEN: u8 = 4;

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

    assert!(NEWT_CURSOR_BLOCK == cursor_shape::BLOCK);
    assert!(NEWT_CURSOR_UNDERLINE == cursor_shape::UNDERLINE);
    assert!(NEWT_CURSOR_BEAM == cursor_shape::BEAM);
    assert!(NEWT_CURSOR_HOLLOW_BLOCK == cursor_shape::HOLLOW_BLOCK);
    assert!(NEWT_CURSOR_HIDDEN == cursor_shape::HIDDEN);
};

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
    clear_last_error();

    let result = catch_unwind(AssertUnwindSafe(|| {
        if cols == 0 || rows == 0 {
            set_last_error("terminal size must be at least 1x1");
            return std::ptr::null_mut();
        }

        let shell = match optional_string(shell) {
            Ok(value) => value,
            Err(message) => {
                set_last_error(message);
                return std::ptr::null_mut();
            }
        };
        let cwd = match optional_string(cwd) {
            Ok(value) => value.map(PathBuf::from),
            Err(message) => {
                set_last_error(message);
                return std::ptr::null_mut();
            }
        };

        let config = SessionConfig {
            size: SizeInCells::new(cols, rows),
            shell,
            cwd,
            scrollback_lines: scrollback_lines as usize,
            ..SessionConfig::default()
        };

        match CoreSession::spawn(config) {
            Ok(session) => Box::into_raw(Box::new(NewtSession {
                session,
                snapshot: CoreSnapshot::default(),
                title: None,
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
