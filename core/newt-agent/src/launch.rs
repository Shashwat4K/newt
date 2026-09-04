//! Turning "start Claude Code here" into a program, argv, and environment.
//!
//! Nothing in this module spawns anything or opens a socket. It produces a
//! recipe and, when hooks are wanted, writes one settings file. That keeps the
//! whole thing testable with a temporary directory and no child process.

use std::path::{Path, PathBuf};

use crate::kind::AgentKind;

/// Claude Code's marker for "you were started by another Claude Code".
///
/// It disables transcript saving in the child, which costs newt every title,
/// token count and cost figure it would otherwise read. A newt agent tab is a
/// *new* top-level session, not a child of whatever happened to launch newt,
/// so the marker is untrue of it and is removed.
///
/// This matters in practice rather than in theory: anyone developing newt from
/// a terminal inside a Claude Code session inherits it, and the symptom is a
/// sidebar that shows state but never a title — which reads as the transcript
/// reader being broken.
pub const CHILD_SESSION_MARKER: &str = "CLAUDE_CODE_CHILD_SESSION";

/// Environment variable naming the socket `newt-hook` reports to.
pub const SOCKET_ENV: &str = "NEWT_HOOK_SOCKET";
/// Environment variable identifying which session a hook belongs to.
///
/// One bridge serves every tab, so the payload has to say who it is about.
/// Claude Code's own `session_id` cannot do that job: it is not known until
/// `SessionStart` fires, and `--fork-session` deliberately mints a new one.
pub const TOKEN_ENV: &str = "NEWT_SESSION_TOKEN";

/// Hook events newt registers.
///
/// `PostToolUse` is included so a long tool call does not leave a tab looking
/// idle between `PreToolUse` and the next prompt.
///
/// `SubagentStop` is deliberately *absent*. It was registered at first on the
/// theory that a subagent finishing meant the main agent was still working;
/// in practice it arrives after `Stop`, so it flipped completed turns back to
/// Running. Since nothing can be concluded from it, asking for it would only
/// spend a process launch inside the user's session.
pub const HOOK_EVENTS: &[&str] = &[
    "SessionStart",
    "UserPromptSubmit",
    "PreToolUse",
    "PostToolUse",
    "Notification",
    "Stop",
    "SessionEnd",
];

/// What to start.
#[derive(Debug, Clone)]
pub struct LaunchRequest {
    pub kind: AgentKind,
    /// Absolute path to the agent executable. See [`crate::detect`].
    pub program: PathBuf,
    pub cwd: Option<PathBuf>,
    /// Agent session to continue from, forking so the parent is untouched.
    ///
    /// `None` starts a fresh conversation. There is deliberately no "resume
    /// without forking": that would attach two tabs to one conversation.
    pub fork_from: Option<String>,
    /// Absolute path to the `newt-hook` helper.
    ///
    /// `None` registers no hooks at all, which is a working agent session with
    /// no state reporting — used before the bridge exists, and the honest
    /// fallback if the helper is missing from the bundle.
    pub hook_helper: Option<PathBuf>,
    /// Where the generated settings file may be written.
    pub runtime_dir: PathBuf,
    pub socket_path: Option<PathBuf>,
    pub session_token: Option<String>,
}

/// A recipe `newt-core` can spawn. Plain data; nothing here is a handle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchPlan {
    pub program: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    /// Inherited variables to strip before starting the agent.
    pub env_remove: Vec<String>,
    /// Directory to start in.
    ///
    /// Carried through rather than left to the caller: an agent's working
    /// directory *is* the project it operates on, and a child tab inheriting
    /// its parent's is the whole of Story 3's "same project". Dropping it here
    /// silently started every agent wherever newt happened to be launched
    /// from, which looks like the agent misbehaving.
    pub cwd: Option<PathBuf>,
}

/// Build the plan for a request, writing a settings file if hooks are wanted.
pub fn plan(request: &LaunchRequest) -> std::io::Result<LaunchPlan> {
    match request.kind {
        AgentKind::Claude => crate::claude::plan(request),
    }
}

/// Quote a path for a shell command line.
///
/// Hook commands are strings that a shell runs, so a path containing a space —
/// `/Users/x/My Apps/newt.app/...` — would otherwise be read as two arguments.
/// Single quotes protect everything except a single quote, which is spliced
/// with the usual `'\''` dance.
pub fn shell_quote(path: &Path) -> String {
    let text = path.to_string_lossy();
    format!("'{}'", text.replace('\'', r"'\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plain_path_is_quoted_whole() {
        assert_eq!(
            shell_quote(Path::new("/usr/local/bin/newt-hook")),
            "'/usr/local/bin/newt-hook'"
        );
    }

    #[test]
    fn a_path_with_a_space_survives_quoting() {
        // `/Applications` is the usual home, but nothing stops someone keeping
        // the bundle in a directory with a space, and an unquoted path there
        // would run the wrong program or nothing at all.
        assert_eq!(
            shell_quote(Path::new(
                "/Users/x/My Apps/newt.app/Contents/MacOS/newt-hook"
            )),
            "'/Users/x/My Apps/newt.app/Contents/MacOS/newt-hook'"
        );
    }

    #[test]
    fn an_apostrophe_is_spliced_rather_than_ending_the_quote() {
        assert_eq!(
            shell_quote(Path::new("/Users/x/Bob's Mac/newt-hook")),
            r"'/Users/x/Bob'\''s Mac/newt-hook'"
        );
    }
}
