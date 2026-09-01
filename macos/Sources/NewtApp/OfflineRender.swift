import AppKit
import Foundation
import NewtKit

/// Renders a session to a PNG without showing a window.
///
/// Exists because verifying a renderer by looking at it does not scale and
/// cannot run unattended: this draws through exactly the same `TerminalView`
/// code path the window uses, so a blank or malformed frame is catchable
/// without a display, a screenshot, or accessibility permissions.
@MainActor
enum OfflineRender {
    static func run(
        command: String?,
        typed: [String] = [],
        find: String? = nil,
        outputPath: String,
        cols: UInt16,
        rows: UInt16,
        fontSize: CGFloat
    ) throws {
        let font = TerminalFont(size: fontSize)
        let session = try TerminalSession(size: TerminalSize(cols: cols, rows: rows))
        session.setCellSize(
            width: UInt16(font.cellWidth.rounded()),
            height: UInt16(font.cellHeight.rounded())
        )

        waitForOutput(session)
        if let command {
            try session.write("\(command)\n")
            settle(session)
        }

        // Typed steps go through the real input path — key events encoded by
        // the core — rather than through `write`, so this exercises what the
        // keyboard actually does.
        for step in typed {
            try type(step, into: session)
            FileHandle.standardError.write(Data("typed \(step)\n".utf8))
            // A longer quiet period than the initial settle: a full-screen
            // program can look idle mid-startup and then flush pending input
            // when it finally enters raw mode, swallowing what was typed.
            settle(session, quietPeriod: 0.9)
            let preview = (try? session.withSnapshot { $0.text() })?
                .split(separator: "\n")
                .prefix(2)
                .joined(separator: " | ") ?? ""
            FileHandle.standardError.write(Data("  -> \(preview)\n".utf8))
        }

        // Searching selects the match, which is what draws the highlight —
        // so this also exercises selection rendering.
        if let find {
            let found = try session.find(find)
            FileHandle.standardError.write(Data("find \(find): \(found)\n".utf8))
        }

        let view = TerminalView(font: font, cols: Int(cols), rows: Int(rows))
        try session.withSnapshot { view.apply($0) }

        guard let bitmap = view.bitmapImageRepForCachingDisplay(in: view.bounds) else {
            throw RenderError.couldNotAllocateBitmap
        }
        view.cacheDisplay(in: view.bounds, to: bitmap)

        guard let png = bitmap.representation(using: .png, properties: [:]) else {
            throw RenderError.couldNotEncodePNG
        }
        try png.write(to: URL(fileURLWithPath: outputPath))

        let text = try session.withSnapshot { $0.text() }
        FileHandle.standardError.write(
            Data("wrote \(outputPath) (\(bitmap.pixelsWide)x\(bitmap.pixelsHigh))\n\(text)\n".utf8)
        )
    }

    /// Send one step as key events. Named keys are written as `<name>`.
    private static func type(_ step: String, into session: TerminalSession) throws {
        if step.hasPrefix("<"), step.hasSuffix(">") {
            let name = String(step.dropFirst().dropLast()).lowercased()
            guard let key = namedKey(name) else { return }
            try session.send(key: key)
            return
        }

        for scalar in step.unicodeScalars {
            try session.send(key: .character(scalar))
        }
    }

    private static func namedKey(_ name: String) -> TerminalKey? {
        switch name {
        case "enter", "return": return .enter
        case "escape", "esc": return .escape
        case "tab": return .tab
        case "backspace": return .backspace
        case "up": return .up
        case "down": return .down
        case "left": return .left
        case "right": return .right
        case "home": return .home
        case "end": return .end
        case "pageup": return .pageUp
        case "pagedown": return .pageDown
        default: return nil
        }
    }

