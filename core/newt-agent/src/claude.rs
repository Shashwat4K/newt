//! The Claude Code adapter: argv, environment, and the hooks settings file.
//!
//! Everything here was checked against Claude Code v2.1.252 by reading `--help`
//! and the transcripts it writes. These are undocumented internals and will
//! drift; the fixture tests beside this module are what turn drift into a
//! failing test rather than a blank sidebar.

use std::path::PathBuf;

use serde_json::{json, Value};

use crate::launch::{
    shell_quote, LaunchPlan, LaunchRequest, CHILD_SESSION_MARKER, HOOK_EVENTS, SOCKET_ENV,
    TOKEN_ENV,
};

/// Name of the settings file written into the request's runtime directory.
pub const SETTINGS_FILE: &str = "settings.json";

/// Build the launch plan, writing the hooks settings file when a helper exists.
pub fn plan(request: &LaunchRequest) -> std::io::Result<LaunchPlan> {
    let mut args = Vec::new();

    if let Some(helper) = &request.hook_helper {
        let settings = write_settings(&request.runtime_dir, helper)?;
        // `--settings` *adds* a source; the user's own settings still load.
        // newt never writes to ~/.claude/settings.json.
        args.push("--settings".to_string());
        args.push(settings.to_string_lossy().into_owned());
    }

    if let Some(parent) = &request.fork_from {
        // Always paired: resuming without forking would put two tabs on one
        // conversation, and `--fork-session` is what gives the child its own
        // session id — which is also why newt never passes `--session-id`.
        args.push("--resume".to_string());
        args.push(parent.clone());
        args.push("--fork-session".to_string());
    }

    let mut env = Vec::new();
    if let Some(socket) = &request.socket_path {
        env.push((
            SOCKET_ENV.to_string(),
            socket.to_string_lossy().into_owned(),
        ));
    }
    if let Some(token) = &request.session_token {
        env.push((TOKEN_ENV.to_string(), token.clone()));
    }

    Ok(LaunchPlan {
        program: request.program.to_string_lossy().into_owned(),
        args,
        env,
        env_remove: vec![CHILD_SESSION_MARKER.to_string()],
        cwd: request.cwd.clone(),
    })
}

/// Write the settings file registering newt's hooks, returning its path.
pub fn write_settings(
    runtime_dir: &std::path::Path,
    helper: &std::path::Path,
) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(runtime_dir)?;
    let path = runtime_dir.join(SETTINGS_FILE);
    std::fs::write(&path, settings_json(helper))?;
    restrict_permissions(&path)?;
    Ok(path)
}

/// The settings document, as text.
///
/// Separated from writing it so the shape can be asserted without a
/// filesystem, which is most of what there is to get wrong here.
pub fn settings_json(helper: &std::path::Path) -> String {
    let command = shell_quote(helper);

    let mut hooks = serde_json::Map::new();
    for event in HOOK_EVENTS {
        hooks.insert(
            (*event).to_string(),
            json!([{
                "hooks": [{
                    "type": "command",
                    "command": command,
                }]
            }]),
        );
    }

    let document = Value::Object(
        [("hooks".to_string(), Value::Object(hooks))]
            .into_iter()
            .collect(),
    );

    // Pretty-printed on purpose: this lands in a temporary directory and the
    // first thing anyone does when hooks misbehave is read it.
    serde_json::to_string_pretty(&document).unwrap_or_default()
}

#[cfg(unix)]
fn restrict_permissions(path: &std::path::Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    // 0600: the file names a socket that accepts state for this user's tabs.
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
}

