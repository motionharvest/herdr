//! Reading a Commander line with Jev, in two requests.
//!
//! Nothing here matches phrases. Every capability herdr has is a [`Tool`]: the
//! actions bound to keys, and what the CLI reaches over the socket API, such as
//! starting an agent, typing into a pane, renaming, closing, and worktrees.
//! Each tool says in plain words what it does and what it needs: a space, tab
//! or pane to act on, a stretch of the typed line (a message, a name, a branch),
//! an agent to start, or a key to press.
//!
//! **Route.** The first request sends the line and everything on screen, and
//! asks which [`Category`] of thing the line wants. In the same request, and
//! before knowing the answer, it asks which space, which tab and which pane the
//! line refers to. Those questions do not depend on the category, so asking
//! them now saves a round trip; the answers that turn out not to matter are
//! ignored.
//!
//! **Select and fill.** The second request asks, for each category that is
//! still plausible (not only the likeliest), which tool within it the line
//! wants. In the same request it finds the stretches of the line each candidate
//! tool would need: where the message begins and ends, where the new name
//! begins and ends, and whether one was given at all. A stretch is picked from
//! the line's own words, never written, so what is sent is what was typed.
//!
//! The tool that wins is the one with the highest probability across both
//! steps: the category's probability times the tool's probability within it.
//! A line whose likeliest category is wrong can still be read correctly when
//! the right category was close behind. The weakest probability behind the
//! final reading is its confidence; the app acts at once only when that is high
//! and the tool cannot lose anything.

use std::collections::BTreeMap;

use serde_json::{json, Value};

use super::intent::{tokens, Action, Command, Entry, Kind, Token};
use super::jev::{Answers, Oracle, Question, Questions};
use crate::api::schema::{ViewChange, ViewPart};
use crate::harness::{Harness, ALL as HARNESSES};
use crate::layout::PaneId;

/// Below this, a category is not worth asking about in the second step, and
/// a tool is not tried when a likelier one could not be built.
const BEAM_FLOOR: f64 = 0.15;
/// A space or tab stands in for a pane the line does not name only when Jev
/// is at least this sure the line names that space or tab. The space and tab
/// questions are asked whether or not the line names one, so a weak answer to
/// them is a guess.
const WIDER_SURE: f64 = 0.8;
/// At most this many categories go on to the second step.
const BEAM_WIDTH: usize = 3;
/// A Choice may have at most 255 options; two are kept for `current` and
/// `missing`.
const MAX_OPTIONS: usize = 253;
/// How many words of the line each stretch option quotes.
const QUOTE_WORDS: usize = 8;

/// A kind of thing a line can want. The first step chooses among these.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Category {
    Navigate,
    Message,
    Layout,
    Create,
    Close,
    Rename,
    Worktree,
    Interface,
}

impl Category {
    const ALL: [Category; 8] = [
        Category::Navigate,
        Category::Message,
        Category::Layout,
        Category::Create,
        Category::Close,
        Category::Rename,
        Category::Worktree,
        Category::Interface,
    ];

    fn id(self) -> &'static str {
        match self {
            Category::Navigate => "navigate",
            Category::Message => "message",
            Category::Layout => "layout",
            Category::Create => "create",
            Category::Close => "close",
            Category::Rename => "rename",
            Category::Worktree => "worktree",
            Category::Interface => "interface",
        }
    }

    fn means(self) -> &'static str {
        match self {
            Category::Navigate => "show or focus a different space, tab or pane, or move focus to a neighbouring pane, tab, space or agent",
            Category::Message => "put words or keys into a pane: hand an agent a message, question or task; run a shell command; type text; or press a key such as ctrl+c, escape or enter",
            Category::Layout => "change how the panes of the current tab are arranged: split a pane, zoom one, or resize them",
            Category::Create => "make something new: a tab, a space, or a newly started coding agent",
            Category::Close => "close or kill a pane, tab or space",
            Category::Rename => "give a space, tab or pane a different name",
            Category::Worktree => "git worktrees: make one on a branch, remove one, or land (merge) one onto its parent branch",
            Category::Interface => "herdr's own interface: fold away or show the sidebar, its spaces section, a space's agent list, a group of worktree spaces, the minimaps, or the agent table; or open settings, keybinding help, the navigator, the composer, copy mode, or reload the config",
        }
    }
}

/// A stretch of the typed line that a tool needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Text {
    /// What to hand an agent or shell, submitted with Enter.
    Message,
    /// What to type into a pane without submitting it.
    Typed,
    /// A new name.
    Name,
    /// What a newly started agent should work on.
    Task,
    /// A git branch.
    Branch,
}

impl Text {
    fn id(self) -> &'static str {
        match self {
            Text::Message => "message",
            Text::Typed => "typed",
            Text::Name => "name",
            Text::Task => "task",
            Text::Branch => "branch",
        }
    }

    fn what(self) -> &'static str {
        match self {
            Text::Message => "the message, question, task or shell command to hand over, as the user wrote it, without the words that say who it is for (a pane, agent, tab or space by any name in `panes`, `tabs` or `spaces`)",
            Text::Typed => "the text to type into the pane, without the words that say where",
            Text::Name => "the new name, without the words that say what is being named",
            Text::Task => "the task the new agent should start working on, without the words that ask for the agent",
            Text::Branch => "the git branch name",
        }
    }

    /// Whether a tool needing this can go ahead without it.
    fn optional(self) -> bool {
        !matches!(self, Text::Message | Text::Typed)
    }
}

/// A key a pane can be sent, as the socket API names it.
const KEYS: &[(&str, &str, &str)] = &[
    (
        "interrupt",
        "ctrl+c",
        "ctrl+c: interrupt, stop or cancel what the pane is running",
    ),
    ("enter", "Enter", "enter or return: submit or confirm"),
    (
        "escape",
        "Esc",
        "escape: dismiss, back out, or stop an agent's turn",
    ),
    ("tab", "Tab", "tab"),
    ("up", "Up", "the up arrow"),
    ("down", "Down", "the down arrow"),
    ("yes", "y", "the letter y, answering yes"),
    ("no", "n", "the letter n, answering no"),
];

/// Everything herdr can be asked to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    /// One of herdr's own key-bound actions, which needs nothing more.
    Act(Action),
    FocusSpace,
    FocusTab,
    FocusPane,
    Prompt,
    TypeText,
    PressKey,
    NewTab,
    NewSpace,
    StartAgent,
    ClosePane,
    CloseTab,
    CloseSpace,
    RenameSpace,
    RenameTab,
    RenamePane,
    NewWorktree,
    RemoveWorktree,
    LandWorktree,
    /// Show, hide or toggle a part of the interface.
    View(ViewPart),
}

/// Every part of the interface that folds.
const VIEW_PARTS: [ViewPart; 6] = [
    ViewPart::Sidebar,
    ViewPart::Spaces,
    ViewPart::AgentTable,
    ViewPart::SpaceAgents,
    ViewPart::SpaceGroup,
    ViewPart::Minimap,
];

/// How a part of the interface is changed, as the API names it.
const VIEW_CHANGES: [(&str, ViewChange, &str); 3] = [
    ("show", ViewChange::Show, "show, open, expand or unfold it"),
    (
        "hide",
        ViewChange::Hide,
        "hide, close, collapse or fold it away",
    ),
    (
        "toggle",
        ViewChange::Toggle,
        "toggle it, or the request does not say which way",
    ),
];

