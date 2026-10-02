//! Turning what was typed into the Commander into something herdr already does.
//!
//! There is no model here and nothing is guessed at random. A line is read in
//! a fixed order, and the first reading that fits wins:
//!
//! 1. **A message for a pane.** `send`, `tell`, `ask`, `prompt` or `message`,
//!    then who, then what: `tell Ada to run the tests`, `send claude in herdr:
//!    fix the build`. A line that starts with a name and a colon is a message
//!    too: `Ada: run the tests`. So is `send <text> to <who>`.
//! 2. **An action.** A short phrase naming one of herdr's own actions, such as
//!    `new tab`, `split right`, `zoom`, `next space` or `toggle sidebar`.
//!    Filler words (`the`, `a`, `please`, `this`) are ignored.
//! 3. **A place to go.** `switch to`, `go to`, `open`, `show` or `focus`,
//!    then a space, tab or pane, or just the name on its own.
//!
//! Names are matched against what herdr shows: a space's name, a tab's name,
//! and a pane's name, label, agent, or title. Case and punctuation do not
//! matter, so `chat ui` finds a space named `chat-ui`. A kind word narrows the
//! search (`the chat ui space`, `tab 2`, `the claude pane`), and `in` names
//! where to look (`claude in herdr`). When two things answer equally well, the
//! one nearer to what is on screen wins; when that still leaves a tie, the line
//! is refused with the candidates named, because acting on the wrong pane is
//! worse than asking again.

use crate::layout::PaneId;

/// The kinds of thing a name can point at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Space,
    Tab,
    Pane,
}

impl Kind {
    fn noun(self) -> &'static str {
        match self {
            Kind::Space => "space",
            Kind::Tab => "tab",
            Kind::Pane => "pane",
        }
    }
}

/// One thing a name can point at, and every name it answers to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub kind: Kind,
    pub ws: usize,
    /// The tab itself, or the tab the pane sits in, or a space's active tab.
    pub tab: usize,
    /// The pane itself, or for a space or tab, the pane that has its focus:
    /// that is where a message sent to a space or tab goes.
    pub pane: Option<PaneId>,
    /// What the Commander calls it when saying what it is about to do.
    pub label: String,
    /// Where it lives, said after the label: the space a tab or pane is in.
    pub context: Option<String>,
    /// Every name it answers to, already normalized by [`normalize`].
    pub names: Vec<String>,
    /// How near it is to what is on screen: 2 for the visible tab, 1 for the
    /// visible space, 0 elsewhere. Breaks ties between equally good names.
    pub locality: u8,
}

impl Entry {
    fn full_label(&self) -> String {
        match &self.context {
            Some(context) => format!("{} in {context}", self.label),
            None => self.label.clone(),
        }
    }
}

/// herdr's own actions that the Commander can name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    NewTab,
    NewSpace,
    SplitRight,
    SplitLeft,
    SplitDown,
    SplitUp,
    ClosePane,
    CloseTab,
    CloseSpace,
    Zoom,
    NextTab,
    PreviousTab,
    NextSpace,
    PreviousSpace,
    NextAgent,
    PreviousAgent,
    LastPane,
    FocusLeft,
    FocusRight,
    FocusUp,
    FocusDown,
    ToggleSidebar,
    ToggleAgentTable,
    RenameTab,
    RenameSpace,
    RenamePane,
    Settings,
    Help,
    Navigator,
    Composer,
    CopyMode,
    Resize,
    ReloadConfig,
}

