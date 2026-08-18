//! Which agents and terminals the composer can start, and how each receives a
//! task.
//!
//! One table, read from both ends. The composer reads it to fill its agent
//! dropdown; starting an agent reads it to build a command line. Nothing here
//! knows about a harness that is not written down, so adding one is a row. The
//! `Terminal` row is always available: its task becomes input to the ordinary
//! interactive shell instead of an argument to an AI harness.
//!
//! How a task reaches a harness differs, and the differences are real rather
//! than cosmetic. `opencode "fix the tests"` opens a directory called `fix the
//! tests`, because opencode's first argument is a project. So the way in is
//! written per harness instead of assumed.
//!
//! `Auto` is the row that is not a harness. It stands for "whichever agent
//! already owns this work", which is a question herdr does not answer itself:
//! it starts Claude Code on `/who <task>`, and the skill behind that command
//! reads the running agents, picks the one with the evidence, hands the message
//! over and switches to its pane. Routing lives in the skill because the
//! evidence lives in the transcripts, and an agent is already the thing that
//! can read them.

use crate::detect::{agent_label, Agent};

/// What selecting a composer row starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Launch {
    /// An AI harness receives the task on its command line.
    Agent { agent: Agent, argv: Vec<String> },
    /// An ordinary interactive shell receives the task as terminal input.
    Terminal { command: String },
}

/// How a harness takes the task it is started on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hand {
    /// `claude "the task"` — the task is the first argument.
    Word,
    /// `opencode run "the task"` — a subcommand, then the task.
    After(&'static str),
    /// `kimi -p "the task"` — a flag, then the task.
    Under(&'static str),
}

/// One row of the composer's agent dropdown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Harness {
    /// The agent actually started. `Auto` starts Claude Code, because the
    /// command it sends is a Claude Code skill; `None` is the Terminal row.
    pub agent: Option<Agent>,
    /// What the row is called on screen. `Auto` is named for what it does
    /// rather than for what it runs, which is the whole of its point.
    pub name: &'static str,
    pub hand: Hand,
    /// What choosing this row writes in front of the task. Naming a harness
    /// puts nothing there; `Auto` writes the `/who` it stands for, so the field
    /// reads as the thing that will actually be sent.
    pub prefix: &'static str,
    /// The words that ask this harness to do the work in a git worktree of its
    /// own, and nothing for a harness that cannot be asked. Two agents editing
    /// one checkout overwrite each other, so a harness that can branch is
    /// started branched.
    ///
    /// The words are only passed where the folder is inside a repository. A
    /// harness told to branch from a repository that is not there refuses to
    /// start at all, so passing them everywhere would trade a shared checkout
    /// for an agent that never runs.
    ///
    /// They go on the end, after the task. `claude -w` names the worktree with
    /// whatever follows it, so a flag written in front of the task is a flag
    /// that eats the task: `claude -w "fix the tests"` asks for a worktree
    /// called `fix the tests` and hands the agent nothing to do.
    pub worktree: &'static [&'static str],
}

const fn harness(agent: Agent, name: &'static str, hand: Hand) -> Harness {
    Harness {
        agent: Some(agent),
        name,
        hand,
        prefix: "",
        worktree: &[],
    }
}

/// A row whose harness takes the work into a worktree of its own.
const fn branching(
    agent: Agent,
    name: &'static str,
    hand: Hand,
    worktree: &'static [&'static str],
) -> Harness {
    Harness {
        agent: Some(agent),
        name,
        hand,
        prefix: "",
        worktree,
    }
}

/// What Claude Code takes to make itself a worktree.
const CLAUDE_WORKTREE: &[&str] = &["-w"];

/// The command `Auto` puts in front of a task, and the skill that answers it.
pub const AUTO_PREFIX: &str = "/who";

