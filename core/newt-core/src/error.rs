//! Error type for the core.
//!
//! Deliberately small and string-backed at the edges: the FFI layer can only
//! carry a message across the boundary anyway, so richer typed errors would be
//! flattened immediately.

use std::fmt;

#[derive(Debug)]
pub enum Error {
    /// Opening the PTY, spawning the child, or resizing failed.
    Pty(String),
    /// Reading from or writing to the PTY failed.
    Io(std::io::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Pty(msg) => write!(f, "pty error: {msg}"),
            Error::Io(e) => write!(f, "io error: {e}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io(e) => Some(e),
            Error::Pty(_) => None,
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}

pub type Result<T> = std::result::Result<T, Error>;
