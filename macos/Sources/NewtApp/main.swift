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

// Offscreen mode: draw one frame to a PNG and exit, without a window.
if let flag = arguments.firstIndex(of: "--render-to"), flag + 1 < arguments.count {
    let path = arguments[flag + 1]
    let commandArgument = arguments.count > flag + 2 ? arguments[flag + 2] : nil
    let command = commandArgument == "--type" ? nil : commandArgument

    // Everything after --type is typed as key events, one step per argument.
    // A step of the form <name> is a named key, e.g. <enter> or <escape>.
    let typed: [String]
    if let typeFlag = arguments.firstIndex(of: "--type") {
        typed = Array(arguments[(typeFlag + 1)...])
    } else {
        typed = []
    }
    application.setActivationPolicy(.prohibited)
    do {
        try OfflineRender.run(
            command: command,
            typed: typed,
            outputPath: path,
            cols: 100,
            rows: 30,
            fontSize: 13
        )
        exit(0)
    } catch {
        FileHandle.standardError.write(Data("newt: \(error)\n".utf8))
        exit(1)
    }
}

let delegate = AppDelegate()
delegate.initialCommand = arguments.first
application.delegate = delegate

application.run()