impl Tool {
    pub fn all() -> Vec<Tool> {
        use Action as A;
        let mut tools = vec![Tool::FocusSpace, Tool::FocusTab, Tool::FocusPane];
        tools.extend(
            [
                A::NextTab,
                A::PreviousTab,
                A::NextSpace,
                A::PreviousSpace,
                A::NextAgent,
                A::PreviousAgent,
                A::LastPane,
                A::FocusLeft,
                A::FocusRight,
                A::FocusUp,
                A::FocusDown,
                A::SplitRight,
                A::SplitLeft,
                A::SplitDown,
                A::SplitUp,
                A::Zoom,
                A::Resize,
                A::Settings,
                A::Help,
                A::Navigator,
                A::Composer,
                A::CopyMode,
                A::ReloadConfig,
            ]
            .map(Tool::Act),
        );
        tools.extend([
            Tool::Prompt,
            Tool::TypeText,
            Tool::PressKey,
            Tool::NewTab,
            Tool::NewSpace,
            Tool::StartAgent,
            Tool::ClosePane,
            Tool::CloseTab,
            Tool::CloseSpace,
            Tool::RenameSpace,
            Tool::RenameTab,
            Tool::RenamePane,
            Tool::NewWorktree,
            Tool::RemoveWorktree,
            Tool::LandWorktree,
        ]);
        tools.extend(VIEW_PARTS.map(Tool::View));
        tools
    }

    fn category(self) -> Category {
        use Action as A;
        match self {
            Tool::FocusSpace | Tool::FocusTab | Tool::FocusPane => Category::Navigate,
            Tool::Act(
                A::NextTab
                | A::PreviousTab
                | A::NextSpace
                | A::PreviousSpace
                | A::NextAgent
                | A::PreviousAgent
                | A::LastPane
                | A::FocusLeft
                | A::FocusRight
                | A::FocusUp
                | A::FocusDown,
            ) => Category::Navigate,
            Tool::Act(
                A::SplitRight | A::SplitLeft | A::SplitDown | A::SplitUp | A::Zoom | A::Resize,
            ) => Category::Layout,
            Tool::Act(_) | Tool::View(_) => Category::Interface,
            Tool::Prompt | Tool::TypeText | Tool::PressKey => Category::Message,
            Tool::NewTab | Tool::NewSpace | Tool::StartAgent => Category::Create,
            Tool::ClosePane | Tool::CloseTab | Tool::CloseSpace => Category::Close,
            Tool::RenameSpace | Tool::RenameTab | Tool::RenamePane => Category::Rename,
            Tool::NewWorktree | Tool::RemoveWorktree | Tool::LandWorktree => Category::Worktree,
        }
    }

    /// The option key this tool has in a Choice.
    fn id(self) -> String {
        match self {
            Tool::Act(action) => format!("{action:?}").to_lowercase(),
            Tool::View(part) => format!("view_{part:?}").to_lowercase(),
            other => format!("{other:?}").to_lowercase(),
        }
    }

    fn means(self) -> &'static str {
        match self {
            Tool::Act(action) => action.label(),
            Tool::FocusSpace => "switch to a named space (project or workspace)",
            Tool::FocusTab => "switch to a named tab",
            Tool::FocusPane => "focus a named pane, agent or terminal",
            Tool::Prompt => {
                "hand a pane or agent a message, question, task or shell command and submit it"
            }
            Tool::TypeText => "type text into a pane without submitting it",
            Tool::PressKey => "press a key in a pane, such as ctrl+c, escape, enter, or y/n",
            Tool::NewTab => "open a new tab, optionally with a name",
            Tool::NewSpace => "open a new space, optionally with a name",
            Tool::StartAgent => {
                "start a new coding agent such as Claude Code or Codex, optionally on a task"
            }
            Tool::ClosePane => "close or kill a pane, agent or terminal",
            Tool::CloseTab => "close a tab",
            Tool::CloseSpace => "close a space",
            Tool::RenameSpace => "rename a space",
            Tool::RenameTab => "rename a tab",
            Tool::RenamePane => "rename a pane or agent",
            Tool::NewWorktree => "make a new git worktree, optionally on a named branch",
            Tool::RemoveWorktree => "remove a worktree and its checkout",
            Tool::LandWorktree => "land or merge a worktree's branch onto its parent branch",
            Tool::View(ViewPart::Sidebar) => "fold away or show the whole sidebar on the left",
            Tool::View(ViewPart::Spaces) => {
                "fold away or show the list of spaces in the sidebar, all of them at once"
            }
            Tool::View(ViewPart::AgentTable) => {
                "fold away or show the agent table, the list of agents at the top"
            }
            Tool::View(ViewPart::SpaceAgents) => {
                "fold away or show the agents and panes listed under one space in the sidebar"
            }
            Tool::View(ViewPart::SpaceGroup) => {
                "fold away or show the worktree spaces grouped under a repository's space"
            }
            Tool::View(ViewPart::Minimap) => {
                "hide or show the minimaps, the small layout previews under tabs in the sidebar"
            }
        }
    }

    /// The kind of thing the tool acts on, if it acts on a chosen one.
    fn target(self) -> Option<Kind> {
        match self {
            Tool::FocusSpace
            | Tool::NewTab
            | Tool::StartAgent
            | Tool::CloseSpace
            | Tool::RenameSpace
            | Tool::NewWorktree
            | Tool::RemoveWorktree
            | Tool::LandWorktree => Some(Kind::Space),
            Tool::FocusTab | Tool::CloseTab | Tool::RenameTab => Some(Kind::Tab),
            Tool::FocusPane
            | Tool::Prompt
            | Tool::TypeText
            | Tool::PressKey
            | Tool::ClosePane
            | Tool::RenamePane => Some(Kind::Pane),
            Tool::View(ViewPart::SpaceAgents | ViewPart::SpaceGroup) => Some(Kind::Space),
            Tool::Act(_) | Tool::NewSpace | Tool::View(_) => None,
        }
    }

    fn text(self) -> Option<Text> {
        match self {
            Tool::Prompt => Some(Text::Message),
            Tool::TypeText => Some(Text::Typed),
            Tool::NewTab
            | Tool::NewSpace
            | Tool::RenameSpace
            | Tool::RenameTab
            | Tool::RenamePane => Some(Text::Name),
            Tool::StartAgent => Some(Text::Task),
            Tool::NewWorktree => Some(Text::Branch),
            _ => None,
        }
    }

    /// Whether carrying it out can lose work, so it waits for a second Enter.
    fn careful(self) -> bool {
        matches!(
            self,
            Tool::ClosePane
                | Tool::CloseTab
                | Tool::CloseSpace
                | Tool::RemoveWorktree
                | Tool::LandWorktree
                | Tool::Act(Action::ReloadConfig)
        )
    }
}

/// What the line was read as.
#[derive(Debug, Clone, PartialEq)]
pub struct Reading {
    pub command: Command,
    /// The weakest probability behind the reading, from 0 to 1.
    pub confidence: f64,
    /// Whether carrying it out could lose work.
    pub careful: bool,
    /// The runner-up, said when it was close.
    pub runner_up: Option<String>,
}

/// The line and what herdr has on screen, taken on the UI thread so the
/// requests can run off it.
#[derive(Debug, Clone)]
pub struct Snapshot {
    pub line: String,
    pub catalog: Vec<Entry>,
    /// The commands already carried out from the same paragraph, in order.
    /// They are what `it`, `there` and `that agent` point back at.
    pub earlier: Vec<Done>,
    /// The pane the step before this one started, if it started one.
    pub made: Option<PaneId>,
}

/// One command of a paragraph that has been carried out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Done {
    /// The words it was read from.
    pub said: String,
    /// What was done, as the Commander described it.
    pub did: String,
}

