//! The terminal emulator: bytes in, screen state out. No PTY, no I/O, no threads.
//!
//! Keeping this free of the PTY is what makes the hard part testable — byte
//! streams can be fed synchronously, split at arbitrary offsets, and compared.
//!
//! `alacritty_terminal` is wrapped rather than re-exported so it stays
//! replaceable: nothing outside this module names its types.

use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{Config, Term};
use alacritty_terminal::vte::ansi::Processor;

use crate::events::EventSink;

/// Terminal dimensions in cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SizeInCells {
    pub cols: u16,
    pub rows: u16,
}

impl SizeInCells {
    pub fn new(cols: u16, rows: u16) -> Self {
        Self { cols, rows }
    }
}

/// `alacritty_terminal` asks for dimensions through this trait. Implementing it
/// on our own type avoids depending on the engine's `term::test` helpers.
impl Dimensions for SizeInCells {
    fn total_lines(&self) -> usize {
        self.screen_lines()
    }

    fn screen_lines(&self) -> usize {
        self.rows as usize
    }

    fn columns(&self) -> usize {
        self.cols as usize
    }
}

/// Default scrollback depth, in lines.
pub const DEFAULT_SCROLLBACK_LINES: usize = 10_000;

/// A terminal screen driven by a byte stream.
pub struct Emulator {
    term: Term<EventSink>,
    processor: Processor,
    events: EventSink,
}

impl Emulator {
    pub fn new(size: SizeInCells, scrollback_lines: usize) -> Self {
        let config = Config {
            scrolling_history: scrollback_lines,
            ..Config::default()
        };

        let events = EventSink::new();
        events.set_size(size.cols, size.rows);

        Self {
            term: Term::new(config, &size, events.clone()),
            processor: Processor::new(),
            events,
        }
    }

    /// Bytes the terminal owes the child process — replies to cursor-position,
    /// device-attribute, and size queries. The session writes these back to the
    /// PTY after every chunk; leaving them undrained hangs any program that
    /// asks the terminal a question.
    #[must_use]
    pub fn take_pty_output(&self) -> Vec<u8> {
        self.events.take_pty_output()
    }

    /// Window title most recently set by the child, if any.
    pub fn title(&self) -> Option<String> {
        self.events.title()
    }

    /// Bells rung since this was last called.
    pub fn take_bells(&self) -> u64 {
        self.events.take_bells()
    }

    /// Report cell metrics from the renderer so size queries can be answered
    /// with real pixel dimensions.
    pub fn set_cell_size(&self, width: u16, height: u16) {
        self.events.set_cell_size(width, height);
    }

    /// Feed output from the child process.
    ///
    /// Safe to call with arbitrarily split input: the parser carries its state
    /// across calls, so an escape sequence broken across chunks still applies.
    pub fn advance(&mut self, bytes: &[u8]) {
        self.processor.advance(&mut self.term, bytes);
    }

    pub fn resize(&mut self, size: SizeInCells) {
        self.term.resize(size);
        self.events.set_size(size.cols, size.rows);
    }

    pub fn size(&self) -> SizeInCells {
        SizeInCells::new(self.term.columns() as u16, self.term.screen_lines() as u16)
    }

    /// Cursor position as (column, line) within the visible screen.
    pub fn cursor(&self) -> (u16, u16) {
        let point = self.term.grid().cursor.point;
        (point.column.0 as u16, point.line.0 as u16)
    }