/// Every row the dropdown can list, in the order it lists them. `Auto` first,
/// because it is the one that needs to be chosen least often and reached for
/// most easily.
pub const ALL: [Harness; 19] = [
    Harness {
        agent: Some(Agent::Claude),
        name: "Auto",
        hand: Hand::Word,
        prefix: AUTO_PREFIX,
        worktree: &[],
    },
    Harness {
        agent: None,
        name: "Terminal",
        hand: Hand::Word,
        prefix: "",
        worktree: &[],
    },
    branching(Agent::Claude, "Claude Code", Hand::Word, CLAUDE_WORKTREE),
    harness(Agent::Codex, "Codex", Hand::Word),
    harness(Agent::Pi, "Pi", Hand::Word),
    harness(Agent::Gemini, "Gemini", Hand::Word),
    harness(Agent::Cursor, "Cursor", Hand::Word),
    harness(Agent::Antigravity, "Antigravity", Hand::Word),
    harness(Agent::Cline, "Cline", Hand::After("task")),
    harness(Agent::OpenCode, "OpenCode", Hand::After("run")),
    harness(Agent::GithubCopilot, "Copilot", Hand::Under("-p")),
    harness(Agent::Kimi, "Kimi", Hand::Under("-p")),
    harness(Agent::Kiro, "Kiro", Hand::Word),
    harness(Agent::Droid, "Droid", Hand::After("exec")),
    harness(Agent::Amp, "Amp", Hand::Under("-x")),
    harness(Agent::Grok, "Grok", Hand::Word),
    harness(Agent::Hermes, "Hermes", Hand::Under("-z")),
    harness(Agent::Kilo, "Kilo", Hand::Word),
    harness(Agent::Qodercli, "Qoder", Hand::Word),
];

/// The row every list starts with.
pub fn auto() -> &'static Harness {
    &ALL[0]
}

impl Harness {
    /// The program this row runs.
    pub fn program(&self) -> Option<&'static str> {
        self.agent.map(agent_label)
    }

    /// The task as it will actually be sent, which is what the field shows.
    pub fn message(&self, task: &str) -> String {
        if self.prefix.is_empty() {
            task.to_string()
        } else {
            format!("{} {task}", self.prefix)
        }
    }

    /// What starts when this row is selected, where `repo` says the folder the
    /// agent will start in is inside a git repository. A harness that can
    /// branch is asked to, so its edits land in a checkout nothing else is
    /// editing; outside a repository there is nothing to branch from and the
    /// harness starts in the folder itself.
    pub fn launch(&self, task: &str, repo: bool) -> Launch {
        let Some(agent) = self.agent else {
            return Launch::Terminal {
                command: task.to_string(),
            };
        };
        let mut argv = vec![agent_label(agent).to_string()];
        match self.hand {
            Hand::Word => {}
            Hand::After(subcommand) => argv.push(subcommand.to_string()),
            Hand::Under(flag) => argv.push(flag.to_string()),
        }
        argv.push(self.message(task));
        if repo {
            argv.extend(self.worktree.iter().map(|word| word.to_string()));
        }
        Launch::Agent { agent, argv }
    }
}

/// The same command with the task taken off the end, for starting a harness
/// again without sending it anything.
///
/// A saved command line carries the task the agent was started on. Running it
/// again would hand that task to a fresh agent, which reads as every agent in
/// the space redoing its last piece of work at once. What starting again wants
/// is the harness itself, idle, in the folder it was in.
///
/// Only the exact shape [`Harness::launch`] writes is read as carrying a task:
/// the program of a known row, that row's way in, one argument that is not a
/// flag, and whatever that row asked for a worktree with. Anything else is a
/// command herdr did not compose — `claude --continue`, or a program of the
/// user's own — and comes back unchanged.
///
/// The worktree words go with the task. Asking again would branch again, and a
/// second empty checkout is not where the first one's work is; starting again
/// wants the agent back where it was.
pub fn without_task(argv: &[String]) -> Vec<String> {
    let Some(program) = argv.first() else {
        return argv.to_vec();
    };
    let name = std::path::Path::new(program)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(program.as_str());
    let rest = &argv[1..];
    let carries_task = ALL.iter().any(|harness| {
        harness.program() == Some(name)
            && [Some(rest), without_worktree(harness, rest)]
                .into_iter()
                .flatten()
                .any(|rest| is_task_line(harness, rest))
    });
    if carries_task {
        vec![program.clone()]
    } else {
        argv.to_vec()
    }
}

/// Whether what follows the program is this row's way in and a task, which is
/// the whole of what [`Harness::launch`] writes there.
fn is_task_line(harness: &Harness, rest: &[String]) -> bool {
    let shape = match harness.hand {
        Hand::Word => rest.len() == 1,
        Hand::After(way_in) | Hand::Under(way_in) => rest.len() == 2 && rest[0] == way_in,
    };
    shape && rest.last().is_some_and(|task| !task.starts_with('-'))
}

/// The same words with what asked for a worktree taken off the end, or `None`
/// when this command asked for none.
fn without_worktree<'a>(harness: &Harness, rest: &'a [String]) -> Option<&'a [String]> {
    let kept = rest.len().checked_sub(harness.worktree.len())?;
    (!harness.worktree.is_empty()
        && rest[kept..]
            .iter()
            .zip(harness.worktree)
            .all(|(word, asked)| word == asked))
    .then(|| &rest[..kept])
}