impl Snapshot {
    /// The entry of `kind` on screen now: the visible space, the visible tab,
    /// or the pane with the keyboard.
    fn current(&self, kind: Kind) -> Option<usize> {
        let tab = self
            .catalog
            .iter()
            .position(|e| e.kind == Kind::Tab && e.locality == 2);
        match kind {
            Kind::Space => self
                .catalog
                .iter()
                .position(|e| e.kind == Kind::Space && e.locality == 1),
            Kind::Tab => tab,
            Kind::Pane => {
                let tab = &self.catalog[tab?];
                self.catalog
                    .iter()
                    .position(|e| e.kind == Kind::Pane && e.ws == tab.ws && e.pane == tab.pane)
            }
        }
    }

    /// The entries a target question offers, nearest the screen first, as
    /// many as one Choice can hold.
    fn offered(&self, kind: Kind) -> Vec<usize> {
        let mut found: Vec<usize> = (0..self.catalog.len())
            .filter(|&i| self.catalog[i].kind == kind)
            .collect();
        found.sort_by_key(|&i| std::cmp::Reverse(self.catalog[i].locality));
        found.truncate(MAX_OPTIONS);
        found
    }
}

fn entry_key(index: usize) -> String {
    format!("e{index}")
}

fn kind_key(kind: Kind) -> &'static str {
    match kind {
        Kind::Space => "space",
        Kind::Tab => "tab",
        Kind::Pane => "pane",
    }
}

/// The state both requests send: the line, and every space, tab and pane by
/// the names herdr shows for it.
fn state(snapshot: &Snapshot) -> Value {
    let describe = |kind: Kind| -> Vec<Value> {
        snapshot
            .catalog
            .iter()
            .enumerate()
            .filter(|(_, e)| e.kind == kind)
            .map(|(i, e)| {
                let mut v = json!({"id": entry_key(i), "name": e.label});
                match kind {
                    Kind::Space => v["number"] = json!(e.ws + 1),
                    Kind::Tab => v["number"] = json!(e.tab + 1),
                    Kind::Pane => {}
                }
                if let Some(context) = &e.context {
                    v["in_space"] = json!(context);
                }
                v["also_called"] = json!(e.names);
                v
            })
            .collect()
    };
    let on_screen = |kind: Kind| {
        snapshot
            .current(kind)
            .map(|i| json!(snapshot.catalog[i].full_label()))
            .unwrap_or(Value::Null)
    };
    let mut state = json!({
        "request": snapshot.line,
        "on_screen": {
            "space": on_screen(Kind::Space),
            "tab": on_screen(Kind::Tab),
            "pane_with_keyboard": on_screen(Kind::Pane),
        },
        "spaces": describe(Kind::Space),
        "tabs": describe(Kind::Tab),
        "panes": describe(Kind::Pane),
    });
    if !snapshot.earlier.is_empty() {
        state["earlier_steps"] = json!(snapshot
            .earlier
            .iter()
            .map(|d| json!({"said": d.said, "did": d.did}))
            .collect::<Vec<_>>());
    }
    let made = snapshot.made.and_then(|pane| {
        snapshot
            .catalog
            .iter()
            .position(|e| e.kind == Kind::Pane && e.pane == Some(pane))
    });
    if let Some(i) = made {
        state["just_started"] = json!({
            "id": entry_key(i),
            "name": snapshot.catalog[i].full_label(),
        });
    }
    state
}

fn target_question(snapshot: &Snapshot, kind: Kind) -> Question {
    let noun = kind_key(kind);
    let mut criteria = BTreeMap::new();
    criteria.insert(
        "current".to_string(),
        json!(format!(
            "the request names no particular {noun}, or means the one on screen now"
        )),
    );
    criteria.insert(
        "missing".to_string(),
        json!(format!(
            "the request names a particular {noun} by a name, agent or number that none of these has"
        )),
    );
    for i in snapshot.offered(kind) {
        let e = &snapshot.catalog[i];
        criteria.insert(
            entry_key(i),
            json!(format!("the {noun} “{}”", e.full_label())),
        );
    }
    Question::choice(
        format!(
            "Which {noun} in `{noun}s` does `request` refer to, by any of its names, its agent, its number, or where it is? A message addressed to someone names the {noun} it is for. `request` may follow `earlier_steps` from the same paragraph: words such as `it`, `there` or `that agent` mean what those steps acted on, and the pane in `just_started` when one was just started."
        ),
        criteria,
    )
}

/// The questions of the first step.
fn route_questions(snapshot: &Snapshot) -> Questions {
    let mut questions = Questions::new();
    let mut criteria: BTreeMap<String, Value> = Category::ALL
        .iter()
        .map(|c| (c.id().to_string(), json!(c.means())))
        .collect();
    criteria.insert(
        "none".into(),
        json!("none of these: not something a terminal workspace manager can do"),
    );
    questions.insert(
        "category".into(),
        Question::choice(
            "`request` was typed into herdr, a terminal workspace manager that holds spaces (projects), tabs and panes, many running coding agents. What kind of thing does it ask herdr to do?",
            criteria,
        ),
    );
    for kind in [Kind::Space, Kind::Tab, Kind::Pane] {
        questions.insert(
            format!("target.{}", kind_key(kind)),
            target_question(snapshot, kind),
        );
    }
    questions
}

/// Up to `MAX_OPTIONS` word positions of the line, for picking a stretch.
fn positions(words: &[Token]) -> Vec<usize> {
    (0..words.len().min(MAX_OPTIONS)).collect()
}

fn quote(line: &str, words: &[Token], from: usize, to: usize) -> String {
    let (from, to) = (from.min(words.len() - 1), to.min(words.len() - 1));
    let text = &line[words[from].start..words[to].end];
    format!("“{text}”")
}

/// The questions that find one stretch of the line: where it begins, where it
/// ends, and, when it is optional, whether the line gives one at all.
/// Whether every stretch of the line fits in one Choice, with one option
/// left for `none`.
fn spans_fit(words: &[Token]) -> bool {
    let n = words.len();
    n * (n + 1) / 2 < MAX_OPTIONS
}

/// Where a stretch ending at word `end` stops in the line. The last word takes
/// the rest of the line with it, so closing punctuation and anything after
/// the last word stay part of the text.
fn stop_at(line: &str, words: &[Token], end: usize) -> usize {
    if end + 1 == words.len() {
        line.len()
    } else {
        words[end].end
    }
}

/// A stretch quoted whole, or its two ends when it is long.
fn quote_span(line: &str, words: &[Token], from: usize, to: usize) -> String {
    let text = if to - from < 2 * QUOTE_WORDS {
        line[words[from].start..stop_at(line, words, to)]
            .trim()
            .to_string()
    } else {
        format!(
            "{} … {}",
            &line[words[from].start..words[from + QUOTE_WORDS - 1].end],
            line[words[to + 1 - QUOTE_WORDS].start..stop_at(line, words, to)].trim()
        )
    };
    format!("“{text}”")
}

