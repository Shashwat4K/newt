//! A flat, copy-free view of the screen for a renderer to draw.
//!
//! Snapshots are plain data laid out for crossing the C ABI without
//! translation: a row-major cell array, a side table for combining marks, and
//! a damage list. Buffers are reused between frames, so producing one allocates
//! nothing in the steady state.
//!
//! Colors are fully resolved here rather than in the shell. The core owns the
//! palette, so the renderer receives RGB and never has to know about named or
//! indexed colors. Inverse video is applied by swapping foreground and
//! background for the same reason — there is no INVERSE flag to re-apply.

use crate::palette::Rgb;

/// Cell attribute bits. Presentational only: anything the renderer can act on
/// directly, with the semantics already resolved.
pub mod flags {
    pub const BOLD: u16 = 1 << 0;
    pub const ITALIC: u16 = 1 << 1;
    pub const UNDERLINE: u16 = 1 << 2;
    pub const DOUBLE_UNDERLINE: u16 = 1 << 3;
    pub const UNDERCURL: u16 = 1 << 4;
    pub const DOTTED_UNDERLINE: u16 = 1 << 5;
    pub const DASHED_UNDERLINE: u16 = 1 << 6;
    pub const STRIKEOUT: u16 = 1 << 7;
    pub const DIM: u16 = 1 << 8;
    pub const HIDDEN: u16 = 1 << 9;
    /// First half of a double-width glyph.
    pub const WIDE: u16 = 1 << 10;
    /// Filler cell after a double-width glyph; draw nothing.
    pub const WIDE_SPACER: u16 = 1 << 11;
    /// This row continues into the next without a newline.
    pub const WRAPLINE: u16 = 1 << 12;
}

/// Cursor shapes, matching the engine's set.
pub mod cursor_shape {
    pub const BLOCK: u8 = 0;
    pub const UNDERLINE: u8 = 1;
    pub const BEAM: u8 = 2;
    pub const HOLLOW_BLOCK: u8 = 3;
    pub const HIDDEN: u8 = 4;
}

/// One screen cell.
///
/// `#[repr(C)]` because this array is handed to the renderer as-is; a
/// conversion pass per frame would be the single most wasteful thing in the
/// pipeline.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cell {
    /// Primary character. Zero means empty.
    pub codepoint: u32,
    /// Offset into [`Snapshot::combining`] for this cell's combining marks.
    pub combining_offset: u32,
    /// Number of combining marks belonging to this cell.
    pub combining_len: u16,
    /// Bitwise OR of [`flags`].
    pub flags: u16,
    pub fg: Color,
    pub bg: Color,
}

impl Default for Cell {
    fn default() -> Self {
        Self {
            codepoint: b' ' as u32,
            combining_offset: 0,
            combining_len: 0,
            flags: 0,
            fg: Color::BLACK,
            bg: Color::BLACK,
        }
    }
}

/// An opaque RGB triple. Alpha is deliberately absent: terminal cells are
/// opaque, and window transparency is a shell concern.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    /// Explicit padding so the layout is identical on every compiler.
    pub _reserved: u8,
}

impl Color {
    pub const BLACK: Color = Color {
        r: 0,
        g: 0,
        b: 0,
        _reserved: 0,
    };

    pub const fn from_rgb(rgb: Rgb) -> Self {
        Self {
            r: rgb.r,
            g: rgb.g,
            b: rgb.b,
            _reserved: 0,
        }
    }
}

/// Where the cursor is and how to draw it.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cursor {
    pub col: u16,
    pub row: u16,
    /// One of [`cursor_shape`].
    pub shape: u8,
    /// False when the cursor is hidden or scrolled out of the viewport.
    pub visible: bool,
}

impl Default for Cursor {
    fn default() -> Self {
        Self {
            col: 0,
            row: 0,
            shape: cursor_shape::BLOCK,
            visible: false,
        }
    }
}

/// A run of changed cells on one row, as half-open `[left, right]` inclusive
/// columns, matching the engine's damage bounds.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DamagedRow {
    pub row: u32,
    pub left: u32,
    pub right: u32,
}

/// One frame of screen state.
#[derive(Debug, Default)]
pub struct Snapshot {
    pub cols: u16,
    pub rows: u16,
    /// `rows * cols` cells, row-major.
    pub cells: Vec<Cell>,
    /// Combining marks, indexed by [`Cell::combining_offset`].
    pub combining: Vec<u32>,
    pub cursor: Cursor,
    /// Rows changed since the last snapshot. Empty when `full_damage` is set.
    pub damage: Vec<DamagedRow>,
    /// Set when everything changed and `damage` should be ignored.
    pub full_damage: bool,
    /// How far the viewport is scrolled back, in lines.
    pub display_offset: u32,
    /// Lines currently held in scrollback.
    pub history_len: u32,
}

impl Snapshot {
    /// Clear the frame's contents while keeping its allocations.
    pub(crate) fn reset(&mut self, cols: u16, rows: u16) {
        self.cols = cols;
        self.rows = rows;
        self.cells.clear();
        self.cells
            .resize(cols as usize * rows as usize, Cell::default());
        self.combining.clear();
        self.damage.clear();
        self.full_damage = false;
    }

    /// Cell at a position, or `None` when out of bounds.
    pub fn cell(&self, col: u16, row: u16) -> Option<&Cell> {
        if col >= self.cols || row >= self.rows {
            return None;
        }
        self.cells
            .get(row as usize * self.cols as usize + col as usize)
    }

    /// Combining marks attached to a cell.
    pub fn combining_marks(&self, cell: &Cell) -> &[u32] {
        let start = cell.combining_offset as usize;
        let end = start + cell.combining_len as usize;
        self.combining.get(start..end).unwrap_or(&[])
    }
}
