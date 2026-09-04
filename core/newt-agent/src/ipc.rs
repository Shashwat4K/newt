//! How a hook process reaches newt.
//!
//! One envelope, two length-prefixed byte strings:
//!
//! ```text
//! [u32 token_len][token][u32 payload_len][payload]
//! ```
//!
//! Length-prefixed rather than newline-delimited because a hook payload is
//! JSON that may legitimately contain a newline, and a line protocol would
//! corrupt it silently. Raw framing rather than a JSON envelope because the
//! helper then never has to parse anything — it copies stdin through, which is
//! the whole of its job and the reason it can be trusted not to misbehave
//! inside someone's agent session.
//!
//! Unix domain sockets are `#[cfg(unix)]`. Windows wants a named pipe, and
//! that difference is absorbed here in exactly one file — the same rule the
//! PTY layer follows for ConPTY. The socket type never reaches the ABI.

use std::io::{self, Read, Write};
use std::path::PathBuf;

/// Refuse anything larger. A hook payload is a small JSON object; a huge
/// length is either a bug or someone probing the socket.
pub const MAX_FRAME_BYTES: usize = 1024 * 1024;

/// Directory holding newt's sockets.
///
/// `/tmp/newt-<uid>` rather than `$TMPDIR`, and this is not arbitrary:
/// `sun_path` is 104 bytes on macOS, and `$TMPDIR` under `/var/folders/…`
/// already spends about half of that before newt adds anything. The settings
/// file, which has no such limit, lives in `$TMPDIR` instead.
pub fn socket_dir() -> PathBuf {
    PathBuf::from(format!("/tmp/newt-{}", current_uid()))
}

#[cfg(unix)]
fn current_uid() -> u32 {
    // SAFETY: `getuid` is always successful and takes no arguments.
    unsafe { libc_getuid() }
}

#[cfg(not(unix))]
fn current_uid() -> u32 {
    0
}

#[cfg(unix)]
extern "C" {
    #[link_name = "getuid"]
    fn libc_getuid() -> u32;
}

/// Write one envelope to `stream`.
pub fn write_frame(stream: &mut impl Write, token: &[u8], payload: &[u8]) -> io::Result<()> {
    let mut frame = Vec::with_capacity(8 + token.len() + payload.len());
    frame.extend_from_slice(&(token.len() as u32).to_le_bytes());
    frame.extend_from_slice(token);
    frame.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    frame.extend_from_slice(payload);
    // One write: a partial frame from a helper that died mid-send would
    // otherwise leave the reader blocked on a length it will never receive.
    stream.write_all(&frame)?;
    stream.flush()
}

/// Read one envelope from `stream`.
pub fn read_frame(stream: &mut impl Read) -> io::Result<(Vec<u8>, Vec<u8>)> {
    let token = read_chunk(stream)?;
    let payload = read_chunk(stream)?;
    Ok((token, payload))
}

fn read_chunk(stream: &mut impl Read) -> io::Result<Vec<u8>> {
    let mut length = [0u8; 4];
    stream.read_exact(&mut length)?;
    let length = u32::from_le_bytes(length) as usize;

    if length > MAX_FRAME_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "frame exceeds the maximum size",
        ));
    }

    let mut buffer = vec![0u8; length];
    stream.read_exact(&mut buffer)?;
    Ok(buffer)
}

/// Create the socket directory, owner-only.
///
/// The mode is the real access control: any process running as this user can
/// still connect, but nobody else can. The worst a same-user process could do
/// is report a wrong number into a sidebar.
#[cfg(unix)]
pub fn ensure_socket_dir() -> io::Result<PathBuf> {
    use std::os::unix::fs::PermissionsExt;

    let directory = socket_dir();
    std::fs::create_dir_all(&directory)?;
    std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700))?;
    Ok(directory)
}

#[cfg(not(unix))]
pub fn ensure_socket_dir() -> io::Result<PathBuf> {
    let directory = socket_dir();
    std::fs::create_dir_all(&directory)?;
    Ok(directory)
}

/// A socket path short enough for `sun_path`.
///
/// Checked rather than assumed: binding a too-long path fails with a message
/// that says nothing about lengths, and silently truncating would bind the
/// wrong socket.
pub fn check_socket_path(path: &std::path::Path) -> io::Result<()> {
    // 104 on macOS, 108 on Linux. The smaller one, minus the NUL terminator.
    const SUN_PATH_LIMIT: usize = 103;

    if path.as_os_str().len() > SUN_PATH_LIMIT {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "socket path is {} bytes, over the {SUN_PATH_LIMIT}-byte limit: {}",
                path.as_os_str().len(),
                path.display()
            ),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_frame_round_trips() {
        let mut buffer = Vec::new();
        write_frame(&mut buffer, b"token-1", br#"{"hook_event_name":"Stop"}"#).unwrap();

        let (token, payload) = read_frame(&mut buffer.as_slice()).unwrap();
        assert_eq!(token, b"token-1");
        assert_eq!(payload, br#"{"hook_event_name":"Stop"}"#);
    }

    #[test]
    fn a_payload_containing_a_newline_survives() {
        // The reason this is not a line protocol. Claude Code emits compact
        // JSON today, but a payload with an embedded newline would silently
        // truncate under newline framing, and the result would look like a
        // malformed hook rather than a framing bug.
        let payload = b"{\n  \"hook_event_name\": \"Stop\"\n}";
        let mut buffer = Vec::new();
        write_frame(&mut buffer, b"t", payload).unwrap();

        let (_, decoded) = read_frame(&mut buffer.as_slice()).unwrap();
        assert_eq!(decoded, payload);
    }

    #[test]
    fn empty_pieces_round_trip() {
        let mut buffer = Vec::new();
        write_frame(&mut buffer, b"", b"").unwrap();
        let (token, payload) = read_frame(&mut buffer.as_slice()).unwrap();
        assert!(token.is_empty() && payload.is_empty());
    }

    #[test]
    fn a_truncated_frame_is_an_error_rather_than_a_hang_on_garbage() {
        let mut buffer = Vec::new();
        write_frame(&mut buffer, b"token", b"payload").unwrap();
        buffer.truncate(buffer.len() - 3);

        assert!(read_frame(&mut buffer.as_slice()).is_err());
    }

    #[test]
    fn an_absurd_length_is_refused_before_allocating() {
        // Without the cap this allocates 4GB because a byte got flipped.
        let mut frame = Vec::new();
        frame.extend_from_slice(&u32::MAX.to_le_bytes());
        assert!(read_frame(&mut frame.as_slice()).is_err());
    }

    #[test]
    fn the_socket_directory_is_short_and_per_user() {
        let directory = socket_dir();
        let rendered = directory.display().to_string();

        assert!(rendered.starts_with("/tmp/newt-"), "was {rendered}");
        // The point of not using $TMPDIR: leave room for a filename inside
        // sun_path's 104 bytes.
        assert!(
            rendered.len() < 40,
            "socket directory is too long: {rendered}"
        );
    }

    #[test]
    fn an_over_long_socket_path_is_rejected_with_a_useful_message() {
        let long = std::path::PathBuf::from(format!("/tmp/{}.sock", "x".repeat(120)));
        let error = check_socket_path(&long).unwrap_err();
        assert!(error.to_string().contains("limit"), "{error}");

        assert!(check_socket_path(&socket_dir().join("abcdef0123456789.sock")).is_ok());
    }
}
