//! Reading what Claude Code writes about itself.
//!
//! Claude Code appends one JSON object per line to
//! `~/.claude/projects/<slug>/<session>.jsonl` while it runs. Four line kinds
//! carry what a sidebar wants:
//!
//! | `type` | field | becomes |
//! |---|---|---|
//! | `ai-title` | `aiTitle` | the tab's title — its own 3–5 word summary |
//! | `assistant` | `message.usage` | token totals |
//! | `assistant` | `message.model` | the model |
//! | `cost-state` | `totalCostUSD` | cost |
//!
//! newt never guesses this path: `SessionStart` hands it over in
//! `transcript_path`, which also means a forked session's transcript is found
//! without knowing anything about how the slug is built.
//!
//! # Incremental, and that is the hard part
//!
//! This tails a file another process is appending to, so a read can land in
//! the middle of a line. The reader keeps a byte offset and consumes only up
//! to the last newline; a partial trailing line stays unconsumed until its
//! remainder arrives. `CLAUDE.md` requires the parser be tested against
//! streams split at every offset, and a JSONL tailer is exactly that problem —
//! the tests beside this do it.
//!
//! # Totals live here
//!
//! The reader owns the running totals and republishes them whole. Everything
//! downstream replaces rather than accumulates, so a re-scan after a
//! truncation cannot double-count. See [`crate::update`].

use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::update::MetadataUpdate;

/// Tails one transcript file.
#[derive(Debug)]
pub struct TranscriptReader {
    path: PathBuf,
    /// How far into the file has been consumed, in bytes.
    offset: u64,
    input_tokens: u64,
    output_tokens: u64,
    cost_micros: u64,
    model: Option<String>,
    title: Option<String>,
    /// Set when a consumed line changed something worth reporting.
    changed: bool,
}

impl TranscriptReader {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            offset: 0,
            input_tokens: 0,
            output_tokens: 0,
            cost_micros: 0,
            model: None,
            title: None,
            changed: false,
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Consume whatever has been appended since the last call.
    ///
    /// Returns `None` when nothing changed, so a caller polling several times
    /// a second does no work in the common case.
    pub fn poll(&mut self) -> Option<MetadataUpdate> {
        let mut file = std::fs::File::open(&self.path).ok()?;
        let length = file.metadata().ok()?.len();

        // Shorter than what has been read means the file was replaced or
        // rewritten — a compaction, or `/clear`. Start again rather than
        // reading from a stale offset into unrelated content.
        if length < self.offset {
            self.reset();
        }
        if length == self.offset {
            return None;
        }

        file.seek(SeekFrom::Start(self.offset)).ok()?;
        let mut chunk = Vec::new();
        file.take(MAX_CHUNK_BYTES).read_to_end(&mut chunk).ok()?;

        // Only whole lines. A trailing fragment is left for the next poll,
        // which is what makes reading a file mid-append safe.
        let consumed = chunk.iter().rposition(|byte| *byte == b'\n')? + 1;
        self.offset += consumed as u64;

        for line in chunk[..consumed].split(|byte| *byte == b'\n') {
            if !line.is_empty() {
                self.consume(line);
            }
        }

        self.report()
    }

    /// The current totals, whether or not anything just changed.
    pub fn snapshot(&self) -> MetadataUpdate {
        MetadataUpdate {
            input_tokens: Some(self.input_tokens),
            output_tokens: Some(self.output_tokens),
            cost_micros: Some(self.cost_micros),
            model: self.model.clone(),
            agent_title: self.title.clone(),
            ..MetadataUpdate::default()
        }
    }

    fn report(&mut self) -> Option<MetadataUpdate> {
        if !self.changed {
            return None;
        }
        self.changed = false;
        Some(self.snapshot())
    }

    fn reset(&mut self) {
        self.offset = 0;
        self.input_tokens = 0;
        self.output_tokens = 0;
        self.cost_micros = 0;
        // The model and title are deliberately kept: a rewritten transcript
        // still belongs to the same session, and blanking them would make the
        // sidebar flicker back to "Claude Code" mid-task.
        self.changed = true;
    }

