import AppKit
import NewtKit

/// Text input: dead keys, accents, and full IME composition.
///
/// Without this, typing `é` on a US layout via Option-E-E, or anything through
/// a Japanese or Chinese input method, cannot work — the composition state
/// lives in the input system and only reaches us through this protocol.
// @preconcurrency: NSTextInputClient predates Swift concurrency and declares
// its requirements nonisolated, while every implementation here touches
// main-actor state. The input system only ever calls these on the main thread.
extension TerminalView: @preconcurrency NSTextInputClient {
    /// A committed string: composition finished, or an ordinary keypress.
    func insertText(_ string: Any, replacementRange: NSRange) {
        let text: String
        switch string {
        case let value as String: text = value
        case let value as NSAttributedString: text = value.string
        default: return
        }

        markedText = ""
        guard !text.isEmpty else { return }
        inputDelegate?.terminalView(self, sendText: text)
    }

    /// Editing commands the input system inferred from a keypress.
    ///
    /// Deliberately ignored: named keys are matched in `keyDown` and encoded by
    /// the core. Acting here as well would send Return or Tab twice.
    override func doCommand(by selector: Selector) {}

    func setMarkedText(_ string: Any, selectedRange: NSRange, replacementRange: NSRange) {
        switch string {
        case let value as String: markedText = value
        case let value as NSAttributedString: markedText = value.string
        default: markedText = ""
        }
    }

    func unmarkText() {
        markedText = ""
    }

    func selectedRange() -> NSRange {
        // No selection model yet; Phase 6.
        NSRange(location: NSNotFound, length: 0)
    }

    func markedRange() -> NSRange {
        markedText.isEmpty
            ? NSRange(location: NSNotFound, length: 0)
            : NSRange(location: 0, length: markedText.utf16.count)
    }

    func hasMarkedText() -> Bool {
        !markedText.isEmpty
    }

    func attributedSubstring(forProposedRange range: NSRange, actualRange: NSRangePointer?) -> NSAttributedString? {
        nil
    }

    func validAttributesForMarkedText() -> [NSAttributedString.Key] {
        []
    }

    /// Where the input method should put its candidate window: at the cursor,
    /// in screen coordinates.
    func firstRect(forCharacterRange range: NSRange, actualRange: NSRangePointer?) -> NSRect {
        let inView = cursorRectInView
        let inWindow = convert(inView, to: nil)
        return window?.convertToScreen(inWindow) ?? inWindow
    }

    func characterIndex(for point: NSPoint) -> Int {
        0
    }
}
