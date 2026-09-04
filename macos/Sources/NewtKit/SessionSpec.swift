import CNewt
import Foundation

/// Everything needed to start a session.
///
/// Mirrors `NewtSessionSpec`. Every field has a default that means "whatever
/// the core would have done anyway", so a caller states only what it cares
/// about — which is the point of a spec over a nine-argument constructor.
public struct SessionSpec: Equatable, Sendable {
    public var size: TerminalSize
    /// Program to run. `nil` is the user's login shell.
    public var program: String?
    /// Arguments, excluding argv[0].
    public var arguments: [String]
    /// Variables added to the inherited environment, overriding on collision.
    ///
    /// An array of pairs rather than a dictionary: the core applies these in
    /// order, and a dictionary would make that order arbitrary.
    public var environment: [(name: String, value: String)]
    public var workingDirectory: String?
    /// Value advertised as `TERM`. `nil` keeps the core's default.
    public var term: String?
    public var scrollbackLines: UInt32

    /// Agent to run instead of a program.
    ///
    /// The only agent knob this side touches. When set, the core resolves the
    /// executable, writes the hooks settings, starts the bridge, and builds the
    /// argument list — none of which the shell should know about.
    public var agent: AgentKind?
    /// Absolute path to the bundled `newt-hook` helper.
    ///
    /// Resolved here because knowing where a bundle keeps its executables is
    /// the shell's job; the core treats it as an opaque path.
    public var agentHelperPath: String?
    /// Agent session to fork from. `nil` starts a fresh conversation.
    public var agentResumeID: String?

    public init(
        size: TerminalSize,
        program: String? = nil,
        arguments: [String] = [],
        environment: [(name: String, value: String)] = [],
        workingDirectory: String? = nil,
        term: String? = nil,
        scrollbackLines: UInt32 = 10_000,
        agent: AgentKind? = nil,
        agentHelperPath: String? = nil,
        agentResumeID: String? = nil
    ) {
        self.size = size
        self.program = program
        self.arguments = arguments
        self.environment = environment
        self.workingDirectory = workingDirectory
        self.term = term
        self.scrollbackLines = scrollbackLines
        self.agent = agent
        self.agentHelperPath = agentHelperPath
        self.agentResumeID = agentResumeID
    }

    public static func == (lhs: SessionSpec, rhs: SessionSpec) -> Bool {
        lhs.size == rhs.size && lhs.program == rhs.program && lhs.arguments == rhs.arguments
            && lhs.workingDirectory == rhs.workingDirectory && lhs.term == rhs.term
            && lhs.scrollbackLines == rhs.scrollbackLines && lhs.agent == rhs.agent
            && lhs.agentHelperPath == rhs.agentHelperPath
            && lhs.agentResumeID == rhs.agentResumeID
            && lhs.environment.count == rhs.environment.count
            && zip(lhs.environment, rhs.environment).allSatisfy { $0 == $1 }
    }
}

extension SessionSpec {
    /// Call `body` with a `NewtSessionSpec` valid only for its duration.
    ///
    /// Every string is copied into one contiguous blob first, and the slices
    /// point into that. Handing the C side pointers taken from separate Swift
    /// `String`s would be a use-after-free waiting to happen: a `String`'s
    /// storage is only guaranteed alive inside its own `withUTF8`, so a loop
    /// building an array of pointers frees each buffer as it goes. One blob,
    /// one lifetime, and the nesting below is what proves it.
    func withNativeSpec<R>(_ body: (UnsafePointer<NewtSessionSpec>) throws -> R) rethrows -> R {
        // Order matters only in that encode and decode agree.
        var blob: [UInt8] = []
        var extents: [(offset: Int, count: Int)] = []

        func add(_ text: String?) {
            let bytes = Array((text ?? "").utf8)
            extents.append((blob.count, bytes.count))
            blob.append(contentsOf: bytes)
        }

        add(program)
        add(workingDirectory)
        add(term)
        add(agentHelperPath)
        add(agentResumeID)
        for argument in arguments { add(argument) }
        for variable in environment {
            add(variable.name)
            add(variable.value)
        }

        return try blob.withUnsafeBufferPointer { buffer in
            func slice(_ index: Int) -> NewtBytes {
                let extent = extents[index]
                // An empty field must be a null slice, not a pointer to zero
                // bytes at the end of the blob — the core reads empty as "not
                // supplied", and `baseAddress` is nil for an empty buffer.
                guard extent.count > 0, let base = buffer.baseAddress else {
                    return NewtBytes(ptr: nil, len: 0)
                }
                return NewtBytes(ptr: base + extent.offset, len: UInt(extent.count))
            }

            // 0..4 are program, cwd, term, helper path, resume id; arguments
            // and environment follow in the order they were added above.
            let fixedFields = 5
            let argumentSlices = (0..<arguments.count).map { slice(fixedFields + $0) }
            let environmentStart = fixedFields + arguments.count
            let environmentPairs = (0..<environment.count).map { index in
                NewtEnvVar(
                    key: slice(environmentStart + index * 2),
                    value: slice(environmentStart + index * 2 + 1)
                )
            }

            return try argumentSlices.withUnsafeBufferPointer { args in
                try environmentPairs.withUnsafeBufferPointer { env in
                    var spec = NewtSessionSpec(
                        cols: size.cols,
                        rows: size.rows,
                        scrollback_lines: scrollbackLines,
                        program: slice(0),
                        args: args.baseAddress,
                        arg_count: UInt(args.count),
                        env: env.baseAddress,
                        env_count: UInt(env.count),
                        cwd: slice(1),
                        term: slice(2),
                        // Not zero when absent: zero is Claude Code.
                        agent_kind: agent?.rawValue ?? UInt8(NEWT_AGENT_KIND_NONE),
                        agent_helper_path: slice(3),
                        agent_resume_id: slice(4)
                    )
                    return try withUnsafePointer(to: &spec) { try body($0) }
                }
            }
        }
    }
}
