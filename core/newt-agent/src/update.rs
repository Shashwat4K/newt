//! What an agent reports, on its way into `SessionMetadata`.
//!
//! # Replace, never add
//!
//! Every field here is `Option<T>` where `Some` *replaces* the current value.
//! Nothing accumulates. That is not a stylistic choice:
//!
//! - `--fork-session` copies a parent conversation into a new transcript, so a
//!   forked child would re-read every token the parent already reported.
//! - A compaction rewrites a transcript in place, so a re-scan sees the same
//!   messages again.
//!
//! An additive `input_tokens += n` double-counts in both cases and there is no
//! way to detect it after the fact. A replacing one is idempotent across a
//! re-scan by construction. Accumulation belongs to whoever owns the running
//! total — the transcript reader — which republishes it.

use std::path::PathBuf;

/// What an agent is doing, as reported by an adapter.
///
/// `newt-agent`'s own enum rather than `newt-core`'s `AgentState`: this crate
/// is a leaf and must not depend on the terminal. `newt-core` maps between
/// them, and the mapping is one match arm.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentStateHint {
    Unknown,
    Idle,
    Running,
    Waiting,
    Error,
}

/// A partial report. Absent fields are left alone.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MetadataUpdate {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub cost_micros: Option<u64>,
    pub agent_state: Option<AgentStateHint>,
    pub model: Option<String>,
    /// The agent's own name for this session — Claude Code's `ai-title`.
    pub agent_title: Option<String>,
    /// The agent's session identifier, learned rather than assigned.
    pub agent_session_id: Option<String>,
    /// Where the agent is writing its transcript. Not metadata; the bridge
    /// uses it to know what to tail, from Phase 13.
    pub transcript_path: Option<PathBuf>,
}

impl MetadataUpdate {
    /// True when this would change nothing, so it can be dropped.
    pub fn is_empty(&self) -> bool {
        *self == MetadataUpdate::default()
    }

    /// Fold `other` on top of this one, later values winning.
    ///
    /// Used when several reports arrive between two reads; the result must be
    /// identical to applying them in order, which is what makes batching safe.
    pub fn merge(&mut self, other: MetadataUpdate) {
        if other.input_tokens.is_some() {
            self.input_tokens = other.input_tokens;
        }
        if other.output_tokens.is_some() {
            self.output_tokens = other.output_tokens;
        }
        if other.cost_micros.is_some() {
            self.cost_micros = other.cost_micros;
        }
        if other.agent_state.is_some() {
            self.agent_state = other.agent_state;
        }
        if other.model.is_some() {
            self.model = other.model;
        }
        if other.agent_title.is_some() {
            self.agent_title = other.agent_title;
        }
        if other.agent_session_id.is_some() {
            self.agent_session_id = other.agent_session_id;
        }
        if other.transcript_path.is_some() {
            self.transcript_path = other.transcript_path;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_default_update_changes_nothing() {
        assert!(MetadataUpdate::default().is_empty());
        assert!(!MetadataUpdate {
            agent_state: Some(AgentStateHint::Idle),
            ..MetadataUpdate::default()
        }
        .is_empty());
    }

    #[test]
    fn merging_keeps_fields_the_later_update_did_not_mention() {
        let mut first = MetadataUpdate {
            model: Some("claude-opus-5".to_string()),
            agent_state: Some(AgentStateHint::Running),
            input_tokens: Some(100),
            ..MetadataUpdate::default()
        };
        first.merge(MetadataUpdate {
            agent_state: Some(AgentStateHint::Idle),
            ..MetadataUpdate::default()
        });

        assert_eq!(first.agent_state, Some(AgentStateHint::Idle));
        assert_eq!(first.model.as_deref(), Some("claude-opus-5"));
        assert_eq!(first.input_tokens, Some(100));
    }

    #[test]
    fn replacing_a_token_count_does_not_double_it() {
        // The case this exists for: a forked session re-reads its parent's
        // transcript, so the same totals arrive twice. Adding would double
        // them; replacing cannot.
        let mut update = MetadataUpdate {
            input_tokens: Some(38_400),
            ..MetadataUpdate::default()
        };
        update.merge(MetadataUpdate {
            input_tokens: Some(38_400),
            ..MetadataUpdate::default()
        });

        assert_eq!(update.input_tokens, Some(38_400));
    }
}
