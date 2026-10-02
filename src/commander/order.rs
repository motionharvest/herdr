//! Working out which commands of a paragraph must wait for which, by asking
//! Claude Code running Opus.
//!
//! Jev cuts a paragraph into commands and reads each one, but whether one
//! command needs another to have happened first is a question about the
//! commands together: `prompt it` needs the agent `add a Claude agent`
//! started, while `tell the Grok agent to check the PR` needs nothing before
//! it. That takes reasoning across the whole paragraph, so it goes to a
//! one-shot `claude -p` with no tools, no hooks and no saved session, which
//! answers with the earlier commands each command waits for.
//!
//! The answer can only point backwards: a command may wait only for commands
//! written before it. That keeps the paragraph's own order as the tiebreak and
//! makes a cycle impossible. When the planner cannot be run, every command
//! waits for the one before it, which is always safe.

use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

/// A plan that has not come back by then is not waited for.
const TIMEOUT: Duration = Duration::from_secs(60);

const SCHEMA: &str = r#"{"type":"object","properties":{"after":{"type":"array","items":{"type":"array","items":{"type":"integer"}}}},"required":["after"]}"#;

/// For each command, the earlier commands it waits for.
pub type After = Vec<Vec<usize>>;

/// Every command waiting for the one before it: the paragraph's order, run
/// one at a time.
pub fn in_turn(count: usize) -> After {
    (0..count)
        .map(|i| if i == 0 { Vec::new() } else { vec![i - 1] })
        .collect()
}

/// The prompt the planner is given.
pub fn prompt(commands: &[String], screen: &str) -> String {
    let numbered: Vec<String> = commands
        .iter()
        .enumerate()
        .map(|(i, c)| format!("{i}. {c}"))
        .collect();
    format!(
        "You schedule commands for herdr, a terminal workspace manager. It holds spaces (projects), each with tabs, each with panes; many panes run coding agents such as Claude Code, Codex or Grok. The user typed one paragraph, which has been cut into these numbered commands:\n\n{}\n\nWhat herdr has now:\n{screen}\n\nThe commands will run at the same time wherever that is safe. For each command, list the numbers of the EARLIER commands that must finish before it starts. A command waits for an earlier one when:\n- it uses something the earlier one makes: an agent, pane, tab, space or worktree;\n- it refers back to it, with words such as it, them, there, that agent;\n- it acts on what is on screen or on the focused pane (split, zoom, new tab, focus, or a message with no named recipient) and an earlier command changes what is on screen or which pane is focused; commands that change or rely on what is on screen keep their order among themselves;\n- both send to the same pane or agent, so the messages arrive in the order written;\n- the earlier one closes, renames or removes something it touches.\nOtherwise it waits for nothing. In particular, a message to a pane or agent named by its name or its agent is delivered wherever that pane is, so it does not wait for switching, focusing or anything else on screen; it waits only for a command that makes that pane or agent, or for an earlier message to the same one. Only list numbers smaller than the command's own. Answer with `after`: one list per command, in order.",
        numbered.join("\n")
    )
}

/// Ask `claude` for the plan.
pub fn plan(claude: &str, model: &str, commands: &[String], screen: &str) -> Result<After, String> {
    let mut command = Command::new(claude);
    command
        .arg("-p")
        .arg(prompt(commands, screen))
        .arg("--model")
        .arg(model)
        .arg("--output-format")
        .arg("json")
        .arg("--json-schema")
        .arg(SCHEMA)
        // Nothing to look at and nothing to run: the answer is reasoning only.
        .arg("--tools")
        .arg("")
        // No user or project settings, so no hooks: herdr's own integration
        // must not mistake this helper for an agent in a pane.
        .arg("--setting-sources")
        .arg("")
        .arg("--strict-mcp-config")
        .arg("--no-session-persistence")
        .arg("--effort")
        .arg("low")
        .current_dir(std::env::temp_dir())
        .env_remove("CLAUDECODE")
        .stdin(Stdio::null());
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("CLAUDE_CODE_") {
            command.env_remove(key);
        }
    }
    let stdout = run(&mut command)?;
    parse(&stdout, commands.len())
}

/// Read the plan out of `claude -p --output-format json`. Numbers that do not
/// point at an earlier command are dropped.
pub fn parse(stdout: &str, count: usize) -> Result<After, String> {
    let envelope: Value =
        serde_json::from_str(stdout.trim()).map_err(|_| "the planner's answer is not JSON")?;
    if envelope.get("is_error").and_then(Value::as_bool) == Some(true) {
        let why = envelope
            .get("result")
            .and_then(Value::as_str)
            .unwrap_or("unknown error");
        return Err(format!("the planner failed: {why}"));
    }
    let structured = envelope.get("structured_output").cloned().or_else(|| {
        envelope
            .get("result")
            .and_then(Value::as_str)
            .and_then(|text| serde_json::from_str(text).ok())
    });
    let after = structured
        .as_ref()
        .and_then(|value| value.get("after"))
        .and_then(Value::as_array)
        .ok_or("the planner's answer has no plan")?;
    if after.len() != count {
        return Err(format!(
            "the planner planned {} commands, not {count}",
            after.len()
        ));
    }
    Ok(after
        .iter()
        .enumerate()
        .map(|(i, waits)| {
            let mut waits: Vec<usize> = waits
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_u64)
                .map(|n| n as usize)
                .filter(|&n| n < i)
                .collect();
            waits.sort_unstable();
            waits.dedup();
            waits
        })
        .collect())
}