    /// Fold one line into the running totals.
    fn consume(&mut self, line: &[u8]) {
        let Ok(value) = serde_json::from_slice::<Value>(line) else {
            // A malformed line is skipped, not fatal. This file is written by
            // another process and read while it is being written.
            return;
        };

        match value.get("type").and_then(Value::as_str) {
            Some("ai-title") => {
                if let Some(title) = value.get("aiTitle").and_then(Value::as_str) {
                    if !title.is_empty() && self.title.as_deref() != Some(title) {
                        self.title = Some(title.to_string());
                        self.changed = true;
                    }
                }
            }
            Some("assistant") => self.consume_assistant(&value),
            Some("cost-state") => {
                if let Some(usd) = value.get("totalCostUSD").and_then(Value::as_f64) {
                    // Cumulative in the file, so it replaces rather than adds.
                    let micros = (usd.max(0.0) * 1_000_000.0).round() as u64;
                    if micros != self.cost_micros {
                        self.cost_micros = micros;
                        self.changed = true;
                    }
                }
            }
            _ => {}
        }
    }

    fn consume_assistant(&mut self, value: &Value) {
        let Some(message) = value.get("message") else {
            return;
        };

        if let Some(model) = message.get("model").and_then(Value::as_str) {
            if !model.is_empty() && self.model.as_deref() != Some(model) {
                self.model = Some(model.to_string());
                self.changed = true;
            }
        }

        let Some(usage) = message.get("usage") else {
            return;
        };

        let field = |name: &str| usage.get(name).and_then(Value::as_u64).unwrap_or(0);

        // Cache reads are counted as input. They are what the model actually
        // saw, and excluding them understates the context by an order of
        // magnitude — a real turn here showed 19,470 cached against 2 fresh.
        // It does mean the displayed number is dominated by cache traffic;
        // flagged in the plan as worth revisiting against a long session.
        let input = field("input_tokens")
            + field("cache_creation_input_tokens")
            + field("cache_read_input_tokens");
        let output = field("output_tokens");

        if input > 0 || output > 0 {
            self.input_tokens = self.input_tokens.saturating_add(input);
            self.output_tokens = self.output_tokens.saturating_add(output);
            self.changed = true;
        }
    }
}

