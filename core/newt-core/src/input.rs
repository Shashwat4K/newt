//! Key and mouse events to the bytes a terminal sends its child.
//!
//! This is core, not shell: what a keypress *means* on the wire is terminal
//! semantics and is identical on every platform. Only event capture, IME, and
//! dead-key composition belong to the UI layer.
//!
//! Encodings follow `xterm`'s control sequences, which is what `TERM=xterm-256color`
//! promises programs. The table is pure and mode-driven so it can be tested
//! without a PTY or a terminal instance.

/// Modifier bits.
pub mod modifiers {
    pub const NONE: u8 = 0;
    pub const SHIFT: u8 = 1 << 0;
    pub const ALT: u8 = 1 << 1;
    pub const CTRL: u8 = 1 << 2;
    /// Command on macOS. Never encoded — it drives application shortcuts, and
    /// sending it to the child is how you get mysterious stray input.
    pub const SUPER: u8 = 1 << 3;
}

/// A key, independent of any platform's key codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    /// A character-producing key, after the platform has applied its layout.
    Char(char),
    Enter,
    Tab,
    Backspace,
    Escape,
    Delete,
    Insert,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    /// F1 through F20.
    Function(u8),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyEvent {
    pub key: Key,
    pub mods: u8,
}

impl KeyEvent {
    pub fn new(key: Key, mods: u8) -> Self {
        Self { key, mods }
    }
}

/// What kind of mouse event occurred.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseKind {
    Press,
    Release,
    /// Pointer moved. Reported only in motion or drag modes.
    Motion,
    ScrollUp,
    ScrollDown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MouseEvent {
    pub kind: MouseKind,
    /// 0 left, 1 middle, 2 right. Ignored for scroll events.
    pub button: u8,
    /// Zero-based cell coordinates; encoded one-based.
    pub col: u16,
    pub row: u16,
    pub mods: u8,
}

/// The terminal modes that change how input is encoded.
///
/// Mirrored into a plain struct so this module never depends on the engine.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct InputModes {
    /// DECCKM: cursor keys send SS3 instead of CSI.
    pub app_cursor: bool,
    /// DECPAM: the keypad sends application sequences.
    pub app_keypad: bool,
    pub bracketed_paste: bool,
    pub sgr_mouse: bool,
    pub mouse_click: bool,
    pub mouse_drag: bool,
    pub mouse_motion: bool,
    pub alt_screen: bool,
    /// Wheel becomes arrow keys on the alternate screen.
    pub alternate_scroll: bool,
}

impl InputModes {
    fn mouse_enabled(&self) -> bool {
        self.mouse_click || self.mouse_drag || self.mouse_motion
    }
}

/// The xterm modifier parameter: 1 plus a bitfield.
fn modifier_parameter(mods: u8) -> u8 {
    let mut value = 1;
    if mods & modifiers::SHIFT != 0 {
        value += 1;
    }
    if mods & modifiers::ALT != 0 {
        value += 2;
    }
    if mods & modifiers::CTRL != 0 {
        value += 4;
    }
    value
}

fn has(mods: u8, flag: u8) -> bool {
    mods & flag != 0
}

/// Encode a key press.
///
/// Returns an empty vector for keys that send nothing, such as a bare Command
/// chord that the application should have handled instead.
pub fn encode_key(event: KeyEvent, modes: InputModes) -> Vec<u8> {
    // Command is an application shortcut on every platform that has it; the
    // child must never see it.
    if has(event.mods, modifiers::SUPER) {
        return Vec::new();
    }

    let mods = event.mods;
    let parameter = modifier_parameter(mods);
    let modified = parameter != 1;

    match event.key {
        Key::Char(character) => encode_char(character, mods),

        // CR, not LF: the line discipline turns it into a newline. Sending LF
        // directly breaks programs that distinguish them.
        Key::Enter => with_alt(mods, b"\r".to_vec()),
        Key::Tab if has(mods, modifiers::SHIFT) => b"\x1b[Z".to_vec(),
        Key::Tab => with_alt(mods, b"\t".to_vec()),

        // DEL, matching the `kbs=\177` in xterm's terminfo. Sending BS here is
        // the classic cause of backspace printing `^H` in a shell.
        Key::Backspace if has(mods, modifiers::CTRL) => with_alt(mods, b"\x08".to_vec()),
        Key::Backspace => with_alt(mods, b"\x7f".to_vec()),
        Key::Escape => with_alt(mods, b"\x1b".to_vec()),

        Key::Up | Key::Down | Key::Right | Key::Left | Key::Home | Key::End => {
            let final_byte = match event.key {
                Key::Up => b'A',
                Key::Down => b'B',
                Key::Right => b'C',
                Key::Left => b'D',
                Key::Home => b'H',
                Key::End => b'F',
                _ => unreachable!("only cursor keys reach this arm"),
            };

            if modified {
                // Modified cursor keys always use CSI, even in application mode.
                format!("\x1b[1;{parameter}{}", final_byte as char).into_bytes()
            } else if modes.app_cursor {
                vec![0x1b, b'O', final_byte]
            } else {
                vec![0x1b, b'[', final_byte]
            }
        }

        Key::Insert => tilde_sequence(2, parameter, modified),
        Key::Delete => tilde_sequence(3, parameter, modified),
        Key::PageUp => tilde_sequence(5, parameter, modified),
        Key::PageDown => tilde_sequence(6, parameter, modified),

        Key::Function(number) => encode_function_key(number, parameter, modified),
    }
}