#[cfg(not(unix))]
fn restrict_permissions(_path: &std::path::Path) -> std::io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kind::AgentKind;

    fn temp_dir(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("newt-claude-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        path
    }

    fn request(name: &str) -> LaunchRequest {
        LaunchRequest {
            kind: AgentKind::Claude,
            program: PathBuf::from("/Users/x/.local/bin/claude"),
            cwd: Some(PathBuf::from("/work/project")),
            fork_from: None,
            hook_helper: None,
            runtime_dir: temp_dir(name),
            socket_path: None,
            session_token: None,
        }
    }

    #[test]
    fn a_fresh_session_with_no_hooks_takes_no_arguments_at_all() {
        let plan = plan(&request("bare")).expect("plan");

        assert_eq!(plan.program, "/Users/x/.local/bin/claude");
        assert!(plan.args.is_empty(), "args were {:?}", plan.args);
        assert!(plan.env.is_empty());
    }

    #[test]
    fn hooks_are_registered_through_settings_and_the_file_is_written() {
        let mut request = request("hooked");
        request.hook_helper = Some(PathBuf::from(
            "/Applications/newt.app/Contents/MacOS/newt-hook",
        ));
        let plan = plan(&request).expect("plan");

        let settings = request.runtime_dir.join(SETTINGS_FILE);
        assert_eq!(
            plan.args,
            vec![
                "--settings".to_string(),
                settings.to_string_lossy().into_owned()
            ]
        );
        assert!(settings.exists(), "settings file was not written");

        let _ = std::fs::remove_dir_all(&request.runtime_dir);
    }

    #[test]
    fn forking_resumes_the_parent_and_asks_for_a_new_session() {
        let mut request = request("fork");
        request.fork_from = Some("9f11a139-d258-433e-bc9c-1f648b761c71".to_string());
        let plan = plan(&request).expect("plan");

        assert_eq!(
            plan.args,
            vec![
                "--resume".to_string(),
                "9f11a139-d258-433e-bc9c-1f648b761c71".to_string(),
                "--fork-session".to_string(),
            ]
        );
    }

    #[test]
    fn newt_never_pins_the_session_id() {
        // `--fork-session` mints a new id, so pre-seeding one with
        // `--session-id` would be wrong for exactly the case Story 3 is about.
        // The id is learned from the SessionStart hook instead.
        let mut request = request("nopin");
        request.fork_from = Some("parent".to_string());
        request.hook_helper = Some(PathBuf::from("/bin/true"));
        let plan = plan(&request).expect("plan");

        assert!(
            !plan.args.iter().any(|arg| arg == "--session-id"),
            "args were {:?}",
            plan.args
        );
        let _ = std::fs::remove_dir_all(&request.runtime_dir);
    }

    #[test]
    fn the_child_session_marker_is_stripped() {
        // Inheriting it disables transcript saving, and with it every title,
        // token count and cost figure the sidebar shows. Verified against a
        // real session: with the marker the metadata stayed empty; without it
        // the agent's own title and token totals arrived.
        let plan = plan(&request("marker")).expect("plan");
        assert!(plan
            .env_remove
            .iter()
            .any(|key| key == CHILD_SESSION_MARKER));
    }

    #[test]
    fn the_working_directory_reaches_the_plan() {
        // The project an agent operates on is its working directory, and a
        // child tab inherits its parent's. A request field that never reached
        // the plan started every agent wherever newt was launched from.
        let plan = plan(&request("cwd")).expect("plan");
        assert_eq!(plan.cwd, Some(PathBuf::from("/work/project")));
    }

    #[test]
    fn the_socket_and_token_are_passed_through_the_environment() {
        let mut request = request("env");
        request.socket_path = Some(PathBuf::from("/tmp/newt-501/abc.sock"));
        request.session_token = Some("deadbeef".to_string());
        let plan = plan(&request).expect("plan");

        assert_eq!(
            plan.env,
            vec![
                (SOCKET_ENV.to_string(), "/tmp/newt-501/abc.sock".to_string()),
                (TOKEN_ENV.to_string(), "deadbeef".to_string()),
            ]
        );
    }

    #[test]
    fn the_settings_document_registers_every_event_against_the_helper() {
        let helper = std::path::Path::new("/Applications/newt.app/Contents/MacOS/newt-hook");
        let document: Value = serde_json::from_str(&settings_json(helper)).expect("valid json");

        let hooks = document
            .get("hooks")
            .and_then(Value::as_object)
            .expect("hooks object");

        assert_eq!(hooks.len(), HOOK_EVENTS.len());
        for event in HOOK_EVENTS {
            let command = hooks[*event][0]["hooks"][0]["command"]
                .as_str()
                .unwrap_or_default();
            assert_eq!(command, "'/Applications/newt.app/Contents/MacOS/newt-hook'");
            assert_eq!(hooks[*event][0]["hooks"][0]["type"], "command");
        }
    }

    #[test]
    fn a_helper_path_with_a_space_stays_one_argument_in_the_document() {
        let helper = std::path::Path::new("/Users/x/My Apps/newt.app/Contents/MacOS/newt-hook");
        let document: Value = serde_json::from_str(&settings_json(helper)).expect("valid json");

        let command = document["hooks"]["Stop"][0]["hooks"][0]["command"]
            .as_str()
            .unwrap();
        assert!(command.starts_with('\''), "command was {command}");
        assert!(command.ends_with('\''));
        assert!(command.contains("My Apps"));
    }

    #[test]
    fn the_settings_file_is_not_world_readable() {
        let directory = temp_dir("perms");
        let path = write_settings(&directory, std::path::Path::new("/bin/true")).expect("write");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600, "mode was {:o}", mode & 0o777);
        }
        assert!(path.ends_with(SETTINGS_FILE));

        let _ = std::fs::remove_dir_all(&directory);
    }
}
