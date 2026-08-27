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
//! what a Codex agent is working on rather than staying blank. Grok does name
//! a session, but only after the first prompt, and then freezes that name.
//! The latest typed prompt lives in `chat_history.jsonl` as a `<user_query>`.
//! The column does not take the whole message: it takes the part that asks
//! for work, folded to a short headline. The first fill sticks on the row
//! until a live OSC 0/2 window title arrives; Grok writes that title as it
//! works, and Summary follows it. Refresh Summary reads the latest prompt on
//! command, the same way the automatic update used to. With
//! `[ui] refresh_summary_with_grok`, that command instead asks a headless
//! `grok -p` session for a 5–8 word sentence naming the latest user request.
//! `generated_title` in `summary.json` still stands in until a prompt has
//! been parsed, and the same file still carries `current_model_id` and
//! `reasoning_effort`.

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
    matches!(agent, Agent::Claude | Agent::Codex | Agent::Grok)
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

    let session_file = resolve_job_session_file(&job)?;
    let modified = if job.agent == Agent::Grok {
        grok_sources_mtime(&session_file)?
    } else {
        fs::metadata(&session_file).ok()?.modified().ok()?
    };
    if job
        .cached
        .as_ref()
        .is_some_and(|cached| cached.session_file == session_file && cached.modified == modified)
    {
        return None;
    }

    if job.agent == Agent::Grok {
        return refresh_grok_job(job, session_file, modified);
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

fn resolve_job_session_file(job: &AgentModelRefreshJob) -> Option<PathBuf> {
    if job.agent == Agent::Grok {
        if let Some(cached) = job.cached.as_ref() {
            if let Some(preferred) = grok_preferred_file(&cached.session_file) {
                return Some(preferred);
            }
        }
        return resolve_session_file(job.agent, &job.session_id);
    }
    job.cached
        .as_ref()
        .map(|cached| cached.session_file.clone())
        .filter(|file| file.is_file())
        .or_else(|| resolve_session_file(job.agent, &job.session_id))
}

fn refresh_grok_job(
    job: AgentModelRefreshJob,
    session_file: PathBuf,
    modified: SystemTime,
) -> Option<AgentModelRefreshResult> {
    let (summary_path, history_path) = grok_session_paths(&session_file);
    let summary_text = fs::read_to_string(&summary_path).ok();
    let info = summary_text
        .as_deref()
        .and_then(parse_grok_summary_tail)
        .or_else(|| job.cached.as_ref().and_then(|cached| cached.info.clone()));

    let scanned_from = job
        .cached
        .as_ref()
        .filter(|cached| cached.session_file == history_path)
        .map(|cached| cached.title_scanned_len);
    let (title, title_scanned_len) = if history_path.is_file() {
        scan_session_title(&history_path, scanned_from, parse_grok_chat_title)
    } else {
        (None, 0)
    };
    let title = title
        .or_else(|| job.cached.as_ref().and_then(|cached| cached.title.clone()))
        .or_else(|| summary_text.as_deref().and_then(parse_grok_summary_title));

    Some(AgentModelRefreshResult {
        terminal_id: job.terminal_id,
        entry: AgentModelCacheEntry {
            session_id: job.session_id,
            session_file: if history_path.is_file() {
                history_path
            } else {
                summary_path
            },
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
        Agent::Grok => grok_session_file(&crate::integration::grok_dir().ok()?, session_id),
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

/// Grok sessions live at `<home>/sessions/<encoded-cwd>/<session-id>/`.
/// Prefer `chat_history.jsonl` (the growing prompt log) and fall back to
/// `summary.json` for a session that has been named but has no history yet.
fn grok_session_file(grok_home: &Path, session_id: &str) -> Option<PathBuf> {
    for entry in fs::read_dir(grok_home.join("sessions")).ok()?.flatten() {
        let dir = entry.path().join(session_id);
        if let Some(preferred) = grok_preferred_file(&dir.join("summary.json")) {
            return Some(preferred);
        }
    }
    None
}

fn grok_session_paths(session_file: &Path) -> (PathBuf, PathBuf) {
    let dir = grok_session_dir(session_file);
    (dir.join("summary.json"), dir.join("chat_history.jsonl"))
}

fn grok_session_dir(session_file: &Path) -> &Path {
    session_file.parent().unwrap_or(session_file)
}

fn grok_preferred_file(cached_file: &Path) -> Option<PathBuf> {
    let dir = grok_session_dir(cached_file);
    let history = dir.join("chat_history.jsonl");
    if history.is_file() {
        return Some(history);
    }
    let summary = dir.join("summary.json");
    if summary.is_file() {
        return Some(summary);
    }
    cached_file.is_file().then(|| cached_file.to_path_buf())
}

fn grok_sources_mtime(session_file: &Path) -> Option<SystemTime> {
    let (summary, history) = grok_session_paths(session_file);
    let mut latest: Option<SystemTime> = None;
    for path in [&summary, &history] {
        if let Ok(modified) = fs::metadata(path).and_then(|metadata| metadata.modified()) {
            latest = Some(match latest {
                Some(previous) => previous.max(modified),
                None => modified,
            });
        }
    }
    latest.or_else(|| fs::metadata(session_file).ok()?.modified().ok())
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
        if let Some(title) = action_title_from_prompt(text) {
            return Some(title);
        }
    }
    None
}

/// The title Grok generated for the session, taken from `summary.json`.
///
/// Grok writes that name after the first prompt and regenerates it a couple of
/// times before freezing it. It is the fallback for a session whose chat
/// history has no typed prompt yet. A missing or blank `generated_title` is a
/// session that has not been named yet, so the column stays empty until a
/// prompt or a generated title appears.
fn parse_grok_summary_title(content: &str) -> Option<String> {
    let entry = serde_json::from_str::<serde_json::Value>(content).ok()?;
    session_title_from_prompt(
        entry
            .get("generated_title")
            .and_then(|title| title.as_str())?,
    )
}

/// The last thing the user actually asked for, taken from the newest `user`
/// record in `chat_history.jsonl`.
///
/// Grok writes more than typed prompts into that role: `<user_info>`,
/// `<system-reminder>`, and other injected blocks, often marked
/// `synthetic_reason`. Those are the harness talking to its own model, not a
/// task. A record that carries `synthetic_reason` is skipped. A record whose
/// text contains a `<user_query>` uses that inner text. Anything else is the
/// same filter Codex uses: a message that opens a tag is not a title.
fn parse_grok_chat_title(tail: &str) -> Option<String> {
    for line in tail.lines().rev() {
        let Ok(entry) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if entry.get("type").and_then(|entry_type| entry_type.as_str()) != Some("user") {
            continue;
        }
        if entry.get("synthetic_reason").is_some() {
            continue;
        }
        let Some(text) = grok_user_text(&entry) else {
            continue;
        };
        if let Some(title) = grok_title_from_user_text(&text) {
            return Some(title);
        }
    }
    None
}

fn grok_user_text(entry: &serde_json::Value) -> Option<String> {
    let content = entry.get("content")?;
    if let Some(text) = content.as_str() {
        return Some(text.to_string());
    }
    let mut text = String::new();
    for part in content.as_array()? {
        if part.get("type").and_then(|kind| kind.as_str()) != Some("text") {
            continue;
        }
        let Some(piece) = part.get("text").and_then(|text| text.as_str()) else {
            continue;
        };
        if piece.trim().is_empty() {
            continue;
        }
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(piece);
    }
    (!text.trim().is_empty()).then_some(text)
}

fn grok_title_from_user_text(text: &str) -> Option<String> {
    action_title_from_prompt(&raw_user_request_text(text)?)
}

/// Inner `<user_query>` text, or the message itself when it is not wrapped in
/// a harness tag. Injected blocks that open a tag are skipped.
fn raw_user_request_text(text: &str) -> Option<String> {
    const OPEN: &str = "<user_query>";
    const CLOSE: &str = "</user_query>";
    let inner = if let Some(start) = text.find(OPEN) {
        let rest = text.get(start + OPEN.len()..)?;
        match rest.find(CLOSE) {
            Some(end) => rest.get(..end)?,
            None => rest,
        }
    } else {
        text
    };
    let trimmed = inner.trim();
    if trimmed.is_empty()
        || trimmed.starts_with('<')
        || trimmed.starts_with("# AGENTS.md instructions")
    {
        return None;
    }
    Some(trimmed.to_string())
}

const LATEST_USER_REQUEST_CHARS: usize = 1500;

/// Newest typed user requests from the session log, newest first.
pub(crate) fn latest_user_requests(agent: Agent, session_id: &str, limit: usize) -> Vec<String> {
    if limit == 0 || !valid_probe_session_id(session_id) {
        return Vec::new();
    }
    let Some(session_file) = resolve_session_file(agent, session_id) else {
        return Vec::new();
    };
    match agent {
        Agent::Grok => grok_latest_user_requests(&session_file, limit),
        Agent::Codex => collect_latest_from_file(&session_file, limit, parse_codex_user_request),
        Agent::Claude => collect_latest_from_file(&session_file, limit, parse_claude_user_request),
        _ => Vec::new(),
    }
}

fn grok_latest_user_requests(session_file: &Path, limit: usize) -> Vec<String> {
    let (_, history_path) = grok_session_paths(session_file);
    if !history_path.is_file() {
        return Vec::new();
    }
    collect_latest_from_file(&history_path, limit, parse_grok_user_request)
}

fn collect_latest_from_file(
    path: &Path,
    limit: usize,
    parse_line: fn(&str) -> Option<String>,
) -> Vec<String> {
    let Ok(tail) = read_tail(path, TITLE_FIRST_SCAN_BYTES) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for line in tail.lines().rev() {
        let Some(text) = parse_line(line) else {
            continue;
        };
        found.push(truncate_user_request(&text));
        if found.len() >= limit {
            break;
        }
    }
    found
}

fn truncate_user_request(text: &str) -> String {
    let condensed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if condensed.chars().count() <= LATEST_USER_REQUEST_CHARS {
        condensed
    } else {
        condensed.chars().take(LATEST_USER_REQUEST_CHARS).collect()
    }
}

fn parse_grok_user_request(line: &str) -> Option<String> {
    let entry = serde_json::from_str::<serde_json::Value>(line).ok()?;
    if entry.get("type").and_then(|entry_type| entry_type.as_str()) != Some("user") {
        return None;
    }
    if entry.get("synthetic_reason").is_some() {
        return None;
    }
    raw_user_request_text(&grok_user_text(&entry)?)
}

fn parse_codex_user_request(line: &str) -> Option<String> {
    let entry = serde_json::from_str::<serde_json::Value>(line).ok()?;
    if entry.get("type").and_then(|entry_type| entry_type.as_str()) != Some("response_item") {
        return None;
    }
    let payload = entry.get("payload")?;
    if payload.get("type").and_then(|kind| kind.as_str()) != Some("message")
        || payload.get("role").and_then(|role| role.as_str()) != Some("user")
    {
        return None;
    }
    let text = payload
        .get("content")
        .and_then(|content| content.as_array())
        .into_iter()
        .flatten()
        .filter_map(|part| part.get("text").and_then(|text| text.as_str()))
        .find(|text| !text.trim().is_empty())?;
    raw_user_request_text(text)
}

fn parse_claude_user_request(line: &str) -> Option<String> {
    let entry = serde_json::from_str::<serde_json::Value>(line).ok()?;
    if entry.get("type").and_then(|entry_type| entry_type.as_str()) != Some("user") {
        return None;
    }
    if entry
        .get("isSidechain")
        .and_then(|sidechain| sidechain.as_bool())
        == Some(true)
    {
        return None;
    }
    let content = entry.get("message")?.get("content")?;
    let text = if let Some(text) = content.as_str() {
        text.to_string()
    } else {
        let mut text = String::new();
        for part in content.as_array()? {
            if part.get("type").and_then(|kind| kind.as_str()) != Some("text") {
                continue;
            }
            let Some(piece) = part.get("text").and_then(|text| text.as_str()) else {
                continue;
            };
            if piece.trim().is_empty() {
                continue;
            }
            if !text.is_empty() {
                text.push('\n');
            }
            text.push_str(piece);
        }
        if text.trim().is_empty() {
            return None;
        }
        text
    };
    raw_user_request_text(&text)
}

/// Verbs that, as the first word, mean the sentence is asking for work.
const TASK_VERBS: &[&str] = &[
    "add",
    "allow",
    "build",
    "change",
    "close",
    "commit",
    "drop",
    "edit",
    "enable",
    "fill",
    "fix",
    "follow",
    "give",
    "hide",
    "highlight",
    "improve",
    "keep",
    "land",
    "make",
    "move",
    "open",
    "paint",
    "paste",
    "point",
    "put",
    "read",
    "remove",
    "rename",
    "replace",
    "restore",
    "revert",
    "run",
    "scroll",
    "set",
    "show",
    "split",
    "start",
    "stop",
    "summarize",
    "switch",
    "update",
    "use",
    "wire",
    "write",
];

/// Hedging that leads a request without being the request.
const HEDGE_PREFIXES: &[&str] = &[
    "i kinda meant that ",
    "i kind of meant that ",
    "i kinda meant ",
    "i meant that ",
    "i think that ",
    "i think ",
    "i want you to ",
    "i want it to ",
    "i need you to ",
    "i would like you to ",
    "i would like ",
    "i want ",
    "i need ",
    "can you please ",
    "could you please ",
    "can you ",
    "could you ",
    "would you ",
    "please ",
    "okay so ",
    "ok so ",
    "okay, ",
    "ok, ",
    "oof. ",
    "oof, ",
    "oof ",
    "yeah, ",
    "yeah ",
    "well, ",
    "so, ",
];

/// A short headline for the work a prompt is asking for, not the whole
/// message. A quoted imperative is preferred when the user named the task.
/// Otherwise the sentence that most looks like a request is kept, hedging is
/// stripped, and the line is cut to what a table cell can hold.
fn action_title_from_prompt(text: &str) -> Option<String> {
    let trimmed = text.trim_start();
    if trimmed.starts_with('<') || trimmed.starts_with("# AGENTS.md instructions") {
        return None;
    }
    let condensed = trimmed.split_whitespace().collect::<Vec<_>>().join(" ");
    if condensed.is_empty() {
        return None;
    }

    if let Some(quoted) = quoted_action_headline(&condensed) {
        return finish_action_title(quoted);
    }

    let sentences = split_sentences(&condensed);
    let chosen = sentences
        .iter()
        .copied()
        .max_by_key(|sentence| action_score(sentence))
        .filter(|sentence| action_score(sentence) > 0)
        .or_else(|| {
            sentences
                .iter()
                .copied()
                .find(|sentence| !sentence.is_empty())
        })
        .unwrap_or(condensed.as_str());
    let stripped = strip_hedges(chosen);
    let source = if stripped.len() == chosen.trim().len() {
        stripped
    } else {
        capitalize_first(&stripped)
    };
    finish_action_title(&source)
}

fn capitalize_first(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

fn quoted_action_headline(text: &str) -> Option<&str> {
    for span in quoted_spans(text) {
        let inner = span.trim();
        let words = inner.split_whitespace().count();
        if !(3..=ACTION_TITLE_MAX_WORDS).contains(&words) {
            continue;
        }
        if first_word_is_task_verb(inner) {
            return Some(inner);
        }
    }
    None
}

fn quoted_spans(text: &str) -> Vec<&str> {
    let mut spans = Vec::new();
    let mut rest = text;
    while let Some(open_at) = rest.find(['"', '\u{201c}']) {
        let open = rest[open_at..]
            .chars()
            .next()
            .expect("find returned a char");
        let close = if open == '\u{201c}' { '\u{201d}' } else { '"' };
        let after_open = open_at + open.len_utf8();
        let inner = &rest[after_open..];
        let Some(close_at) = inner.find(close) else {
            break;
        };
        spans.push(&inner[..close_at]);
        rest = &inner[close_at + close.len_utf8()..];
    }
    spans
}

fn split_sentences(text: &str) -> Vec<&str> {
    let mut sentences = Vec::new();
    let mut start = 0;
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let is_end = matches!(bytes[i], b'.' | b'?' | b'!');
        let next_is_break = i + 1 == bytes.len()
            || bytes
                .get(i + 1)
                .is_some_and(|next| next.is_ascii_whitespace());
        if is_end && next_is_break {
            let piece = text[start..i].trim();
            if !piece.is_empty() {
                sentences.push(piece);
            }
            i += 1;
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            start = i;
            continue;
        }
        i += 1;
    }
    let piece = text[start..].trim();
    if !piece.is_empty() {
        sentences.push(piece);
    }
    if sentences.is_empty() {
        sentences.push(text.trim());
    }
    sentences
}

fn action_score(sentence: &str) -> i32 {
    let trimmed = sentence.trim();
    if trimmed.is_empty() {
        return -100;
    }
    let lower = trimmed.to_ascii_lowercase();
    if matches!(
        lower.trim_end_matches(['.', '?', '!']),
        "oof"
            | "yeah"
            | "ok"
            | "okay"
            | "you know"
            | "something like that"
            | "thanks"
            | "thank you"
    ) {
        return -50;
    }

    let mut score = 0;
    let words: Vec<&str> = trimmed.split_whitespace().collect();
    if first_word_is_task_verb(trimmed) {
        score += 6;
    }
    if lower.starts_with("can you")
        || lower.starts_with("could you")
        || lower.starts_with("can the")
        || lower.starts_with("could the")
        || lower.starts_with("can it")
    {
        score += 5;
    }
    if lower.starts_with("i want")
        || lower.starts_with("i need")
        || lower.starts_with("i kinda")
        || lower.starts_with("please")
        || lower.starts_with("this one might be")
    {
        score += 5;
    }
    if lower.contains(" should ") || lower.starts_with("should ") {
        score += 4;
    }
    if lower.contains(" make it ") || lower.contains("can it") {
        score += 2;
    }
    if lower.starts_with("that doesn't")
        || lower.starts_with("that does not")
        || lower.starts_with("it currently")
    {
        score -= 3;
    }
    let n = words.len();
    if (4..=16).contains(&n) {
        score += 2;
    }
    if n < 3 {
        score -= 2;
    }
    score
}

fn first_word_is_task_verb(text: &str) -> bool {
    let Some(word) = text.split_whitespace().next() else {
        return false;
    };
    let cleaned: String = word
        .chars()
        .filter(|ch| ch.is_ascii_alphabetic())
        .map(|ch| ch.to_ascii_lowercase())
        .collect();
    TASK_VERBS.contains(&cleaned.as_str())
}

fn strip_hedges(text: &str) -> String {
    let mut rest = text.trim().to_string();
    loop {
        let lower = rest.to_ascii_lowercase();
        let Some(prefix) = HEDGE_PREFIXES
            .iter()
            .find(|prefix| lower.starts_with(*prefix))
        else {
            break;
        };
        rest = rest.chars().skip(prefix.chars().count()).collect();
        rest = rest
            .trim_start_matches(|ch: char| ch == ',' || ch == '.' || ch.is_whitespace())
            .to_string();
    }
    rest
}

fn finish_action_title(text: &str) -> Option<String> {
    let trimmed = text.trim().trim_end_matches(['.', '?', '!', ',', ';']);
    if trimmed.is_empty() {
        return None;
    }
    let mut words: Vec<&str> = trimmed.split_whitespace().collect();
    if words.is_empty() {
        return None;
    }
    if words.len() > ACTION_TITLE_MAX_WORDS {
        words.truncate(ACTION_TITLE_MAX_WORDS);
    }
    let mut title = words.join(" ");
    if title.chars().count() > ACTION_TITLE_MAX_CHARS {
        let mut kept = String::new();
        for word in title.split_whitespace() {
            let next_len =
                kept.chars().count() + word.chars().count() + usize::from(!kept.is_empty());
            if next_len > ACTION_TITLE_MAX_CHARS {
                break;
            }
            if !kept.is_empty() {
                kept.push(' ');
            }
            kept.push_str(word);
        }
        if kept.is_empty() {
            return None;
        }
        title = kept;
    }
    Some(title)
}

const ACTION_TITLE_MAX_CHARS: usize = 72;
const ACTION_TITLE_MAX_WORDS: usize = 12;

/// `current_model_id` plus `reasoning_effort` from the same summary file.
fn parse_grok_summary_tail(content: &str) -> Option<AgentModelInfo> {
    let entry = serde_json::from_str::<serde_json::Value>(content).ok()?;
    let model = entry
        .get("current_model_id")
        .and_then(|model| model.as_str())?;
    if model.is_empty() {
        return None;
    }
    Some(AgentModelInfo {
        model: model.to_string(),
        effort: entry
            .get("reasoning_effort")
            .and_then(|effort| effort.as_str())
            .filter(|effort| !effort.is_empty())
            .map(str::to_string),
    })
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
        // A Claude transcript is read for its model alone. Codex and Grok are
        // the branches of `refresh_job` that look for a title.
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
        assert!(valid_probe_session_id(
            "01a016ad-b38c-7c12-9e2b-32bd13e0cb7c"
        ));
        assert!(!valid_probe_session_id("../../etc/passwd"));
        assert!(!valid_probe_session_id("a/b"));
        assert!(!valid_probe_session_id(""));
    }

    #[test]
    fn grok_sessions_are_probe_supported() {
        assert!(probe_supported(Agent::Grok));
    }

    #[test]
    fn grok_refresh_reads_generated_title_and_model_from_summary() {
        let root = std::env::temp_dir().join(format!(
            "herdr-agent-model-grok-refresh-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        let summary = root.join("summary.json");
        fs::write(
            &summary,
            r#"{
              "generated_title": "Grok agents missing session summary status",
              "session_summary": "Grok agents missing session summary status",
              "current_model_id": "grok-4.6",
              "reasoning_effort": "high"
            }"#,
        )
        .unwrap();
        let modified = fs::metadata(&summary).unwrap().modified().unwrap();

        let refreshed = refresh_job(AgentModelRefreshJob {
            terminal_id: TerminalId::alloc(),
            agent: Agent::Grok,
            session_id: "01a016ad-b38c-7c12-9e2b-32bd13e0cb7c".into(),
            cached: Some(AgentModelCacheEntry {
                session_id: "01a016ad-b38c-7c12-9e2b-32bd13e0cb7c".into(),
                session_file: summary,
                modified: modified - std::time::Duration::from_secs(60),
                info: None,
                title: None,
                title_scanned_len: 0,
            }),
        })
        .unwrap();

        assert_eq!(
            refreshed.entry.title.as_deref(),
            Some("Grok agents missing session summary status")
        );
        assert_eq!(
            refreshed.entry.info,
            Some(AgentModelInfo {
                model: "grok-4.6".into(),
                effort: Some("high".into()),
            })
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn grok_title_is_absent_until_the_session_is_named() {
        assert_eq!(
            parse_grok_summary_title(r#"{"current_model_id":"grok-4.6"}"#),
            None
        );
        assert_eq!(
            parse_grok_summary_title(r#"{"generated_title":"","session_summary":""}"#),
            None
        );
    }

    fn grok_user_line(text: &str) -> String {
        format!(
            r#"{{"type":"user","content":[{{"type":"text","text":{}}}]}}"#,
            serde_json::to_string(text).unwrap()
        )
    }

    #[test]
    fn grok_chat_title_is_the_latest_typed_prompt() {
        let tail = [
            grok_user_line("<user_info>\nOS Version: linux\n</user_info>"),
            grok_user_line("<user_query>\nfix the pane border\n</user_query>"),
            r#"{"type":"user","synthetic_reason":"system_reminder","content":[{"type":"text","text":"<system-reminder>\nskills\n</system-reminder>"}]}"#.to_string(),
            grok_user_line("<user_query>\nreorder\n  the   agent list\n</user_query>"),
            r#"{"type":"assistant","content":[{"type":"text","text":"on it"}]}"#.to_string(),
        ]
        .join("\n");

        assert_eq!(
            parse_grok_chat_title(&tail).as_deref(),
            Some("reorder the agent list"),
            "the newest typed prompt wins, on one line, past injected context"
        );
    }

    #[test]
    fn grok_latest_user_requests_are_newest_first_and_skip_injected_blocks() {
        let tail = [
            grok_user_line("<user_info>\nOS Version: linux\n</user_info>"),
            grok_user_line("<user_query>\nImprove Agent Summary to be useful\n</user_query>"),
            r#"{"type":"user","synthetic_reason":"system_reminder","content":[{"type":"text","text":"<system-reminder>\nskills\n</system-reminder>"}]}"#.to_string(),
            grok_user_line("<user_query>\nLand this on parent\n</user_query>"),
        ]
        .join("\n");
        let requests = collect_latest_from_file_for_test(&tail, 3, parse_grok_user_request);
        assert_eq!(
            requests,
            vec![
                "Land this on parent".to_string(),
                "Improve Agent Summary to be useful".to_string(),
            ]
        );
    }

    fn collect_latest_from_file_for_test(
        tail: &str,
        limit: usize,
        parse_line: fn(&str) -> Option<String>,
    ) -> Vec<String> {
        let mut found = Vec::new();
        for line in tail.lines().rev() {
            let Some(text) = parse_line(line) else {
                continue;
            };
            found.push(truncate_user_request(&text));
            if found.len() >= limit {
                break;
            }
        }
        found
    }

    #[test]
    fn action_title_uses_a_quoted_imperative_headline() {
        let prompt = concat!(
            "Oof. That doesn't help tell me what the agent did. ",
            "I kinda meant that I want it to summarize the part of the prompt that encites action. ",
            r#"this one might be "Improve Agent Summary to be useful". You know? something like that."#
        );
        assert_eq!(
            action_title_from_prompt(prompt).as_deref(),
            Some("Improve Agent Summary to be useful")
        );
    }

    #[test]
    fn action_title_picks_the_ask_out_of_a_rambling_prompt() {
        let prompt = concat!(
            "Can the Summary update based on what the user has sent next. ",
            "It currently is set when the first message comes back. ",
            "but can it also be evaluated to contain a summary of what the user submitted as a prompt?"
        );
        assert_eq!(
            action_title_from_prompt(prompt).as_deref(),
            Some("Can the Summary update based on what the user has sent next")
        );
    }

    #[test]
    fn action_title_prefers_the_command_over_the_setup() {
        assert_eq!(
            action_title_from_prompt("Can you see the Test Section. Change it to green.")
                .as_deref(),
            Some("Change it to green")
        );
    }

    #[test]
    fn action_title_strips_hedging_from_a_request() {
        assert_eq!(
            action_title_from_prompt("I kinda meant that I want it to summarize the action.")
                .as_deref(),
            Some("Summarize the action")
        );
    }

    #[test]
    fn grok_chat_title_is_absent_until_the_user_asks_for_something() {
        let tail = [
            grok_user_line("<user_info>\nOS Version: linux\n</user_info>"),
            r#"{"type":"user","synthetic_reason":"compaction_meta","content":[{"type":"text","text":"continued from a previous conversation"}]}"#.to_string(),
            r#"{"type":"assistant","content":[{"type":"text","text":"ready"}]}"#.to_string(),
        ]
        .join("\n");

        assert_eq!(parse_grok_chat_title(&tail), None);
    }

    #[test]
    fn grok_refresh_prefers_the_latest_user_query_over_generated_title() {
        let root = std::env::temp_dir().join(format!(
            "herdr-agent-model-grok-prompt-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        let summary = root.join("summary.json");
        let history = root.join("chat_history.jsonl");
        fs::write(
            &summary,
            r#"{
              "generated_title": "Update Summary from Subsequent User Prompts",
              "current_model_id": "grok-4.6",
              "reasoning_effort": "high"
            }"#,
        )
        .unwrap();
        fs::write(
            &history,
            format!(
                "{}\n{}\n",
                grok_user_line("<user_query>\nCan the Summary update based on the first prompt?\n</user_query>"),
                grok_user_line("<user_query>\nCan the Summary update based on what the user has sent next?\n</user_query>")
            ),
        )
        .unwrap();
        let modified = fs::metadata(&summary).unwrap().modified().unwrap();

        let refreshed = refresh_job(AgentModelRefreshJob {
            terminal_id: TerminalId::alloc(),
            agent: Agent::Grok,
            session_id: "01a016ad-b38c-7c12-9e2b-32bd13e0cb7c".into(),
            cached: Some(AgentModelCacheEntry {
                session_id: "01a016ad-b38c-7c12-9e2b-32bd13e0cb7c".into(),
                session_file: summary,
                modified: modified - std::time::Duration::from_secs(60),
                info: None,
                title: Some("Update Summary from Subsequent User Prompts".into()),
                title_scanned_len: 0,
            }),
        })
        .unwrap();

        assert_eq!(
            refreshed.entry.title.as_deref(),
            Some("Can the Summary update based on what the user has sent next")
        );
        assert_eq!(refreshed.entry.session_file, history);
        assert_eq!(
            refreshed.entry.info,
            Some(AgentModelInfo {
                model: "grok-4.6".into(),
                effort: Some("high".into()),
            })
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn grok_refresh_keeps_a_prompt_title_when_only_the_generated_name_changes() {
        let root = std::env::temp_dir().join(format!(
            "herdr-agent-model-grok-keep-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        let summary = root.join("summary.json");
        let history = root.join("chat_history.jsonl");
        let first = format!(
            "{}\n",
            grok_user_line("<user_query>\nfix the pane border\n</user_query>")
        );
        fs::write(&history, &first).unwrap();
        fs::write(
            &summary,
            r#"{"generated_title":"Fix The Pane Border","current_model_id":"grok-4.6"}"#,
        )
        .unwrap();

        let stale = fs::metadata(&history).unwrap().modified().unwrap()
            - std::time::Duration::from_secs(60);
        let first_pass = refresh_job(AgentModelRefreshJob {
            terminal_id: TerminalId::alloc(),
            agent: Agent::Grok,
            session_id: "01a016ad-b38c-7c12-9e2b-32bd13e0cb7c".into(),
            cached: Some(AgentModelCacheEntry {
                session_id: "01a016ad-b38c-7c12-9e2b-32bd13e0cb7c".into(),
                session_file: summary.clone(),
                modified: stale,
                info: None,
                title: None,
                title_scanned_len: 0,
            }),
        })
        .unwrap();
        assert_eq!(
            first_pass.entry.title.as_deref(),
            Some("fix the pane border")
        );

        fs::write(
            &summary,
            r#"{"generated_title":"A Frozen Generated Title","current_model_id":"grok-4.6"}"#,
        )
        .unwrap();
        let mut cached = first_pass.entry.clone();
        cached.modified -= std::time::Duration::from_secs(60);
        let second_pass = refresh_job(AgentModelRefreshJob {
            terminal_id: TerminalId::alloc(),
            agent: Agent::Grok,
            session_id: "01a016ad-b38c-7c12-9e2b-32bd13e0cb7c".into(),
            cached: Some(cached),
        })
        .unwrap();
        assert_eq!(
            second_pass.entry.title.as_deref(),
            Some("fix the pane border"),
            "a later generated_title must not replace a prompt already on the row"
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn grok_session_file_resolves_cwd_grouped_summaries() {
        let root = std::env::temp_dir().join(format!(
            "herdr-agent-model-grok-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let session = root
            .join("sessions")
            .join("%2Fhome%2Faaron%2Flab%2Fherdr")
            .join("01a016ad-b38c-7c12-9e2b-32bd13e0cb7c");
        fs::create_dir_all(&session).unwrap();
        let summary = session.join("summary.json");
        fs::write(&summary, "{}").unwrap();

        assert_eq!(
            grok_session_file(&root, "01a016ad-b38c-7c12-9e2b-32bd13e0cb7c"),
            Some(summary.clone())
        );

        let history = session.join("chat_history.jsonl");
        fs::write(&history, "{}\n").unwrap();
        assert_eq!(
            grok_session_file(&root, "01a016ad-b38c-7c12-9e2b-32bd13e0cb7c"),
            Some(history),
            "the prompt log is preferred once it exists"
        );
        assert_eq!(grok_session_file(&root, "missing"), None);

        let _ = fs::remove_dir_all(root);
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
