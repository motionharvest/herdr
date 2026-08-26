//! Headless Grok summaries for Refresh Summary.
//!
//! When `[ui] refresh_summary_with_grok` is on, the agent-row menu asks a
//! one-shot `grok -p` session to name the latest user request in 5–8 words.
//! The live agent pane is left alone: Grok sessions are resumed with
//! `--fork-session`, and other harnesses pass the latest typed requests as
//! prompt context.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::detect::Agent;
use crate::terminal::TerminalId;

const GROK_SUMMARY_TIMEOUT: Duration = Duration::from_secs(45);
const GROK_SUMMARY_MAX_TURNS: &str = "1";
const GROK_SUMMARY_SCHEMA: &str =
    r#"{"type":"object","properties":{"summary":{"type":"string"}},"required":["summary"]}"#;
const SUMMARY_PROMPT: &str = "In 5-8 words, name the most recent thing the user asked this AI to do. Reply with only that sentence.";
const MAX_SUMMARY_WORDS: usize = 8;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrokSummaryJob {
    pub terminal_id: TerminalId,
    pub session_id: String,
    pub agent: Agent,
    pub cwd: PathBuf,
    pub latest_requests: Vec<String>,
}

impl GrokSummaryJob {
    fn resume_session_id(&self) -> Option<&str> {
        (self.agent == Agent::Grok).then_some(self.session_id.as_str())
    }

    fn has_context(&self) -> bool {
        self.resume_session_id().is_some() || !self.latest_requests.is_empty()
    }
}

pub fn grok_summary_prompt(job: &GrokSummaryJob) -> String {
    if job.latest_requests.is_empty() {
        return SUMMARY_PROMPT.to_string();
    }
    let mut prompt = String::from(SUMMARY_PROMPT);
    prompt.push_str("\n\nMost recent user requests, newest first:\n");
    for (index, request) in job.latest_requests.iter().enumerate() {
        prompt.push_str(&(index + 1).to_string());
        prompt.push_str(". ");
        prompt.push_str(request);
        prompt.push('\n');
    }
    prompt
}

pub fn parse_summary_output(stdout: &str) -> Option<String> {
    let trimmed = strip_code_fence(stdout.trim());
    if trimmed.is_empty() {
        return None;
    }
    // `--json-schema` implies `--output-format json`, so stdout is a grok
    // envelope (`text`, `stopReason`, `sessionId`, …), not the schema object.
    // If that envelope parses, never fall through to the first line: pretty
    // JSON starts with `{`, which used to become the Summary column.
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) {
        return summary_from_json(&value, 0);
    }
    let line = trimmed
        .lines()
        .map(str::trim)
        .find(|line| summary_text_is_usable(line))?;
    clamp_summary(line)
}

fn summary_from_json(value: &serde_json::Value, depth: usize) -> Option<String> {
    if depth > 4 {
        return None;
    }
    match value {
        serde_json::Value::String(text) => {
            let inner = strip_code_fence(text.trim());
            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(inner) {
                if let Some(summary) = summary_from_json(&parsed, depth + 1) {
                    return Some(summary);
                }
            }
            clamp_summary(inner)
        }
        serde_json::Value::Object(map) => {
            for key in ["summary", "text", "structured_output", "result"] {
                if let Some(nested) = map.get(key) {
                    if let Some(summary) = summary_from_json(nested, depth + 1) {
                        return Some(summary);
                    }
                }
            }
            None
        }
        serde_json::Value::Array(items) => items
            .iter()
            .find_map(|item| summary_from_json(item, depth + 1)),
        _ => None,
    }
}

fn summary_text_is_usable(text: &str) -> bool {
    text.chars().any(|c| c.is_alphanumeric())
}

fn strip_code_fence(text: &str) -> &str {
    let trimmed = text.trim();
    let Some(rest) = trimmed.strip_prefix("```") else {
        return trimmed;
    };
    let rest = rest
        .strip_prefix("json")
        .or_else(|| rest.strip_prefix("JSON"))
        .unwrap_or(rest)
        .trim_start_matches('\n');
    rest.strip_suffix("```").unwrap_or(rest).trim()
}