/// `CSI <number> ~`, with the modifier parameter when one applies.
fn tilde_sequence(number: u8, parameter: u8, modified: bool) -> Vec<u8> {
    if modified {
        format!("\x1b[{number};{parameter}~").into_bytes()
    } else {
        format!("\x1b[{number}~").into_bytes()
    }
}

fn encode_function_key(number: u8, parameter: u8, modified: bool) -> Vec<u8> {
    // F1–F4 are SS3 sequences; F5 and up are numbered tilde sequences, and the
    // numbering deliberately skips 16, 22, and 27 — that is xterm's table, not
    // a mistake.
    match number {
        1..=4 => {
            let final_byte = b'P' + (number - 1);
            if modified {
                format!("\x1b[1;{parameter}{}", final_byte as char).into_bytes()
            } else {
                vec![0x1b, b'O', final_byte]
            }
        }
        5..=20 => {
            let code = match number {
                5 => 15,
                6 => 17,
                7 => 18,
                8 => 19,
                9 => 20,
                10 => 21,
                11 => 23,
                12 => 24,
                13 => 25,
                14 => 26,
                15 => 28,
                16 => 29,
                17 => 31,
                18 => 32,
                19 => 33,
                20 => 34,
                _ => unreachable!("range is bounded by the match arm"),
            };
            tilde_sequence(code, parameter, modified)
        }
        _ => Vec::new(),
    }
}

/// Encode a character key, applying Control and Alt.
fn encode_char(character: char, mods: u8) -> Vec<u8> {
    let mut bytes = if has(mods, modifiers::CTRL) {
        match control_code(character) {
            Some(code) => vec![code],
            // Control with a key that has no control code sends the character
            // unchanged, which is what xterm does.
            None => character.to_string().into_bytes(),
        }
    } else {
        character.to_string().into_bytes()
    };

    if has(mods, modifiers::ALT) {
        // Alt is sent as an ESC prefix rather than by setting the high bit;
        // eight-bit input breaks UTF-8.
        let mut prefixed = vec![0x1b];
        prefixed.append(&mut bytes);
        return prefixed;
    }

    bytes
}

/// The C0 control code produced by Control plus a character.
fn control_code(character: char) -> Option<u8> {
    match character {
        ' ' | '@' => Some(0x00),
        'a'..='z' => Some(character as u8 - b'a' + 1),
        'A'..='Z' => Some(character as u8 - b'A' + 1),
        '[' => Some(0x1b),
        '\\' => Some(0x1c),
        ']' => Some(0x1d),
        '^' => Some(0x1e),
        '_' => Some(0x1f),
        '?' => Some(0x7f),
        _ => None,
    }
}

fn with_alt(mods: u8, mut bytes: Vec<u8>) -> Vec<u8> {
    if has(mods, modifiers::ALT) {
        let mut prefixed = vec![0x1b];
        prefixed.append(&mut bytes);
        return prefixed;
    }
    bytes
}

/// Encode text that the platform produced directly, such as an IME commit.
///
/// Layout and composition are the platform's job; by this point the text is
/// final and goes out as UTF-8.
pub fn encode_text(text: &str) -> Vec<u8> {
    text.as_bytes().to_vec()
}

