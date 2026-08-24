//! Outbound events from the emulator.
//!
//! Some escape sequences are *questions* — cursor position reports, device
//! attributes, text-area size. The engine answers them by emitting events, and
//! those answers have to reach the child process. Dropping them hangs any
//! program that asks: a zsh prompt theme that queries the terminal will wait
//! forever for a reply that never arrives.
//!
//! This sink collects the replies for the session to write back to the PTY, and
//! records the presentational events the shell will want later.

use std::sync::{Arc, Mutex};

use alacritty_terminal::event::{Event, EventListener, WindowSize};
use alacritty_terminal::vte::ansi::Rgb as EngineRgb;

use crate::palette::default_color;

#[derive(Default)]
struct SinkState {
    /// Bytes the terminal owes the child process.
    pty_output: Vec<u8>,
    /// Latest window title set via OSC.
    title: Option<String>,
    /// Bells rung since last drained.
    bells: u64,
}

/// Shared, cloneable event sink handed to the engine.
#[derive(Clone, Default)]
pub struct EventSink {
    state: Arc<Mutex<SinkState>>,
    /// Cell size in pixels, needed to answer text-area size queries.
    /// Zero until the renderer reports real metrics.
    cell_size: Arc<Mutex<(u16, u16)>>,
    size: Arc<Mutex<(u16, u16)>>,
}

impl EventSink {
    pub fn new() -> Self {
        Self::default()
    }

    /// Take everything the terminal owes the child.
    pub fn take_pty_output(&self) -> Vec<u8> {
        let mut state = self.state.lock().expect("event sink poisoned");
        std::mem::take(&mut state.pty_output)
    }

    pub fn title(&self) -> Option<String> {
        self.state
            .lock()
            .expect("event sink poisoned")
            .title
            .clone()
    }

    pub fn take_bells(&self) -> u64 {
        let mut state = self.state.lock().expect("event sink poisoned");
        std::mem::replace(&mut state.bells, 0)
    }

    /// Record the grid size so size queries can be answered accurately.
    pub fn set_size(&self, cols: u16, rows: u16) {
        *self.size.lock().expect("event sink poisoned") = (cols, rows);
    }

    /// Record cell metrics from the renderer. Until this is called, pixel
    /// dimensions are reported as zero, which is what a terminal without that
    /// information should say.
    pub fn set_cell_size(&self, width: u16, height: u16) {
        *self.cell_size.lock().expect("event sink poisoned") = (width, height);
    }

    fn push(&self, bytes: &str) {
        let mut state = self.state.lock().expect("event sink poisoned");
        state.pty_output.extend_from_slice(bytes.as_bytes());
    }
}

impl EventListener for EventSink {
    fn send_event(&self, event: Event) {
        match event {
            // The replies. These must reach the child or it blocks.
            Event::PtyWrite(text) => self.push(&text),
            Event::TextAreaSizeRequest(format) => {
                let (cols, rows) = *self.size.lock().expect("event sink poisoned");
                let (cell_width, cell_height) =
                    *self.cell_size.lock().expect("event sink poisoned");
                let reply = format(WindowSize {
                    num_lines: rows,
                    num_cols: cols,
                    cell_width,
                    cell_height,
                });
                self.push(&reply);
            }

            // Presentational state the shell reads when it has a window.
            Event::Title(title) => {
                self.state.lock().expect("event sink poisoned").title = Some(title);
            }
            Event::ResetTitle => {
                self.state.lock().expect("event sink poisoned").title = None;
            }
            Event::Bell => {
                self.state.lock().expect("event sink poisoned").bells += 1;
            }

            // OSC 4/10/11 color queries. Prompt frameworks ask these during
            // startup and wait on the answer, so the core carries a default
            // palette rather than leaving them unanswered.
            Event::ColorRequest(index, format) => {
                let color = default_color(index);
                let reply = format(EngineRgb {
                    r: color.r,
                    g: color.g,
                    b: color.b,
                });
                self.push(&reply);
            }

            // Clipboard access needs a clipboard, which does not exist until
            // the shell does. Unlike the queries above, programs issuing these
            // do not block on a reply.
            Event::ClipboardStore(..)
            | Event::ClipboardLoad(..)
            | Event::MouseCursorDirty
            | Event::CursorBlinkingChange
            | Event::Wakeup
            | Event::Exit
            | Event::ChildExit(_) => {}
        }
    }
}
