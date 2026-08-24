import AppKit

/// A small search field overlaid on the terminal.
///
/// Overlaid rather than docked so showing it does not resize the grid — a find
/// bar that reflows the whole scrollback on open would be a strange way to
/// look for something in it.
@MainActor
final class FindBar: NSView, NSTextFieldDelegate {
    /// Runs a search. Returns whether anything was found.
    var onFind: ((String, Bool) -> Bool)?
    /// Called when the bar closes, so focus can return to the terminal.
    var onClose: (() -> Void)?

    private let field = NSTextField()

    init() {
        super.init(frame: NSRect(x: 0, y: 0, width: 280, height: 32))

        wantsLayer = true
        layer?.backgroundColor = NSColor.windowBackgroundColor.cgColor
        layer?.cornerRadius = 6
        layer?.borderWidth = 1
        layer?.borderColor = NSColor.separatorColor.cgColor

        field.frame = NSRect(x: 8, y: 6, width: 190, height: 20)
        field.placeholderString = "Find"
        field.bezelStyle = .roundedBezel
        field.delegate = self
        field.focusRingType = .none
        addSubview(field)

        addButton(title: "<", x: 204) { [weak self] in self?.search(forward: false) }
        addButton(title: ">", x: 234) { [weak self] in self?.search(forward: true) }
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("newt does not use storyboards")
    }

    /// Show and take focus, keeping any previous query so repeating a search is
    /// a single keystroke.
    func focus() {
        isHidden = false
        window?.makeFirstResponder(field)
        field.currentEditor()?.selectAll(nil)
    }

    func close() {
        isHidden = true
        onClose?()
    }

    /// Run the current query again, for Find Next and Find Previous.
    func repeatSearch(forward: Bool) {
        search(forward: forward)
    }

    private func search(forward: Bool) {
        let query = field.stringValue
        guard !query.isEmpty else { return }

        let found = onFind?(query, forward) ?? false
        // Red text is the whole "not found" affordance: a terminal find bar
        // that grows an error label would be more chrome than the MVP needs.
        field.textColor = found ? .labelColor : .systemRed
    }

    private func addButton(title: String, x: CGFloat, action: @escaping () -> Void) {
        let button = ActionButton(title: title, action: action)
        button.frame = NSRect(x: x, y: 4, width: 26, height: 24)
        button.bezelStyle = .rounded
        addSubview(button)
    }

    func controlTextDidChange(_ notification: Notification) {
        // Typing restarts the search from the current match, so results update
        // as the query narrows.
        field.textColor = .labelColor
    }

    func control(_ control: NSControl, textView: NSTextView, doCommandBy selector: Selector) -> Bool {
        switch selector {
        case #selector(NSResponder.insertNewline(_:)):
            search(forward: !NSEvent.modifierFlags.contains(.shift))
            return true
        case #selector(NSResponder.cancelOperation(_:)):
            close()
            return true
        default:
            return false
        }
    }
}

/// A button that calls a closure, so the find bar does not need a target object
/// per control.
@MainActor
private final class ActionButton: NSButton {
    private let handler: () -> Void

    init(title: String, action: @escaping () -> Void) {
        handler = action
        super.init(frame: .zero)
        self.title = title
        target = self
        self.action = #selector(invoke)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("newt does not use storyboards")
    }

    @objc private func invoke() {
        handler()
    }
}
