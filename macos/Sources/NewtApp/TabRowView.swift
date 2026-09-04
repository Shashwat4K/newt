import AppKit
import NewtKit

/// One row in the sidebar.
///
/// Layout, left to right: the tab's accent bar, the agent's badge, the title
/// over its subtitle, and the running indicator.
///
/// The accent identifies the *tab*; the badge identifies the *agent*; the dot
/// says what that agent is doing. Keeping them in three slots rather than
/// overloading one glyph is what makes the sidebar scannable — the question a
/// person asks is "which of these needs me", and that must be answerable
/// without decoding a symbol.
@MainActor
final class TabRowView: NSTableCellView {
    private let accentBar = NSView()
    private let badge: AgentBadgeView
    private let titleLabel = NSTextField(labelWithString: "")
    private let subtitleLabel = NSTextField(labelWithString: "")
    private let stateDot = AgentStateDot(frame: .zero)

    init(kind: TabKind) {
        badge = AgentBadgeView(kind: kind)
        super.init(frame: .zero)

        accentBar.wantsLayer = true
        accentBar.layer?.cornerRadius = 1.5

        titleLabel.font = .systemFont(ofSize: 13, weight: .regular)
        titleLabel.lineBreakMode = .byTruncatingTail
        titleLabel.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)

        // Dim and small: the numbers are for glancing at, not reading.
        subtitleLabel.font = .systemFont(ofSize: 10, weight: .regular)
        subtitleLabel.textColor = .secondaryLabelColor
        subtitleLabel.lineBreakMode = .byTruncatingTail
        subtitleLabel.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)

        let text = NSStackView(views: [titleLabel, subtitleLabel])
        text.orientation = .vertical
        text.alignment = .leading
        text.spacing = 1

        for view in [accentBar, badge, text, stateDot] {
            view.translatesAutoresizingMaskIntoConstraints = false
            addSubview(view)
        }

        NSLayoutConstraint.activate([
            accentBar.leadingAnchor.constraint(equalTo: leadingAnchor),
            accentBar.centerYAnchor.constraint(equalTo: centerYAnchor),
            accentBar.widthAnchor.constraint(equalToConstant: 3),
            accentBar.heightAnchor.constraint(equalTo: heightAnchor, multiplier: 0.62),

            badge.leadingAnchor.constraint(equalTo: accentBar.trailingAnchor, constant: 7),
            badge.centerYAnchor.constraint(equalTo: centerYAnchor),
            badge.widthAnchor.constraint(equalToConstant: 18),
            badge.heightAnchor.constraint(equalToConstant: 18),

            text.leadingAnchor.constraint(equalTo: badge.trailingAnchor, constant: 7),
            text.centerYAnchor.constraint(equalTo: centerYAnchor),
            text.trailingAnchor.constraint(lessThanOrEqualTo: stateDot.leadingAnchor, constant: -6),

            stateDot.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -8),
            stateDot.centerYAnchor.constraint(equalTo: centerYAnchor),
            stateDot.widthAnchor.constraint(equalToConstant: 10),
            stateDot.heightAnchor.constraint(equalToConstant: 10),
        ])

        textField = titleLabel
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("newt does not use storyboards")
    }

    func configure(tab: TerminalTabController, accentIndex: Int, metadata: SessionMetadata) {
        accentBar.layer?.backgroundColor = TabAccent.color(accentIndex).cgColor
        titleLabel.stringValue = tab.displayTitle
        // What the row is actually about to draw, as opposed to what the core
        // believes. Off unless asked for; when an indicator looks wrong these
        // two together say whether the state or the drawing is at fault.
        if ProcessInfo.processInfo.environment["NEWT_AGENT_TRACE"] != nil {
            FileHandle.standardError.write(
                Data("[newt-ui] row \(tab.displayTitle) draws \(metadata.agentState)\n".utf8)
            )
        }
        stateDot.update(state: metadata.agentState, kind: tab.kind)

        let subtitle = Self.subtitle(for: metadata, kind: tab.kind)
        subtitleLabel.stringValue = subtitle
        subtitleLabel.isHidden = subtitle.isEmpty
    }

    /// `opus · 45.2k · $0.83`, and nothing at all for a plain shell.
    ///
    /// Only the parts that are actually known are shown: a tab that has
    /// reported a model but no tokens yet reads better as `opus` than as
    /// `opus · 0 · $0.00`, which looks like a stalled session.
    static func subtitle(for metadata: SessionMetadata, kind: TabKind) -> String {
        guard kind.agent != nil else { return "" }

        var parts: [String] = []
        if let model = metadata.model, !model.isEmpty {
            parts.append(shortModelName(model))
        }
        if metadata.totalTokens > 0 {
            parts.append(abbreviated(metadata.totalTokens))
        }
        if metadata.costMicros > 0 {
            parts.append(String(format: "$%.2f", Double(metadata.costMicros) / 1_000_000))
        }
        return parts.joined(separator: " · ")
    }

    /// `claude-opus-5` reads as `opus` in a 200pt sidebar. The vendor prefix
    /// and version carry no information when every row shares them.
    static func shortModelName(_ model: String) -> String {
        for family in ["opus", "sonnet", "haiku", "fable"] where model.contains(family) {
            return family
        }
        return model
    }

    static func abbreviated(_ tokens: UInt64) -> String {
        switch tokens {
        case 1_000_000...:
            return String(format: "%.1fM", Double(tokens) / 1_000_000)
        case 1_000...:
            return String(format: "%.1fk", Double(tokens) / 1_000)
        default:
            return "\(tokens)"
        }
    }
}