/// Each action and the phrases that name it, after filler words are dropped.
const PHRASES: &[(Action, &[&str])] = &[
    (
        Action::NewTab,
        &["new tab", "open new tab", "create tab", "add tab"],
    ),
    (
        Action::NewSpace,
        &[
            "new space",
            "new workspace",
            "create space",
            "create workspace",
            "add space",
            "add workspace",
        ],
    ),
    (
        Action::SplitRight,
        &["split", "split right", "split vertical", "split vertically"],
    ),
    (Action::SplitLeft, &["split left"]),
    (
        Action::SplitDown,
        &[
            "split down",
            "split below",
            "split horizontal",
            "split horizontally",
        ],
    ),
    (Action::SplitUp, &["split up", "split above"]),
    (Action::ClosePane, &["close pane", "close", "kill pane"]),
    (Action::CloseTab, &["close tab"]),
    (Action::CloseSpace, &["close space", "close workspace"]),
    (
        Action::Zoom,
        &["zoom", "unzoom", "toggle zoom", "maximize", "zoom pane"],
    ),
    (Action::NextTab, &["next tab"]),
    (
        Action::PreviousTab,
        &["previous tab", "prev tab", "last tab"],
    ),
    (Action::NextSpace, &["next space", "next workspace"]),
    (
        Action::PreviousSpace,
        &[
            "previous space",
            "prev space",
            "previous workspace",
            "prev workspace",
        ],
    ),
    (Action::NextAgent, &["next agent"]),
    (Action::PreviousAgent, &["previous agent", "prev agent"]),
    (Action::LastPane, &["last pane", "previous pane", "back"]),
    (Action::FocusLeft, &["focus left", "left", "pane left"]),
    (Action::FocusRight, &["focus right", "right", "pane right"]),
    (
        Action::FocusUp,
        &["focus up", "up", "pane up", "pane above"],
    ),
    (
        Action::FocusDown,
        &["focus down", "down", "pane down", "pane below"],
    ),
    (
        Action::ToggleSidebar,
        &["toggle sidebar", "hide sidebar", "show sidebar", "sidebar"],
    ),
    (
        Action::ToggleAgentTable,
        &[
            "toggle agent table",
            "hide agent table",
            "show agent table",
            "agent table",
            "toggle agents",
        ],
    ),
    (Action::RenameTab, &["rename tab"]),
    (Action::RenameSpace, &["rename space", "rename workspace"]),
    (Action::RenamePane, &["rename pane"]),
    (
        Action::Settings,
        &["settings", "open settings", "preferences"],
    ),
    (
        Action::Help,
        &["help", "keybindings", "keys", "shortcuts", "show help"],
    ),
    (
        Action::Navigator,
        &["navigator", "open navigator", "session navigator"],
    ),
    (
        Action::Composer,
        &["composer", "start agent", "new agent", "open composer"],
    ),
    (Action::CopyMode, &["copy mode", "copy"]),
    (Action::Resize, &["resize", "resize mode", "resize pane"]),
    (Action::ReloadConfig, &["reload config", "reload"]),
];

impl Action {
    /// What the Commander says it is about to do.
    pub fn label(self) -> &'static str {
        match self {
            Action::NewTab => "open a new tab",
            Action::NewSpace => "open a new space",
            Action::SplitRight => "split the pane to the right",
            Action::SplitLeft => "split the pane to the left",
            Action::SplitDown => "split the pane downward",
            Action::SplitUp => "split the pane upward",
            Action::ClosePane => "close the focused pane",
            Action::CloseTab => "close this tab",
            Action::CloseSpace => "close this space",
            Action::Zoom => "toggle zoom",
            Action::NextTab => "go to the next tab",
            Action::PreviousTab => "go to the previous tab",
            Action::NextSpace => "go to the next space",
            Action::PreviousSpace => "go to the previous space",
            Action::NextAgent => "go to the next agent",
            Action::PreviousAgent => "go to the previous agent",
            Action::LastPane => "go back to the last pane",
            Action::FocusLeft => "focus the pane to the left",
            Action::FocusRight => "focus the pane to the right",
            Action::FocusUp => "focus the pane above",
            Action::FocusDown => "focus the pane below",
            Action::ToggleSidebar => "toggle the sidebar",
            Action::ToggleAgentTable => "toggle the agent table",
            Action::RenameTab => "rename this tab",
            Action::RenameSpace => "rename this space",
            Action::RenamePane => "rename the focused pane",
            Action::Settings => "open settings",
            Action::Help => "show keybindings",
            Action::Navigator => "open the navigator",
            Action::Composer => "open the composer",
            Action::CopyMode => "enter copy mode",
            Action::Resize => "enter resize mode",
            Action::ReloadConfig => "reload the config",
        }
    }
}

/// What a line means.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// Show a space or tab, or focus a pane.
    Go(Entry),
    /// Paste `text` into a pane and submit it.
    Send { to: Entry, text: String },
    /// Run one of herdr's actions.
    Act(Action),
}

