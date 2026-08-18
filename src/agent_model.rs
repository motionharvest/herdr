//! Model, reasoning-effort, and session-title detection for agent sessions.
//!
//! Agents that report a session id through the herdr integration also write a
//! session log on disk (Claude Code transcripts, Codex rollouts). The log
//! carries the model and reasoning effort actually used for each turn, so it
//! stays correct across mid-session model switches. A background refresh
//! resolves each terminal's session file, tail-reads it when its mtime
//! changes, and reports the latest model info back to the main loop.
//!
//! The same read answers a second question for harnesses that have no way to
//! announce one. Claude Code names its own session and herdr's statusline
//! reports that name, so a Claude pane wears a title the harness wrote. Codex
//! never names a session, and its hooks carry only the session id, so a Codex
//! pane had nothing to show. The rollout does record what the user last asked
//! for, so that prompt becomes the title instead: the summary column then says
//! what a Codex agent is working on rather than staying blank.

use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::detect::Agent;
use crate::terminal::TerminalId;

/// Tail window read from a session file. Assistant/turn entries can trail
/// large tool-result lines, so the window is generous; reads only happen
/// when the file's mtime changes.
const TAIL_READ_BYTES: u64 = 256 * 1024;

/// How far back the first title scan of a session reaches. A prompt is written
/// once, at the start of a turn, and everything the agent then does is written
/// after it, so in a long session the current prompt sits far from the end —
/// past the tail window the model read uses. The first scan is therefore
/// generous. Every scan after it starts where the previous one stopped, so it
/// reads only what the session has written since, which is small.
const TITLE_FIRST_SCAN_BYTES: u64 = 32 * 1024 * 1024;

/// The model an agent session is running, as recorded in its session log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentModelInfo {
    /// Raw model id, e.g. `claude-fable-5`.
    pub model: String,
    /// Reasoning effort level, e.g. `high`, when the log records one.
    pub effort: Option<String>,
}

