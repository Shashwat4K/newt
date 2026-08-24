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
use alacritty_terminal::term::color::Colors;
use alacritty_terminal::term::{Config, Term, TermDamage};
use alacritty_terminal::vte::ansi::{Color as EngineColor, CursorShape, Processor};

use crate::events::EventSink;
use crate::palette::{default_color, Rgb};
use crate::snapshot::{cursor_shape, flags, Cell, Color, Cursor, DamagedRow, Snapshot};

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

    /// Fill `out` with the current screen.
    ///
    /// Reuses the snapshot's buffers, so a steady-state frame allocates
    /// nothing. Damage is consumed: each call reports what changed since the
    /// previous one, which is what lets the renderer redraw only dirty rows.
    pub fn snapshot_into(&mut self, out: &mut Snapshot) {
        let cols = self.term.columns() as u16;
        let rows = self.term.screen_lines() as u16;
        out.reset(cols, rows);

        // Damage first: it needs a mutable borrow, and reading it before the
        // content keeps the two views of the same frame consistent.
        match self.term.damage() {
            TermDamage::Full => out.full_damage = true,
            TermDamage::Partial(damaged) => {
                for line in damaged {
                    out.damage.push(DamagedRow {
                        row: line.line as u32,
                        left: line.left as u32,
                        right: line.right as u32,
                    });
                }
            }
        }
        self.term.reset_damage();

        let content = self.term.renderable_content();
        let display_offset = content.display_offset as i32;
        let colors = content.colors;

        for indexed in content.display_iter {
            let row = indexed.point.line.0 + display_offset;
            let col = indexed.point.column.0;
            if row < 0 || row >= rows as i32 || col >= cols as usize {
                continue;
            }

            let cell = indexed.cell;
            let index = row as usize * cols as usize + col;
            out.cells[index] = convert_cell(cell, colors, &mut out.combining);
        }

        let cursor_row = content.cursor.point.line.0 + display_offset;
        let visible = content.cursor.shape != CursorShape::Hidden
            && cursor_row >= 0
            && cursor_row < rows as i32;
        out.cursor = Cursor {
            col: content.cursor.point.column.0 as u16,
            row: cursor_row.max(0) as u16,
            shape: convert_cursor_shape(content.cursor.shape),
            visible,
        };

        out.display_offset = content.display_offset as u32;
        out.history_len = self.term.grid().history_size() as u32;
    }

    /// Allocate a fresh snapshot. Convenience for tests and one-off reads;
    /// render loops should reuse one via [`Emulator::snapshot_into`].
    pub fn snapshot(&mut self) -> Snapshot {
        let mut out = Snapshot::default();
        self.snapshot_into(&mut out);
        out
    }

    /// Scroll the viewport by `delta` lines: positive scrolls into history.
    pub fn scroll(&mut self, delta: i32) {
        self.term
            .scroll_display(alacritty_terminal::grid::Scroll::Delta(delta));
    }

    /// Non-empty visible rows joined by newlines.
    pub fn visible_string(&self) -> String {
        self.visible_text().join("\n").trim_end().to_string()
    }
}

/// Translate one engine cell into the flat representation the renderer reads.
fn convert_cell(
    cell: &alacritty_terminal::term::cell::Cell,
    colors: &Colors,
    combining: &mut Vec<u32>,
) -> Cell {
    let engine_flags = cell.flags;

    let mut fg = resolve_color(
        cell.fg,
        colors,
        default_color(crate::palette::FOREGROUND_INDEX),
    );
    let mut bg = resolve_color(
        cell.bg,
        colors,
        default_color(crate::palette::BACKGROUND_INDEX),
    );

    // Inverse is applied here so the renderer never has to. Doing it late, in
    // the shell, is how one ends up with selection and inverse fighting.
    if engine_flags.contains(Flags::INVERSE) {
        std::mem::swap(&mut fg, &mut bg);
    }

    let mut out_flags = 0u16;
    for (engine_flag, flag) in [
        (Flags::BOLD, flags::BOLD),
        (Flags::ITALIC, flags::ITALIC),
        (Flags::UNDERLINE, flags::UNDERLINE),
        (Flags::DOUBLE_UNDERLINE, flags::DOUBLE_UNDERLINE),
        (Flags::UNDERCURL, flags::UNDERCURL),
        (Flags::DOTTED_UNDERLINE, flags::DOTTED_UNDERLINE),
        (Flags::DASHED_UNDERLINE, flags::DASHED_UNDERLINE),
        (Flags::STRIKEOUT, flags::STRIKEOUT),
        (Flags::DIM, flags::DIM),
        (Flags::HIDDEN, flags::HIDDEN),
        (Flags::WIDE_CHAR, flags::WIDE),
        (Flags::WIDE_CHAR_SPACER, flags::WIDE_SPACER),
        (Flags::WRAPLINE, flags::WRAPLINE),
    ] {
        if engine_flags.contains(engine_flag) {
            out_flags |= flag;
        }
    }

    // Spacers carry no glyph of their own; giving them a codepoint would draw
    // the wide character twice.
    let codepoint = if engine_flags.contains(Flags::WIDE_CHAR_SPACER) {
        0
    } else {
        cell.c as u32
    };

    let combining_offset = combining.len() as u32;
    let mut combining_len = 0u16;
    if let Some(marks) = cell.zerowidth() {
        for mark in marks {
            combining.push(*mark as u32);
            combining_len += 1;
        }
    }

    Cell {
        codepoint,
        combining_offset,
        combining_len,
        flags: out_flags,
        fg: Color::from_rgb(fg),
        bg: Color::from_rgb(bg),
    }
}