fn text_questions(questions: &mut Questions, line: &str, words: &[Token], text: Text) {
    let id = text.id();
    let what = text.what();
    // A short line offers every stretch of it as one option each, so the
    // answer is the whole stretch at once. Asking for its two ends apart lets
    // each be judged without the other, and a message's last word was then
    // about as likely to be cut as kept.
    if spans_fit(words) {
        let mut spans = BTreeMap::new();
        for from in 0..words.len() {
            for to in from..words.len() {
                spans.insert(
                    format!("s{from}_{to}"),
                    json!(quote_span(line, words, from, to)),
                );
            }
        }
        if text.optional() {
            spans.insert("none".to_string(), json!("`request` does not state one"));
        }
        questions.insert(
            format!("span.{id}"),
            Question::choice(
                format!("Which stretch of `request` is exactly {what}?"),
                spans,
            ),
        );
        return;
    }
    let last = words.len() - 1;
    let mut starts = BTreeMap::new();
    let mut ends = BTreeMap::new();
    // Each option says what it keeps and what it leaves out. An option that
    // only quoted what it keeps reads as plausible at every word, and the
    // last words of a message were then as likely to be cut as kept.
    for i in positions(words) {
        let ahead = (i + QUOTE_WORDS - 1).min(last);
        let tail = if ahead < last { " …" } else { "" };
        let before = if i == 0 {
            "nothing before it".to_string()
        } else {
            let from = i.saturating_sub(QUOTE_WORDS);
            let head = if from > 0 { "… " } else { "" };
            format!("{head}{} before it", quote(line, words, from, i - 1))
        };
        starts.insert(
            format!("w{i}"),
            json!(format!(
                "begins with {}{tail}, leaving out {before}",
                quote(line, words, i, ahead)
            )),
        );
        let behind = i.saturating_sub(QUOTE_WORDS - 1);
        let head = if behind > 0 { "… " } else { "" };
        let after = if i == last {
            "runs to the end of `request`, leaving nothing out after it".to_string()
        } else {
            let to = (i + QUOTE_WORDS).min(last);
            let more = if to < last { " …" } else { "" };
            format!(
                "leaves out {}{more} after it",
                quote(line, words, i + 1, to)
            )
        };
        ends.insert(
            format!("w{i}"),
            json!(format!(
                "ends with {head}{}, and {after}",
                quote(line, words, behind, i)
            )),
        );
    }
    let what = text.what();
    questions.insert(
        format!("start.{id}"),
        Question::choice(format!("In `request`, where does {what} begin?"), starts),
    );
    questions.insert(
        format!("end.{id}"),
        Question::choice(format!("In `request`, where does {what} end?"), ends),
    );
    if text.optional() {
        questions.insert(
            format!("given.{id}"),
            Question::noul(format!("Does `request` actually state {what}?")),
        );
    }
}

/// The questions of the second step, for the categories still in the running.
fn fill_questions(snapshot: &Snapshot, words: &[Token], beam: &[(Category, f64)]) -> Questions {
    let mut questions = Questions::new();
    let mut texts = Vec::new();
    let mut harness = false;
    let mut key = false;
    let mut view = false;
    for (category, _) in beam {
        let tools: Vec<Tool> = Tool::all()
            .into_iter()
            .filter(|t| t.category() == *category)
            .collect();
        let criteria = tools.iter().map(|t| (t.id(), json!(t.means()))).collect();
        questions.insert(
            format!("tool.{}", category.id()),
            Question::choice(
                format!(
                    "Suppose `request` asks herdr to {}. Which of these does it ask for?",
                    category.means()
                ),
                criteria,
            ),
        );
        for tool in tools {
            if let Some(text) = tool.text() {
                if !texts.contains(&text) {
                    texts.push(text);
                }
            }
            harness |= tool == Tool::StartAgent;
            key |= tool == Tool::PressKey;
            view |= matches!(tool, Tool::View(_));
        }
    }
    if !words.is_empty() {
        for text in texts {
            text_questions(&mut questions, &snapshot.line, words, text);
        }
    }
    if harness {
        let mut criteria: BTreeMap<String, Value> = BTreeMap::new();
        criteria.insert("unnamed".into(), json!("no particular agent is named"));
        for (i, h) in HARNESSES.iter().enumerate() {
            let what = match h.agent {
                _ if h.prefix == crate::harness::AUTO_PREFIX => {
                    "Auto: hand the task to whichever running agent already owns that work"
                        .to_string()
                }
                None => "a plain terminal shell, not an AI agent".to_string(),
                Some(agent) => format!("{} ({})", h.name, crate::detect::agent_label(agent)),
            };
            criteria.insert(format!("h{i}"), json!(what));
        }
        questions.insert(
            "harness".into(),
            Question::choice("If `request` asks to start an agent, which one?", criteria),
        );
    }
    if view {
        let criteria = VIEW_CHANGES
            .iter()
            .map(|(id, _, means)| (id.to_string(), json!(means)))
            .collect();
        questions.insert(
            "view_change".into(),
            Question::choice(
                "If `request` asks to change a part of herdr's interface, which way?",
                criteria,
            ),
        );
    }
    if key {
        let criteria = KEYS
            .iter()
            .map(|(id, _, means)| (id.to_string(), json!(means)))
            .collect();
        questions.insert(
            "key".into(),
            Question::choice("If `request` asks to press a key, which key?", criteria),
        );
    }
    questions
}

fn picked<'a>(answers: &'a Answers, id: &str) -> Result<(&'a str, f64), String> {
    answers
        .get(id)
        .and_then(|a| a.picked())
        .ok_or_else(|| format!("Jev did not answer {id}"))
}

/// The entry a target answer names, falling back to what is on screen.
/// A pane tool aimed at a named space or tab goes to that one's focused pane.
fn target(snapshot: &Snapshot, answers: &Answers, kind: Kind) -> Result<(Entry, f64), String> {
    // `current` and the entry on screen are two options naming one thing, so
    // a choice of either is as sure as both together.
    let sureness = |kind: Kind, i: usize, p: f64| -> f64 {
        let probabilities = answers
            .get(&format!("target.{}", kind_key(kind)))
            .and_then(|a| a.probabilities());
        match probabilities {
            Some(probabilities) if snapshot.current(kind) == Some(i) => {
                probabilities.get("current").copied().unwrap_or(0.0)
                    + probabilities.get(&entry_key(i)).copied().unwrap_or(0.0)
            }
            _ => p,
        }
    };
    let pick = |kind: Kind| -> Result<Option<(usize, f64)>, String> {
        let (choice, p) = picked(answers, &format!("target.{}", kind_key(kind)))?;
        Ok(choice
            .strip_prefix('e')
            .and_then(|n| n.parse::<usize>().ok())
            .filter(|&i| snapshot.catalog.get(i).is_some_and(|e| e.kind == kind))
            .map(|i| (i, sureness(kind, i, p))))
    };
    if let Some((i, p)) = pick(kind)? {
        return Ok((snapshot.catalog[i].clone(), p));
    }
    let pick = |wider: Kind| -> Result<Option<(usize, f64)>, String> {
        Ok(pick(wider)?.filter(|&(_, p)| p >= WIDER_SURE))
    };
    // A name that matches no pane may still name a space or tab, whose
    // focused pane is meant; a name that matches nothing is refused, because
    // acting on the wrong one is worse than asking again.
    let missing = |kind: Kind| {
        picked(answers, &format!("target.{}", kind_key(kind)))
            .is_ok_and(|(choice, _)| choice == "missing")
    };
    let wider_named = match kind {
        Kind::Pane => [Kind::Tab, Kind::Space]
            .iter()
            .any(|&k| pick(k).is_ok_and(|p| p.is_some())),
        Kind::Tab => pick(Kind::Space).is_ok_and(|p| p.is_some()),
        Kind::Space => false,
    };
    if missing(kind) && !wider_named {
        return Err(format!("no {} here goes by that name", kind_key(kind)));
    }
    if kind == Kind::Pane {
        for wider in [Kind::Tab, Kind::Space] {
            if let Some((i, p)) = pick(wider)? {
                let e = &snapshot.catalog[i];
                let pane = snapshot
                    .catalog
                    .iter()
                    .find(|p| p.kind == Kind::Pane && p.ws == e.ws && p.pane == e.pane)
                    .cloned()
                    .unwrap_or_else(|| e.clone());
                return Ok((pane, p));
            }
        }
    }
    if kind == Kind::Tab {
        if let Some((i, p)) = pick(Kind::Space)? {
            let space = &snapshot.catalog[i];
            if let Some(tab) = snapshot
                .catalog
                .iter()
                .find(|t| t.kind == Kind::Tab && t.ws == space.ws && t.tab == space.tab)
            {
                return Ok((tab.clone(), p));
            }
        }
    }
    let (_, p) = picked(answers, &format!("target.{}", kind_key(kind)))?;
    snapshot
        .current(kind)
        .map(|i| (snapshot.catalog[i].clone(), sureness(kind, i, p)))
        .ok_or_else(|| format!("which {}? nothing is on screen", kind_key(kind)))
}