impl Command {
    /// One line saying what submitting would do, shown under the field as it
    /// is typed so nothing happens that was not first said.
    pub fn describe(&self) -> String {
        match self {
            Command::Go(entry) => match entry.kind {
                Kind::Space => format!("switch to space {}", entry.label),
                Kind::Tab => format!("switch to tab {}", entry.full_label()),
                Kind::Pane => format!("focus {}", entry.full_label()),
            },
            Command::Send { to, text } => {
                let preview: String = text.chars().take(48).collect();
                let more = if text.chars().count() > 48 { "…" } else { "" };
                let who = match to.kind {
                    Kind::Pane => to.full_label(),
                    kind => format!("the focused pane of {} {}", kind.noun(), to.label),
                };
                format!("send to {who}: {preview}{more}")
            }
            Command::Act(action) => action.label().to_string(),
        }
    }
}

const SEND_VERBS: &[&str] = &["send", "tell", "ask", "prompt", "message", "msg"];
const GO_VERBS: &[&[&str]] = &[
    &["switch", "to"],
    &["go", "to"],
    &["jump", "to"],
    &["take", "me", "to"],
    &["bring", "up"],
    &["switch"],
    &["goto"],
    &["open"],
    &["show"],
    &["focus"],
    &["view"],
    &["select"],
];
const POLITE: &[&str] = &["please", "hey", "ok", "okay", "commander"];
const FILLER: &[&str] = &["the", "a", "an", "my", "our", "this", "that", "current"];
/// Words that may sit between who a message is for and what it says.
const CONNECTORS: &[&str] = &["to", "that", "and"];

/// Lowercase, with every run of punctuation or space turned into one space.
pub fn normalize(text: &str) -> String {
    words(text).join(" ")
}

fn words(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect()
}

/// A word of the line as typed, and where in the line it sits, so a message
/// can be cut out of the original text with its case and punctuation intact.
#[derive(Debug, Clone)]
struct Token {
    word: String,
    start: usize,
    end: usize,
}

fn tokens(text: &str) -> Vec<Token> {
    let mut out = Vec::new();
    let mut start = None;
    for (i, c) in text.char_indices() {
        if c.is_alphanumeric() {
            start.get_or_insert(i);
        } else if let Some(s) = start.take() {
            out.push(Token {
                word: text[s..i].to_lowercase(),
                start: s,
                end: i,
            });
        }
    }
    if let Some(s) = start {
        out.push(Token {
            word: text[s..].to_lowercase(),
            start: s,
            end: text.len(),
        });
    }
    out
}

/// Read one line. `Err` carries a sentence saying why it could not be read.
pub fn interpret(input: &str, catalog: &[Entry]) -> Result<Command, String> {
    let line = input.trim();
    if line.is_empty() {
        return Err(String::new());
    }
    let mut toks = tokens(line);
    while toks
        .first()
        .is_some_and(|t| POLITE.contains(&t.word.as_str()))
    {
        toks.remove(0);
    }
    if toks.is_empty() {
        return Err(String::new());
    }
    let body = &line[toks[0].start..];
    let toks = tokens(body);

    if let Some(command) = read_message(body, &toks, catalog)? {
        return Ok(command);
    }
    if let Some(action) = read_action(&toks) {
        return Ok(Command::Act(action));
    }
    let after_verb = GO_VERBS
        .iter()
        .find(|verb| {
            verb.len() < toks.len() && verb.iter().zip(&toks).all(|(v, t)| *v == t.word.as_str())
        })
        .map_or(0, |verb| verb.len());
    let target: Vec<&str> = toks[after_verb..].iter().map(|t| t.word.as_str()).collect();
    match resolve(&target, catalog, Purpose::Go) {
        Ok(entry) => Ok(Command::Go(entry)),
        Err(Unresolved::Ambiguous(message)) => Err(message),
        Err(Unresolved::Missing) if after_verb > 0 => Err(format!(
            "nothing is called “{}”",
            strip_noise(&target).join(" ")
        )),
        Err(Unresolved::Missing) => Err(
            "not a name or action herdr knows — try “switch to …”, “tell … to …”, or “split right”"
                .to_string(),
        ),
    }
}

