//! A session pairs one PTY and its child process with one [`Emulator`].
//!
//! Deliberately *not* using alacritty's `tty` or `event_loop` modules: owning
//! the PTY through `portable-pty` is what keeps the Windows port viable, since
//! that crate already absorbs the POSIX/ConPTY difference.

use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};

use crate::emulator::{Emulator, SizeInCells, DEFAULT_SCROLLBACK_LINES};
use crate::error::{Error, Result};

/// The PTY write end, shared with the reader thread so the terminal's replies
/// to queries can be sent without a round trip through the shell.
type SharedWriter = Arc<Mutex<Box<dyn Write + Send>>>;

/// Observer for raw PTY traffic, for debugging what a program actually sends.
///
/// Terminal bugs are usually invisible in the rendered grid — the interesting
/// evidence is the byte stream — so this seam is worth keeping permanently.
pub type Trace = Box<dyn FnMut(Direction, &[u8]) + Send>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// Bytes produced by the child process.
    Output,
    /// Bytes sent to the child, including the terminal's own query replies.
    Input,
}

/// Read buffer size. Large chunks on purpose — reading a byte at a time is the
/// classic way to make a terminal feel slow under heavy output.
const READ_BUFFER_BYTES: usize = 64 * 1024;

/// How a session should be started.
#[derive(Debug, Clone)]
pub struct SessionConfig {
    pub size: SizeInCells,
    /// Program to run. `None` uses the user's login shell.
    pub shell: Option<String>,
    pub cwd: Option<PathBuf>,
    pub scrollback_lines: usize,
    /// Value advertised as `TERM`.
    pub term: String,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            size: SizeInCells::new(80, 24),
            shell: None,
            cwd: None,
            scrollback_lines: DEFAULT_SCROLLBACK_LINES,
            term: "xterm-256color".to_string(),
        }
    }
}

/// A running terminal session.
///
/// Dropping it stops the child and joins the reader thread.
pub struct Session {
    emulator: Arc<Mutex<Emulator>>,
    master: Box<dyn MasterPty + Send>,
    writer: SharedWriter,
    child: Mutex<Box<dyn Child + Send + Sync>>,
    reader: Option<JoinHandle<()>>,
    child_exited: Arc<AtomicBool>,
}

impl Session {
    pub fn spawn(config: SessionConfig) -> Result<Self> {
        Self::spawn_with_trace(config, None)
    }

    /// Spawn with an observer on raw PTY traffic. See [`Trace`].
    pub fn spawn_with_trace(config: SessionConfig, trace: Option<Trace>) -> Result<Self> {
        let pty = native_pty_system();
        let pair = pty
            .openpty(pty_size(config.size))
            .map_err(|e| Error::Pty(format!("openpty failed: {e}")))?;

        let mut cmd = CommandBuilder::new(config.shell.clone().unwrap_or_else(default_shell));
        cmd.env("TERM", &config.term);
        if let Some(cwd) = &config.cwd {
            cmd.cwd(cwd);
        }

        let child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|e| Error::Pty(format!("spawning the shell failed: {e}")))?;
        // The slave handle is not needed once the child holds it, and keeping it
        // open would stop the read side from ever seeing EOF.
        drop(pair.slave);

        let reader_handle = pair
            .master
            .try_clone_reader()
            .map_err(|e| Error::Pty(format!("cloning the pty reader failed: {e}")))?;
        let writer = pair
            .master
            .take_writer()
            .map_err(|e| Error::Pty(format!("taking the pty writer failed: {e}")))?;

        let emulator = Arc::new(Mutex::new(Emulator::new(
            config.size,
            config.scrollback_lines,
        )));
        let child_exited = Arc::new(AtomicBool::new(false));

        let writer: SharedWriter = Arc::new(Mutex::new(writer));

        let reader = spawn_reader_thread(
            reader_handle,
            Arc::clone(&emulator),
            Arc::clone(&writer),
            Arc::clone(&child_exited),
            trace,
        );