/// What this machine can actually start: Terminal, plus every agent row whose
/// program is on the path. Offering a harness that is not installed would be
/// offering a row that starts an agent which dies before it draws anything,
/// and a long dropdown of names to find the two that work is a list that has
/// to be read rather than used.
pub fn installed() -> Vec<&'static Harness> {
    ALL.iter()
        .filter(|harness| harness.program().is_none_or(on_path))
        .collect()
}

/// The row a name stands for, by the name the dropdown shows or by the program
/// it runs. `Auto` and `Claude Code` run the same program, so a lookup by
/// program finds `Auto` first and a saved choice of one is never read as the
/// other — the name is what is written down.
#[cfg(test)]
pub fn named(name: &str) -> Option<&'static Harness> {
    ALL.iter()
        .find(|harness| harness.name.eq_ignore_ascii_case(name))
        .or_else(|| {
            ALL.iter().find(|harness| {
                harness
                    .program()
                    .is_some_and(|program| program.eq_ignore_ascii_case(name))
            })
        })
}

/// Whether a program is somewhere on `PATH`. Asked of the path rather than by
/// running anything: a harness that is installed but broken is still a harness
/// worth offering, and the way to find that out is to start it.
pub fn on_path(program: &str) -> bool {
    if program.is_empty() {
        return false;
    }
    std::env::var_os("PATH").is_some_and(|path| {
        std::env::split_paths(&path).any(|directory| runnable(&directory.join(program)))
    })
}

