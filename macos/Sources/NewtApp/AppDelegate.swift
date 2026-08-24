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

        let mainMenu = NSMenu()
        mainMenu.addItem(appMenuItem)
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
