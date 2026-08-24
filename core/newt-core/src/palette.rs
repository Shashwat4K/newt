//! Default color table.
//!
//! The core owns a palette because color queries (OSC 4/10/11) are questions
//! the terminal must answer, and those arrive long before a renderer exists.
//! The shell can override entries once it has a theme; until then these are
//! what `newt` reports and what Phase 3 will draw.
//!
//! Index space matches the engine's:
//! `0..16` named ANSI, `16..232` color cube, `232..256` grayscale ramp,
//! `256` foreground, `257` background, `258` cursor, then dim/bright variants.

/// An 8-bit-per-channel color.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb {
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }
}

pub const FOREGROUND_INDEX: usize = 256;
pub const BACKGROUND_INDEX: usize = 257;
pub const CURSOR_INDEX: usize = 258;

pub const DEFAULT_FOREGROUND: Rgb = Rgb::new(0xd8, 0xd8, 0xd8);
pub const DEFAULT_BACKGROUND: Rgb = Rgb::new(0x14, 0x14, 0x18);

/// The 16 named ANSI colors, normal then bright.
const ANSI: [Rgb; 16] = [
    Rgb::new(0x1c, 0x1c, 0x1c), // black
    Rgb::new(0xd7, 0x4e, 0x4e), // red
    Rgb::new(0x69, 0xb2, 0x5f), // green
    Rgb::new(0xd0, 0xa2, 0x4e), // yellow
    Rgb::new(0x51, 0x8f, 0xd6), // blue
    Rgb::new(0xa9, 0x6f, 0xc9), // magenta
    Rgb::new(0x53, 0xb0, 0xb0), // cyan
    Rgb::new(0xd8, 0xd8, 0xd8), // white
    Rgb::new(0x5c, 0x5c, 0x5c), // bright black
    Rgb::new(0xf0, 0x6d, 0x6d), // bright red
    Rgb::new(0x8a, 0xd0, 0x7f), // bright green
    Rgb::new(0xf0, 0xc3, 0x6d), // bright yellow
    Rgb::new(0x74, 0xad, 0xf0), // bright blue
    Rgb::new(0xc8, 0x8f, 0xe6), // bright magenta
    Rgb::new(0x74, 0xd0, 0xd0), // bright cyan
    Rgb::new(0xff, 0xff, 0xff), // bright white
];

/// Color reported for `index`, following the xterm-256 layout.
pub fn default_color(index: usize) -> Rgb {
    match index {
        0..=15 => ANSI[index],
        16..=231 => cube_color(index - 16),
        232..=255 => grayscale_color(index - 232),
        BACKGROUND_INDEX => DEFAULT_BACKGROUND,
        // Foreground, cursor, and the dim/bright aliases all fall back to the
        // default foreground; a wrong-but-plausible answer beats no answer,
        // which is what makes a querying program hang.
        _ => DEFAULT_FOREGROUND,
    }
}

/// The 6x6x6 color cube, indices 16..232.
fn cube_color(offset: usize) -> Rgb {
    const LEVELS: [u8; 6] = [0x00, 0x5f, 0x87, 0xaf, 0xd7, 0xff];
    let r = LEVELS[(offset / 36) % 6];
    let g = LEVELS[(offset / 6) % 6];
    let b = LEVELS[offset % 6];
    Rgb::new(r, g, b)
}

/// The 24-step grayscale ramp, indices 232..256.
fn grayscale_color(step: usize) -> Rgb {
    let value = (8 + step * 10) as u8;
    Rgb::new(value, value, value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cube_endpoints_match_xterm() {
        assert_eq!(default_color(16), Rgb::new(0x00, 0x00, 0x00));
        assert_eq!(default_color(231), Rgb::new(0xff, 0xff, 0xff));
        // 21 is the pure-blue corner of the cube.
        assert_eq!(default_color(21), Rgb::new(0x00, 0x00, 0xff));
    }

    #[test]
    fn grayscale_ramp_endpoints_match_xterm() {
        assert_eq!(default_color(232), Rgb::new(8, 8, 8));
        assert_eq!(default_color(255), Rgb::new(238, 238, 238));
    }

    #[test]
    fn named_indices_resolve() {
        assert_eq!(default_color(BACKGROUND_INDEX), DEFAULT_BACKGROUND);
        assert_eq!(default_color(FOREGROUND_INDEX), DEFAULT_FOREGROUND);
        assert_eq!(default_color(CURSOR_INDEX), DEFAULT_FOREGROUND);
    }

    #[test]
    fn every_index_answers() {
        // The point of the table: no query goes unanswered.
        for index in 0..269 {
            let _ = default_color(index);
        }
    }
}