    /// The visible screen as text, one string per row, trailing blanks trimmed.
    ///
    /// For tests and the headless CLI. The renderer never goes through this —
    /// it reads cells and attributes, which Phase 2 exposes as snapshots.
    pub fn visible_text(&self) -> Vec<String> {
        let grid = self.term.grid();
        (0..grid.screen_lines())
            .map(|line| {
                let row = &grid[Line(line as i32)];
                let mut out = String::with_capacity(grid.columns());
                for col in 0..grid.columns() {
                    let cell = &row[Column(col)];
                    // The cell following a double-width glyph is a spacer with
                    // no character of its own; emitting it would double-count.
                    if cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
                        continue;
                    }
                    out.push(cell.c);
                }
                out.trim_end().to_string()
            })
            .collect()
    }

    /// Non-empty visible rows joined by newlines.
    pub fn visible_string(&self) -> String {
        self.visible_text().join("\n").trim_end().to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn emulator() -> Emulator {
        Emulator::new(SizeInCells::new(20, 5), 100)
    }

    #[test]
    fn plain_text_lands_on_the_grid() {
        let mut e = emulator();
        e.advance(b"hello");
        assert_eq!(e.visible_text()[0], "hello");
        assert_eq!(e.cursor(), (5, 0));
    }

    #[test]
    fn newline_and_carriage_return_move_the_cursor() {
        let mut e = emulator();
        e.advance(b"one\r\ntwo");
        assert_eq!(e.visible_text()[0], "one");
        assert_eq!(e.visible_text()[1], "two");
    }

    #[test]
    fn cursor_positioning_sequence_is_applied() {
        let mut e = emulator();
        e.advance(b"\x1b[3;5Hx");
        assert_eq!(e.visible_text()[2], "    x");
    }

    #[test]
    fn erase_in_display_clears_the_screen() {
        let mut e = emulator();
        e.advance(b"junk\r\nmore");
        e.advance(b"\x1b[2J\x1b[H");
        assert_eq!(e.visible_string(), "");
    }

    /// The classic bug class: an escape sequence split across two reads must
    /// behave identically to the same bytes delivered whole. Proven at every
    /// possible split point, not just a convenient one.
    #[test]
    fn split_at_every_offset_matches_unsplit() {
        // Deliberately mixes CSI, SGR, OSC, and a multi-byte UTF-8 grapheme so
        // the split can land inside any of them.
        let stream: &[u8] =
            b"\x1b[2J\x1b[H\x1b[1;32mgreen\x1b[0m\r\n\x1b]0;title\x07plain \xe4\xb8\xad\r\n\x1b[3;7Hx";

        let mut whole = emulator();
        whole.advance(stream);
        let expected = whole.visible_text();

        for split in 0..=stream.len() {
            let mut e = emulator();
            e.advance(&stream[..split]);
            e.advance(&stream[split..]);
            assert_eq!(
                e.visible_text(),
                expected,
                "output differed when split at offset {split}"
            );
        }
    }

    /// Same guarantee, byte at a time — the pathological case.
    #[test]
    fn byte_at_a_time_matches_unsplit() {
        let stream: &[u8] = b"\x1b[1;31mred\x1b[0m\r\n\x1b[4munder\x1b[0m";

        let mut whole = emulator();
        whole.advance(stream);

        let mut drip = emulator();
        for byte in stream {
            drip.advance(&[*byte]);
        }

        assert_eq!(drip.visible_text(), whole.visible_text());
    }

    /// Queries are the sequences a program *waits* on. An unanswered one hangs
    /// the child, so each supported query is proven to produce a reply.
    #[test]
    fn cursor_position_report_is_answered() {
        let mut e = emulator();
        e.advance(b"\x1b[3;5H\x1b[6n");
        // DSR reply is 1-based: row 3, column 5.
        assert_eq!(e.take_pty_output(), b"\x1b[3;5R");
    }

    #[test]
    fn device_attributes_query_is_answered() {
        let mut e = emulator();
        e.advance(b"\x1b[c");
        let reply = e.take_pty_output();
        assert!(
            !reply.is_empty(),
            "primary device attributes went unanswered"
        );
        assert!(
            reply.starts_with(b"\x1b["),
            "unexpected DA reply: {reply:?}"
        );
    }

    #[test]
    fn background_color_query_is_answered_from_the_palette() {
        let mut e = emulator();
        e.advance(b"\x1b]11;?\x07");
        let reply = String::from_utf8(e.take_pty_output()).expect("reply was not utf-8");
        assert!(reply.contains("rgb:"), "unexpected color reply: {reply:?}");
    }

    #[test]
    fn text_area_size_query_is_answered() {
        let mut e = emulator();
        // CSI 18 t — report text area size in characters.
        e.advance(b"\x1b[18t");
        let reply = String::from_utf8(e.take_pty_output()).expect("reply was not utf-8");
        assert!(
            reply.contains("20"),
            "size reply lacked the column count: {reply:?}"
        );
    }

    #[test]
    fn draining_replies_empties_the_queue() {
        let mut e = emulator();
        e.advance(b"\x1b[6n");
        assert!(!e.take_pty_output().is_empty());
        assert!(
            e.take_pty_output().is_empty(),
            "replies were delivered twice"
        );
    }

    #[test]
    fn window_title_is_captured() {
        let mut e = emulator();
        e.advance(b"\x1b]0;newt\x07");
        assert_eq!(e.title().as_deref(), Some("newt"));
    }

    #[test]
    fn resize_updates_reported_size() {
        let mut e = emulator();
        e.advance(b"hello");
        e.resize(SizeInCells::new(40, 10));
        assert_eq!(e.size(), SizeInCells::new(40, 10));
        assert_eq!(e.visible_text()[0], "hello");
    }

    /// Full-screen programs (vim, htop, less) draw on the alternate screen and
    /// restore the primary one on exit. Reading the wrong grid means the UI
    /// shows a stale shell prompt while vim is on screen.
    #[test]
    fn alternate_screen_is_the_visible_grid() {
        let mut e = emulator();
        e.advance(b"shell prompt");
        assert_eq!(e.visible_text()[0], "shell prompt");

        e.advance(b"\x1b[?1049h\x1b[HFULLSCREEN APP");
        assert_eq!(
            e.visible_text()[0],
            "FULLSCREEN APP",
            "alternate screen content was not visible"
        );

        e.advance(b"\x1b[?1049l");
        assert_eq!(
            e.visible_text()[0],
            "shell prompt",
            "primary screen was not restored"
        );
    }

    #[test]
    fn wide_glyphs_are_not_double_counted() {
        let mut e = emulator();
        e.advance("中文".as_bytes());
        assert_eq!(e.visible_text()[0], "中文");
    }
}