/// What herdr has, in a few lines, for the planner's context.
pub fn screen(catalog: &[super::intent::Entry]) -> String {
    use super::intent::Kind;
    let list = |kind: Kind| -> Value {
        json!(catalog
            .iter()
            .filter(|e| e.kind == kind)
            .map(|e| e.full_label())
            .collect::<Vec<_>>())
    };
    let on_screen = catalog
        .iter()
        .find(|e| e.kind == Kind::Tab && e.locality == 2)
        .map(|e| e.full_label());
    json!({
        "on_screen": on_screen,
        "spaces": list(Kind::Space),
        "tabs": list(Kind::Tab),
        "panes": list(Kind::Pane),
    })
    .to_string()
}

fn run(command: &mut Command) -> Result<String, String> {
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .map_err(|err| format!("could not start claude: {err}"))?;
    let mut stdout = child.stdout.take().ok_or("claude stdout was not piped")?;
    let mut stderr = child.stderr.take().ok_or("claude stderr was not piped")?;
    let out = std::thread::spawn(move || {
        let mut buf = String::new();
        let _ = stdout.read_to_string(&mut buf);
        buf
    });
    let err = std::thread::spawn(move || {
        let mut buf = String::new();
        let _ = stderr.read_to_string(&mut buf);
        buf
    });
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() >= TIMEOUT => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("the planner took too long".into());
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(25)),
            Err(e) => return Err(format!("could not wait for claude: {e}")),
        }
    };
    let stdout = out.join().unwrap_or_default();
    let stderr = err.join().unwrap_or_default();
    if !status.success() && stdout.trim().is_empty() {
        let line = stderr.lines().find(|l| !l.trim().is_empty());
        return Err(match line {
            Some(line) => format!("the planner failed: {}", line.trim()),
            None => format!("the planner exited {status}"),
        });
    }
    Ok(stdout)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_structured_answer_is_read() {
        let stdout = r#"{"type":"result","is_error":false,"result":"{\"after\":[[],[0],[1],[]]}","structured_output":{"after":[[],[0],[1],[]]}}"#;
        assert_eq!(
            parse(stdout, 4).unwrap(),
            vec![Vec::<usize>::new(), vec![0], vec![1], vec![]]
        );
    }

    #[test]
    fn a_wait_on_a_later_command_or_itself_is_dropped() {
        let stdout = r#"{"is_error":false,"structured_output":{"after":[[1],[1,0,0],[5,0]]}}"#;
        assert_eq!(parse(stdout, 3).unwrap(), vec![vec![], vec![0], vec![0]]);
    }

    #[test]
    fn a_plan_of_the_wrong_length_is_refused() {
        let stdout = r#"{"is_error":false,"structured_output":{"after":[[]]}}"#;
        assert!(parse(stdout, 2).is_err());
    }

    #[test]
    fn the_text_result_is_used_when_there_is_no_structured_one() {
        let stdout = r#"{"is_error":false,"result":"{\"after\":[[],[]]}"}"#;
        assert_eq!(parse(stdout, 2).unwrap(), vec![Vec::<usize>::new(), vec![]]);
    }

    #[test]
    fn in_turn_chains_each_to_the_one_before() {
        assert_eq!(in_turn(3), vec![vec![], vec![0], vec![1]]);
    }

    /// Asks the real planner. Run with `cargo test planner_live -- --ignored`.
    #[test]
    #[ignore = "runs claude"]
    fn planner_live() {
        let commands: Vec<String> = [
            "Switch to the fifth space",
            "add a Claude Code agent",
            "prompt it to do an audit of the last five diffs from this week",
            "tell the Grok agent to check the status of the PR",
        ]
        .map(String::from)
        .to_vec();
        let after = plan("claude", "claude-opus-5-5", &commands, "{}").unwrap();
        assert!(after[1].contains(&0), "{after:?}");
        assert!(after[2].contains(&1), "{after:?}");
        assert!(after[3].is_empty(), "{after:?}");

        let commands: Vec<String> = [
            "tell Ada to run the tests",
            "tell Bruno to fix the lint",
            "split right",
            "zoom",
        ]
        .map(String::from)
        .to_vec();
        let after = plan("claude", "claude-opus-5-5", &commands, "{}").unwrap();
        assert!(after[1].is_empty(), "{after:?}");
        assert!(after[3].contains(&2), "{after:?}");
    }
}