/// A message for a pane, if the line is one. `Ok(None)` means it is not shaped
/// like a message; `Err` means it is, but who it is for could not be settled.
fn read_message(body: &str, toks: &[Token], catalog: &[Entry]) -> Result<Option<Command>, String> {
    // `Ada: run the tests` — a name, then a colon. Only a colon that comes
    // before any message text counts, so a colon inside a message is safe.
    if let Some(colon) = body.find(':') {
        let head: Vec<&str> = toks
            .iter()
            .take_while(|t| t.end <= colon)
            .map(|t| t.word.as_str())
            .collect();
        let head = match head.first() {
            Some(first) if SEND_VERBS.contains(first) => &head[1..],
            _ => &head[..],
        };
        let text = body[colon + 1..].trim();
        if !head.is_empty() && !text.is_empty() {
            match resolve(head, catalog, Purpose::Send) {
                Ok(to) => {
                    return Ok(Some(Command::Send {
                        to,
                        text: text.to_string(),
                    }))
                }
                Err(Unresolved::Ambiguous(message)) => return Err(message),
                Err(Unresolved::Missing) => {}
            }
        }
    }

    let Some(first) = toks.first() else {
        return Ok(None);
    };
    if !SEND_VERBS.contains(&first.word.as_str()) || toks.len() < 3 {
        return Ok(None);
    }

    // `tell Ada to run the tests` — the longest run of words after the verb
    // that names something is who; everything after it, less a connector, is
    // what.
    let mut ambiguity = None;
    for split in (2..toks.len()).rev() {
        let who: Vec<&str> = toks[1..split].iter().map(|t| t.word.as_str()).collect();
        match resolve(&who, catalog, Purpose::Send) {
            Ok(to) => {
                let mut rest = &toks[split..];
                if rest.len() > 1 && CONNECTORS.contains(&rest[0].word.as_str()) {
                    rest = &rest[1..];
                }
                let text = body[rest[0].start..].trim().to_string();
                return Ok(Some(Command::Send { to, text }));
            }
            Err(Unresolved::Ambiguous(message)) => {
                ambiguity.get_or_insert(message);
            }
            Err(Unresolved::Missing) => {}
        }
    }

    // `send run the tests to Ada` — who comes last, after the final `to`.
    for (index, tok) in toks.iter().enumerate().rev() {
        if tok.word != "to" || index < 2 || index + 1 >= toks.len() {
            continue;
        }
        let who: Vec<&str> = toks[index + 1..].iter().map(|t| t.word.as_str()).collect();
        match resolve(&who, catalog, Purpose::Send) {
            Ok(to) => {
                let text = body[toks[1].start..tok.start].trim().to_string();
                return Ok(Some(Command::Send { to, text }));
            }
            Err(Unresolved::Ambiguous(message)) => {
                ambiguity.get_or_insert(message);
            }
            Err(Unresolved::Missing) => {}
        }
    }

    Err(ambiguity
        .unwrap_or_else(|| format!("who should get this? nothing is called “{}”", toks[1].word)))
}

/// An action, if the line names one. The word `pane` is dropped from both the
/// line and the phrases before they are compared, because every action that
/// has no other object acts on the focused pane: `split the pane right` and
/// `split right` say the same thing.
fn read_action(toks: &[Token]) -> Option<Action> {
    let plain = |words: &mut dyn Iterator<Item = &str>| {
        words
            .filter(|w| !FILLER.contains(w) && !POLITE.contains(w) && *w != "pane")
            .collect::<Vec<_>>()
            .join(" ")
    };
    let phrase = plain(&mut toks.iter().map(|t| t.word.as_str()));
    if phrase.is_empty() {
        return None;
    }
    PHRASES
        .iter()
        .find(|(_, phrases)| phrases.iter().any(|p| plain(&mut p.split(' ')) == phrase))
        .map(|(action, _)| *action)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Purpose {
    Go,
    Send,
}

#[derive(Debug)]
enum Unresolved {
    Missing,
    Ambiguous(String),
}

fn kind_word(word: &str) -> Option<Kind> {
    match word {
        "space" | "workspace" | "project" => Some(Kind::Space),
        "tab" => Some(Kind::Tab),
        "pane" | "agent" | "terminal" | "shell" => Some(Kind::Pane),
        _ => None,
    }
}

fn strip_noise<'a>(words: &[&'a str]) -> Vec<&'a str> {
    let mut words: Vec<&str> = words
        .iter()
        .copied()
        .filter(|w| !FILLER.contains(w) && !POLITE.contains(w))
        .collect();
    while words.last().is_some_and(|w| kind_word(w).is_some()) {
        words.pop();
    }
    while words.len() > 1 && words.first().is_some_and(|w| kind_word(w).is_some()) {
        words.remove(0);
    }
    words
}

