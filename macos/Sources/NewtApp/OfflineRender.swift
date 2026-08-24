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
