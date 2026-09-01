import AppKit
import Foundation

// Phase 3: a window showing live shell output. Output-only — the input path
// arrives in Phase 4, so an optional argument is written into the session at
// startup to demonstrate that output reaches the screen.
//
//     NewtApp                       # just the shell prompt
//     NewtApp "ls -la"              # run a command at startup

let application = NSApplication.shared
// Without a bundle there is no Info.plist to say this is a GUI app, so the
// activation policy has to be set explicitly or the window never focuses.
application.setActivationPolicy(.regular)

let arguments = Array(CommandLine.arguments.dropFirst())

// Verification paths run a known shell so a check never depends on whose
// dotfiles are installed. --login-shell opts back into the real one, for when
// the point is real-world rendering: Nerd Font glyphs, a themed prompt.
let verificationShell: String? =
    arguments.contains("--login-shell") ? nil : OfflineRender.verificationShell

// Headless check that a backgrounded tab keeps running. No window, no PNG.
if arguments.contains("--background-check") {
    application.setActivationPolicy(.prohibited)
    do {
        exit(
            try OfflineRender.runBackgroundCheck(fontSize: 13, shell: verificationShell) ? 0 : 1
        )
    } catch {
        FileHandle.standardError.write(Data("newt: \(error)\n".utf8))
        exit(1)
    }
}

// Offscreen mode: draw one frame to a PNG and exit, without a window.
if let flag = arguments.firstIndex(of: "--render-to"), flag + 1 < arguments.count {
    let path = arguments[flag + 1]
    // Any flag here is a flag, not a command to type. Phase 5 logged the same
    // bug from the other direction, where `--keys` swallowed the flags after
    // it and typed `--resize 40 8` into vim as literal text.
    let commandArgument = arguments.count > flag + 2 ? arguments[flag + 2] : nil
    let command = (commandArgument?.hasPrefix("--") ?? true) ? nil : commandArgument

    // Everything after --type is typed as key events, one step per argument.
    // A step of the form <name> is a named key, e.g. <enter> or <escape>.
    let typed: [String]
    if let typeFlag = arguments.firstIndex(of: "--type") {
        var end = typeFlag + 1
        while end < arguments.count, !arguments[end].hasPrefix("--") { end += 1 }
        typed = Array(arguments[(typeFlag + 1)..<end])
    } else {
        typed = []
    }

    var find: String?
    if let findFlag = arguments.firstIndex(of: "--find"), findFlag + 1 < arguments.count {
        find = arguments[findFlag + 1]
    }
    application.setActivationPolicy(.prohibited)

    // --panes N splits the selected tab; --tabs N opens N tabs in the sidebar.
    // Either one renders the whole window rather than a single grid.
    func intFlag(_ name: String) -> Int? {
        guard let flag = arguments.firstIndex(of: name), flag + 1 < arguments.count else {
            return nil
        }
        return Int(arguments[flag + 1])
    }

    let panes = intFlag("--panes")
    let tabs = intFlag("--tabs")
    if panes != nil || tabs != nil {
        do {
            try OfflineRender.runSplit(
                panes: panes ?? 1,
                tabs: tabs ?? 1,
                outputPath: path,
                fontSize: 13,
                shell: verificationShell
            )
            exit(0)
        } catch {
            FileHandle.standardError.write(Data("newt: \(error)\n".utf8))
            exit(1)
        }
    }

    do {
        try OfflineRender.run(
            command: command,
            typed: typed,
            find: find,
            outputPath: path,
            cols: 100,
            rows: 30,
            fontSize: 13,
            shell: verificationShell
        )
        exit(0)
    } catch {
        FileHandle.standardError.write(Data("newt: \(error)\n".utf8))
        exit(1)
    }
}

let delegate = AppDelegate()
// Launched from Finder, macOS passes arguments of its own (-psn_0_12345 and
// friends). Typing those into the user's shell would be a memorable bug.
delegate.initialCommand = arguments.first.flatMap { $0.hasPrefix("-") ? nil : $0 }
application.delegate = delegate

application.run()