/// Never read more than this in one poll.
///
/// A transcript grows without bound over a long session; a first poll on a
/// large one should not stall the tailer thread. The remainder is picked up on
/// the next poll, because the offset only advances by what was consumed.
const MAX_CHUNK_BYTES: u64 = 4 * 1024 * 1024;

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &[u8] = include_bytes!("../tests/fixtures/claude-transcript.jsonl");

    /// A transcript file that cleans itself up.
    struct Scratch {
        path: PathBuf,
    }

    impl Scratch {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "newt-transcript-{name}-{}-{}.jsonl",
                std::process::id(),
                SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ));
            let _ = std::fs::remove_file(&path);
            Self { path }
        }

        fn write(&self, bytes: &[u8]) {
            std::fs::write(&self.path, bytes).expect("write");
        }

        fn append(&self, bytes: &[u8]) {
            use std::io::Write;
            let mut file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&self.path)
                .expect("open");
            file.write_all(bytes).expect("append");
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.path);
        }
    }

    static SEQUENCE: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

    fn read_whole(bytes: &[u8], name: &str) -> MetadataUpdate {
        let scratch = Scratch::new(name);
        scratch.write(bytes);
        let mut reader = TranscriptReader::new(&scratch.path);
        reader.poll();
        reader.snapshot()
    }

    #[test]
    fn the_fixture_yields_the_totals_it_describes() {
        let update = read_whole(FIXTURE, "whole");

        // Two assistant messages: (12+8834+19470) + (4+100+21000).
        assert_eq!(
            update.input_tokens,
            Some(12 + 8834 + 19470 + 4 + 100 + 21000)
        );
        assert_eq!(update.output_tokens, Some(140 + 60));
        assert_eq!(update.model.as_deref(), Some("claude-opus-5"));
        // The later title wins; it is rewritten as the session evolves.
        assert_eq!(
            update.agent_title.as_deref(),
            Some("Reflow and resize tests")
        );
        // 0.831234 USD, to the nearest millionth.
        assert_eq!(update.cost_micros, Some(831_234));
    }

    #[test]
    fn splitting_the_stream_at_every_offset_changes_nothing() {
        // `CLAUDE.md`: "Test the parser against byte streams split at every
        // offset — chunk-boundary resumption is not optional." A tailer
        // reading a file another process is appending to is that problem, and
        // a split landing mid-line is the likeliest source of a wrong total.
        let expected = read_whole(FIXTURE, "reference");

        for split in 0..FIXTURE.len() {
            let scratch = Scratch::new("split");
            let mut reader = TranscriptReader::new(&scratch.path);

            scratch.write(&FIXTURE[..split]);
            reader.poll();
            scratch.append(&FIXTURE[split..]);
            reader.poll();

            assert_eq!(
                reader.snapshot(),
                expected,
                "differed when split at byte {split}"
            );
        }
    }

    #[test]
    fn a_partial_final_line_is_not_consumed_until_it_is_complete() {
        let scratch = Scratch::new("partial");
        let mut reader = TranscriptReader::new(&scratch.path);

        scratch.write(br#"{"type":"ai-title","aiTitle":"half writ"#);
        assert_eq!(reader.poll(), None, "a fragment must not be parsed");

        scratch.append(b"ten\",\"sessionId\":\"s\"}\n");
        let update = reader.poll().expect("the completed line");
        assert_eq!(update.agent_title.as_deref(), Some("half written"));
    }

    #[test]
    fn a_rewritten_transcript_is_rescanned_rather_than_doubled() {
        // What a compaction does: the file is replaced in place and is shorter
        // than what has already been read. Reading on from the old offset
        // would land in unrelated content; adding to the old totals would
        // double them.
        let scratch = Scratch::new("compact");
        let mut reader = TranscriptReader::new(&scratch.path);

        scratch.write(FIXTURE);
        reader.poll();
        let before = reader.snapshot();

        let compacted = &FIXTURE[..FIXTURE.len() / 2];
        let boundary = compacted
            .iter()
            .rposition(|b| *b == b'\n')
            .map(|i| i + 1)
            .unwrap_or(0);
        scratch.write(&FIXTURE[..boundary]);
        reader.poll();
        let after = reader.snapshot();

        assert!(
            after.input_tokens < before.input_tokens,
            "totals were {before:?} then {after:?}; a rescan must not accumulate"
        );
        assert_eq!(
            read_whole(&FIXTURE[..boundary], "compact-ref").input_tokens,
            after.input_tokens
        );
    }

    #[test]
    fn polling_an_unchanged_file_reports_nothing() {
        let scratch = Scratch::new("quiet");
        scratch.write(FIXTURE);
        let mut reader = TranscriptReader::new(&scratch.path);

        assert!(reader.poll().is_some());
        // A sidebar polls several times a second; a file that has not grown
        // must cost nothing and produce no update.
        assert_eq!(reader.poll(), None);
        assert_eq!(reader.poll(), None);
    }

    #[test]
    fn a_missing_file_is_not_an_error() {
        // The transcript does not exist until Claude Code writes it, and the
        // path arrives from `SessionStart` before the first line is flushed.
        let scratch = Scratch::new("missing");
        let mut reader = TranscriptReader::new(&scratch.path);

        assert_eq!(reader.poll(), None);
        scratch.write(b"{\"type\":\"ai-title\",\"aiTitle\":\"arrived\"}\n");
        assert_eq!(
            reader.poll().and_then(|u| u.agent_title).as_deref(),
            Some("arrived")
        );
    }

    #[test]
    fn malformed_and_unknown_lines_are_skipped_without_losing_the_rest() {
        // The fixture carries both, between lines that matter. If either
        // aborted the scan, the totals above would come out short.
        let update = read_whole(FIXTURE, "junk");
        assert_eq!(update.output_tokens, Some(200));
    }

    #[test]
    fn a_line_arriving_one_byte_at_a_time_still_parses() {
        let line = br#"{"type":"assistant","message":{"model":"m","usage":{"output_tokens":7}}}"#;
        let scratch = Scratch::new("bytewise");
        let mut reader = TranscriptReader::new(&scratch.path);

        for byte in line.iter() {
            scratch.append(std::slice::from_ref(byte));
            assert_eq!(
                reader.poll(),
                None,
                "no newline yet, so nothing is complete"
            );
        }
        scratch.append(b"\n");

        let update = reader.poll().expect("complete line");
        assert_eq!(update.output_tokens, Some(7));
        assert_eq!(update.model.as_deref(), Some("m"));
    }

    #[test]
    fn cost_replaces_rather_than_accumulates() {
        let scratch = Scratch::new("cost");
        let mut reader = TranscriptReader::new(&scratch.path);

        scratch.append(b"{\"type\":\"cost-state\",\"totalCostUSD\":0.10}\n");
        reader.poll();
        scratch.append(b"{\"type\":\"cost-state\",\"totalCostUSD\":0.25}\n");
        let update = reader.poll().expect("update");

        // The field is cumulative in the file; adding would give 0.35.
        assert_eq!(update.cost_micros, Some(250_000));
    }
}