/// Find the one thing `words` names.
fn resolve(words: &[&str], catalog: &[Entry], purpose: Purpose) -> Result<Entry, Unresolved> {
    // `claude in herdr`: the part after the last `in` says where to look.
    if let Some(at) = words
        .iter()
        .rposition(|w| matches!(*w, "in" | "on" | "from"))
    {
        if at > 0 && at + 1 < words.len() {
            if let Ok(scope) = resolve(&words[at + 1..], catalog, Purpose::Go) {
                let inside: Vec<Entry> = catalog
                    .iter()
                    .filter(|e| {
                        e.ws == scope.ws
                            && (scope.kind != Kind::Tab || e.tab == scope.tab)
                            && e.kind as u8 > scope.kind as u8
                    })
                    .cloned()
                    .collect();
                return resolve(&words[..at], &inside, purpose);
            }
        }
    }

    // A kind word narrows the search when it ends the name (`the chat ui
    // space`) or leads it (`tab 2`).
    let meaningful: Vec<&str> = words
        .iter()
        .copied()
        .filter(|w| !FILLER.contains(w))
        .collect();
    let kind = meaningful.last().and_then(|w| kind_word(w)).or_else(|| {
        meaningful
            .first()
            .filter(|_| meaningful.len() > 1)
            .and_then(|w| kind_word(w))
    });
    let name = strip_noise(words).join(" ");
    if name.is_empty() {
        return Err(Unresolved::Missing);
    }
    // `tab 2` and `space 3` keep their kind word, because the number is only a
    // name together with it.
    let numbered = meaningful.join(" ");

    let mut best: Vec<(&Entry, (u8, u8, u8))> = Vec::new();
    for entry in catalog {
        if kind.is_some_and(|k| k != entry.kind) {
            continue;
        }
        let score = entry
            .names
            .iter()
            .map(|n| match_score(&name, n).max(match_score(&numbered, n)))
            .max()
            .unwrap_or(0);
        if score == 0 {
            continue;
        }
        let preference = match (purpose, entry.kind) {
            (Purpose::Send, Kind::Pane) | (Purpose::Go, Kind::Space) => 2,
            (_, Kind::Tab) => 1,
            _ => 0,
        };
        let key = (score, preference, entry.locality);
        match best.first().map(|(_, k)| *k) {
            Some(top) if key < top => {}
            Some(top) if key == top => best.push((entry, key)),
            _ => best = vec![(entry, key)],
        }
    }
    match best.as_slice() {
        [] => Err(Unresolved::Missing),
        [(entry, _)] => Ok((*entry).clone()),
        many => {
            let names: Vec<String> = many.iter().take(3).map(|(e, _)| e.full_label()).collect();
            let more = if many.len() > 3 { ", …" } else { "" };
            Err(Unresolved::Ambiguous(format!(
                "“{name}” could be {}{more}",
                names.join(", ")
            )))
        }
    }
}

