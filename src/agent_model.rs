//! Model and reasoning-effort detection for agent sessions.
//!
//! Agents that report a session id through the herdr integration also write a
//! session log on disk (Claude Code transcripts, Codex rollouts). The log
//! carries the model and reasoning effort actually used for each turn, so it
//! stays correct across mid-session model switches. A background refresh
//! resolves each terminal's session file, tail-reads it when its mtime
//! changes, and reports the latest model info back to the main loop.

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
    .or_else(|| job.cached.and_then(|cached| cached.info));

    Some(AgentModelRefreshResult {
        terminal_id: job.terminal_id,
        entry: AgentModelCacheEntry {
            session_id: job.session_id,
            session_file,
            modified,
            info,
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
            }),
        })
        .unwrap();
        assert_eq!(held.entry.info, Some(info));

        let _ = fs::remove_dir_all(root);
    }
}