        Ok(Self {
            emulator,
            master: pair.master,
            writer,
            child: Mutex::new(child),
            reader: Some(reader),
            child_exited,
        })
    }

    /// Send input to the child process.
    pub fn write(&self, bytes: &[u8]) -> Result<()> {
        let mut writer = self.writer.lock().expect("writer mutex poisoned");
        writer.write_all(bytes)?;
        writer.flush()?;
        Ok(())
    }

    /// Resize both the emulator and the PTY.
    ///
    /// Order matters: the child is told last, so by the time it redraws in
    /// response to SIGWINCH the grid is already the new shape.
    pub fn resize(&self, size: SizeInCells) -> Result<()> {
        self.with_emulator(|e| e.resize(size));
        self.master
            .resize(pty_size(size))
            .map_err(|e| Error::Pty(format!("resizing the pty failed: {e}")))
    }

    /// Run a closure against the emulator while holding its lock.
    pub fn with_emulator<T>(&self, f: impl FnOnce(&mut Emulator) -> T) -> T {
        let mut emulator = self.emulator.lock().expect("emulator mutex poisoned");
        f(&mut emulator)
    }

    /// Whether the child has closed the PTY.
    pub fn has_exited(&self) -> bool {
        self.child_exited.load(Ordering::Acquire)
    }

    /// Wait for the child to exit, returning its exit code.
    pub fn wait(&self) -> Result<u32> {
        let mut child = self.child.lock().expect("child mutex poisoned");
        child
            .wait()
            .map(|status| status.exit_code())
            .map_err(|e| Error::Pty(format!("waiting on the child failed: {e}")))
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        // Kill first: the reader thread only unblocks once the child closes the
        // pty, so joining without this can hang on a shell that is still alive.
        if let Ok(mut child) = self.child.lock() {
            let _ = child.kill();
            let _ = child.wait();
        }
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

fn spawn_reader_thread(
    mut reader: Box<dyn Read + Send>,
    emulator: Arc<Mutex<Emulator>>,
    writer: SharedWriter,
    child_exited: Arc<AtomicBool>,
    mut trace: Option<Trace>,
) -> JoinHandle<()> {
    thread::Builder::new()
        .name("newt-pty-reader".to_string())
        .spawn(move || {
            let mut buf = vec![0u8; READ_BUFFER_BYTES];
            loop {
                match reader.read(&mut buf) {
                    // EOF: the child closed the pty.
                    Ok(0) => break,
                    Ok(n) => {
                        if let Some(trace) = trace.as_mut() {
                            trace(Direction::Output, &buf[..n]);
                        }

                        let replies = {
                            let mut emulator = match emulator.lock() {
                                Ok(guard) => guard,
                                // A panic elsewhere poisoned the lock; there is
                                // nothing useful this thread can do but stop.
                                Err(_) => break,
                            };
                            emulator.advance(&buf[..n]);
                            emulator.take_pty_output()
                        };

                        // Answer the child's questions immediately. Held past
                        // this point, a program waiting on a cursor-position
                        // report would hang.
                        if !replies.is_empty() {
                            if let Some(trace) = trace.as_mut() {
                                trace(Direction::Input, &replies);
                            }
                            if let Ok(mut writer) = writer.lock() {
                                if writer.write_all(&replies).is_err() {
                                    break;
                                }
                                let _ = writer.flush();
                            }
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(_) => break,
                }
            }
            child_exited.store(true, Ordering::Release);
        })
        .expect("spawning the pty reader thread failed")
}

fn pty_size(size: SizeInCells) -> PtySize {
    PtySize {
        rows: size.rows,
        cols: size.cols,
        // Pixel dimensions are only consulted by programs drawing images; the
        // shell reports them, so leaving them zero is correct until we support
        // an image protocol.
        pixel_width: 0,
        pixel_height: 0,
    }
}

fn default_shell() -> String {
    std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string())
}