/// The stretch of the line an answer picks, if the line gives one.
fn stretch(
    line: &str,
    words: &[Token],
    answers: &Answers,
    text: Text,
) -> Result<Option<(String, f64)>, String> {
    if words.is_empty() {
        return Ok(None);
    }
    let id = text.id();
    if let Some(answer) = answers.get(&format!("span.{id}")) {
        let (choice, p) = answer
            .picked()
            .ok_or_else(|| format!("Jev did not answer span.{id}"))?;
        if choice == "none" {
            return Ok(None);
        }
        let (from, to) = choice
            .strip_prefix('s')
            .and_then(|rest| rest.split_once('_'))
            .and_then(|(a, b)| Some((a.parse::<usize>().ok()?, b.parse::<usize>().ok()?)))
            .filter(|&(a, b)| a <= b && b < words.len())
            .ok_or_else(|| format!("Jev picked a stretch that is not there: {choice}"))?;
        let text = line[words[from].start..stop_at(line, words, to)]
            .trim()
            .to_string();
        return Ok((!text.is_empty()).then_some((text, p)));
    }
    if text.optional() {
        let given = answers
            .get(&format!("given.{id}"))
            .and_then(|a| a.yes())
            .unwrap_or(0.0);
        if given < 0.5 {
            return Ok(None);
        }
    }
    let at = |key: &str| -> Result<(usize, f64), String> {
        let (choice, p) = picked(answers, key)?;
        let index = choice
            .strip_prefix('w')
            .and_then(|n| n.parse::<usize>().ok())
            .filter(|&i| i < words.len())
            .ok_or_else(|| format!("Jev picked a word that is not there: {choice}"))?;
        Ok((index, p))
    };
    let (start, p_start) = at(&format!("start.{id}"))?;
    let (end, p_end) = at(&format!("end.{id}"))?;
    let end = if end < start { words.len() - 1 } else { end };
    let text = line[words[start].start..stop_at(line, words, end)]
        .trim()
        .to_string();
    Ok((!text.is_empty()).then_some((text, p_start.min(p_end))))
}

/// Turn the chosen tool and the answers into a command.
/// The space a part is changed for, when the part is per space.
fn part_space(part: ViewPart, to: impl FnOnce() -> Entry) -> Option<Entry> {
    matches!(part, ViewPart::SpaceAgents | ViewPart::SpaceGroup).then(to)
}

fn build(
    snapshot: &Snapshot,
    words: &[Token],
    answers: &Answers,
    tool: Tool,
) -> Result<(Command, f64), String> {
    let mut confidence = 1.0_f64;
    let mut aim = |kind: Kind| -> Result<Entry, String> {
        let (entry, p) = target(snapshot, answers, kind)?;
        confidence = confidence.min(p);
        Ok(entry)
    };
    let to = match tool.target() {
        Some(kind) => Some(aim(kind)?),
        None => None,
    };
    let text = match tool.text() {
        Some(text) => match stretch(&snapshot.line, words, answers, text)? {
            Some((found, p)) => {
                confidence = confidence.min(p);
                Some(found)
            }
            None if !text.optional() => {
                return Err(format!("what should be sent? Jev found no {}", text.id()))
            }
            None => None,
        },
        None => None,
    };
    let to = || to.clone().expect("tool has a target");
    let command = match tool {
        Tool::Act(action) => Command::Act(action),
        Tool::FocusSpace | Tool::FocusTab | Tool::FocusPane => Command::Go(to()),
        Tool::Prompt => Command::Send {
            to: to(),
            text: text.unwrap_or_default(),
        },
        Tool::TypeText => Command::Type {
            to: to(),
            text: text.unwrap_or_default(),
        },
        Tool::PressKey => {
            let (choice, p) = picked(answers, "key")?;
            confidence = confidence.min(p);
            let (_, key, means) = KEYS
                .iter()
                .find(|(id, _, _)| *id == choice)
                .ok_or_else(|| format!("Jev picked a key herdr does not know: {choice}"))?;
            Command::Keys {
                to: to(),
                keys: vec![key.to_string()],
                said: means.split(':').next().unwrap_or(means),
            }
        }
        Tool::NewTab => Command::NewTab {
            space: to(),
            name: text,
        },
        Tool::NewSpace => Command::NewSpace { name: text },
        Tool::StartAgent => {
            let (choice, p) = picked(answers, "harness")?;
            let harness: &'static Harness = choice
                .strip_prefix('h')
                .and_then(|n| n.parse::<usize>().ok())
                .and_then(|i| HARNESSES.get(i))
                .unwrap_or_else(|| {
                    HARNESSES
                        .iter()
                        .find(|h| h.name == "Claude Code")
                        .unwrap_or(&HARNESSES[0])
                });
            if choice != "unnamed" {
                confidence = confidence.min(p);
            }
            Command::StartAgent {
                space: to(),
                harness,
                task: text.unwrap_or_default(),
            }
        }
        Tool::ClosePane | Tool::CloseTab | Tool::CloseSpace => Command::Close(to()),
        Tool::RenameSpace | Tool::RenameTab | Tool::RenamePane => match text {
            Some(name) => Command::Rename { target: to(), name },
            None => {
                let target = to();
                let on_screen = snapshot
                    .current(target.kind)
                    .is_some_and(|i| snapshot.catalog[i] == target);
                if !on_screen {
                    return Err(format!(
                        "what should {} be called? say “rename … to <name>”",
                        target.full_label()
                    ));
                }
                Command::Act(match target.kind {
                    Kind::Space => Action::RenameSpace,
                    Kind::Tab => Action::RenameTab,
                    Kind::Pane => Action::RenamePane,
                })
            }
        },
        Tool::NewWorktree => Command::NewWorktree {
            space: to(),
            branch: text.map(|b| b.split_whitespace().collect::<Vec<_>>().join("-")),
        },
        Tool::RemoveWorktree => Command::RemoveWorktree(to()),
        Tool::LandWorktree => Command::LandWorktree(to()),
        Tool::View(part) => {
            let (choice, p) = picked(answers, "view_change")?;
            let change = VIEW_CHANGES
                .iter()
                .find(|(id, _, _)| *id == choice)
                .map_or(ViewChange::Toggle, |(_, change, _)| *change);
            // Toggling is the safe reading of an unsure answer: it can be
            // undone by saying the same thing again.
            if change != ViewChange::Toggle {
                confidence = confidence.min(p);
            }
            let space = part_space(part, to);
            Command::View {
                part,
                change,
                space,
            }
        }
    };
    Ok((command, confidence))
}

