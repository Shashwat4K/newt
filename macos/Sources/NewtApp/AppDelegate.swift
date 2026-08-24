import AppKit
import NewtKit

@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate {
    /// Windows are retained here; AppKit only holds them weakly once they are
    /// closed, and a released controller takes its sessions down with it.
    private var controllers: [TerminalWindowController] = []
    private let font = TerminalFont(size: 13)

    /// Command written into the first session at startup. Convenient for
    /// scripted demos; typing is the normal path.
    var initialCommand: String?

    func applicationDidFinishLaunching(_ notification: Notification) {
        installMenu()

        do {
            let controller = try makeWindow()
            controller.start(runningCommand: initialCommand)
        } catch {
            presentStartupFailure(error)
        }

        NSApp.activate(ignoringOtherApps: true)
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool {
        true
    }

    /// Clicking the dock icon with no windows open should give you a terminal,
    /// not nothing.
    func applicationShouldHandleReopen(
        _ sender: NSApplication,
        hasVisibleWindows: Bool
    ) -> Bool {
        if !hasVisibleWindows {
            newWindow(nil)
        }
        return true
    }

    // MARK: - Windows and tabs

    @discardableResult
    private func makeWindow() throws -> TerminalWindowController {
        let controller = try TerminalWindowController(font: font, cols: 100, rows: 30)
        controllers.append(controller)

        NotificationCenter.default.addObserver(
            forName: NSWindow.willCloseNotification,
            object: controller.hostWindow,
            queue: .main
        ) { [weak self] _ in
            MainActor.assumeIsolated {
                self?.controllers.removeAll { $0 === controller }
            }
        }

        return controller
    }

    @objc func newWindow(_ sender: Any?) {
        do {
            try makeWindow().start()
        } catch {
            presentStartupFailure(error, fatal: false)
        }
    }

    @objc func newTab(_ sender: Any?) {
        do {
            let controller = try makeWindow()
            // Attaching to the key window makes this a tab of it rather than a
            // separate window; native tabbing does the rest.
            if let current = NSApp.keyWindow {
                current.addTabbedWindow(controller.hostWindow, ordered: .above)
            }
            controller.start()
        } catch {
            presentStartupFailure(error, fatal: false)
        }
    }

    // MARK: - Menu

    private func installMenu() {
        let mainMenu = NSMenu()
        mainMenu.addItem(submenu(appMenu(), title: "newt"))
        mainMenu.addItem(submenu(shellMenu(), title: "Shell"))
        mainMenu.addItem(submenu(editMenu(), title: "Edit"))
        mainMenu.addItem(submenu(viewMenu(), title: "View"))
        NSApp.mainMenu = mainMenu
    }

    private func submenu(_ menu: NSMenu, title: String) -> NSMenuItem {
        let item = NSMenuItem()
        item.title = title
        item.submenu = menu
        return item
    }

    private func appMenu() -> NSMenu {
        let menu = NSMenu()
        menu.addItem(
            withTitle: "Quit newt",
            action: #selector(NSApplication.terminate(_:)),
            keyEquivalent: "q"
        )
        return menu
    }

    private func shellMenu() -> NSMenu {
        let menu = NSMenu(title: "Shell")
        menu.addItem(withTitle: "New Window", action: #selector(newWindow(_:)), keyEquivalent: "n")
        menu.addItem(withTitle: "New Tab", action: #selector(newTab(_:)), keyEquivalent: "t")
        menu.addItem(NSMenuItem.separator())
        menu.addItem(
            withTitle: "Split Right",
            action: #selector(TerminalWindowController.splitVertically(_:)),
            keyEquivalent: "d"
        )
        menu.addItem(
            withTitle: "Split Down",
            action: #selector(TerminalWindowController.splitHorizontally(_:)),
            keyEquivalent: "D"
        )
        menu.addItem(NSMenuItem.separator())
        menu.addItem(
            withTitle: "Close Pane",
            action: #selector(TerminalWindowController.closeFocusedPane(_:)),
            keyEquivalent: "w"
        )
        return menu
    }

    private func editMenu() -> NSMenu {
        let menu = NSMenu(title: "Edit")
        menu.addItem(withTitle: "Copy", action: #selector(NSText.copy(_:)), keyEquivalent: "c")
        menu.addItem(withTitle: "Paste", action: #selector(NSText.paste(_:)), keyEquivalent: "v")
        menu.addItem(NSMenuItem.separator())
        menu.addItem(
            withTitle: "Find…",
            action: #selector(TerminalWindowController.showFindBar(_:)),
            keyEquivalent: "f"
        )
        menu.addItem(
            withTitle: "Find Next",
            action: #selector(TerminalWindowController.findNext(_:)),
            keyEquivalent: "g"
        )
        menu.addItem(
            withTitle: "Find Previous",
            action: #selector(TerminalWindowController.findPrevious(_:)),
            keyEquivalent: "G"
        )
        return menu
    }

    private func viewMenu() -> NSMenu {
        let menu = NSMenu(title: "View")
        let next = NSMenuItem(
            title: "Select Next Pane",
            action: #selector(TerminalWindowController.focusNextPane(_:)),
            keyEquivalent: "]"
        )
        next.keyEquivalentModifierMask = [.command, .option]
        menu.addItem(next)

        let previous = NSMenuItem(
            title: "Select Previous Pane",
            action: #selector(TerminalWindowController.focusPreviousPane(_:)),
            keyEquivalent: "["
        )
        previous.keyEquivalentModifierMask = [.command, .option]
        menu.addItem(previous)
        return menu
    }

    private func presentStartupFailure(_ error: Error, fatal: Bool = true) {
        let alert = NSAlert()
        alert.messageText = "newt could not start a terminal session"
        alert.informativeText = String(describing: error)
        alert.alertStyle = fatal ? .critical : .warning
        alert.runModal()
        if fatal {
            NSApp.terminate(nil)
        }
    }
}