/// Encode a mouse event, or `None` when the terminal is not asking for one.
pub fn encode_mouse(event: MouseEvent, modes: InputModes) -> Option<Vec<u8>> {
    let scrolling = matches!(event.kind, MouseKind::ScrollUp | MouseKind::ScrollDown);

    // On the alternate screen, a wheel that nothing is listening for becomes
    // arrow keys, which is what makes the wheel scroll in `less` and `man`.
    if scrolling && !modes.mouse_enabled() {
        if modes.alt_screen && modes.alternate_scroll {
            let key = if event.kind == MouseKind::ScrollUp {
                Key::Up
            } else {
                Key::Down
            };
            return Some(encode_key(KeyEvent::new(key, modifiers::NONE), modes));
        }
        return None;
    }

    if !modes.mouse_enabled() {
        return None;
    }

    if event.kind == MouseKind::Motion {
        // Motion is only reported when something asked for it: any-motion
        // tracking, or drag tracking while a button is down.
        let dragging = modes.mouse_drag && event.button != NO_BUTTON;
        if !modes.mouse_motion && !dragging {
            return None;
        }
    }

    let mut code = match event.kind {
        MouseKind::ScrollUp => 64,
        MouseKind::ScrollDown => 65,
        MouseKind::Motion => {
            32 + u32::from(if event.button == NO_BUTTON {
                3
            } else {
                event.button
            })
        }
        _ => u32::from(event.button.min(2)),
    };

    if has(event.mods, modifiers::SHIFT) {
        code += 4;
    }
    if has(event.mods, modifiers::ALT) {
        code += 8;
    }
    if has(event.mods, modifiers::CTRL) {
        code += 16;
    }

    // Coordinates go out one-based.
    let col = event.col as u32 + 1;
    let row = event.row as u32 + 1;

    if modes.sgr_mouse {
        let final_byte = if event.kind == MouseKind::Release {
            'm'
        } else {
            'M'
        };
        return Some(format!("\x1b[<{code};{col};{row}{final_byte}").into_bytes());
    }

    // Legacy X10 encoding: release is button 3, and every value is offset by
    // 32 so it lands in printable ASCII. Coordinates above 223 cannot be
    // represented, so they are dropped rather than sent as garbage.
    let legacy_code = if event.kind == MouseKind::Release {
        3 + (code & !3)
    } else {
        code
    };
    if col > 223 || row > 223 {
        return None;
    }
    Some(vec![
        0x1b,
        b'[',
        b'M',
        (32 + legacy_code) as u8,
        (32 + col) as u8,
        (32 + row) as u8,
    ])
}

/// Sentinel for "no button held" in a motion event.
pub const NO_BUTTON: u8 = u8::MAX;