/// Read `snapshot.line` by asking `oracle` twice.
pub fn read(oracle: &dyn Oracle, snapshot: &Snapshot) -> Result<Reading, String> {
    let state = state(snapshot);

    // Route: which kind of thing, and, speculatively, which space, tab and pane.
    let route = oracle.ask(&state, &route_questions(snapshot))?;
    let categories = route
        .get("category")
        .and_then(|a| a.probabilities())
        .ok_or("Jev did not answer the category")?;
    let mut ranked: Vec<(Category, f64)> = Category::ALL
        .iter()
        .map(|c| (*c, categories.get(c.id()).copied().unwrap_or(0.0)))
        .collect();
    ranked.sort_by(|a, b| b.1.total_cmp(&a.1));
    let none = categories.get("none").copied().unwrap_or(0.0);
    if none > ranked[0].1 {
        return Err("Jev does not read that as something herdr can do".into());
    }
    let beam: Vec<(Category, f64)> = ranked
        .iter()
        .enumerate()
        .filter(|(i, (_, p))| *i == 0 || *p >= BEAM_FLOOR)
        .take(BEAM_WIDTH)
        .map(|(_, c)| *c)
        .collect();

    // Select and fill: which tool in each plausible category, and the
    // stretches of the line those tools would need.
    let words = tokens(&snapshot.line);
    let mut answers = oracle.ask(&state, &fill_questions(snapshot, &words, &beam))?;
    answers.extend(route);

    let mut scored: Vec<(Tool, f64)> = Vec::new();
    for (category, p_category) in &beam {
        let Some(tools) = answers
            .get(&format!("tool.{}", category.id()))
            .and_then(|a| a.probabilities())
        else {
            continue;
        };
        for tool in Tool::all()
            .into_iter()
            .filter(|t| t.category() == *category)
        {
            let p = tools.get(&tool.id()).copied().unwrap_or(0.0);
            scored.push((tool, p_category * p));
        }
    }
    scored.sort_by(|a, b| b.1.total_cmp(&a.1));

    // The best tool that can be built wins; one that cannot (a message with
    // no message in it) gives way to the next.
    let mut trouble = None;
    for (rank, (tool, p)) in scored.iter().enumerate() {
        if rank > 0 && *p < BEAM_FLOOR {
            break;
        }
        match build(snapshot, &words, &answers, *tool) {
            Ok((command, confidence)) => {
                let runner_up = scored
                    .iter()
                    .skip(rank + 1)
                    .find(|(_, q)| *q >= 0.2)
                    .map(|(t, _)| t.means().to_string());
                return Ok(Reading {
                    command,
                    confidence: confidence.min(*p),
                    careful: tool.careful(),
                    runner_up,
                });
            }
            Err(reason) => {
                trouble.get_or_insert(reason);
            }
        }
    }
    Err(trouble.unwrap_or_else(|| "Jev could not settle what that asks for".into()))
}

/// Words that may join two commands. They are left off both.
const JOINERS: &[&str] = &[
    "and",
    "then",
    "also",
    "after",
    "afterwards",
    "next",
    "finally",
    "plus",
    "lastly",
];
/// At most this many places are asked about when splitting a paragraph.
const MAX_BOUNDARIES: usize = 60;
/// How many words either side of a place the split question quotes.
const SPLIT_CONTEXT: usize = 10;

/// A place where one command could end and the next begin.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Boundary {
    /// Where the earlier command's last word ends.
    pub end: usize,
    /// Where the next command's first word begins, past any joining words.
    pub start: usize,
}

/// Every place in `line` where a new command could begin: after a comma,
/// semicolon, full stop, question mark, exclamation mark or line break, or
/// at a joining word such as `and` or `then`. Most such places are inside a
/// single command, such as an `and` within a message; Jev decides which are
/// real.
pub fn boundaries(line: &str) -> Vec<Boundary> {
    let words = tokens(line);
    let joiner = |i: usize| JOINERS.contains(&words[i].word.as_str());
    let mut found: Vec<Boundary> = Vec::new();
    for i in 1..words.len() {
        let gap = &line[words[i - 1].end..words[i].start];
        if !(joiner(i) || gap.contains([',', ';', '.', '!', '?', '\n'])) {
            continue;
        }
        let mut first = i;
        while first < words.len() && joiner(first) {
            first += 1;
        }
        // A joining word at the very start of the paragraph, or a run of
        // them with nothing after, starts nothing.
        if first >= words.len() || joiner(i - 1) {
            continue;
        }
        let boundary = Boundary {
            end: words[i - 1].end,
            start: words[first].start,
        };
        if found.last().is_none_or(|b| b.start != boundary.start) {
            found.push(boundary);
        }
    }
    found.truncate(MAX_BOUNDARIES);
    found
}

/// The words around a place, for quoting in a question.
fn around(line: &str, at: usize, before: bool) -> String {
    let words = tokens(line);
    let picked: Vec<&Token> = if before {
        let upto: Vec<&Token> = words.iter().filter(|t| t.end <= at).collect();
        upto[upto.len().saturating_sub(SPLIT_CONTEXT)..].to_vec()
    } else {
        words
            .iter()
            .filter(|t| t.start >= at)
            .take(SPLIT_CONTEXT)
            .collect()
    };
    match (picked.first(), picked.last()) {
        (Some(first), Some(last)) => line[first.start..last.end].to_string(),
        _ => String::new(),
    }
}