/// How well a typed name fits one of an entry's names: 4 for the whole name, 3
/// for its leading words, 2 when every typed word starts a word of it in order,
/// 0 for no fit. Letters in the middle of a word do not count, so `run` does
/// not find `Bruno`.
fn match_score(typed: &str, name: &str) -> u8 {
    if typed.is_empty() || name.is_empty() {
        return 0;
    }
    if typed == name {
        return 4;
    }
    if name.starts_with(typed) && name[typed.len()..].starts_with(' ') {
        return 3;
    }
    let mut name_words = name.split(' ');
    if typed
        .split(' ')
        .all(|t| name_words.any(|n| n.starts_with(t)))
    {
        return 2;
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pane(n: u32) -> PaneId {
        PaneId::from_raw(n)
    }

    fn entry(kind: Kind, ws: usize, tab: usize, p: u32, label: &str, names: &[&str]) -> Entry {
        Entry {
            kind,
            ws,
            tab,
            pane: Some(pane(p)),
            label: label.into(),
            context: None,
            names: names.iter().map(|n| normalize(n)).collect(),
            locality: if ws == 0 { 1 } else { 0 },
        }
    }

    fn catalog() -> Vec<Entry> {
        vec![
            entry(Kind::Space, 0, 0, 1, "herdr", &["herdr", "1"]),
            entry(Kind::Space, 1, 0, 3, "chat-ui", &["chat-ui", "2"]),
            entry(Kind::Tab, 1, 1, 4, "server", &["server", "tab 2"]),
            entry(Kind::Pane, 0, 0, 1, "Ada", &["Ada", "claude"]),
            entry(Kind::Pane, 0, 0, 2, "Bruno", &["Bruno", "codex"]),
            entry(Kind::Pane, 1, 0, 3, "Cleo", &["Cleo", "claude"]),
            entry(Kind::Pane, 1, 1, 4, "Dante", &["Dante", "zsh"]),
        ]
    }

    #[test]
    fn switching_to_a_space_by_a_loose_name() {
        let cmd = interpret("switch to the chat UI space", &catalog()).unwrap();
        assert!(matches!(cmd, Command::Go(ref e) if e.kind == Kind::Space && e.ws == 1));
    }

    #[test]
    fn a_bare_name_goes_there() {
        let cmd = interpret("herdr", &catalog()).unwrap();
        assert!(matches!(cmd, Command::Go(ref e) if e.kind == Kind::Space && e.ws == 0));
    }

    #[test]
    fn a_numbered_tab() {
        let cmd = interpret("go to tab 2", &catalog()).unwrap();
        assert!(matches!(cmd, Command::Go(ref e) if e.kind == Kind::Tab && e.tab == 1));
    }

    #[test]
    fn telling_a_pane_keeps_the_message_as_typed() {
        let cmd = interpret("tell Ada to Run the tests, please!", &catalog()).unwrap();
        assert_eq!(
            cmd,
            Command::Send {
                to: catalog()[3].clone(),
                text: "Run the tests, please!".into()
            }
        );
    }

    #[test]
    fn a_name_and_a_colon_is_a_message() {
        let cmd = interpret("bruno: fix it: now", &catalog()).unwrap();
        assert!(
            matches!(cmd, Command::Send { ref to, ref text } if to.label == "Bruno" && text == "fix it: now")
        );
    }

    #[test]
    fn the_agent_nearer_the_screen_wins_a_tie() {
        // Two panes run claude; the one in the visible space is meant.
        let cmd = interpret("ask claude to summarize", &catalog()).unwrap();
        assert!(matches!(cmd, Command::Send { ref to, .. } if to.label == "Ada"));
    }

    #[test]
    fn in_names_where_to_look() {
        let cmd = interpret("send claude in chat ui: hello", &catalog()).unwrap();
        assert!(
            matches!(cmd, Command::Send { ref to, ref text } if to.label == "Cleo" && text == "hello")
        );
    }

    #[test]
    fn who_can_come_last() {
        let cmd = interpret("send run the build to dante", &catalog()).unwrap();
        assert!(
            matches!(cmd, Command::Send { ref to, ref text } if to.label == "Dante" && text == "run the build")
        );
    }

    #[test]
    fn a_message_to_a_space_goes_to_its_focused_pane() {
        let cmd = interpret("tell chat-ui to rebuild", &catalog()).unwrap();
        assert!(
            matches!(cmd, Command::Send { ref to, .. } if to.kind == Kind::Space && to.pane == Some(pane(3)))
        );
    }

    #[test]
    fn actions_ignore_filler() {
        assert_eq!(
            interpret("please split the pane right", &catalog()),
            Ok(Command::Act(Action::SplitRight))
        );
        assert_eq!(
            interpret("open a new tab", &catalog()),
            Ok(Command::Act(Action::NewTab))
        );
        assert_eq!(
            interpret("Next space.", &catalog()),
            Ok(Command::Act(Action::NextSpace))
        );
    }

    #[test]
    fn a_true_tie_is_refused_with_the_candidates() {
        let mut catalog = catalog();
        for e in &mut catalog {
            e.locality = 0;
        }
        let err = interpret("ask claude to summarize", &catalog).unwrap_err();
        assert!(err.contains("Ada") && err.contains("Cleo"), "{err}");
    }

    #[test]
    fn unknown_names_are_refused() {
        assert!(interpret("switch to nowhere", &catalog()).is_err());
        assert!(interpret("tell nobody to do it", &catalog()).is_err());
        assert!(interpret("", &catalog()).is_err());
    }
}
