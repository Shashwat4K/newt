//! Finding installed agent CLIs, and resolving them to absolute paths.
//!
//! # Why this cannot just use `PATH`
//!
//! `portable-pty`'s `CommandBuilder` resolves argv[0] against the environment
//! *the spawning process inherited*. When newt runs from a terminal, that is
//! the developer's `PATH` and `claude` resolves fine. When `newt.app` is
//! double-clicked, launchd hands it a minimal `PATH` of
//! `/usr/bin:/bin:/usr/sbin:/sbin` — and Claude Code installs to
//! `~/.local/bin`, which is not on it.
//!
//! So a bare program name would work under `swift build && swift run` and fail
//! from `/Applications`, which is the worst possible split. Everything here
//! returns an **absolute** path, and the spec carries that across the ABI.

use std::path::{Path, PathBuf};

use crate::kind::AgentKind;

/// Directories searched before `PATH`, in order.
///
/// These are where the agent CLIs actually install on macOS, and none of them
/// is on launchd's default `PATH`. `$HOME` is substituted at call time.
const EXTRA_DIRECTORIES: &[&str] = &[
    "$HOME/.local/bin",
    "$HOME/bin",
    "/opt/homebrew/bin",
    "/usr/local/bin",
];

/// Absolute path to an agent's executable, or `None` if it is not installed.
pub fn find(kind: AgentKind) -> Option<PathBuf> {
    find_in(kind.program_name(), &search_path())
}

/// Every agent newt knows about that is actually installed here.
pub fn installed() -> Vec<AgentKind> {
    let path = search_path();
    AgentKind::ALL
        .iter()
        .copied()
        .filter(|kind| find_in(kind.program_name(), &path).is_some())
        .collect()
}

/// The directories [`find`] searches, in order: the extras above, then `PATH`.
///
/// Duplicates are kept — checking a directory twice is cheaper than
/// deduplicating, and the first hit wins either way.
pub fn search_path() -> Vec<PathBuf> {
    let home = std::env::var("HOME").ok();

    let mut directories: Vec<PathBuf> = EXTRA_DIRECTORIES
        .iter()
        .filter_map(|entry| match entry.strip_prefix("$HOME/") {
            Some(rest) => home.as_ref().map(|home| Path::new(home).join(rest)),
            None => Some(PathBuf::from(entry)),
        })
        .collect();

    if let Ok(path) = std::env::var("PATH") {
        directories.extend(std::env::split_paths(&path));
    }

    directories
}

/// Look for `program` in `directories`, returning the first executable match.
///
/// Separated from [`find`] so tests can supply their own directories rather
/// than depending on what happens to be installed on the machine running them.
pub fn find_in(program: &str, directories: &[PathBuf]) -> Option<PathBuf> {
    // An absolute or explicitly relative name is a path, not something to look
    // up — the same rule a shell follows.
    if program.contains('/') {
        let candidate = PathBuf::from(program);
        return is_executable(&candidate).then_some(candidate);
    }

    directories
        .iter()
        .map(|directory| directory.join(program))
        .find(|candidate| is_executable(candidate))
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;

    // `metadata` follows symlinks, which is what we want: Claude Code installs
    // `~/.local/bin/claude` as a symlink into a versioned directory.
    match std::fs::metadata(path) {
        Ok(metadata) => metadata.is_file() && metadata.permissions().mode() & 0o111 != 0,
        Err(_) => false,
    }
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    // Windows decides executability by extension and PATHEXT rather than by a
    // mode bit. Left for the port; being wrong here would only mean failing to
    // find an agent, never running the wrong thing.
    path.is_file()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A directory holding a fake executable, cleaned up on drop.
    struct Fixture {
        root: PathBuf,
    }

    impl Fixture {
        fn new(name: &str) -> Self {
            let root =
                std::env::temp_dir().join(format!("newt-detect-{}-{}", name, std::process::id()));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(&root).expect("create fixture");
            Self { root }
        }

        fn executable(&self, name: &str) -> PathBuf {
            let path = self.root.join(name);
            std::fs::write(&path, "#!/bin/sh\nexit 0\n").expect("write");
            set_executable(&path);
            path
        }

        fn plain_file(&self, name: &str) -> PathBuf {
            let path = self.root.join(name);
            std::fs::write(&path, "not executable").expect("write");
            path
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    #[cfg(unix)]
    fn set_executable(path: &Path) {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = std::fs::metadata(path).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(path, permissions).unwrap();
    }

    #[cfg(not(unix))]
    fn set_executable(_path: &Path) {}

    #[test]
    fn an_executable_is_found_and_reported_absolute() {
        let fixture = Fixture::new("found");
        let expected = fixture.executable("pretend-agent");

        let found = find_in("pretend-agent", std::slice::from_ref(&fixture.root));

        assert_eq!(found.as_deref(), Some(expected.as_path()));
        // Absolute is the whole point: a bare name would resolve against
        // launchd's minimal PATH once newt runs as a bundle.
        assert!(found.unwrap().is_absolute());
    }

    #[test]
    fn a_non_executable_file_of_the_right_name_is_not_a_match() {
        let fixture = Fixture::new("mode");
        fixture.plain_file("pretend-agent");

        assert_eq!(
            find_in("pretend-agent", std::slice::from_ref(&fixture.root)),
            None
        );
    }

    #[test]
    fn the_first_directory_holding_it_wins() {
        let first = Fixture::new("first");
        let second = Fixture::new("second");
        let expected = first.executable("pretend-agent");
        second.executable("pretend-agent");

        let found = find_in("pretend-agent", &[first.root.clone(), second.root.clone()]);

        assert_eq!(found.as_deref(), Some(expected.as_path()));
    }

    #[test]
    fn a_missing_program_is_none_rather_than_a_guess() {
        let fixture = Fixture::new("missing");
        assert_eq!(
            find_in("no-such-agent", std::slice::from_ref(&fixture.root)),
            None
        );
        assert_eq!(find_in("no-such-agent", &[]), None);
    }

    #[test]
    fn a_name_containing_a_slash_is_treated_as_a_path() {
        let fixture = Fixture::new("explicit");
        let path = fixture.executable("pretend-agent");

        // The directory list is empty, so a lookup would fail; this must
        // resolve because the name is already a path.
        let found = find_in(path.to_str().unwrap(), &[]);
        assert_eq!(found.as_deref(), Some(path.as_path()));

        assert_eq!(find_in("/nonexistent/pretend-agent", &[]), None);
    }

    #[test]
    fn the_search_path_leads_with_directories_launchd_omits() {
        let directories = search_path();
        let rendered: Vec<String> = directories
            .iter()
            .map(|d| d.display().to_string())
            .collect();

        // The failure this guards against is silent: if these ever fall behind
        // PATH, detection still succeeds from a terminal and fails from the
        // bundle, which reads as "the app is broken" rather than "the search
        // order changed".
        assert!(
            rendered.iter().any(|d| d.ends_with("/.local/bin")),
            "search path was {rendered:?}"
        );
        assert!(rendered.iter().any(|d| d == "/opt/homebrew/bin"));
    }
}