/// Metadata rather than a symlink's own facts, because a program on the path is
/// very often a link to where it was installed.
#[cfg(unix)]
fn runnable(path: &std::path::Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .is_ok_and(|found| found.is_file() && found.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn runnable(path: &std::path::Path) -> bool {
    std::fs::metadata(path).is_ok_and(|found| found.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_row_is_named_once() {
        for (at, harness) in ALL.iter().enumerate() {
            assert!(
                ALL[..at].iter().all(|earlier| earlier.name != harness.name),
                "{} is listed twice",
                harness.name
            );
        }
    }

    #[test]
    fn auto_is_the_only_row_that_writes_anything_in_front_of_the_task() {
        assert_eq!(auto().prefix, AUTO_PREFIX);
        for harness in ALL.iter().skip(1) {
            assert!(
                harness.prefix.is_empty(),
                "{} writes a prefix",
                harness.name
            );
        }
    }

    #[test]
    fn a_harness_is_handed_the_task_the_way_that_harness_takes_one() {
        assert_eq!(
            named("Claude Code").unwrap().launch("run the tests", false),
            Launch::Agent {
                agent: Agent::Claude,
                argv: vec!["claude".into(), "run the tests".into()],
            },
            "the task is the first argument"
        );
        assert_eq!(
            named("OpenCode").unwrap().launch("run the tests", true),
            Launch::Agent {
                agent: Agent::OpenCode,
                argv: vec!["opencode".into(), "run".into(), "run the tests".into()],
            },
            "a project is opencode's first argument, so the task goes after a subcommand"
        );
        assert_eq!(
            named("Kimi").unwrap().launch("run the tests", true),
            Launch::Agent {
                agent: Agent::Kimi,
                argv: vec!["kimi".into(), "-p".into(), "run the tests".into()],
            },
            "the task goes under a flag"
        );
    }

    #[test]
    fn auto_starts_claude_code_on_the_command_it_stands_for() {
        assert_eq!(
            auto().launch("fix the drag preview", true),
            Launch::Agent {
                agent: Agent::Claude,
                argv: vec!["claude".into(), "/who fix the drag preview".into()],
            }
        );
        assert_eq!(auto().program(), Some("claude"));
    }

    #[test]
    fn terminal_runs_the_task_as_a_shell_command() {
        assert_eq!(
            named("Terminal").unwrap().launch("just check", true),
            Launch::Terminal {
                command: "just check".into(),
            }
        );
        assert_eq!(named("Terminal").unwrap().program(), None);
    }

    fn argv(words: &[&str]) -> Vec<String> {
        words.iter().map(|word| word.to_string()).collect()
    }

    #[test]
    fn starting_again_never_sends_the_task_a_second_time() {
        assert_eq!(
            without_task(&argv(&["claude", "run the tests"])),
            argv(&["claude"]),
            "the task is the first argument, so dropping it leaves the harness"
        );
        assert_eq!(
            without_task(&argv(&["opencode", "run", "run the tests"])),
            argv(&["opencode"]),
            "a subcommand exists to carry the task and goes with it"
        );
        assert_eq!(
            without_task(&argv(&["kimi", "-p", "run the tests"])),
            argv(&["kimi"])
        );
        assert_eq!(
            without_task(&argv(&["/usr/local/bin/claude", "run the tests"])),
            argv(&["/usr/local/bin/claude"]),
            "a program named by path is still that program"
        );
    }

    #[test]
    fn a_command_herdr_did_not_compose_comes_back_as_it_is() {
        assert_eq!(
            without_task(&argv(&["claude", "--continue"])),
            argv(&["claude", "--continue"]),
            "a flag is not a task"
        );
        assert_eq!(
            without_task(&argv(&["claude", "--resume", "abc"])),
            argv(&["claude", "--resume", "abc"])
        );
        assert_eq!(without_task(&argv(&["claude"])), argv(&["claude"]));
        assert_eq!(without_task(&argv(&["htop"])), argv(&["htop"]));
        assert_eq!(
            without_task(&argv(&["opencode", "/some/project"])),
            argv(&["opencode", "/some/project"]),
            "opencode's first argument is a project, and a project is not a task"
        );
        assert_eq!(without_task(&[]), Vec::<String>::new());
    }

    #[test]
    fn every_row_that_can_be_started_can_be_started_again_without_its_task() {
        for harness in ALL.iter() {
            let Launch::Agent { argv, .. } = harness.launch("run the tests", true) else {
                continue;
            };
            assert_eq!(
                without_task(&argv),
                vec![harness.program().unwrap().to_string()],
                "{} keeps its task when started again",
                harness.name
            );
        }
    }

    #[test]
    fn claude_code_takes_the_work_into_a_worktree_of_its_own() {
        assert_eq!(
            named("Claude Code").unwrap().launch("run the tests", true),
            Launch::Agent {
                agent: Agent::Claude,
                argv: vec!["claude".into(), "run the tests".into(), "-w".into()],
            },
            "the flag goes after the task, because a worktree flag takes a name and \
             the task would be read as one"
        );
    }

    #[test]
    fn a_folder_outside_a_repository_starts_the_harness_where_it_is() {
        assert_eq!(
            named("Claude Code").unwrap().launch("run the tests", false),
            Launch::Agent {
                agent: Agent::Claude,
                argv: vec!["claude".into(), "run the tests".into()],
            },
            "there is nothing to branch from, and asking anyway starts nothing at all"
        );
    }

    #[test]
    fn only_a_harness_that_can_branch_is_asked_to() {
        for harness in ALL.iter() {
            let (Launch::Agent { argv, .. }, Launch::Agent { argv: plain, .. }) = (
                harness.launch("run the tests", true),
                harness.launch("run the tests", false),
            ) else {
                continue;
            };
            if harness.worktree.is_empty() {
                assert_eq!(argv, plain, "{} was asked for a worktree", harness.name);
            } else {
                assert_eq!(
                    argv.len(),
                    plain.len() + harness.worktree.len(),
                    "{} did not ask for a worktree",
                    harness.name
                );
            }
        }
    }

    #[test]
    fn starting_again_leaves_the_agent_where_it_is_rather_than_branching_again() {
        assert_eq!(
            without_task(&argv(&["claude", "run the tests", "-w"])),
            argv(&["claude"]),
            "a second worktree is not where the first one's work is"
        );
        assert_eq!(
            without_task(&argv(&["claude", "-w"])),
            argv(&["claude", "-w"]),
            "a command with no task to drop is a command herdr did not compose"
        );
    }

    #[test]
    fn a_name_finds_its_row_by_either_of_its_names() {
        assert_eq!(named("auto"), Some(auto()));
        assert_eq!(
            named("Claude Code").map(|found| found.name),
            Some("Claude Code")
        );
        assert_eq!(named("opencode").map(|found| found.name), Some("OpenCode"));
        assert_eq!(named("something nobody has written"), None);
    }

    #[test]
    fn looking_a_row_up_by_program_never_turns_claude_code_into_auto() {
        // Both rows run `claude`, so the name is what tells them apart. A saved
        // choice of one must never come back as the other.
        assert_eq!(
            named("Claude Code").map(|found| found.name),
            Some("Claude Code")
        );
        assert_eq!(named("Auto").map(|found| found.name), Some("Auto"));
    }
}
