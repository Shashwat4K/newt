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
    func terminalView(
        _ view: TerminalView,
        startSelection col: UInt16,
        row: UInt16,
        sideRight: Bool,
        mode: SelectionMode
    )
    func terminalView(_ view: TerminalView, updateSelection col: UInt16, row: UInt16, sideRight: Bool)
    func terminalViewSelectedText(_ view: TerminalView) -> String?
    func terminalViewScrollToBottom(_ view: TerminalView)
    func terminalViewDidBecomeFocused(_ view: TerminalView)
}

extension TerminalView {
    override var acceptsFirstResponder: Bool { true }

    override func becomeFirstResponder() -> Bool {
        // Focus decides which pane receives keystrokes and which one the
        // window's commands act on, so the pane needs to know it moved.
        inputDelegate?.terminalViewDidBecomeFocused(self)
        return true
    }

    // MARK: - Keyboard

    override func keyDown(with event: NSEvent) {
        // Command chords are application shortcuts; the menu gets them.
        if event.modifierFlags.contains(.command) {
            super.keyDown(with: event)
            return
        }

        let modifiers = Self.modifiers(from: event)

        // Scrollback bindings are handled here rather than sent to the child.
        // These chords are ours: no program expects Shift-PageUp, and a
        // terminal you cannot scroll with the keyboard is a poor one.
        if modifiers.contains(.shift), let key = Self.specialKey(for: event) {
            let page = Int32(max(1, gridSize.rows - 1))
            switch key {
            case .pageUp:
                inputDelegate?.terminalView(self, scrollByLines: page)
                return
            case .pageDown:
                inputDelegate?.terminalView(self, scrollByLines: -page)
                return
            case .home:
                // Far enough to reach the top of any scrollback we keep.
                inputDelegate?.terminalView(self, scrollByLines: Int32.max / 2)
                return
            case .end:
                inputDelegate?.terminalViewScrollToBottom(self)
                return
            default:
                break
            }
        }

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

    // MARK: - Copy and paste

    /// Standard Edit ▸ Paste. Reaches here through the responder chain.
    @objc func paste(_ sender: Any?) {
        guard let text = NSPasteboard.general.string(forType: .string) else { return }
        inputDelegate?.terminalView(self, paste: text)
    }

    /// Standard Edit ▸ Copy.
    @objc func copy(_ sender: Any?) {
        guard let text = inputDelegate?.terminalViewSelectedText(self), !text.isEmpty else {
            return
        }
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(text, forType: .string)
    }

    /// Grey out Copy when there is nothing selected.
    ///
    /// Declared through NSMenuItemValidation rather than as an override: NSView
    /// does not declare this method.
    @objc func validateMenuItem(_ menuItem: NSMenuItem) -> Bool {
        if menuItem.action == #selector(copy(_:)) {
            return inputDelegate?.terminalViewSelectedText(self)?.isEmpty == false
        }
        return true
    }

    // MARK: - Mouse

    override func mouseDown(with event: NSEvent) {
        // A program that asked for mouse reporting gets the event; only when
        // nothing wants it does the click mean "select text".
        if sendMouse(.press, button: .left, event: event) { return }

        let (col, row) = cell(for: event)
        let mode: SelectionMode
        switch event.clickCount {
        case 2: mode = .word
        case 3: mode = .line
        default: mode = event.modifierFlags.contains(.option) ? .block : .simple
        }

        inputDelegate?.terminalView(
            self,
            startSelection: col,
            row: row,
            sideRight: isRightHalf(of: event),
            mode: mode
        )
    }

    override func mouseUp(with event: NSEvent) {
        _ = sendMouse(.release, button: .left, event: event)
    }

    override func mouseDragged(with event: NSEvent) {
        if sendMouse(.motion, button: .left, event: event) { return }

        let (col, row) = cell(for: event)
        inputDelegate?.terminalView(
            self,
            updateSelection: col,
            row: row,
            sideRight: isRightHalf(of: event)
        )
    }

    override func rightMouseDown(with event: NSEvent) {
        _ = sendMouse(.press, button: .right, event: event)
    }
    override func rightMouseUp(with event: NSEvent) {
        _ = sendMouse(.release, button: .right, event: event)
    }
    override func rightMouseDragged(with event: NSEvent) {
        _ = sendMouse(.motion, button: .right, event: event)
    }

    override func otherMouseDown(with event: NSEvent) {
        _ = sendMouse(.press, button: .middle, event: event)
    }
    override func otherMouseUp(with event: NSEvent) {
        _ = sendMouse(.release, button: .middle, event: event)
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

    /// - Returns: whether the program consumed the event.
    @discardableResult
    private func sendMouse(_ kind: MouseEventKind, button: MouseButton, event: NSEvent) -> Bool {
        let (col, row) = cell(for: event)
        return inputDelegate?.terminalView(
            self,
            sendMouse: kind,
            button: button,
            col: col,
            row: row,
            modifiers: Self.modifiers(from: event)
        ) ?? false
    }

    /// Whether the pointer is in the right half of its cell, which decides
    /// whether that cell is included in a selection.
    private func isRightHalf(of event: NSEvent) -> Bool {
        let point = convert(event.locationInWindow, from: nil)
        return point.x.truncatingRemainder(dividingBy: font.cellWidth) > font.cellWidth / 2
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