impl AgentModelInfo {
    /// Human-readable label, e.g. `Fable 5 high`.
    pub fn display_label(&self) -> String {
        let mut label = display_model_name(&self.model);
        if let Some(effort) = &self.effort {
            label.push(' ');
            label.push_str(effort);
        }
        label
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentModelCacheEntry {
    pub session_id: String,
    pub session_file: PathBuf,
    pub modified: SystemTime,
    pub info: Option<AgentModelInfo>,
    /// A title read out of the session log, for harnesses that never report
    /// one themselves. `None` for a harness whose integration announces its
    /// own title, so a probe can never overwrite what the harness said.
    pub title: Option<String>,
    /// Byte offset the title scan has read up to, always on a line boundary.
    /// The next scan starts here, so a prompt is found once and the growing
    /// session log is never re-read.
    pub title_scanned_len: u64,
}

#[derive(Debug)]
pub struct AgentModelRefreshJob {
    pub terminal_id: TerminalId,
    pub agent: Agent,
    pub session_id: String,
    pub cached: Option<AgentModelCacheEntry>,
}

#[derive(Debug)]
pub struct AgentModelRefreshResult {
    pub terminal_id: TerminalId,
    pub entry: AgentModelCacheEntry,
}

/// Whether model info can be probed for this agent's sessions.
pub fn probe_supported(agent: Agent) -> bool {
    matches!(agent, Agent::Claude | Agent::Codex)
}

pub(crate) fn refresh_agent_model_infos(
    jobs: Vec<AgentModelRefreshJob>,
) -> Vec<AgentModelRefreshResult> {
    jobs.into_iter().filter_map(refresh_job).collect()
}

fn refresh_job(job: AgentModelRefreshJob) -> Option<AgentModelRefreshResult> {
    if !valid_probe_session_id(&job.session_id) {
        return None;
    }

    let session_file = job
        .cached
        .as_ref()
        .map(|cached| cached.session_file.clone())
        .filter(|file| file.is_file())
        .or_else(|| resolve_session_file(job.agent, &job.session_id))?;
    let modified = fs::metadata(&session_file).ok()?.modified().ok()?;
    if job
        .cached
        .as_ref()
        .is_some_and(|cached| cached.session_file == session_file && cached.modified == modified)
    {
        return None;
    }

    let tail = read_tail(&session_file, TAIL_READ_BYTES).ok()?;
    let info = match job.agent {
        Agent::Claude => parse_claude_transcript_tail(&tail),
        Agent::Codex => parse_codex_rollout_tail(&tail),
        _ => None,
    }
    // The tail window can land past the last model-bearing entry (e.g. a huge
    // trailing tool result); hold the previous observation instead of blanking.
    .or_else(|| job.cached.as_ref().and_then(|cached| cached.info.clone()));

    let (title, title_scanned_len) = match job.agent {
        Agent::Codex => scan_session_title(
            &session_file,
            job.cached
                .as_ref()
                .filter(|cached| cached.session_file == session_file)
                .map(|cached| cached.title_scanned_len),
            parse_codex_rollout_title,
        ),
        _ => (None, 0),
    };
    let title = title.or_else(|| job.cached.and_then(|cached| cached.title));

    Some(AgentModelRefreshResult {
        terminal_id: job.terminal_id,
        entry: AgentModelCacheEntry {
            session_id: job.session_id,
            session_file,
            modified,
            info,
            title,
            title_scanned_len,
        },
    })
}

fn resolve_session_file(agent: Agent, session_id: &str) -> Option<PathBuf> {
    match agent {
        Agent::Claude => claude_session_file(&crate::integration::claude_dir().ok()?, session_id),
        Agent::Codex => codex_session_file(&crate::integration::codex_dir().ok()?, session_id),
        _ => None,
    }
}

/// Session ids double as file-name fragments; only accept shapes that cannot
/// escape the session directories.
fn valid_probe_session_id(session_id: &str) -> bool {
    !session_id.is_empty()
        && session_id.len() <= 128
        && session_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Claude Code transcripts live at `<config>/projects/<cwd-slug>/<session>.jsonl`.
/// Scanning for the file by session id avoids re-implementing the cwd slug.
fn claude_session_file(claude_dir: &Path, session_id: &str) -> Option<PathBuf> {
    let file_name = format!("{session_id}.jsonl");
    for entry in fs::read_dir(claude_dir.join("projects")).ok()?.flatten() {
        let candidate = entry.path().join(&file_name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

/// Codex rollouts live at `<home>/sessions/YYYY/MM/DD/rollout-<ts>-<session>.jsonl`.
/// Directories are walked newest-first so recent sessions resolve quickly.
fn codex_session_file(codex_home: &Path, session_id: &str) -> Option<PathBuf> {
    let suffix = format!("-{session_id}.jsonl");
    find_file_with_suffix(&codex_home.join("sessions"), &suffix, 4)
}

fn find_file_with_suffix(dir: &Path, suffix: &str, depth: u8) -> Option<PathBuf> {
    let mut entries: Vec<PathBuf> = fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .collect();
    entries.sort();
    for path in entries.into_iter().rev() {
        if path.is_dir() {
            if depth > 0 {
                if let Some(found) = find_file_with_suffix(&path, suffix, depth - 1) {
                    return Some(found);
                }
            }
        } else if path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.ends_with(suffix))
        {
            return Some(path);
        }
    }
    None
}

fn read_tail(path: &Path, max_bytes: u64) -> std::io::Result<String> {
    let mut file = fs::File::open(path)?;
    let len = file.metadata()?.len();
    file.seek(SeekFrom::Start(len.saturating_sub(max_bytes)))?;
    let mut buf = Vec::new();
    file.read_to_end(&mut buf)?;
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

/// Read whatever the session has written since the last scan and look for a
/// title in it, returning the title found and the offset the next scan starts
/// from.
///
/// `scanned_len` is `None` the first time a session is probed, and is dropped
/// when it is past the end of the file — a log that shrank is a different log,
/// under the same name — so both cases fall back to the generous first scan.
/// The returned offset stops at the last newline, so a line half-written when
/// the read happened is read whole by the next one.
fn scan_session_title(
    path: &Path,
    scanned_len: Option<u64>,
    parse: fn(&str) -> Option<String>,
) -> (Option<String>, u64) {
    let Ok(len) = fs::metadata(path).map(|metadata| metadata.len()) else {
        return (None, scanned_len.unwrap_or(0));
    };
    let start = scanned_len
        .filter(|start| *start <= len)
        .unwrap_or_else(|| len.saturating_sub(TITLE_FIRST_SCAN_BYTES));
    if start >= len {
        return (None, start);
    }
    let Ok(chunk) = read_from(path, start) else {
        return (None, start);
    };
    let consumed = match chunk.rfind('\n') {
        Some(last_newline) => start + last_newline as u64 + 1,
        None => start,
    };
    (parse(&chunk), consumed)
}

fn read_from(path: &Path, start: u64) -> std::io::Result<String> {
    let mut file = fs::File::open(path)?;
    file.seek(SeekFrom::Start(start))?;
    let mut buf = Vec::new();
    file.read_to_end(&mut buf)?;
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

/// Latest main-conversation assistant entry: `message.model` plus the
/// top-level `effort` field newer Claude Code versions stamp per response.
fn parse_claude_transcript_tail(tail: &str) -> Option<AgentModelInfo> {
    for line in tail.lines().rev() {
        let Ok(entry) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if entry.get("type").and_then(|t| t.as_str()) != Some("assistant") {
            continue;
        }
        // Sidechain entries are subagent turns and can run a different model.
        if entry.get("isSidechain").and_then(|s| s.as_bool()) == Some(true) {
            continue;
        }
        let Some(model) = entry
            .get("message")
            .and_then(|message| message.get("model"))
            .and_then(|model| model.as_str())
        else {
            continue;
        };
        // Error placeholder entries record "<synthetic>" instead of a model.
        if model.is_empty() || model.starts_with('<') {
            continue;
        }
        return Some(AgentModelInfo {
            model: model.to_string(),
            effort: entry
                .get("effort")
                .and_then(|effort| effort.as_str())
                .map(str::to_string),
        });
    }
    None
}

/// Latest `turn_context` entry: `payload.model` plus `payload.effort`.
fn parse_codex_rollout_tail(tail: &str) -> Option<AgentModelInfo> {
    for line in tail.lines().rev() {
        let Ok(entry) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if entry.get("type").and_then(|t| t.as_str()) != Some("turn_context") {
            continue;
        }
        let payload = entry.get("payload")?;
        let model = payload.get("model").and_then(|model| model.as_str())?;
        if model.is_empty() {
            continue;
        }
        return Some(AgentModelInfo {
            model: model.to_string(),
            effort: payload
                .get("effort")
                .and_then(|effort| effort.as_str())
                .map(str::to_string),
        });
    }
    None
}

/// A session title is a row in a table and a line under a pane name, so it is
/// cut to a length either can hold before it ever reaches the screen.
const SESSION_TITLE_MAX_CHARS: usize = 120;

/// The last thing the user actually asked for, taken from the newest `user`
/// message in the rollout.
///
/// Codex writes more than typed prompts into the `user` role: the project's
/// `AGENTS.md`, and context blocks wrapped in tags such as
/// `<environment_context>`. Those are the harness talking to its own model, not
/// a task, so a message that opens a tag or opens the `AGENTS.md` header is
/// skipped and the scan keeps walking backwards.
fn parse_codex_rollout_title(tail: &str) -> Option<String> {
    for line in tail.lines().rev() {
        let Ok(entry) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if entry.get("type").and_then(|entry_type| entry_type.as_str()) != Some("response_item") {
            continue;
        }
        let Some(payload) = entry.get("payload") else {
            continue;
        };
        if payload.get("type").and_then(|kind| kind.as_str()) != Some("message")
            || payload.get("role").and_then(|role| role.as_str()) != Some("user")
        {
            continue;
        }
        let Some(text) = payload
            .get("content")
            .and_then(|content| content.as_array())
            .into_iter()
            .flatten()
            .filter_map(|part| part.get("text").and_then(|text| text.as_str()))
            .find(|text| !text.trim().is_empty())
        else {
            continue;
        };
        if let Some(title) = session_title_from_prompt(text) {
            return Some(title);
        }
    }
    None
}

/// One line, cut to length — or nothing at all when the text is the harness
/// briefing its model rather than a person asking for something.
fn session_title_from_prompt(text: &str) -> Option<String> {
    let trimmed = text.trim_start();
    if trimmed.starts_with('<') || trimmed.starts_with("# AGENTS.md instructions") {
        return None;
    }
    let condensed = trimmed.split_whitespace().collect::<Vec<_>>().join(" ");
    if condensed.is_empty() {
        return None;
    }
    if condensed.chars().count() <= SESSION_TITLE_MAX_CHARS {
        return Some(condensed);
    }
    Some(
        condensed
            .chars()
            .take(SESSION_TITLE_MAX_CHARS.saturating_sub(1))
            .collect::<String>()
            + "…",
    )
}

/// Prettify a raw model id: `claude-fable-5` → `Fable 5`,
/// `claude-opus-4-5-20251101` → `Opus 4.5`, `gpt-5.6-sol` → `GPT-5.6 Sol`.
pub fn display_model_name(raw: &str) -> String {
    let trimmed = raw.trim();
    let trimmed = trimmed.strip_suffix("[1m]").unwrap_or(trimmed).trim();
    let mut tokens: Vec<&str> = trimmed.split('-').filter(|t| !t.is_empty()).collect();
    if tokens
        .first()
        .is_some_and(|t| t.eq_ignore_ascii_case("claude"))
    {
        tokens.remove(0);
    }
    if tokens
        .last()
        .is_some_and(|t| t.len() == 8 && t.chars().all(|c| c.is_ascii_digit()))
    {
        tokens.pop();
    }

    let mut parts: Vec<String> = Vec::new();
    let mut last_numeric = false;
    for token in tokens {
        let numeric = token.chars().any(|c| c.is_ascii_digit())
            && token.chars().all(|c| c.is_ascii_digit() || c == '.');
        if numeric && last_numeric {
            let last = parts.last_mut().expect("numeric run has a head");
            last.push('.');
            last.push_str(token);
        } else if numeric && parts.last().is_some_and(|p| p == "GPT") {
            let last = parts.last_mut().expect("checked non-empty");
            last.push('-');
            last.push_str(token);
        } else if numeric {
            parts.push(token.to_string());
        } else if token.eq_ignore_ascii_case("gpt") {
            parts.push("GPT".to_string());
        } else {
            let mut chars = token.chars();
            let capitalized = match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            };
            parts.push(capitalized);
        }
        last_numeric = numeric;
    }

    if parts.is_empty() {
        return trimmed.to_string();
    }
    parts.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_model_name_prettifies_known_ids() {
        assert_eq!(display_model_name("claude-fable-5"), "Fable 5");
        assert_eq!(display_model_name("claude-opus-4-5-20251101"), "Opus 4.5");
        assert_eq!(
            display_model_name("claude-3-5-sonnet-20241022"),
            "3.5 Sonnet"
        );
        assert_eq!(display_model_name("gpt-5.6-sol"), "GPT-5.6 Sol");
        assert_eq!(display_model_name("claude-fable-5[1m]"), "Fable 5");
        assert_eq!(display_model_name("mystery"), "Mystery");
        assert_eq!(display_model_name(""), "");
    }

    #[test]
    fn display_label_appends_effort() {
        let info = AgentModelInfo {
            model: "claude-fable-5".into(),
            effort: Some("high".into()),
        };
        assert_eq!(info.display_label(), "Fable 5 high");

        let info = AgentModelInfo {
            model: "claude-fable-5".into(),
            effort: None,
        };
        assert_eq!(info.display_label(), "Fable 5");
    }

    #[test]
    fn claude_tail_takes_last_main_conversation_assistant_entry() {
        let tail = concat!(
            r#"{"type":"assistant","effort":"low","message":{"model":"claude-opus-4-5"}}"#,
            "\n",
            r#"{"type":"assistant","isSidechain":true,"message":{"model":"claude-haiku-4-5"}}"#,
            "\n",
            r#"{"type":"assistant","effort":"high","message":{"model":"claude-fable-5"}}"#,
            "\n",
            r#"{"type":"user","message":{"role":"user"}}"#,
            "\n",
        );

        let info = parse_claude_transcript_tail(tail).unwrap();
        assert_eq!(info.model, "claude-fable-5");
        assert_eq!(info.effort.as_deref(), Some("high"));
    }

    #[test]
    fn claude_tail_skips_synthetic_models_and_partial_lines() {
        let tail = concat!(
            r#"sistant","message":{"model":"claude-truncated"}}"#,
            "\n",
            r#"{"type":"assistant","message":{"model":"claude-fable-5"}}"#,
            "\n",
            r#"{"type":"assistant","message":{"model":"<synthetic>"}}"#,
            "\n",
        );

        let info = parse_claude_transcript_tail(tail).unwrap();
        assert_eq!(info.model, "claude-fable-5");
        assert_eq!(info.effort, None);
    }

    #[test]
    fn claude_tail_without_assistant_entries_is_none() {
        assert_eq!(
            parse_claude_transcript_tail(r#"{"type":"user","message":{}}"#),
            None
        );
    }

    #[test]
    fn codex_tail_takes_last_turn_context() {
        let tail = concat!(
            r#"{"type":"turn_context","payload":{"model":"gpt-5.5","effort":"low"}}"#,
            "\n",
            r#"{"type":"turn_context","payload":{"model":"gpt-5.6-sol","effort":"medium"}}"#,
            "\n",
            r#"{"type":"response_item","payload":{}}"#,
            "\n",
        );

        let info = parse_codex_rollout_tail(tail).unwrap();
        assert_eq!(info.model, "gpt-5.6-sol");
        assert_eq!(info.effort.as_deref(), Some("medium"));
    }

    #[test]
    fn codex_title_is_the_newest_typed_prompt() {
        let tail = concat!(
            r##"{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"# AGENTS.md instructions for /home/user/lab\n\nrules"}]}}"##,
            "\n",
            r#"{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"fix the pane border"}]}}"#,
            "\n",
            r#"{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"reorder\n  the   agent list"}]}}"#,
            "\n",
            r#"{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"<environment_context>\n  <cwd>/home/user/lab</cwd>\n</environment_context>"}]}}"#,
            "\n",
            r#"{"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"on it"}]}}"#,
            "\n",
        );

        assert_eq!(
            parse_codex_rollout_title(tail).as_deref(),
            Some("reorder the agent list"),
            "the newest typed prompt wins, on one line, past the injected context blocks"
        );
    }

    #[test]
    fn codex_title_is_absent_until_the_user_asks_for_something() {
        let tail = concat!(
            r#"{"type":"session_meta","payload":{"id":"abc"}}"#,
            "\n",
            r#"{"type":"turn_context","payload":{"model":"gpt-5.6-sol"}}"#,
            "\n",
        );

        assert_eq!(parse_codex_rollout_title(tail), None);
    }

    #[test]
    fn a_long_prompt_is_cut_to_a_title() {
        let prompt = "word ".repeat(60);
        let title = session_title_from_prompt(&prompt).unwrap();
        assert_eq!(title.chars().count(), SESSION_TITLE_MAX_CHARS);
        assert!(title.ends_with('…'));
    }

    #[test]
    fn claude_sessions_report_their_own_title_so_the_probe_reads_none() {
        let tail = r#"{"type":"user","message":{"role":"user","content":"fix the tests"}}"#;
        // Only the Codex branch of `refresh_job` looks for a title; a Claude
        // transcript is read for its model alone.
        assert_eq!(parse_codex_rollout_title(tail), None);
    }

    #[test]
    fn a_title_scan_reads_only_what_the_session_wrote_since_the_last_one() {
        let root = std::env::temp_dir().join(format!(
            "herdr-agent-model-title-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        let rollout = root.join("rollout.jsonl");
        let prompt = |text: &str| {
            format!(
                r#"{{"type":"response_item","payload":{{"type":"message","role":"user","content":[{{"type":"input_text","text":"{text}"}}]}}}}"#
            )
        };
        let noise = r#"{"type":"response_item","payload":{"type":"reasoning"}}"#;

        fs::write(&rollout, format!("{}\n{noise}\n", prompt("first ask"))).unwrap();
        let (title, scanned) = scan_session_title(&rollout, None, parse_codex_rollout_title);
        assert_eq!(title.as_deref(), Some("first ask"));
        assert_eq!(scanned, fs::metadata(&rollout).unwrap().len());

        // A turn's worth of output and no new prompt: nothing found, and the
        // caller keeps what it already had.
        fs::write(
            &rollout,
            format!("{}\n{noise}\n{noise}\n", prompt("first ask")),
        )
        .unwrap();
        let (title, scanned_again) =
            scan_session_title(&rollout, Some(scanned), parse_codex_rollout_title);
        assert_eq!(title, None);
        assert_eq!(scanned_again, fs::metadata(&rollout).unwrap().len());

        // The next prompt is found in the bytes written after it.
        fs::write(
            &rollout,
            format!(
                "{}\n{noise}\n{noise}\n{}\n",
                prompt("first ask"),
                prompt("second ask")
            ),
        )
        .unwrap();
        let (title, _) =
            scan_session_title(&rollout, Some(scanned_again), parse_codex_rollout_title);
        assert_eq!(title.as_deref(), Some("second ask"));

        // A line still being written is left for the next scan.
        let complete = fs::metadata(&rollout).unwrap().len();
        fs::write(
            &rollout,
            format!(
                "{}\n{noise}\n{noise}\n{}\n{{\"type\":\"resp",
                prompt("first ask"),
                prompt("second ask")
            ),
        )
        .unwrap();
        let (_, scanned_partial) =
            scan_session_title(&rollout, Some(scanned_again), parse_codex_rollout_title);
        assert_eq!(
            scanned_partial, complete,
            "the scan stops at the last newline"
        );

        // A log that shrank is a different log: the scan starts over.
        fs::write(&rollout, format!("{}\n", prompt("a fresh session"))).unwrap();
        let (title, _) = scan_session_title(&rollout, Some(u64::MAX), parse_codex_rollout_title);
        assert_eq!(title.as_deref(), Some("a fresh session"));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn probe_session_ids_that_are_not_path_safe_are_rejected() {
        assert!(valid_probe_session_id(
            "f9b54ddd-8bf9-47d9-81d7-a3b37ee93a93"
        ));
        assert!(!valid_probe_session_id("../../etc/passwd"));
        assert!(!valid_probe_session_id("a/b"));
        assert!(!valid_probe_session_id(""));
    }

    #[test]
    fn claude_session_file_resolves_across_project_dirs() {
        let root = std::env::temp_dir().join(format!(
            "herdr-agent-model-claude-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let project = root.join("projects").join("-home-user-lab");
        fs::create_dir_all(&project).unwrap();
        let session = project.join("abc-123.jsonl");
        fs::write(&session, "{}").unwrap();

        assert_eq!(claude_session_file(&root, "abc-123"), Some(session));
        assert_eq!(claude_session_file(&root, "missing"), None);

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn codex_session_file_resolves_dated_rollouts() {
        let root = std::env::temp_dir().join(format!(
            "herdr-agent-model-codex-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let day = root.join("sessions").join("2026").join("08").join("03");
        fs::create_dir_all(&day).unwrap();
        let session = day.join("rollout-2026-08-03T10-00-00-abc-123.jsonl");
        fs::write(&session, "{}").unwrap();

        assert_eq!(codex_session_file(&root, "abc-123"), Some(session));
        assert_eq!(codex_session_file(&root, "missing"), None);

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn refresh_skips_unchanged_files_and_holds_info_on_empty_tail() {
        let root = std::env::temp_dir().join(format!(
            "herdr-agent-model-refresh-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        let session_file = root.join("abc.jsonl");
        fs::write(&session_file, r#"{"type":"user"}"#).unwrap();
        let modified = fs::metadata(&session_file).unwrap().modified().unwrap();
        let info = AgentModelInfo {
            model: "claude-fable-5".into(),
            effort: Some("high".into()),
        };

        let unchanged = refresh_job(AgentModelRefreshJob {
            terminal_id: TerminalId::alloc(),
            agent: Agent::Claude,
            session_id: "abc".into(),
            cached: Some(AgentModelCacheEntry {
                session_id: "abc".into(),
                session_file: session_file.clone(),
                modified,
                info: Some(info.clone()),
                title: None,
                title_scanned_len: 0,
            }),
        });
        assert!(unchanged.is_none());

        let held = refresh_job(AgentModelRefreshJob {
            terminal_id: TerminalId::alloc(),
            agent: Agent::Claude,
            session_id: "abc".into(),
            cached: Some(AgentModelCacheEntry {
                session_id: "abc".into(),
                session_file: session_file.clone(),
                modified: modified - std::time::Duration::from_secs(60),
                info: Some(info.clone()),
                title: None,
                title_scanned_len: 0,
            }),
        })
        .unwrap();
        assert_eq!(held.entry.info, Some(info));

        let _ = fs::remove_dir_all(root);
    }
}