/// Encode pasted text.
///
/// With bracketed paste enabled the text is wrapped in markers so the
/// application can tell it apart from typing — which is what stops an editor
/// from auto-indenting a paste into a staircase.
pub fn encode_paste(text: &str, modes: InputModes) -> Vec<u8> {
    // Newlines become CR: that is what a terminal delivers when you press
    // Return, and pasted LF would otherwise be seen as a different key.
    let normalized: String = text.replace("\r\n", "\r").replace('\n', "\r");

    if modes.bracketed_paste {
        // The end marker must not appear inside the payload, or the paste can
        // be terminated early by its own content.
        let sanitized = normalized.replace("\x1b[201~", "");
        let mut bytes = b"\x1b[200~".to_vec();
        bytes.extend_from_slice(sanitized.as_bytes());
        bytes.extend_from_slice(b"\x1b[201~");
        return bytes;
    }

    // Without bracketing, strip escapes: pasted control sequences would be
    // interpreted as commands by the program reading them.
    normalized
        .chars()
        .filter(|c| *c == '\r' || *c == '\t' || !c.is_control())
        .collect::<String>()
        .into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(key: Key, mods: u8) -> Vec<u8> {
        encode_key(KeyEvent::new(key, mods), InputModes::default())
    }

    fn key_in(key: Key, mods: u8, modes: InputModes) -> Vec<u8> {
        encode_key(KeyEvent::new(key, mods), modes)
    }

    // --- characters ---

    #[test]
    fn plain_characters_go_out_as_utf8() {
        assert_eq!(key(Key::Char('a'), modifiers::NONE), b"a");
        assert_eq!(key(Key::Char('中'), modifiers::NONE), "中".as_bytes());
    }

    #[test]
    fn control_maps_letters_to_c0_codes() {
        assert_eq!(key(Key::Char('c'), modifiers::CTRL), vec![0x03]);
        assert_eq!(key(Key::Char('C'), modifiers::CTRL), vec![0x03]);
        assert_eq!(key(Key::Char('a'), modifiers::CTRL), vec![0x01]);
        assert_eq!(key(Key::Char('z'), modifiers::CTRL), vec![0x1a]);
    }

    #[test]
    fn control_maps_the_punctuation_cases() {
        assert_eq!(key(Key::Char(' '), modifiers::CTRL), vec![0x00]);
        assert_eq!(key(Key::Char('['), modifiers::CTRL), vec![0x1b]);
        assert_eq!(key(Key::Char('\\'), modifiers::CTRL), vec![0x1c]);
        assert_eq!(key(Key::Char('?'), modifiers::CTRL), vec![0x7f]);
    }

    #[test]
    fn alt_sends_an_escape_prefix_rather_than_setting_the_high_bit() {
        // High-bit encoding would corrupt UTF-8.
        assert_eq!(key(Key::Char('x'), modifiers::ALT), vec![0x1b, b'x']);
        assert_eq!(
            key(Key::Char('c'), modifiers::ALT | modifiers::CTRL),
            vec![0x1b, 0x03]
        );
    }

    #[test]
    fn command_is_never_sent_to_the_child() {
        assert!(key(Key::Char('c'), modifiers::SUPER).is_empty());
        assert!(key(Key::Char('v'), modifiers::SUPER | modifiers::SHIFT).is_empty());
    }

    // --- named keys ---

    #[test]
    fn enter_sends_carriage_return() {
        assert_eq!(key(Key::Enter, modifiers::NONE), b"\r");
        assert_eq!(key(Key::Enter, modifiers::ALT), vec![0x1b, b'\r']);
    }

    #[test]
    fn backspace_sends_del_not_backspace() {
        // kbs=\177 in xterm terminfo; sending BS is why backspace sometimes
        // prints ^H instead of erasing.
        assert_eq!(key(Key::Backspace, modifiers::NONE), vec![0x7f]);
        assert_eq!(key(Key::Backspace, modifiers::CTRL), vec![0x08]);
        assert_eq!(key(Key::Backspace, modifiers::ALT), vec![0x1b, 0x7f]);
    }

    #[test]
    fn tab_and_back_tab() {
        assert_eq!(key(Key::Tab, modifiers::NONE), b"\t");
        assert_eq!(key(Key::Tab, modifiers::SHIFT), b"\x1b[Z");
    }

    // --- cursor keys ---

    #[test]
    fn cursor_keys_follow_the_application_mode() {
        let normal = InputModes::default();
        let application = InputModes {
            app_cursor: true,
            ..InputModes::default()
        };

        assert_eq!(key_in(Key::Up, modifiers::NONE, normal), b"\x1b[A");
        assert_eq!(key_in(Key::Up, modifiers::NONE, application), b"\x1bOA");
        assert_eq!(key_in(Key::Down, modifiers::NONE, application), b"\x1bOB");
        assert_eq!(key_in(Key::Right, modifiers::NONE, normal), b"\x1b[C");
        assert_eq!(key_in(Key::Left, modifiers::NONE, normal), b"\x1b[D");
    }

    /// Modified cursor keys use CSI even in application mode — a detail that
    /// breaks word-wise movement in shells when it is got wrong.
    #[test]
    fn modified_cursor_keys_always_use_csi() {
        let application = InputModes {
            app_cursor: true,
            ..InputModes::default()
        };

        assert_eq!(key_in(Key::Up, modifiers::CTRL, application), b"\x1b[1;5A");
        assert_eq!(key_in(Key::Left, modifiers::ALT, application), b"\x1b[1;3D");
        assert_eq!(
            key_in(Key::Right, modifiers::SHIFT, application),
            b"\x1b[1;2C"
        );
        assert_eq!(
            key_in(Key::Up, modifiers::CTRL | modifiers::SHIFT, application),
            b"\x1b[1;6A"
        );
    }

    #[test]
    fn modifier_parameters_match_the_xterm_table() {
        assert_eq!(modifier_parameter(modifiers::NONE), 1);
        assert_eq!(modifier_parameter(modifiers::SHIFT), 2);
        assert_eq!(modifier_parameter(modifiers::ALT), 3);
        assert_eq!(modifier_parameter(modifiers::ALT | modifiers::SHIFT), 4);
        assert_eq!(modifier_parameter(modifiers::CTRL), 5);
        assert_eq!(
            modifier_parameter(modifiers::CTRL | modifiers::ALT | modifiers::SHIFT),
            8
        );
    }

    #[test]
    fn home_and_end_follow_application_mode() {
        let application = InputModes {
            app_cursor: true,
            ..InputModes::default()
        };
        assert_eq!(key(Key::Home, modifiers::NONE), b"\x1b[H");
        assert_eq!(key(Key::End, modifiers::NONE), b"\x1b[F");
        assert_eq!(key_in(Key::Home, modifiers::NONE, application), b"\x1bOH");
    }

    #[test]
    fn editing_keys_use_tilde_sequences() {
        assert_eq!(key(Key::Insert, modifiers::NONE), b"\x1b[2~");
        assert_eq!(key(Key::Delete, modifiers::NONE), b"\x1b[3~");
        assert_eq!(key(Key::PageUp, modifiers::NONE), b"\x1b[5~");
        assert_eq!(key(Key::PageDown, modifiers::NONE), b"\x1b[6~");
        assert_eq!(key(Key::Delete, modifiers::SHIFT), b"\x1b[3;2~");
    }

    #[test]
    fn function_keys_match_the_xterm_table() {
        assert_eq!(key(Key::Function(1), modifiers::NONE), b"\x1bOP");
        assert_eq!(key(Key::Function(4), modifiers::NONE), b"\x1bOS");
        assert_eq!(key(Key::Function(5), modifiers::NONE), b"\x1b[15~");
        assert_eq!(key(Key::Function(6), modifiers::NONE), b"\x1b[17~");
        assert_eq!(key(Key::Function(10), modifiers::NONE), b"\x1b[21~");
        assert_eq!(key(Key::Function(12), modifiers::NONE), b"\x1b[24~");
        assert_eq!(key(Key::Function(1), modifiers::CTRL), b"\x1b[1;5P");
        assert!(key(Key::Function(21), modifiers::NONE).is_empty());
    }

    // --- mouse ---

    fn mouse_modes() -> InputModes {
        InputModes {
            mouse_click: true,
            sgr_mouse: true,
            ..InputModes::default()
        }
    }

    fn mouse(
        kind: MouseKind,
        button: u8,
        col: u16,
        row: u16,
        modes: InputModes,
    ) -> Option<Vec<u8>> {
        encode_mouse(
            MouseEvent {
                kind,
                button,
                col,
                row,
                mods: modifiers::NONE,
            },
            modes,
        )
    }

    #[test]
    fn sgr_mouse_reports_press_and_release_distinctly() {
        let modes = mouse_modes();
        assert_eq!(
            mouse(MouseKind::Press, 0, 0, 0, modes).unwrap(),
            b"\x1b[<0;1;1M"
        );
        assert_eq!(
            mouse(MouseKind::Release, 0, 0, 0, modes).unwrap(),
            b"\x1b[<0;1;1m"
        );
        assert_eq!(
            mouse(MouseKind::Press, 2, 9, 4, modes).unwrap(),
            b"\x1b[<2;10;5M"
        );
    }

    #[test]
    fn scroll_uses_the_wheel_button_codes() {
        let modes = mouse_modes();
        assert_eq!(
            mouse(MouseKind::ScrollUp, 0, 0, 0, modes).unwrap(),
            b"\x1b[<64;1;1M"
        );
        assert_eq!(
            mouse(MouseKind::ScrollDown, 0, 0, 0, modes).unwrap(),
            b"\x1b[<65;1;1M"
        );
    }

    #[test]
    fn mouse_modifiers_are_added_to_the_button_code() {
        let modes = mouse_modes();
        let event = MouseEvent {
            kind: MouseKind::Press,
            button: 0,
            col: 0,
            row: 0,
            mods: modifiers::CTRL | modifiers::SHIFT,
        };
        // 0 + shift(4) + ctrl(16)
        assert_eq!(encode_mouse(event, modes).unwrap(), b"\x1b[<20;1;1M");
    }

    #[test]
    fn nothing_is_reported_when_the_program_is_not_asking() {
        let modes = InputModes::default();
        assert!(mouse(MouseKind::Press, 0, 0, 0, modes).is_none());
        assert!(mouse(MouseKind::ScrollUp, 0, 0, 0, modes).is_none());
    }

    /// What makes the wheel scroll in `less` and `man`, which do not enable
    /// mouse reporting at all.
    #[test]
    fn wheel_becomes_arrow_keys_on_the_alternate_screen() {
        let modes = InputModes {
            alt_screen: true,
            alternate_scroll: true,
            ..InputModes::default()
        };
        assert_eq!(
            mouse(MouseKind::ScrollUp, 0, 0, 0, modes).unwrap(),
            b"\x1b[A"
        );
        assert_eq!(
            mouse(MouseKind::ScrollDown, 0, 0, 0, modes).unwrap(),
            b"\x1b[B"
        );

        // Not on the primary screen: there the scrollback should move instead.
        let primary = InputModes {
            alternate_scroll: true,
            ..InputModes::default()
        };
        assert!(mouse(MouseKind::ScrollUp, 0, 0, 0, primary).is_none());
    }

    #[test]
    fn motion_is_reported_only_when_tracked() {
        let click_only = mouse_modes();
        assert!(mouse(MouseKind::Motion, NO_BUTTON, 1, 1, click_only).is_none());

        let any_motion = InputModes {
            mouse_motion: true,
            ..mouse_modes()
        };
        assert!(mouse(MouseKind::Motion, NO_BUTTON, 1, 1, any_motion).is_some());

        // Drag mode reports motion only while a button is held.
        let drag = InputModes {
            mouse_drag: true,
            ..mouse_modes()
        };
        assert!(mouse(MouseKind::Motion, NO_BUTTON, 1, 1, drag).is_none());
        assert!(mouse(MouseKind::Motion, 0, 1, 1, drag).is_some());
    }

    #[test]
    fn legacy_encoding_offsets_by_32_and_reports_release_as_button_three() {
        let modes = InputModes {
            mouse_click: true,
            ..InputModes::default()
        };
        assert_eq!(
            mouse(MouseKind::Press, 0, 0, 0, modes).unwrap(),
            vec![0x1b, b'[', b'M', 32, 33, 33]
        );
        assert_eq!(
            mouse(MouseKind::Release, 0, 0, 0, modes).unwrap(),
            vec![0x1b, b'[', b'M', 35, 33, 33]
        );
    }

    /// The legacy encoding cannot express coordinates past 223; sending them
    /// anyway would place the click in the wrong cell.
    #[test]
    fn legacy_encoding_declines_coordinates_it_cannot_represent() {
        let modes = InputModes {
            mouse_click: true,
            ..InputModes::default()
        };
        assert!(mouse(MouseKind::Press, 0, 300, 0, modes).is_none());

        let sgr = mouse_modes();
        assert!(mouse(MouseKind::Press, 0, 300, 0, sgr).is_some());
    }

    // --- paste ---

    #[test]
    fn bracketed_paste_wraps_the_text() {
        let modes = InputModes {
            bracketed_paste: true,
            ..InputModes::default()
        };
        assert_eq!(encode_paste("hi", modes), b"\x1b[200~hi\x1b[201~");
    }

    #[test]
    fn paste_normalizes_newlines_to_carriage_returns() {
        let modes = InputModes {
            bracketed_paste: true,
            ..InputModes::default()
        };
        assert_eq!(encode_paste("a\nb", modes), b"\x1b[200~a\rb\x1b[201~");
        assert_eq!(encode_paste("a\r\nb", modes), b"\x1b[200~a\rb\x1b[201~");
    }

    /// Payload containing the end marker would otherwise terminate its own
    /// paste and let the remainder run as typed input.
    #[test]
    fn paste_cannot_be_terminated_by_its_own_content() {
        let modes = InputModes {
            bracketed_paste: true,
            ..InputModes::default()
        };
        let encoded = encode_paste("evil\x1b[201~rm -rf /", modes);
        let text = String::from_utf8(encoded).unwrap();

        assert_eq!(text.matches("\x1b[201~").count(), 1);
        assert!(text.ends_with("\x1b[201~"));
    }

    #[test]
    fn unbracketed_paste_strips_control_sequences() {
        let modes = InputModes::default();
        let encoded = encode_paste("a\x1b[31mb", modes);
        let text = String::from_utf8(encoded).unwrap();

        assert!(
            !text.contains('\x1b'),
            "escape survived an unbracketed paste: {text:?}"
        );
        assert!(text.contains('a') && text.contains('b'));
    }
}
