//! Agent awareness for newt: which coding-agent CLIs are installed, how to
//! launch one, and — from Phase 12 — what it reports back while it runs.
//!
//! # Why this is in the core
//!
//! None of it is macOS work. Probing directories, building an argument list,
//! knowing what `--fork-session` means, and parsing a JSON payload are the same
//! on every platform, so putting them here means the Linux and Windows ports
//! inherit them rather than reimplementing them. `CLAUDE.md` is explicit that
//! the Swift shell owns no semantics, and "what an `ai-title` is" is semantics.
//!
//! It is also what makes this testable. There is no window, no PTY, and no
//! `claude` process anywhere in this crate's tests — detection runs against
//! fixture directories and the launch recipes are asserted as data.
//!
//! # Shape
//!
//! `detect` → `launch`/`claude` today. The adapter for a second agent slots in
//! beside `claude` behind the same two entry points, which is the extension
//! point rather than a trait hierarchy built before there is a second case.

pub mod claude;
pub mod detect;
pub mod kind;
pub mod launch;

pub use kind::AgentKind;
pub use launch::{plan, LaunchPlan, LaunchRequest};

pub mod bridge;
pub mod hooks;
pub mod ipc;
pub mod transcript;
pub mod update;

pub use bridge::{AgentBridge, Mailbox};
pub use transcript::TranscriptReader;
pub use update::{AgentStateHint, MetadataUpdate};