/// Resolve an engine color to RGB, falling back to our palette when the
/// terminal has no override for that slot.
fn resolve_color(color: EngineColor, colors: &Colors, fallback: Rgb) -> Rgb {
    let engine_rgb = match color {
        EngineColor::Spec(rgb) => Some(rgb),
        EngineColor::Named(named) => match colors[named] {
            Some(rgb) => Some(rgb),
            None => return default_color(named as usize),
        },
        EngineColor::Indexed(index) => match colors[index as usize] {
            Some(rgb) => Some(rgb),
            None => return default_color(index as usize),
        },
    };

    match engine_rgb {
        Some(rgb) => Rgb::new(rgb.r, rgb.g, rgb.b),
        None => fallback,
    }
}

fn convert_cursor_shape(shape: CursorShape) -> u8 {
    match shape {
        CursorShape::Block => cursor_shape::BLOCK,
        CursorShape::Underline => cursor_shape::UNDERLINE,
        CursorShape::Beam => cursor_shape::BEAM,
        CursorShape::HollowBlock => cursor_shape::HOLLOW_BLOCK,
        CursorShape::Hidden => cursor_shape::HIDDEN,
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

    // --- snapshots ---

    fn char_at(snap: &Snapshot, col: u16, row: u16) -> char {
        char::from_u32(snap.cell(col, row).expect("cell out of bounds").codepoint)
            .expect("cell held an invalid codepoint")
    }

    #[test]
    fn snapshot_carries_text_and_dimensions() {
        let mut e = emulator();
        e.advance(b"hi");
        let snap = e.snapshot();

        assert_eq!((snap.cols, snap.rows), (20, 5));
        assert_eq!(snap.cells.len(), 100);
        assert_eq!(char_at(&snap, 0, 0), 'h');
        assert_eq!(char_at(&snap, 1, 0), 'i');
    }

    #[test]
    fn sgr_colors_resolve_to_palette_rgb() {
        let mut e = emulator();
        // Bright red foreground (SGR 91) is palette index 9.
        e.advance(b"\x1b[91mx");
        let snap = e.snapshot();
        let cell = snap.cell(0, 0).unwrap();

        let expected = crate::palette::default_color(9);
        assert_eq!(cell.fg, Color::from_rgb(expected));
        assert_ne!(cell.fg, cell.bg);
    }

    #[test]
    fn truecolor_is_passed_through_exactly() {
        let mut e = emulator();
        e.advance(b"\x1b[38;2;18;52;86mx");
        let snap = e.snapshot();
        let cell = snap.cell(0, 0).unwrap();

        assert_eq!((cell.fg.r, cell.fg.g, cell.fg.b), (18, 52, 86));
    }

    #[test]
    fn inverse_video_is_resolved_by_swapping_colors() {
        let mut e = emulator();
        e.advance(b"a\x1b[7mb");
        let snap = e.snapshot();

        let plain = snap.cell(0, 0).unwrap();
        let inverted = snap.cell(1, 0).unwrap();

        assert_eq!(inverted.fg, plain.bg, "inverse did not swap foreground");
        assert_eq!(inverted.bg, plain.fg, "inverse did not swap background");
    }

    #[test]
    fn attributes_reach_the_snapshot_as_flags() {
        let mut e = emulator();
        e.advance(b"\x1b[1;3;4;9mx");
        let cell = *e.snapshot().cell(0, 0).unwrap();

        assert!(cell.flags & flags::BOLD != 0);
        assert!(cell.flags & flags::ITALIC != 0);
        assert!(cell.flags & flags::UNDERLINE != 0);
        assert!(cell.flags & flags::STRIKEOUT != 0);
    }

    #[test]
    fn wide_glyph_occupies_two_cells_with_one_drawn() {
        let mut e = emulator();
        e.advance("中".as_bytes());
        let snap = e.snapshot();

        let lead = snap.cell(0, 0).unwrap();
        let spacer = snap.cell(1, 0).unwrap();

        assert_eq!(char::from_u32(lead.codepoint), Some('中'));
        assert!(lead.flags & flags::WIDE != 0);
        assert!(spacer.flags & flags::WIDE_SPACER != 0);
        assert_eq!(
            spacer.codepoint, 0,
            "spacer carried a glyph and would draw the character twice"
        );
    }

    /// The fiddliest part of the ABI: a cell holds one primary codepoint, and
    /// anything combining with it lives in a side table the cell points into.
    #[test]
    fn combining_marks_land_in_the_side_table() {
        let mut e = emulator();
        // 'e' followed by U+0301 COMBINING ACUTE ACCENT.
        e.advance("e\u{301}".as_bytes());
        let snap = e.snapshot();
        let cell = snap.cell(0, 0).unwrap();

        assert_eq!(char::from_u32(cell.codepoint), Some('e'));
        assert_eq!(cell.combining_len, 1);
        assert_eq!(snap.combining_marks(cell), &[0x301]);
    }

    #[test]
    fn cells_without_combining_marks_borrow_nothing() {
        let mut e = emulator();
        e.advance(b"ab");
        let snap = e.snapshot();

        assert_eq!(snap.cell(0, 0).unwrap().combining_len, 0);
        assert!(snap.combining_marks(snap.cell(0, 0).unwrap()).is_empty());
        assert!(snap.combining.is_empty(), "side table grew for plain text");
    }

    #[test]
    fn cursor_position_and_visibility_are_reported() {
        let mut e = emulator();
        e.advance(b"\x1b[2;4H");
        let snap = e.snapshot();

        assert_eq!((snap.cursor.col, snap.cursor.row), (3, 1));
        assert!(snap.cursor.visible);
        assert_eq!(snap.cursor.shape, cursor_shape::BLOCK);
    }

    #[test]
    fn hidden_cursor_is_reported_as_invisible() {
        let mut e = emulator();
        e.advance(b"\x1b[?25l");
        let snap = e.snapshot();

        assert!(!snap.cursor.visible);
        assert_eq!(snap.cursor.shape, cursor_shape::HIDDEN);
    }

    #[test]
    fn damage_reports_changed_rows_and_then_clears() {
        let mut e = emulator();
        // The first snapshot of a fresh terminal reports everything.
        let _ = e.snapshot();

        e.advance(b"\x1b[3;1Hchanged");
        let snap = e.snapshot();
        assert!(
            snap.full_damage || snap.damage.iter().any(|d| d.row == 2),
            "row 2 changed but was not reported damaged: {:?}",
            snap.damage
        );

        // Nothing has changed since. The engine still reports the cursor cell
        // every frame by design, so it can be redrawn for blinking — an idle
        // frame should report that and nothing more.
        let snap = e.snapshot();
        assert!(!snap.full_damage, "idle frame reported full damage");
        assert!(
            snap.damage.iter().all(|d| d.row == snap.cursor.row as u32
                && d.left == snap.cursor.col as u32
                && d.right == snap.cursor.col as u32),
            "idle frame damaged more than the cursor cell: {:?}",
            snap.damage
        );
    }

    #[test]
    fn scrollback_moves_the_viewport_and_reports_offset() {
        let mut e = Emulator::new(SizeInCells::new(20, 3), 100);
        for line in 0..10 {
            e.advance(format!("line{line}\r\n").as_bytes());
        }

        let snap = e.snapshot();
        assert_eq!(snap.display_offset, 0);
        assert!(snap.history_len > 0, "nothing entered scrollback");

        // The bottom row is the empty line after "line9", so the viewport
        // shows line7..line9; scrolling back two lines puts line6 on top.
        let bottom_before = e.visible_text();
        assert_eq!(bottom_before[0].trim(), "line8");

        e.scroll(2);
        let snap = e.snapshot();
        assert_eq!(snap.display_offset, 2);
        assert_eq!(
            char_at(&snap, 4, 0),
            '6',
            "viewport did not move into history"
        );
    }

    #[test]
    fn snapshot_buffers_are_reused_across_frames() {
        let mut e = emulator();
        let mut snap = Snapshot::default();

        e.advance(b"first");
        e.snapshot_into(&mut snap);
        let capacity = snap.cells.capacity();

        e.advance(b"\r\nsecond");
        e.snapshot_into(&mut snap);

        assert_eq!(snap.cells.capacity(), capacity, "snapshot reallocated");
        assert_eq!(char_at(&snap, 0, 1), 's');
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
