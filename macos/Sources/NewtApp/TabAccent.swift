import AppKit
import NewtKit

/// Per-tab colour and the drawn agent mark.
///
/// Story 2 allows a random colour per tab. These are assigned round-robin from
/// a fixed palette instead: adjacent tabs never collide, a colour stays put
/// across redraws, and `--render-to` output is reproducible byte for byte,
/// which a random palette would quietly destroy.
enum TabAccent {
    /// Eight hues, spaced around the wheel and held at a saturation that reads
    /// on both a light and a dark sidebar.
    static let palette: [NSColor] = [
        NSColor(srgbRed: 0.77, green: 0.38, blue: 0.25, alpha: 1),  // terracotta
        NSColor(srgbRed: 0.30, green: 0.55, blue: 0.78, alpha: 1),  // blue
        NSColor(srgbRed: 0.42, green: 0.64, blue: 0.38, alpha: 1),  // green
        NSColor(srgbRed: 0.72, green: 0.51, blue: 0.24, alpha: 1),  // amber
        NSColor(srgbRed: 0.55, green: 0.44, blue: 0.75, alpha: 1),  // violet
        NSColor(srgbRed: 0.26, green: 0.62, blue: 0.62, alpha: 1),  // teal
        NSColor(srgbRed: 0.78, green: 0.42, blue: 0.55, alpha: 1),  // rose
        NSColor(srgbRed: 0.47, green: 0.53, blue: 0.60, alpha: 1),  // slate
    ]

    static func color(_ index: Int) -> NSColor {
        palette[((index % palette.count) + palette.count) % palette.count]
    }

    /// Colour of an agent's badge. Fixed per agent rather than per tab, so the
    /// mark identifies the agent while the accent identifies the tab.
    static func badgeColor(for kind: TabKind) -> NSColor {
        switch kind {
        case .shell:
            return NSColor.secondaryLabelColor
        case .agent(.claude):
            return NSColor(srgbRed: 0.77, green: 0.38, blue: 0.25, alpha: 1)
        }
    }

    static func badgeLetter(for kind: TabKind) -> String {
        switch kind {
        case .shell: return "\u{203A}"  // ›
        case .agent(let agent): return agent.badgeLetter
        }
    }
}

/// The agent mark: a letter in a rounded badge.
///
/// Drawn geometry rather than a bundled image. Shipping a vendor's real mark
/// would put third-party trademarked artwork in the repository and raise a
/// licensing question per agent, and a detailed logo is illegible at the 16pt
/// this occupies anyway.
@MainActor
final class AgentBadgeView: NSView {
    private let letter: String
    private let color: NSColor

    init(kind: TabKind) {
        letter = TabAccent.badgeLetter(for: kind)
        color = TabAccent.badgeColor(for: kind)
        super.init(frame: NSRect(x: 0, y: 0, width: 18, height: 18))
        wantsLayer = true
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("newt does not use storyboards")
    }

    override var intrinsicContentSize: NSSize { NSSize(width: 18, height: 18) }

    override func draw(_ dirtyRect: NSRect) {
        guard let context = NSGraphicsContext.current?.cgContext else { return }
        let box = bounds.insetBy(dx: 1, dy: 1)

        let path = NSBezierPath(roundedRect: box, xRadius: 4, yRadius: 4)
        color.withAlphaComponent(0.18).setFill()
        path.fill()
        color.withAlphaComponent(0.55).setStroke()
        path.lineWidth = 1
        path.stroke()

        let attributes: [NSAttributedString.Key: Any] = [
            .font: NSFont.systemFont(ofSize: 10, weight: .semibold),
            .foregroundColor: color,
        ]
        let text = NSAttributedString(string: letter, attributes: attributes)
        let size = text.size()
        text.draw(
            at: NSPoint(
                x: box.midX - size.width / 2,
                y: box.midY - size.height / 2
            )
        )
        _ = context
    }
}

/// The running indicator: a dot that pulses while an agent is working.
///
/// Core Animation rather than a per-frame redraw or an `NSProgressIndicator`
/// per row — the animation runs off the main thread, and an idle sidebar does
/// no per-frame work at all.
@MainActor
final class AgentStateDot: NSView {
    private let dot = CALayer()
    private var current: AgentState = .unknown
    private var kind: TabKind = .shell

    override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
        wantsLayer = true
        dot.cornerRadius = 4
        dot.frame = CGRect(x: 0, y: 0, width: 8, height: 8)
        layer?.addSublayer(dot)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("newt does not use storyboards")
    }

    override var intrinsicContentSize: NSSize { NSSize(width: 10, height: 10) }

    override func layout() {
        super.layout()
        dot.frame = CGRect(x: bounds.midX - 4, y: bounds.midY - 4, width: 8, height: 8)
    }

    func update(state: AgentState, kind: TabKind) {
        guard state != current || kind != self.kind else { return }
        current = state
        self.kind = kind

        let accent = TabAccent.badgeColor(for: kind)
        dot.removeAnimation(forKey: "pulse")

        switch state {
        case .running:
            dot.backgroundColor = accent.cgColor
            dot.opacity = 1
            add(pulse: 0.9, from: 1, to: 0.25)
        case .waiting:
            // Distinct from running on purpose: "needs you" and "working" are
            // the two states a person actually scans the sidebar for.
            dot.backgroundColor = NSColor.systemOrange.cgColor
            dot.opacity = 1
            add(pulse: 1.6, from: 1, to: 0.4)
        case .idle:
            dot.backgroundColor = NSColor.tertiaryLabelColor.cgColor
            dot.opacity = 1
        case .error:
            dot.backgroundColor = NSColor.systemRed.cgColor
            dot.opacity = 1
        case .unknown:
            // No agent has reported. Deliberately not the same as idle.
            dot.backgroundColor = NSColor.quaternaryLabelColor.cgColor
            dot.opacity = kind.agent == nil ? 0.6 : 1
        }
    }

    private func add(pulse duration: CFTimeInterval, from: Float, to: Float) {
        let animation = CABasicAnimation(keyPath: "opacity")
        animation.fromValue = from
        animation.toValue = to
        animation.duration = duration
        animation.autoreverses = true
        animation.repeatCount = .infinity
        animation.timingFunction = CAMediaTimingFunction(name: .easeInEaseOut)
        dot.add(animation, forKey: "pulse")
    }
}
