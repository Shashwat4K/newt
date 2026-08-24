import CNewt

/// A key, independent of macOS key codes.
///
/// What a key *means* on the wire is decided in the core; this only names the
/// key so it can cross the ABI.
public enum TerminalKey: Equatable, Sendable {
    case character(Unicode.Scalar)
    case enter
    case tab
    case backspace
    case escape
    case delete
    case insert
    case up
    case down
    case left
    case right
    case home
    case end
    case pageUp
    case pageDown
    /// F1 through F20.
    case function(Int)

    var identifier: UInt32 {
        switch self {
        case .character(let scalar): return scalar.value
        case .enter: return UInt32(NEWT_KEY_ENTER)
        case .tab: return UInt32(NEWT_KEY_TAB)
        case .backspace: return UInt32(NEWT_KEY_BACKSPACE)
        case .escape: return UInt32(NEWT_KEY_ESCAPE)
        case .delete: return UInt32(NEWT_KEY_DELETE)
        case .insert: return UInt32(NEWT_KEY_INSERT)
        case .up: return UInt32(NEWT_KEY_UP)
        case .down: return UInt32(NEWT_KEY_DOWN)
        case .left: return UInt32(NEWT_KEY_LEFT)
        case .right: return UInt32(NEWT_KEY_RIGHT)
        case .home: return UInt32(NEWT_KEY_HOME)
        case .end: return UInt32(NEWT_KEY_END)
        case .pageUp: return UInt32(NEWT_KEY_PAGE_UP)
        case .pageDown: return UInt32(NEWT_KEY_PAGE_DOWN)
        case .function(let number): return UInt32(NEWT_KEY_F1) &+ UInt32(max(1, number) - 1)
        }
    }
}

public struct KeyModifiers: OptionSet, Sendable {
    public let rawValue: UInt8
    public init(rawValue: UInt8) { self.rawValue = rawValue }

    public static let shift = KeyModifiers(rawValue: UInt8(NEWT_MOD_SHIFT))
    public static let option = KeyModifiers(rawValue: UInt8(NEWT_MOD_ALT))
    public static let control = KeyModifiers(rawValue: UInt8(NEWT_MOD_CTRL))
    /// Command. Never reaches the child — it drives application shortcuts.
    public static let command = KeyModifiers(rawValue: UInt8(NEWT_MOD_SUPER))
}

public enum MouseEventKind: Sendable {
    case press
    case release
    case motion
    case scrollUp
    case scrollDown

    var identifier: UInt8 {
        switch self {
        case .press: return UInt8(NEWT_MOUSE_PRESS)
        case .release: return UInt8(NEWT_MOUSE_RELEASE)
        case .motion: return UInt8(NEWT_MOUSE_MOTION)
        case .scrollUp: return UInt8(NEWT_MOUSE_SCROLL_UP)
        case .scrollDown: return UInt8(NEWT_MOUSE_SCROLL_DOWN)
        }
    }
}

public enum MouseButton: UInt8, Sendable {
    case left = 0
    case middle = 1
    case right = 2
    /// No button held, for motion events.
    case none = 255
}

extension TerminalSession {
    /// Send a key press.
    public func send(key: TerminalKey, modifiers: KeyModifiers = []) throws {
        guard newt_session_send_key(rawHandle, key.identifier, modifiers.rawValue) else {
            throw TerminalError.lastError(fallback: "sending a key failed")
        }
    }

    /// Send text the platform produced directly, such as an IME commit.
    public func send(text: String) throws {
        guard !text.isEmpty else { return }
        var utf8 = Array(text.utf8)
        let ok = utf8.withUnsafeMutableBufferPointer { buffer in
            newt_session_send_text(rawHandle, buffer.baseAddress, UInt(buffer.count))
        }
        guard ok else { throw TerminalError.lastError(fallback: "sending text failed") }
    }

    /// Send a mouse event.
    ///
    /// - Returns: whether the program wanted it. When false the caller should
    ///   apply its own behavior, such as scrolling the viewport.
    @discardableResult
    public func send(
        mouse kind: MouseEventKind,
        button: MouseButton = .none,
        col: UInt16,
        row: UInt16,
        modifiers: KeyModifiers = []
    ) throws -> Bool {
        var handled = false
        let ok = newt_session_send_mouse(
            rawHandle,
            kind.identifier,
            button.rawValue,
            col,
            row,
            modifiers.rawValue,
            &handled
        )
        guard ok else { throw TerminalError.lastError(fallback: "sending a mouse event failed") }
        return handled
    }

    /// Paste text, bracketed when the program has asked for it.
    public func paste(_ text: String) throws {
        guard !text.isEmpty else { return }
        var utf8 = Array(text.utf8)
        let ok = utf8.withUnsafeMutableBufferPointer { buffer in
            newt_session_send_paste(rawHandle, buffer.baseAddress, UInt(buffer.count))
        }
        guard ok else { throw TerminalError.lastError(fallback: "pasting failed") }
    }
}
