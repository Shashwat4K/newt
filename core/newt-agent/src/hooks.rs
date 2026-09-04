//! Claude Code hook payloads → what the sidebar should show.
//!
//! A hook arrives as one JSON object on the helper's stdin. The fields newt
//! reads were confirmed present in Claude Code v2.1.252: `hook_event_name`,
//! `session_id`, `transcript_path`, `tool_name`, `permission_mode`.
//!
//! Everything here is a pure function over bytes. Nothing spawns, connects, or
//! reads a file, which is why the whole mapping is tested with string
//! literals and no `claude` process.

use std::path::PathBuf;

use serde_json::Value;

use crate::update::{AgentStateHint, MetadataUpdate};

/// What one hook payload told us.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HookOutcome {
    pub update: MetadataUpdate,
    /// The event's name, kept for tracing and for deciding session lifetime.
    pub event: String,
    /// True for `SessionEnd`, so a caller can stop tailing and deregister.
    pub ended: bool,
}

/// Map a raw hook payload onto a metadata update.
///
/// Returns `None` only when the payload is not JSON at all. An *unknown* event
/// still produces an outcome carrying the session id and transcript path,
/// because those are useful whatever the event was — and because a future
/// Claude Code release adding an event must not make newt drop the identifiers
/// it already understood.
pub fn parse(payload: &[u8]) -> Option<HookOutcome> {
    let value: Value = serde_json::from_slice(payload).ok()?;

    let event = value
        .get("hook_event_name")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();

    // Notifications are not all alike, so the kind is needed to interpret one.
    let notification_type = value.get("notification_type").and_then(Value::as_str);

    let mut update = MetadataUpdate {
        agent_state: state_for(&event, notification_type),
        ..MetadataUpdate::default()
    };

    if let Some(id) = value.get("session_id").and_then(Value::as_str) {
        if !id.is_empty() {
            update.agent_session_id = Some(id.to_string());
        }
    }
    if let Some(path) = value.get("transcript_path").and_then(Value::as_str) {
        if !path.is_empty() {
            update.transcript_path = Some(PathBuf::from(path));
        }
    }

    Some(HookOutcome {
        update,
        ended: event == "SessionEnd",
        event,
    })
}

