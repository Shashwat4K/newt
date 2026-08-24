//! Per-session metadata.
//!
//! This exists for the end goal — a multiplexer built for AI agents, where
//! token usage, cost, and agent state are shown alongside the grid. None of
//! those features are built yet; what matters now is that a session *carries*
//! this as a first-class field rather than having it bolted on later, when
//! every layer would need widening at once.
//!
//! It is session bookkeeping, not terminal semantics, so it lives beside the
//! terminal rather than inside it: nothing here affects what is drawn.

/// What an agent driving this session is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AgentState {
    /// No agent, or nothing reported yet.
    #[default]
    Unknown,
    Idle,
    Running,
    /// Blocked on something outside the session, such as a person.
    Waiting,
    Error,
}

impl AgentState {
    pub fn as_u8(self) -> u8 {
        match self {
            AgentState::Unknown => 0,
            AgentState::Idle => 1,
            AgentState::Running => 2,
            AgentState::Waiting => 3,
            AgentState::Error => 4,
        }
    }

    pub fn from_u8(value: u8) -> Option<Self> {
        match value {
            0 => Some(AgentState::Unknown),
            1 => Some(AgentState::Idle),
            2 => Some(AgentState::Running),
            3 => Some(AgentState::Waiting),
            4 => Some(AgentState::Error),
            _ => None,
        }
    }
}

/// Bookkeeping a UI can observe without reaching into the grid.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SessionMetadata {
    pub input_tokens: u64,
    pub output_tokens: u64,
    /// Cost in millionths of a currency unit.
    ///
    /// An integer rather than a float: these accumulate over a long session,
    /// and repeated addition of fractional cents drifts.
    pub cost_micros: u64,
    pub agent_state: AgentState,
    /// Model driving this session, if any.
    pub model: Option<String>,
}

impl SessionMetadata {
    pub fn total_tokens(&self) -> u64 {
        self.input_tokens.saturating_add(self.output_tokens)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_states_round_trip_through_their_abi_values() {
        for state in [
            AgentState::Unknown,
            AgentState::Idle,
            AgentState::Running,
            AgentState::Waiting,
            AgentState::Error,
        ] {
            assert_eq!(AgentState::from_u8(state.as_u8()), Some(state));
        }
        assert_eq!(AgentState::from_u8(9), None);
    }

    #[test]
    fn totals_saturate_rather_than_overflow() {
        let metadata = SessionMetadata {
            input_tokens: u64::MAX,
            output_tokens: 10,
            ..SessionMetadata::default()
        };
        assert_eq!(metadata.total_tokens(), u64::MAX);
    }

    #[test]
    fn a_new_session_reports_nothing_rather_than_zero_cost() {
        // Unknown is deliberately distinct from Idle: "no agent has spoken"
        // and "an agent is idle" are different states to show.
        let metadata = SessionMetadata::default();
        assert_eq!(metadata.agent_state, AgentState::Unknown);
        assert_eq!(metadata.model, None);
    }
}
