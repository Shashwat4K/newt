//! Portable terminal core for `newt`.
//!
//! Everything in this crate is platform-neutral: no macOS, AppKit, CoreText, or
//! Metal concepts appear here, not even in naming. Platform differences (POSIX
//! PTYs vs Windows ConPTY) are absorbed in [`session`] and nowhere else.
//!
//! The layering is deliberate. [`emulator`] is pure — bytes in, screen state
//! out, no I/O and no threads — which is what makes the hard correctness
//! problems testable. [`session`] adds the PTY, the child process, and the
//! reader thread around it.
//!
//! `alacritty_terminal` is wrapped, never re-exported, so the engine stays
//! replaceable behind this API.

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub mod emulator;
pub mod error;
pub mod events;
pub mod input;
pub mod palette;
pub mod session;
pub mod snapshot;

pub use emulator::{Emulator, SizeInCells, DEFAULT_SCROLLBACK_LINES};
pub use error::{Error, Result};
pub use events::EventSink;
pub use input::{InputModes, Key, KeyEvent, MouseEvent, MouseKind};
pub use palette::Rgb;
pub use session::{Direction, Session, SessionConfig, Trace};
pub use snapshot::{Cell, Color, Cursor, DamagedRow, Snapshot};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_is_populated() {
        assert!(!VERSION.is_empty());
    }
}