/// The state each event implies.
///
/// The ones that are not obvious, and are why this is a table rather than an
/// inline match:
///
/// - **A `Notification` is only `Waiting` when it actually wants something.**
///   Claude Code raises this event for two unrelated situations, and the
///   payload's `notification_type` is what separates them. `idle_prompt` —
///   confirmed from a live payload reading *"Claude is waiting for your
///   input"* — fires a minute or so after a turn ends and means the agent is
///   idle, not blocked. Treating it as `Waiting` painted finished tabs orange,
///   which reads as "this one needs rescuing" and, because a stale row only
///   repainted when something else rebuilt the sidebar, looked like one tab's
///   indicator responding to another tab's activity.
/// - **`SubagentStop` is `Running`, not `Idle`.** A subagent finishing does not
///   mean the main agent stopped; mapping it to Idle shows a finished tab
///   mid-task, which is worse than showing nothing.
///
/// `SessionEnd` reports `Unknown` rather than `Idle`: the agent is gone, and
/// "no agent has reported" is the honest state for a tab whose agent exited.
fn state_for(event: &str, notification_type: Option<&str>) -> Option<AgentStateHint> {
    match event {
        "SessionStart" => Some(AgentStateHint::Idle),
        "UserPromptSubmit" | "PreToolUse" | "PostToolUse" | "SubagentStop" => {
            Some(AgentStateHint::Running)
        }
        "Notification" => Some(match notification_type {
            // The agent is asking for a prompt, which is what an idle agent
            // does. Only this one value is special-cased, because it is the
            // only one observed in a real payload.
            Some("idle_prompt") => AgentStateHint::Idle,
            // Everything else is assumed to want a person — a permission
            // request being the case that matters. Over-signalling "needs you"
            // is the safer error: a missed prompt stalls the session silently,
            // while a spurious one is merely noticed and dismissed.
            _ => AgentStateHint::Waiting,
        }),
        "Stop" => Some(AgentStateHint::Idle),
        "SessionEnd" => Some(AgentStateHint::Unknown),
        // An event newt did not register, or one added by a later release.
        // Leaving the state alone is right: guessing would flicker the tab.
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payload(event: &str) -> String {
        format!(
            r#"{{"hook_event_name":"{event}",
                "session_id":"9f11a139-d258-433e-bc9c-1f648b761c71",
                "transcript_path":"/Users/x/.claude/projects/-work/abc.jsonl",
                "cwd":"/work"}}"#
        )
    }

    fn state_of(event: &str) -> Option<AgentStateHint> {
        parse(payload(event).as_bytes()).unwrap().update.agent_state
    }

    fn notification_state(kind: Option<&str>) -> Option<AgentStateHint> {
        let body = match kind {
            Some(kind) => {
                format!(r#"{{"hook_event_name":"Notification","notification_type":"{kind}"}}"#)
            }
            None => r#"{"hook_event_name":"Notification"}"#.to_string(),
        };
        parse(body.as_bytes()).unwrap().update.agent_state
    }

    #[test]
    fn every_registered_event_maps_to_a_state() {
        use AgentStateHint::*;
        assert_eq!(state_of("SessionStart"), Some(Idle));
        assert_eq!(state_of("UserPromptSubmit"), Some(Running));
        assert_eq!(state_of("PreToolUse"), Some(Running));
        assert_eq!(state_of("PostToolUse"), Some(Running));
        // Without a type, a notification is assumed to want attention.
        assert_eq!(state_of("Notification"), Some(Waiting));
        assert_eq!(state_of("Stop"), Some(Idle));
        assert_eq!(state_of("SessionEnd"), Some(Unknown));
    }

    #[test]
    fn a_subagent_finishing_leaves_the_main_agent_running() {
        // Idle here would show a finished tab while the agent is still working.
        assert_eq!(state_of("SubagentStop"), Some(AgentStateHint::Running));
    }

    #[test]
    fn a_notification_is_distinguishable_from_working() {
        // The whole point of the sidebar is answering "which of these needs
        // me", so these two must never collapse into one state.
        assert_ne!(state_of("Notification"), state_of("PreToolUse"));
        assert_eq!(state_of("Notification"), Some(AgentStateHint::Waiting));
    }

    #[test]
    fn an_idle_prompt_notification_leaves_the_tab_idle() {
        // Observed live: after a turn ends, Claude Code sends
        // {"notification_type":"idle_prompt","message":"Claude is waiting for
        // your input"}. That is a finished session, not a blocked one, and
        // showing it as blocked is what made a completed tab pulse for
        // attention it did not need.
        assert_eq!(
            notification_state(Some("idle_prompt")),
            Some(AgentStateHint::Idle)
        );
    }

    #[test]
    fn any_other_notification_still_asks_for_a_person() {
        // The permission-request case, and anything a later release adds.
        // Defaulting to Waiting means a new notification kind is noticed rather
        // than silently ignored.
        assert_eq!(
            notification_state(Some("permission_request")),
            Some(AgentStateHint::Waiting)
        );
        assert_eq!(
            notification_state(Some("something_new")),
            Some(AgentStateHint::Waiting)
        );
        assert_eq!(notification_state(None), Some(AgentStateHint::Waiting));
    }

    #[test]
    fn the_registered_events_and_the_mapping_agree() {
        // Registering an event newt cannot interpret would leave a tab stuck;
        // interpreting one it never registered is dead code. Both directions.
        for event in crate::launch::HOOK_EVENTS {
            assert!(
                state_for(event, None).is_some(),
                "{event} is registered but maps to no state"
            );
        }
    }

    #[test]
    fn the_session_id_and_transcript_path_are_carried_out() {
        let outcome = parse(payload("SessionStart").as_bytes()).unwrap();

        // These are the two facts that make everything downstream possible:
        // the id a child tab forks from, and the file Phase 13 tails. Both are
        // learned here rather than assigned, which is what makes
        // `--fork-session` work without special casing.
        assert_eq!(
            outcome.update.agent_session_id.as_deref(),
            Some("9f11a139-d258-433e-bc9c-1f648b761c71")
        );
        assert_eq!(
            outcome.update.transcript_path,
            Some(PathBuf::from("/Users/x/.claude/projects/-work/abc.jsonl"))
        );
    }

    #[test]
    fn session_end_is_flagged_so_the_caller_can_stop_listening() {
        assert!(parse(payload("SessionEnd").as_bytes()).unwrap().ended);
        assert!(!parse(payload("Stop").as_bytes()).unwrap().ended);
    }

    #[test]
    fn an_unknown_event_keeps_its_identifiers_and_leaves_the_state_alone() {
        // A later Claude Code adding an event must not make newt forget the
        // session it already knew about.
        let outcome = parse(payload("SomeFutureEvent").as_bytes()).unwrap();
        assert_eq!(outcome.update.agent_state, None);
        assert!(outcome.update.agent_session_id.is_some());
        assert_eq!(outcome.event, "SomeFutureEvent");
    }

    #[test]
    fn malformed_input_is_none_rather_than_a_panic() {
        // This runs on a thread serving a socket any local process can reach.
        assert!(parse(b"").is_none());
        assert!(parse(b"not json at all").is_none());
        assert!(parse(b"{\"unterminated\": ").is_none());
        // Valid JSON of the wrong shape is not an error, just uninformative.
        let outcome = parse(b"{}").unwrap();
        assert!(outcome.update.is_empty());
        assert_eq!(outcome.event, "");
    }

    #[test]
    fn empty_identifier_strings_are_treated_as_absent() {
        let outcome =
            parse(br#"{"hook_event_name":"Stop","session_id":"","transcript_path":""}"#).unwrap();
        assert_eq!(outcome.update.agent_session_id, None);
        assert_eq!(outcome.update.transcript_path, None);
    }
}
