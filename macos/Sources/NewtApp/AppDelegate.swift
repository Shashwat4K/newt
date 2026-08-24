import AppKit

@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate {
    private var controller: TerminalWindowController?

    /// Command written into the session once it starts. Temporary: there is no
    /// input path until Phase 4.
    var initialCommand: String?

    func applicationDidFinishLaunching(_ notification: Notification) {
        installMenu()

        do {
            let controller = try TerminalWindowController(cols: 100, rows: 30, fontSize: 13)
            controller.start(runningCommand: initialCommand)
            self.controller = controller
        } catch {
            presentStartupFailure(error)
        }

        NSApp.activate(ignoringOtherApps: true)
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool {
        true
    }

    /// Minimal menu so the standard shortcuts work. There is no bundle yet, so
    /// nothing supplies one for us.
    private func installMenu() {
        let appMenu = NSMenu()
        appMenu.addItem(
            withTitle: "Quit newt",
            action: #selector(NSApplication.terminate(_:)),
            keyEquivalent: "q"
        )

        let appMenuItem = NSMenuItem()
        appMenuItem.submenu = appMenu

        // Paste goes through the responder chain to the terminal view, which
        // is what lets the core wrap it in bracketed-paste markers.
        let editMenu = NSMenu(title: "Edit")
        editMenu.addItem(
            withTitle: "Copy",
            action: #selector(NSText.copy(_:)),
            keyEquivalent: "c"
        )
        editMenu.addItem(
            withTitle: "Paste",
            action: #selector(NSText.paste(_:)),
            keyEquivalent: "v"
        )
        editMenu.addItem(NSMenuItem.separator())
        editMenu.addItem(
            withTitle: "Find…",
            action: #selector(TerminalWindowController.showFindBar(_:)),
            keyEquivalent: "f"
        )
        editMenu.addItem(
            withTitle: "Find Next",
            action: #selector(TerminalWindowController.findNext(_:)),
            keyEquivalent: "g"
        )
        editMenu.addItem(
            withTitle: "Find Previous",
            action: #selector(TerminalWindowController.findPrevious(_:)),
            keyEquivalent: "G"
        )
        let editMenuItem = NSMenuItem()
        editMenuItem.title = "Edit"
        editMenuItem.submenu = editMenu

        let mainMenu = NSMenu()
        mainMenu.addItem(appMenuItem)
        mainMenu.addItem(editMenuItem)
        NSApp.mainMenu = mainMenu
    }

    private func presentStartupFailure(_ error: Error) {
        let alert = NSAlert()
        alert.messageText = "newt could not start a terminal session"
        alert.informativeText = String(describing: error)
        alert.alertStyle = .critical
        alert.runModal()
        NSApp.terminate(nil)
    }
}