fn clamp_summary(text: &str) -> Option<String> {
    let stripped = text
        .trim()
        .trim_matches(|c| c == '"' || c == '\'' || c == '`')
        .trim()
        .trim_end_matches(['.', '!', '?'])
        .trim();
    if !summary_text_is_usable(stripped) {
        return None;
    }
    let words: Vec<&str> = stripped.split_whitespace().collect();
    if words.is_empty() {
        return None;
    }
    let kept = if words.len() > MAX_SUMMARY_WORDS {
        &words[..MAX_SUMMARY_WORDS]
    } else {
        &words
    };
    Some(kept.join(" "))
}

pub fn run_grok_summary(job: &GrokSummaryJob) -> Result<String, String> {
    run_grok_summary_with(job, Path::new("grok"))
}

pub fn run_grok_summary_with(job: &GrokSummaryJob, grok: &Path) -> Result<String, String> {
    if !job.has_context() {
        return Err("no session context to summarize".into());
    }
    if let Some(session_id) = job.resume_session_id() {
        if let Ok(title) = invoke_grok(job, grok, Some(session_id)) {
            return Ok(title);
        }
    }
    if job.latest_requests.is_empty() {
        return Err("grok -p could not summarize this session".into());
    }
    invoke_grok(job, grok, None)
}

fn invoke_grok(
    job: &GrokSummaryJob,
    grok: &Path,
    resume_session_id: Option<&str>,
) -> Result<String, String> {
    let prompt = grok_summary_prompt(job);
    let mut command = Command::new(grok);
    command
        .arg("--cwd")
        .arg(&job.cwd)
        .arg("--always-approve")
        .arg("--no-subagents")
        .arg("--disable-web-search")
        .arg("--no-plan")
        .arg("--max-turns")
        .arg(GROK_SUMMARY_MAX_TURNS)
        .arg("--verbatim")
        .arg("--json-schema")
        .arg(GROK_SUMMARY_SCHEMA);
    if let Some(session_id) = resume_session_id {
        command
            .arg("--resume")
            .arg(session_id)
            .arg("--fork-session");
    }
    command.arg("-p").arg(&prompt);
    let output = run_command_with_timeout(&mut command, GROK_SUMMARY_TIMEOUT)?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let detail = stderr.lines().find(|line| !line.trim().is_empty());
        return Err(match detail {
            Some(line) => format!("grok -p failed: {}", line.trim()),
            None => format!("grok -p exited {}", output.status),
        });
    }
    parse_summary_output(&String::from_utf8_lossy(&output.stdout))
        .ok_or_else(|| "grok -p returned an empty summary".into())
}

