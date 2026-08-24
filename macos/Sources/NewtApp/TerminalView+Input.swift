import AppKit
import NewtKit

/// Where the view sends the input it captures.
///
/// The view knows how to *capture* events; the controller owns the session that
/// receives them. No encoding happens on either side — that is the core's job.
@MainActor
protocol TerminalInputDelegate: AnyObject {
    func terminalView(_ view: TerminalView, send key: TerminalKey, modifiers: KeyModifiers)
    func terminalView(_ view: TerminalView, sendText text: String)
    @discardableResult
    func terminalView(
        _ view: TerminalView,
        sendMouse kind: MouseEventKind,
        button: MouseButton,
        col: UInt16,
        row: UInt16,
        modifiers: KeyModifiers
    ) -> Bool
    func terminalView(_ view: TerminalView, scrollByLines lines: Int32)
    func terminalView(_ view: TerminalView, paste text: String)
}

extension TerminalView {
    override var acceptsFirstResponder: Bool { true }

    override func becomeFirstResponder() -> Bool { true }

    // MARK: - Keyboard

    override func keyDown(with event: NSEvent) {
        // Command chords are application shortcuts; the menu gets them.
        if event.modifierFlags.contains(.command) {
            super.keyDown(with: event)
            return
        }

        let modifiers = Self.modifiers(from: event)

        if let key = Self.specialKey(for: event) {
            inputDelegate?.terminalView(self, send: key, modifiers: modifiers)
            return
        }

        // Control and Option chords are encoded from the unmodified character.
        // Routing them through the text input system instead would have it
        // swallow them as editing commands — Control-A becoming "move to start
        // of line" rather than reaching the shell.
        if modifiers.contains(.control) || modifiers.contains(.option) {
            if let scalar = event.charactersIgnoringModifiers?.unicodeScalars.first {
                inputDelegate?.terminalView(self, send: .character(scalar), modifiers: modifiers)
                return
            }
        }

        // Everything else is ordinary typing, which must go through the input
        // context so dead keys and IME composition work.
        inputContext?.handleEvent(event)
    }

    override func flagsChanged(with event: NSEvent) {
        super.flagsChanged(with: event)
    }

    static func modifiers(from event: NSEvent) -> KeyModifiers {
        var modifiers: KeyModifiers = []
        let flags = event.modifierFlags
        if flags.contains(.shift) { modifiers.insert(.shift) }
        if flags.contains(.option) { modifiers.insert(.option) }
        if flags.contains(.control) { modifiers.insert(.control) }
        if flags.contains(.command) { modifiers.insert(.command) }
        return modifiers
    }

    /// Map an event to a named key, or nil when it is ordinary text.
    static func specialKey(for event: NSEvent) -> TerminalKey? {
        // Escape has no `specialKey` value, so it is matched by key code.
        if event.keyCode == 53 { return .escape }

        guard let special = event.specialKey else { return nil }

        switch special {
        case .upArrow: return .up
        case .downArrow: return .down
        case .leftArrow: return .left
        case .rightArrow: return .right
        case .home: return .home
        case .end: return .end
        case .pageUp: return .pageUp
        case .pageDown: return .pageDown
        // On macOS the Delete key sends backspace; forward delete is separate.
        case .delete: return .backspace
        case .deleteForward: return .delete
        case .insert: return .insert
        case .carriageReturn, .enter, .newline: return .enter
        case .tab, .backTab: return .tab
        case .f1: return .function(1)
        case .f2: return .function(2)
        case .f3: return .function(3)
        case .f4: return .function(4)
        case .f5: return .function(5)
        case .f6: return .function(6)
        case .f7: return .function(7)
        case .f8: return .function(8)
        case .f9: return .function(9)
        case .f10: return .function(10)
        case .f11: return .function(11)
        case .f12: return .function(12)
        default: return nil
        }
    }

    // MARK: - Paste

    /// Standard Edit ▸ Paste. Reaches here through the responder chain.
    @objc func paste(_ sender: Any?) {
        guard let text = NSPasteboard.general.string(forType: .string) else { return }
        inputDelegate?.terminalView(self, paste: text)
    }

    // MARK: - Mouse

    override func mouseDown(with event: NSEvent) { sendMouse(.press, button: .left, event: event) }
    override func mouseUp(with event: NSEvent) { sendMouse(.release, button: .left, event: event) }
    override func mouseDragged(with event: NSEvent) {
        sendMouse(.motion, button: .left, event: event)
    }

    override func rightMouseDown(with event: NSEvent) {
        sendMouse(.press, button: .right, event: event)
    }
    override func rightMouseUp(with event: NSEvent) {
        sendMouse(.release, button: .right, event: event)
    }
    override func rightMouseDragged(with event: NSEvent) {
        sendMouse(.motion, button: .right, event: event)
    }

    override func otherMouseDown(with event: NSEvent) {
        sendMouse(.press, button: .middle, event: event)
    }
    override func otherMouseUp(with event: NSEvent) {
        sendMouse(.release, button: .middle, event: event)
    }

    override func scrollWheel(with event: NSEvent) {
        let lines = Self.scrollLines(from: event, cellHeight: font.cellHeight)
        guard lines != 0 else { return }

        let kind: MouseEventKind = lines > 0 ? .scrollUp : .scrollDown
        let (col, row) = cell(for: event)
        var handled = false

        for _ in 0..<abs(lines) {
            handled =
                inputDelegate?
                .terminalView(
                    self,
                    sendMouse: kind,
                    button: .none,
                    col: col,
                    row: row,
                    modifiers: Self.modifiers(from: event)
                ) ?? false
        }

        // Nothing wanted the wheel, so it moves our own viewport instead.
        if !handled {
            inputDelegate?.terminalView(self, scrollByLines: Int32(lines))
        }
    }

    private func sendMouse(_ kind: MouseEventKind, button: MouseButton, event: NSEvent) {
        let (col, row) = cell(for: event)
        inputDelegate?.terminalView(
            self,
            sendMouse: kind,
            button: button,
            col: col,
            row: row,
            modifiers: Self.modifiers(from: event)
        )
    }

    /// Cell under the pointer, clamped to the grid so a drag past the edge
    /// still reports a valid position.
    private func cell(for event: NSEvent) -> (col: UInt16, row: UInt16) {
        let point = convert(event.locationInWindow, from: nil)
        let size = gridSize
        guard size.cols > 0, size.rows > 0 else { return (0, 0) }

        let col = Int(point.x / font.cellWidth)
        let row = Int((bounds.height - point.y) / font.cellHeight)
        return (
            UInt16(min(max(col, 0), size.cols - 1)),
            UInt16(min(max(row, 0), size.rows - 1))
        )
    }

    /// Wheel movement in whole lines.
    ///
    /// Trackpads report pixel-precise deltas, which have to be divided by the
    /// cell height or a gentle swipe scrolls the buffer by hundreds of lines.
    static func scrollLines(from event: NSEvent, cellHeight: CGFloat) -> Int {
        if event.hasPreciseScrollingDeltas {
            return Int((event.scrollingDeltaY / cellHeight).rounded())
        }
        return Int(event.scrollingDeltaY.rounded())
    }
}
