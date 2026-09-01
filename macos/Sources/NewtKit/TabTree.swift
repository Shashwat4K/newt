import Foundation

/// Identity of a tab. A value type so the tree can be copied and compared
/// without any of the controllers it stands for.
public struct TabID: Hashable, Sendable {
    public let rawValue: UUID

    public init(rawValue: UUID = UUID()) {
        self.rawValue = rawValue
    }
}

/// What is running in a tab.
///
/// The raw values mirror the core's agent numbering across the ABI; keep them
/// in step with `newt-agent`'s `AgentKind` rather than reordering here.
public enum AgentKind: UInt8, Sendable, CaseIterable {
    case claude = 0

    /// Name shown to a person.
    public var displayName: String {
        switch self {
        case .claude: return "Claude Code"
        }
    }

    /// Single character drawn in the tab's badge.
    ///
    /// A drawn letter rather than a bundled logo: no third-party trademarked
    /// artwork in the repository, and it stays legible at sidebar sizes where
    /// a detailed mark would not.
    public var badgeLetter: String {
        switch self {
        case .claude: return "C"
        }
    }
}

public enum TabKind: Equatable, Sendable {
    case shell
    case agent(AgentKind)

    public var agent: AgentKind? {
        if case .agent(let kind) = self { return kind }
        return nil
    }
}

/// One tab. Holds no session and no view — those live in the shell, keyed by
/// `id`, which is what lets this whole model be tested without AppKit.
public struct TabNode: Equatable, Sendable {
    public let id: TabID
    public internal(set) var parent: TabID?
    public var kind: TabKind
    /// Title reported by whatever is running, if anything has reported one.
    public var title: String?
    /// Index into the shell's accent palette. Children share their parent's,
    /// so a family reads as one group.
    public let accentIndex: Int

    public var isAgent: Bool { kind.agent != nil }
}

/// The tab tree: ordered roots, nested children, and the selection.
///
/// This lives in `NewtKit` rather than the app target on purpose. Subtree
/// removal, selection surviving a close, and stable display order are real
/// invariants with real edge cases, and stranding them in `NewtApp` would put
/// them where nothing can test them — the same mistake `GridGeometry` was
/// moved here to correct.
public struct TabTree: Sendable {
    /// How many distinct accent colours the shell's palette offers. The tree
    /// only deals in indices; the colours themselves are a rendering concern.
    public static let accentCount = 8

    private var nodes: [TabID: TabNode] = [:]
    private var children: [TabID: [TabID]] = [:]
    private(set) public var roots: [TabID] = []
    private(set) public var selected: TabID?
    /// Monotonic, so a long-lived window keeps cycling the palette rather than
    /// reusing the colour of a tab that just closed.
    private var accentCursor = 0

    public init() {}

    // MARK: - Reading

    public var isEmpty: Bool { nodes.isEmpty }
    public var count: Int { nodes.count }

    public func node(_ id: TabID) -> TabNode? { nodes[id] }

    public func children(of id: TabID) -> [TabID] { children[id] ?? [] }

    /// Display order: depth-first, parents immediately above their children.
    public func flattened() -> [(node: TabNode, depth: Int)] {
        var result: [(node: TabNode, depth: Int)] = []
        result.reserveCapacity(nodes.count)

        func visit(_ id: TabID, depth: Int) {
            guard let node = nodes[id] else { return }
            result.append((node, depth))
            for child in children[id] ?? [] {
                visit(child, depth: depth + 1)
            }
        }

        for root in roots { visit(root, depth: 0) }
        return result
    }

    /// A tab and everything nested beneath it, parents before children.
    public func subtree(of id: TabID) -> [TabID] {
        guard nodes[id] != nil else { return [] }
        var result: [TabID] = []

        func visit(_ id: TabID) {
            result.append(id)
            for child in children[id] ?? [] { visit(child) }
        }

        visit(id)
        return result
    }

    public func depth(of id: TabID) -> Int {
        var depth = 0
        var current = nodes[id]?.parent
        while let parent = current {
            depth += 1
            current = nodes[parent]?.parent
        }
        return depth
    }

    // MARK: - Mutating

    /// Add a tab, optionally nested under an existing one.
    ///
    /// The first tab inserted becomes the selection; later ones do not steal
    /// it, so the caller decides whether creating a tab also switches to it.
    @discardableResult
    public mutating func insert(
        kind: TabKind,
        title: String? = nil,
        under parent: TabID? = nil,
        id: TabID = TabID()
    ) -> TabID {
        // An unknown parent would silently orphan the tab; treat it as a root
        // rather than dropping it or trapping.
        let parent = parent.flatMap { nodes[$0] != nil ? $0 : nil }

        // Children share their parent's accent; roots take the next one.
        let accentIndex: Int
        if let parent, let node = nodes[parent] {
            accentIndex = node.accentIndex
        } else {
            accentIndex = accentCursor % Self.accentCount
            accentCursor += 1
        }

        nodes[id] = TabNode(
            id: id,
            parent: parent,
            kind: kind,
            title: title,
            accentIndex: accentIndex
        )

        if let parent {
            children[parent, default: []].append(id)
        } else {
            roots.append(id)
        }

        if selected == nil { selected = id }
        return id
    }

    /// Remove a tab and everything nested under it.
    ///
    /// - Returns: every id removed, parents first, so the caller can tear down
    ///   the matching sessions. Empty if the id is unknown.
    @discardableResult
    public mutating func remove(_ id: TabID) -> [TabID] {
        guard let node = nodes[id] else { return [] }

        // Everything below is resolved against the order *before* the removal,
        // so the new selection lands on the neighbour a person would expect
        // rather than wherever the survivors happen to fall.
        let order = flattened().map(\.node.id)
        let removed = subtree(of: id)
        let doomed = Set(removed)

        let successor: TabID?
        if let index = order.firstIndex(of: id) {
            successor =
                order[..<index].last { !doomed.contains($0) }
                ?? order[index...].first { !doomed.contains($0) }
        } else {
            successor = nil
        }

        for id in doomed {
            nodes[id] = nil
            children[id] = nil
        }

        if let parent = node.parent {
            children[parent]?.removeAll { $0 == id }
        } else {
            roots.removeAll { $0 == id }
        }

        if let selected, doomed.contains(selected) {
            self.selected = nodes.isEmpty ? nil : successor
        }

        return removed
    }

    public mutating func select(_ id: TabID) {
        guard nodes[id] != nil else { return }
        selected = id
    }

    public mutating func selectNext() { moveSelection(by: 1) }
    public mutating func selectPrevious() { moveSelection(by: -1) }

    /// Select by position in display order, for ⌘1…⌘9.
    public mutating func select(displayIndex index: Int) {
        let order = flattened()
        guard order.indices.contains(index) else { return }
        selected = order[index].node.id
    }

    public mutating func setTitle(_ title: String?, for id: TabID) {
        nodes[id]?.title = title
    }

    // MARK: - Private

    private mutating func moveSelection(by offset: Int) {
        let order = flattened().map(\.node.id)
        guard !order.isEmpty else { return }
        guard let selected, let current = order.firstIndex(of: selected) else {
            self.selected = order.first
            return
        }
        let next = (current + offset % order.count + order.count) % order.count
        self.selected = order[next]
    }
}