/// Cut `line` into the commands it holds, in order, by asking whether a new
/// command begins at each [`boundary`](boundaries). A paragraph with no such
/// place is one command and needs no request.
pub fn split(oracle: &dyn Oracle, line: &str) -> Result<Vec<String>, String> {
    let candidates = boundaries(line);
    if candidates.is_empty() {
        return Ok(vec![line.trim().to_string()]);
    }
    let mut questions = Questions::new();
    for (k, b) in candidates.iter().enumerate() {
        questions.insert(
            format!("b{k}"),
            Question::Noul {
                instructions: json!({
                    "question": "`request` was typed into herdr, a terminal workspace manager, and may hold several separate commands for it, one after another. Does a new, separate command begin with `next`, right after `before`? Words that are part of a message, question or task being handed to an agent belong to that message, even when joined by `and` or `then`. A new command switches somewhere, starts, closes or renames something, shows, hides or folds a part of herdr's interface, or addresses a different pane or agent.",
                    "before": around(line, b.end, true),
                    "next": around(line, b.start, false),
                }),
            },
        );
    }
    let answers = oracle.ask(&json!({ "request": line }), &questions)?;
    let mut clauses = Vec::new();
    let mut from = 0;
    for (k, b) in candidates.iter().enumerate() {
        let yes = answers
            .get(&format!("b{k}"))
            .and_then(|a| a.yes())
            .ok_or_else(|| format!("Jev did not answer b{k}"))?;
        if yes >= 0.5 {
            clauses.push(&line[from..b.end]);
            from = b.start;
        }
    }
    clauses.push(&line[from..]);
    Ok(clauses
        .into_iter()
        .map(|c| c.trim().trim_end_matches([',', ';']).trim().to_string())
        .filter(|c| !c.is_empty())
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commander::jev::Answer;
    use crate::layout::PaneId;
    use std::cell::RefCell;

    fn entry(kind: Kind, ws: usize, tab: usize, p: u32, label: &str, locality: u8) -> Entry {
        Entry {
            kind,
            ws,
            tab,
            pane: Some(PaneId::from_raw(p)),
            label: label.into(),
            context: None,
            names: vec![super::super::intent::normalize(label)],
            locality,
        }
    }

    fn snapshot(line: &str) -> Snapshot {
        Snapshot {
            line: line.into(),
            earlier: Vec::new(),
            made: None,
            catalog: vec![
                entry(Kind::Space, 0, 0, 1, "herdr", 1),
                entry(Kind::Space, 1, 0, 3, "chat-ui", 0),
                entry(Kind::Tab, 0, 0, 1, "main", 2),
                entry(Kind::Tab, 1, 0, 3, "server", 0),
                entry(Kind::Pane, 0, 0, 1, "Ada (claude)", 2),
                entry(Kind::Pane, 1, 0, 3, "Cleo (codex)", 0),
            ],
        }
    }

    fn choice(pick: &str, p: f64) -> Answer {
        let mut probabilities = BTreeMap::new();
        probabilities.insert(pick.to_string(), p);
        probabilities.insert("other".to_string(), 1.0 - p);
        Answer::Choice {
            choice: pick.into(),
            probabilities,
            confidence: p,
        }
    }

    /// Answers each question from a script, or with the first option it was
    /// offered, and records every request.
    struct Script {
        answers: Vec<(&'static str, Answer)>,
        asked: RefCell<Vec<Questions>>,
    }

    impl Oracle for Script {
        fn ask(&self, _state: &Value, questions: &Questions) -> Result<Answers, String> {
            self.asked.borrow_mut().push(questions.clone());
            Ok(questions
                .iter()
                .map(|(id, q)| {
                    let scripted = self
                        .answers
                        .iter()
                        .find(|(k, _)| k == id)
                        .map(|(_, a)| a.clone());
                    let answer = scripted.unwrap_or_else(|| match q {
                        Question::Noul { .. } => Answer::Noul { noul: 0.0 },
                        Question::Choice { criteria, .. } => {
                            let first = if criteria.contains_key("current") {
                                "current"
                            } else {
                                criteria.keys().next().unwrap()
                            };
                            choice(first, 0.9)
                        }
                    });
                    (id.clone(), answer)
                })
                .collect())
        }
    }

    fn script(answers: Vec<(&'static str, Answer)>) -> Script {
        Script {
            answers,
            asked: RefCell::new(Vec::new()),
        }
    }

    #[test]
    fn a_message_is_cut_from_the_line_as_typed() {
        let oracle = script(vec![
            ("category", choice("message", 0.95)),
            ("target.pane", choice("e5", 0.9)),
            ("tool.message", choice("prompt", 0.97)),
            ("span.message", choice("s2_4", 0.85)),
        ]);
        let reading = read(&oracle, &snapshot("tell cleo Run the tests!")).unwrap();
        match reading.command {
            Command::Send { to, text } => {
                assert_eq!(to.label, "Cleo (codex)");
                assert_eq!(text, "Run the tests!");
            }
            other => panic!("{other:?}"),
        }
        assert!((reading.confidence - 0.85).abs() < 1e-9);
        assert!(!reading.careful);
    }

    #[test]
    fn the_second_step_asks_only_about_plausible_categories() {
        let mut categories = BTreeMap::new();
        categories.insert("create".to_string(), 0.6);
        categories.insert("worktree".to_string(), 0.3);
        categories.insert("close".to_string(), 0.1);
        let oracle = script(vec![(
            "category",
            Answer::Choice {
                choice: "create".into(),
                probabilities: categories,
                confidence: 0.5,
            },
        )]);
        let _ = read(&oracle, &snapshot("new worktree for herdr"));
        let asked = oracle.asked.borrow();
        assert_eq!(asked.len(), 2);
        assert!(asked[1].contains_key("tool.create"));
        assert!(asked[1].contains_key("tool.worktree"));
        assert!(!asked[1].contains_key("tool.close"));
        assert!(asked[1].contains_key("harness"));
        assert!(asked[1].contains_key("span.branch"));
    }

    #[test]
    fn a_close_runner_up_category_can_win() {
        // Create is likelier, but within it nothing fits well, while worktree
        // is sure of its tool: 0.55 × 0.3 < 0.45 × 0.95.
        let mut categories = BTreeMap::new();
        categories.insert("create".to_string(), 0.55);
        categories.insert("worktree".to_string(), 0.45);
        let mut create = BTreeMap::new();
        create.insert("newtab".to_string(), 0.3);
        create.insert("newspace".to_string(), 0.3);
        create.insert("startagent".to_string(), 0.4);
        let oracle = script(vec![
            (
                "category",
                Answer::Choice {
                    choice: "create".into(),
                    probabilities: categories,
                    confidence: 0.2,
                },
            ),
            (
                "tool.create",
                Answer::Choice {
                    choice: "startagent".into(),
                    probabilities: create,
                    confidence: 0.1,
                },
            ),
            ("tool.worktree", choice("newworktree", 0.95)),
            ("span.branch", choice("s2_3", 0.9)),
        ]);
        let reading = read(&oracle, &snapshot("worktree on fix login")).unwrap();
        assert_eq!(
            reading.command,
            Command::NewWorktree {
                space: snapshot("").catalog[0].clone(),
                branch: Some("fix-login".into()),
            }
        );
        assert!(reading.runner_up.is_some());
    }

    #[test]
    fn a_message_to_a_space_goes_to_its_focused_pane() {
        let oracle = script(vec![
            ("category", choice("message", 0.95)),
            ("target.space", choice("e1", 0.9)),
            ("tool.message", choice("prompt", 0.97)),
            ("span.message", choice("s3_3", 0.9)),
        ]);
        let reading = read(&oracle, &snapshot("tell chat ui rebuild")).unwrap();
        assert!(
            matches!(reading.command, Command::Send { ref to, ref text } if to.label == "Cleo (codex)" && text == "rebuild")
        );
    }

    #[test]
    fn folding_a_space_names_the_part_the_way_and_the_space() {
        let oracle = script(vec![
            ("category", choice("interface", 0.95)),
            ("target.space", choice("e1", 0.9)),
            ("tool.interface", choice("view_spaceagents", 0.9)),
            ("view_change", choice("hide", 0.9)),
        ]);
        let reading = read(&oracle, &snapshot("collapse the agents under chat-ui")).unwrap();
        assert!(!reading.careful);
        assert!(matches!(
            reading.command,
            Command::View {
                part: ViewPart::SpaceAgents,
                change: ViewChange::Hide,
                space: Some(ref e),
            } if e.label == "chat-ui"
        ));
    }

    #[test]
    fn a_part_that_is_not_per_space_names_none() {
        let oracle = script(vec![
            ("category", choice("interface", 0.95)),
            ("tool.interface", choice("view_minimap", 0.9)),
            ("view_change", choice("show", 0.9)),
        ]);
        let reading = read(&oracle, &snapshot("show the minimaps")).unwrap();
        assert_eq!(
            reading.command,
            Command::View {
                part: ViewPart::Minimap,
                change: ViewChange::Show,
                space: None,
            }
        );
    }

    #[test]
    fn closing_waits_for_a_second_enter() {
        let oracle = script(vec![
            ("category", choice("close", 0.95)),
            ("target.tab", choice("e3", 0.9)),
            ("tool.close", choice("closetab", 0.97)),
        ]);
        let reading = read(&oracle, &snapshot("close the server tab")).unwrap();
        assert!(reading.careful);
        assert!(matches!(reading.command, Command::Close(ref e) if e.label == "server"));
    }

    #[test]
    fn starting_an_agent_names_the_harness_and_task() {
        let claude = HARNESSES
            .iter()
            .position(|h| h.name == "Claude Code")
            .unwrap();
        let harness_key: &'static str = Box::leak(format!("h{claude}").into_boxed_str());
        let oracle = script(vec![
            ("category", choice("create", 0.95)),
            ("tool.create", choice("startagent", 0.9)),
            ("harness", choice(harness_key, 0.9)),
            ("span.task", choice("s5_7", 0.9)),
        ]);
        let reading = read(&oracle, &snapshot("start claude in herdr to fix the build")).unwrap();
        match reading.command {
            Command::StartAgent {
                space,
                harness,
                task,
            } => {
                assert_eq!(space.label, "herdr");
                assert_eq!(harness.name, "Claude Code");
                assert_eq!(task, "fix the build");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_long_line_asks_for_each_end_apart() {
        let long = "tell cleo ".to_string() + &"word ".repeat(30);
        let mut questions = Questions::new();
        let words = tokens(&long);
        text_questions(&mut questions, &long, &words, Text::Message);
        assert!(questions.contains_key("start.message"));
        assert!(!questions.contains_key("span.message"));
        let short = "tell cleo run it";
        let mut questions = Questions::new();
        text_questions(&mut questions, short, &tokens(short), Text::Name);
        let Question::Choice { criteria, .. } = &questions["span.name"] else {
            panic!("not a choice");
        };
        assert_eq!(criteria.len(), 10 + 1);
        assert_eq!(criteria["s2_3"], json!("“run it”"));
    }

    #[test]
    fn current_and_the_entry_on_screen_count_as_one() {
        let mut probabilities = BTreeMap::new();
        probabilities.insert("current".to_string(), 0.5);
        probabilities.insert("e2".to_string(), 0.45);
        probabilities.insert("e3".to_string(), 0.05);
        let oracle = script(vec![
            ("category", choice("close", 0.95)),
            ("tool.close", choice("closetab", 0.97)),
            (
                "target.tab",
                Answer::Choice {
                    choice: "current".into(),
                    probabilities,
                    confidence: 0.4,
                },
            ),
        ]);
        let reading = read(&oracle, &snapshot("close this tab")).unwrap();
        assert!(matches!(reading.command, Command::Close(ref e) if e.label == "main"));
        assert!(
            (reading.confidence - 0.92).abs() < 0.01,
            "{}",
            reading.confidence
        );
    }

    #[test]
    fn a_name_that_matches_nothing_is_refused() {
        let oracle = script(vec![
            ("category", choice("message", 0.95)),
            ("target.pane", choice("missing", 0.9)),
            ("tool.message", choice("prompt", 0.97)),
            ("span.message", choice("s2_3", 0.9)),
        ]);
        let err = read(&oracle, &snapshot("tell Zed echo hi")).unwrap_err();
        assert!(err.contains("no pane"), "{err}");
    }

    #[test]
    fn a_plain_action_needs_nothing_more() {
        let oracle = script(vec![
            ("category", choice("layout", 0.95)),
            ("tool.layout", choice("splitdown", 0.9)),
        ]);
        let reading = read(&oracle, &snapshot("put a terminal underneath")).unwrap();
        assert_eq!(reading.command, Command::Act(Action::SplitDown));
    }

    #[test]
    fn something_herdr_cannot_do_is_refused() {
        let mut categories = BTreeMap::new();
        categories.insert("none".to_string(), 0.8);
        categories.insert("message".to_string(), 0.2);
        let oracle = script(vec![(
            "category",
            Answer::Choice {
                choice: "none".into(),
                probabilities: categories,
                confidence: 0.6,
            },
        )]);
        assert!(read(&oracle, &snapshot("order me a pizza")).is_err());
        assert_eq!(oracle.asked.borrow().len(), 1);
    }

    #[test]
    fn places_to_split_are_joining_words_and_punctuation() {
        let line = "switch to herdr and then start claude. Also tell grok: check the PR";
        let found: Vec<&str> = boundaries(line)
            .iter()
            .map(|b| &line[b.start..])
            .map(|rest| rest.split(' ').next().unwrap())
            .collect();
        // A colon addresses a message; it does not end a command.
        assert_eq!(found, vec!["start", "tell"]);
        assert!(boundaries("split right").is_empty());
    }

    #[test]
    fn a_paragraph_is_cut_where_jev_says_a_command_begins() {
        let line = "Switch to the fifth space and add a Claude Code agent and then prompt it to audit this and that. And also tell the Grok agent to check the PR.";
        let candidates = boundaries(line);
        let starts: Vec<&str> = candidates
            .iter()
            .map(|b| line[b.start..].split([' ', '.']).next().unwrap())
            .collect();
        assert_eq!(starts, vec!["add", "prompt", "that", "tell"]);
        // Every place but the `and` inside the audit's message is a new command.
        let oracle = script(vec![
            ("b0", Answer::Noul { noul: 0.9 }),
            ("b1", Answer::Noul { noul: 0.8 }),
            ("b2", Answer::Noul { noul: 0.1 }),
            ("b3", Answer::Noul { noul: 0.95 }),
        ]);
        assert_eq!(
            split(&oracle, line).unwrap(),
            vec![
                "Switch to the fifth space",
                "add a Claude Code agent",
                "prompt it to audit this and that",
                "tell the Grok agent to check the PR.",
            ]
        );
    }

    #[test]
    fn a_single_command_needs_no_split_request() {
        let oracle = script(Vec::new());
        assert_eq!(split(&oracle, "zoom").unwrap(), vec!["zoom"]);
        assert!(oracle.asked.borrow().is_empty());
    }

    #[test]
    fn earlier_steps_and_the_new_pane_reach_the_state() {
        let mut snap = snapshot("prompt it to audit");
        snap.earlier.push(Done {
            said: "add a claude agent".into(),
            did: "start Claude Code in herdr".into(),
        });
        snap.made = Some(PaneId::from_raw(3));
        let state = state(&snap);
        assert_eq!(
            state["earlier_steps"][0]["did"],
            "start Claude Code in herdr"
        );
        assert_eq!(state["just_started"]["id"], "e5");
    }

    /// Reads lines with the real Jev and prints every answer, for tuning
    /// questions. Run with `TYPESAFE_API_KEY=… cargo test read_live -- --ignored --nocapture`.
    #[test]
    #[ignore = "reaches the network"]
    fn read_live() {
        struct Printing(crate::commander::jev::Jev);
        impl Oracle for Printing {
            fn ask(&self, state: &Value, questions: &Questions) -> Result<Answers, String> {
                let answers = self.0.ask(state, questions)?;
                for (id, answer) in &answers {
                    if let Answer::Choice { probabilities, .. } = answer {
                        let mut top: Vec<_> = probabilities.iter().collect();
                        top.sort_by(|a, b| b.1.total_cmp(a.1));
                        top.truncate(3);
                        println!("  {id}: {top:?}");
                    } else {
                        println!("  {id}: {answer:?}");
                    }
                }
                Ok(answers)
            }
        }
        let oracle = Printing(crate::commander::jev::Jev {
            api_key: std::env::var("TYPESAFE_API_KEY").expect("TYPESAFE_API_KEY"),
            model: "jev-latest".into(),
        });
        if let Ok(paragraph) = std::env::var("SPLIT_LINE") {
            println!("split: {:?}", split(&oracle, &paragraph));
        }
        let lines = std::env::var("READ_LINES").unwrap_or_else(|_| "tell Eve to echo beta".into());
        for line in lines.split('|') {
            let mut snap = snapshot(line);
            snap.catalog[4].label = "Ryan".into();
            snap.catalog[4].names = vec!["ryan".into(), "zsh".into()];
            snap.catalog[5] = Entry {
                ws: 0,
                tab: 0,
                locality: 2,
                label: "Eve".into(),
                names: vec!["eve".into(), "zsh".into()],
                ..snap.catalog[5].clone()
            };
            println!("{line}");
            let reading = read(&oracle, &snap);
            println!("=> {reading:?}\n");
        }
    }

    #[test]
    fn every_tool_has_a_distinct_id_within_its_category() {
        let tools = Tool::all();
        for category in Category::ALL {
            let ids: Vec<String> = tools
                .iter()
                .filter(|t| t.category() == category)
                .map(|t| t.id())
                .collect();
            let mut unique = ids.clone();
            unique.sort();
            unique.dedup();
            assert_eq!(ids.len(), unique.len(), "{category:?}");
            assert!(!ids.is_empty(), "{category:?} has no tools");
        }
    }
}