struct CommandOutput {
    status: std::process::ExitStatus,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

fn run_command_with_timeout(
    command: &mut Command,
    timeout: Duration,
) -> Result<CommandOutput, String> {
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .map_err(|err| format!("failed to start grok -p: {err}"))?;
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| "grok -p stdout was not piped".to_string())?;
    let mut stderr = child
        .stderr
        .take()
        .ok_or_else(|| "grok -p stderr was not piped".to_string())?;
    let stdout_handle = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stdout.read_to_end(&mut buf);
        buf
    });
    let stderr_handle = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stderr.read_to_end(&mut buf);
        buf
    });

    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() >= timeout => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("grok -p timed out".into());
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(err) => return Err(format!("failed to wait for grok -p: {err}")),
        }
    };
    let stdout = stdout_handle.join().unwrap_or_default();
    let stderr = stderr_handle.join().unwrap_or_default();
    Ok(CommandOutput {
        status,
        stdout,
        stderr,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::detect::Agent;
    use crate::terminal::TerminalId;

    fn job(requests: &[&str]) -> GrokSummaryJob {
        GrokSummaryJob {
            terminal_id: TerminalId::alloc(),
            session_id: "01a016ad-b38c-7c12-9e2b-32bd13e0cb7c".into(),
            agent: Agent::Grok,
            cwd: PathBuf::from("/tmp"),
            latest_requests: requests
                .iter()
                .map(|request| (*request).to_string())
                .collect(),
        }
    }

    #[test]
    fn prompt_asks_for_a_five_to_eight_word_sentence() {
        let prompt = grok_summary_prompt(&job(&["Land this on parent"]));
        assert!(prompt.contains("5-8 words"));
        assert!(prompt.contains("most recent thing the user asked"));
        assert!(prompt.contains("1. Land this on parent"));
    }

    #[test]
    fn prompt_without_extracted_requests_is_still_the_headline_ask() {
        assert_eq!(grok_summary_prompt(&job(&[])), SUMMARY_PROMPT);
    }

    #[test]
    fn parse_summary_output_takes_json_summary() {
        assert_eq!(
            parse_summary_output(r#"{"summary":"Land the herdr worktree"}"#).as_deref(),
            Some("Land the herdr worktree")
        );
    }

    #[test]
    fn parse_summary_output_reads_grok_json_envelope() {
        let stdout = r#"{
  "text": "{\"summary\":\"Land the herdr worktree\"}",
  "stopReason": "end_turn",
  "sessionId": "abc123",
  "requestId": "xyz789"
}"#;
        assert_eq!(
            parse_summary_output(stdout).as_deref(),
            Some("Land the herdr worktree")
        );
    }

    #[test]
    fn parse_summary_output_reads_plain_text_in_json_envelope() {
        let stdout =
            "{\n  \"text\": \"Land the herdr worktree\",\n  \"stopReason\": \"end_turn\"\n}";
        assert_eq!(
            parse_summary_output(stdout).as_deref(),
            Some("Land the herdr worktree")
        );
    }

    #[test]
    fn parse_summary_output_does_not_use_a_bare_brace() {
        let stdout = "{\n  \"stopReason\": \"end_turn\",\n  \"sessionId\": \"abc123\"\n}";
        assert_eq!(parse_summary_output(stdout), None);
        assert_eq!(parse_summary_output("{"), None);
    }

    #[test]
    fn parse_summary_output_clamps_to_eight_words_and_strips_quotes() {
        assert_eq!(
            parse_summary_output("\"Please land this clean branch onto parent main now thanks\"")
                .as_deref(),
            Some("Please land this clean branch onto parent main")
        );
    }

    #[test]
    fn parse_summary_output_reads_a_fenced_json_block() {
        let stdout = "```json\n{\"summary\":\"Refresh the agent summary\"}\n```\n";
        assert_eq!(
            parse_summary_output(stdout).as_deref(),
            Some("Refresh the agent summary")
        );
    }

    #[test]
    fn grok_summary_uses_the_stub_binary_output() {
        let root = std::env::temp_dir().join(format!(
            "herdr-grok-summary-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let grok = root.join("grok");
        std::fs::write(
            &grok,
            concat!(
                "#!/bin/sh\n",
                "printf '%s\\n' \"$@\" > \"$(dirname \"$0\")/args\"\n",
                "cat <<'EOF'\n",
                "{\n",
                "  \"text\": \"{\\\"summary\\\":\\\"Land the herdr worktree\\\"}\",\n",
                "  \"stopReason\": \"end_turn\",\n",
                "  \"sessionId\": \"abc123\"\n",
                "}\n",
                "EOF\n",
            ),
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut permissions = std::fs::metadata(&grok).unwrap().permissions();
            permissions.set_mode(0o755);
            std::fs::set_permissions(&grok, permissions).unwrap();
        }

        let title = run_grok_summary_with(&job(&["Land this on parent"]), &grok).unwrap();
        assert_eq!(title, "Land the herdr worktree");

        let args = std::fs::read_to_string(root.join("args")).unwrap();
        assert!(args.contains("-p"));
        assert!(args.contains("--fork-session"));
        assert!(args.contains("01a016ad-b38c-7c12-9e2b-32bd13e0cb7c"));
        assert!(args.contains("5-8 words"));
        assert!(args.contains("Land this on parent"));

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn grok_summary_without_context_does_not_start_grok() {
        let job = GrokSummaryJob {
            agent: Agent::Codex,
            latest_requests: Vec::new(),
            ..job(&[])
        };
        assert_eq!(
            run_grok_summary_with(&job, Path::new("/bin/false")).unwrap_err(),
            "no session context to summarize"
        );
    }
}
