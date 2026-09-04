import CNewt

/// What an agent driving a session is doing.
///
/// `unknown` is deliberately distinct from `idle`: "no agent has reported" and
/// "an agent is idle" are different things to show.
public enum AgentState: UInt8, Sendable {
    case unknown = 0
    case idle = 1
    case running = 2
    case waiting = 3
    case error = 4
}

/// Per-session bookkeeping the UI can observe without reaching into the grid.
///
/// The seam for the end goal — token, cost, and agent-state display alongside
/// the terminal. Carried as a first-class field from the start so those
/// features do not require widening every layer at once.
public struct SessionMetadata: Equatable, Sendable {
    public var inputTokens: UInt64
    public var outputTokens: UInt64
    /// Cost in millionths of a currency unit; an integer so long sessions do
    /// not accumulate floating-point drift.
    public var costMicros: UInt64
    public var agentState: AgentState
    public var model: String?
    /// The agent's own name for this session — Claude Code's `ai-title`.
    ///
    /// Kept apart from the terminal's OSC title so a row can fall back from
    /// one to the other instead of letting them overwrite each other.
    public var agentTitle: String?
    /// The agent's session identifier, as the agent reported it.
    ///
    /// Learned, never assigned; a child tab forks from this.
    public var agentSessionID: String?

    public init(
        inputTokens: UInt64 = 0,
        outputTokens: UInt64 = 0,
        costMicros: UInt64 = 0,
        agentState: AgentState = .unknown,
        model: String? = nil,
        agentTitle: String? = nil,
        agentSessionID: String? = nil
    ) {
        self.inputTokens = inputTokens
        self.outputTokens = outputTokens
        self.costMicros = costMicros
        self.agentState = agentState
        self.model = model
        self.agentTitle = agentTitle
        self.agentSessionID = agentSessionID
    }

    public var totalTokens: UInt64 {
        inputTokens.addingReportingOverflow(outputTokens).partialValue
    }
}

extension TerminalSession {
    /// This session's metadata.
    public var metadata: SessionMetadata {
        var raw = NewtSessionMetadata()
        guard newt_session_metadata(rawHandle, &raw) else { return SessionMetadata() }

        return SessionMetadata(
            inputTokens: raw.input_tokens,
            outputTokens: raw.output_tokens,
            costMicros: raw.cost_micros,
            agentState: AgentState(rawValue: raw.agent_state) ?? .unknown,
            // Copied immediately: these borrow session-owned storage that the
            // next metadata call replaces.
            model: raw.model.map { String(cString: $0) },
            agentTitle: raw.agent_title.map { String(cString: $0) },
            agentSessionID: raw.agent_session_id.map { String(cString: $0) }
        )
    }

    /// Replace this session's metadata.
    public func updateMetadata(_ metadata: SessionMetadata) throws {
        let ok: Bool
        if let model = metadata.model {
            var utf8 = Array(model.utf8)
            ok = utf8.withUnsafeMutableBufferPointer { buffer in
                newt_session_set_metadata(
                    rawHandle,
                    metadata.inputTokens,
                    metadata.outputTokens,
                    metadata.costMicros,
                    metadata.agentState.rawValue,
                    buffer.baseAddress,
                    UInt(buffer.count)
                )
            }
        } else {
            ok = newt_session_set_metadata(
                rawHandle,
                metadata.inputTokens,
                metadata.outputTokens,
                metadata.costMicros,
                metadata.agentState.rawValue,
                nil,
                0
            )
        }

        guard ok else { throw TerminalError.lastError(fallback: "updating metadata failed") }
    }
}