    /// Build a window, split it, and draw the whole pane tree to a PNG.
    ///
    /// Verifies the split layout without a display: panes, dividers, and each
    /// pane's independently sized grid all go through the same code the real
    /// window uses.
    static func runSplit(panes: Int, tabs: Int, outputPath: String, fontSize: CGFloat) throws {
        let font = TerminalFont(size: fontSize)
        let controller = try TerminalWindowController(font: font, cols: 80, rows: 24)
        controller.start()

        // Extra tabs first, so the split below lands in the tab left selected.
        for _ in 0..<max(0, tabs - 1) {
            controller.newTab(nil)
        }

        for index in 0..<max(0, panes - 1) {
            // Alternate direction so the result is a tree, not a single row.
            if index % 2 == 0 {
                controller.splitVertically(nil)
            } else {
                controller.splitHorizontally(nil)
            }
        }

        // Let each shell draw its prompt before capturing.
        let deadline = Date().addingTimeInterval(8)
        while Date() < deadline {
            RunLoop.main.run(until: Date().addingTimeInterval(0.05))
            let ready = controller.panes.allSatisfy { pane in
                ((try? pane.session.withSnapshot { !$0.text().isEmpty }) ?? false)
            }
            if ready { break }
        }

        // Then wait for the screen to stop changing. First output is not the
        // same as a finished prompt: splitting resizes every pane, which makes
        // the shell redraw, and capturing during that redraw yields the mostly
        // blank panes this check exists to prevent.
        var last = paneText(controller)
        var unchangedSince = Date()
        let settleDeadline = Date().addingTimeInterval(6)
        while Date() < settleDeadline {
            RunLoop.main.run(until: Date().addingTimeInterval(0.05))
            let current = paneText(controller)
            if current != last {
                last = current
                unchangedSince = Date()
            } else if Date().timeIntervalSince(unchangedSince) >= 0.6 {
                break
            }
        }

        guard let content = controller.hostWindow.contentView else {
            throw RenderError.couldNotAllocateBitmap
        }
        guard let bitmap = content.bitmapImageRepForCachingDisplay(in: content.bounds) else {
            throw RenderError.couldNotAllocateBitmap
        }
        content.cacheDisplay(in: content.bounds, to: bitmap)

        guard let png = bitmap.representation(using: .png, properties: [:]) else {
            throw RenderError.couldNotEncodePNG
        }
        try png.write(to: URL(fileURLWithPath: outputPath))

        let sizes = controller.panes.map { pane in
            let size = (try? pane.session.withSnapshot { ($0.cols, $0.rows) }) ?? (0, 0)
            return "\(size.0)x\(size.1)"
        }
        // Pixel frames alongside the grids: a degenerate pane is almost always
        // a layout problem rather than a session one, and the grid sizes alone
        // do not say which view collapsed.
        let frames = controller.panes.map { pane in
            let frame = pane.view.frame
            return "\(Int(frame.width))x\(Int(frame.height))"
        }
        FileHandle.standardError.write(
            Data(
                """
                wrote \(outputPath): \(controller.panes.count) panes \(sizes)
                  content \(Int(content.bounds.width))x\(Int(content.bounds.height)) \
                pane frames \(frames)
                  \(controller.debugLayout)

                """.utf8)
        )
    }

    /// Prove that a tab in the background keeps running.
    ///
    /// This is the assumption the whole sidebar rests on: selecting away from a
    /// tab suspends its display link, and *only* its display link — the PTY
    /// reader thread and the parser live in the core and never stop. If that
    /// were wrong, every background agent would silently stall the moment you
    /// looked at another tab, which is the one failure this design cannot
    /// afford. Checked rather than assumed, and cheap to keep checking.
    ///
    /// - Returns: true if the backgrounded tab produced its output.
    static func runBackgroundCheck(fontSize: CGFloat) throws -> Bool {
        let marker = "BACKGROUND_OK"
        let font = TerminalFont(size: fontSize)
        let controller = try TerminalWindowController(font: font, cols: 80, rows: 24)
        controller.start()

        guard let first = controller.selectedTab, let pane = first.focusedPane else { return false }
        spin(seconds: 1.5)

        // Output arrives while the tab is in the background, not before it.
        try pane.session.write("sleep 2; echo \(marker)\n")

        controller.newTab(nil)
        guard controller.selectedTab !== first else {
            FileHandle.standardError.write(Data("background check: new tab was not selected\n".utf8))
            return false
        }
        FileHandle.standardError.write(
            Data("background check: switched away, first tab active=\(first.isActive)\n".utf8)
        )

        spin(seconds: 4)
        controller.selectPreviousTab(nil)
        spin(seconds: 1)

        let text = (try? pane.session.withSnapshot { $0.text() }) ?? ""
        let found = text.contains(marker)
        FileHandle.standardError.write(
            Data("background check: \(found ? "PASS" : "FAIL") — marker \(marker)\n".utf8)
        )
        if !found {
            FileHandle.standardError.write(Data("\(text)\n".utf8))
        }
        return found
    }

    private static func spin(seconds: TimeInterval) {
        let deadline = Date().addingTimeInterval(seconds)
        while Date() < deadline {
            RunLoop.main.run(until: Date().addingTimeInterval(0.05))
        }
    }

    /// Every pane's visible text, as one string, for settling.
    private static func paneText(_ controller: TerminalWindowController) -> String {
        controller.panes
            .map { (try? $0.session.withSnapshot { $0.text() }) ?? "" }
            .joined(separator: "\u{1}")
    }

    enum RenderError: Error {
        case couldNotAllocateBitmap
        case couldNotEncodePNG
    }

    /// Wait for the shell to draw its prompt. An empty screen must not be
    /// mistaken for a settled one — a heavy startup takes seconds.
    private static func waitForOutput(_ session: TerminalSession, timeout: TimeInterval = 10) {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            Thread.sleep(forTimeInterval: 0.02)
            if let hasOutput = try? session.withSnapshot({ !$0.text().isEmpty }), hasOutput {
                return
            }
        }
    }

    private static func settle(
        _ session: TerminalSession,
        quietPeriod: TimeInterval = 0.4,
        timeout: TimeInterval = 10
    ) {
        let deadline = Date().addingTimeInterval(timeout)
        var last = (try? session.withSnapshot { $0.text() }) ?? ""
        var unchangedSince = Date()

        while Date() < deadline {
            Thread.sleep(forTimeInterval: 0.02)
            let current = (try? session.withSnapshot { $0.text() }) ?? ""
            if current != last {
                last = current
                unchangedSince = Date()
            } else if Date().timeIntervalSince(unchangedSince) >= quietPeriod {
                return
            }
        }
    }
}
