import AppKit
import Foundation

/// One app-wide timer that polls every session's out-of-band state.
///
/// newt now has two clocks, and keeping them apart is the point:
///
/// - the **render tick** is a `CADisplayLink` per pane, foreground only, at
///   screen rate. It draws.
/// - the **status tick** is this, one timer for the whole app at 5 Hz, covering
///   every session in every tab in every window. It notices titles, exits, and
///   agent state.
///
/// Before tabs, both jobs lived in the display link. That stops working the
/// moment a tab can be in the background: a suspended pane's link does not
/// fire, so a shell that exited there would never be reaped and its tab would
/// sit in the sidebar forever.
///
/// 5 Hz is chosen against what it drives — a person reading a sidebar, not an
/// animation. The pulsing indicator is Core Animation and runs independently.
@MainActor
final class StatusTicker {
    /// Deliberately not faster: `newt_session_metadata` allocates a fresh
    /// `CString` per string field per call, so the rate is multiplied by the
    /// number of open sessions.
    private static let interval: TimeInterval = 0.2

    private var timer: Timer?
    private let poll: () -> Void

    init(poll: @escaping () -> Void) {
        self.poll = poll
    }

    func start() {
        guard timer == nil else { return }
        let timer = Timer(timeInterval: Self.interval, repeats: true) { [weak self] _ in
            MainActor.assumeIsolated {
                self?.poll()
            }
        }
        // .common so polling survives a menu tracking loop or a live resize —
        // otherwise a tab that exits while a menu is open goes unnoticed.
        RunLoop.main.add(timer, forMode: .common)
        self.timer = timer
    }

    func stop() {
        timer?.invalidate()
        timer = nil
    }
}
