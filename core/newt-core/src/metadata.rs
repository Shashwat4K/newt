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
    /// The agent's own name for what it is doing.
    ///
    /// Kept distinct from the terminal's OSC title: they are different facts
    /// with different lifetimes, and a UI wants to fall back from one to the
    /// other rather than have them overwrite each other.
    pub agent_title: Option<String>,
    /// The agent's session identifier, as reported by the agent itself.
    ///
    /// Learned, never assigned — it is what a child tab forks from.
    pub agent_session_id: Option<String>,
}

impl SessionMetadata {
    /// Fold a report from an agent adapter into this metadata.
    ///
    /// Every field replaces rather than accumulates; see
    /// [`newt_agent::MetadataUpdate`] for why that is load-bearing rather than
    /// stylistic. Absent fields are left alone, so a hook that only knows the
    /// state does not erase a model the transcript reported.
    pub fn apply(&mut self, update: &newt_agent::MetadataUpdate) {
        if let Some(value) = update.input_tokens {
            self.input_tokens = value;
        }
        if let Some(value) = update.output_tokens {
            self.output_tokens = value;
        }
        if let Some(value) = update.cost_micros {
            self.cost_micros = value;
        }
        if let Some(hint) = update.agent_state {
            self.agent_state = AgentState::from(hint);
        }
        if let Some(value) = &update.model {
            self.model = Some(value.clone());
        }
        if let Some(value) = &update.agent_title {
            self.agent_title = Some(value.clone());
        }
        if let Some(value) = &update.agent_session_id {
            self.agent_session_id = Some(value.clone());
        }
    }
}

impl From<newt_agent::AgentStateHint> for AgentState {
    fn from(hint: newt_agent::AgentStateHint) -> Self {
        use newt_agent::AgentStateHint as Hint;
        match hint {
            Hint::Unknown => AgentState::Unknown,
            Hint::Idle => AgentState::Idle,
            Hint::Running => AgentState::Running,
            Hint::Waiting => AgentState::Waiting,
            Hint::Error => AgentState::Error,
        }
    }
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
    fn applying_an_update_replaces_only_what_it_mentions() {
        let mut metadata = SessionMetadata {
            model: Some("claude-opus-5".to_string()),
            input_tokens: 100,
            ..SessionMetadata::default()
        };

        metadata.apply(&newt_agent::MetadataUpdate {
            agent_state: Some(newt_agent::AgentStateHint::Running),
            ..Default::default()
        });

        assert_eq!(metadata.agent_state, AgentState::Running);
        // A hook knows the state and nothing else; it must not erase what the
        // transcript reported.
        assert_eq!(metadata.model.as_deref(), Some("claude-opus-5"));
        assert_eq!(metadata.input_tokens, 100);
    }

    #[test]
    fn applied_token_counts_replace_rather_than_accumulate() {
        let mut metadata = SessionMetadata::default();
        let update = newt_agent::MetadataUpdate {
            input_tokens: Some(500),
            ..Default::default()
        };

        metadata.apply(&update);
        metadata.apply(&update);

        // A forked session re-reads its parent's transcript, so identical
        // totals arrive twice. Adding would double them.
        assert_eq!(metadata.input_tokens, 500);
    }

    #[test]
    fn every_state_hint_maps_onto_a_state() {
        use newt_agent::AgentStateHint as Hint;
        assert_eq!(AgentState::from(Hint::Unknown), AgentState::Unknown);
        assert_eq!(AgentState::from(Hint::Idle), AgentState::Idle);
        assert_eq!(AgentState::from(Hint::Running), AgentState::Running);
        assert_eq!(AgentState::from(Hint::Waiting), AgentState::Waiting);
        assert_eq!(AgentState::from(Hint::Error), AgentState::Error);
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
