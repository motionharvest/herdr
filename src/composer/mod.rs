//! The band across the top of the frame that starts agents.
//!
//! Four controls, read left to right: where to work, whether in a worktree,
//! who works, what to do. Type a path, `Enter`, type a task, `Enter`, and an
//! agent is running. An empty task still starts the chosen agent, with nothing
//! typed into it. Opening the directory copies the folder on show into the
//! field so it can be edited. While a path is being typed, the best match
//! fills in the letters not yet typed; `Tab` takes that guess and `Enter`
//! takes only what was typed. Settling a control hands the keyboard on to the
//! next: directory, agent, task. The worktree box sits between directory and
//! agent and is flipped by a click.
//!
//! Nothing here starts anything or draws anything. This is what the band holds
//! and what a key does to it; `app::composer` starts the agent and
//! `ui::composer` draws the controls.

pub mod field;
pub mod suggest;

use std::path::{Path, PathBuf};

use crate::harness::Harness;

pub use field::TextField;

/// Which control has the keyboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Folder,
    Agent,
    Task,
}

/// A folder the composer can start an agent in, and the label it is listed
/// under. The label is worked out once, when the folder is added, because it is
/// what the dropdown is measured by on every frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Folder {
    pub path: PathBuf,
    pub label: String,
}

impl Folder {
    pub fn new(path: PathBuf) -> Self {
        let label = crate::workspace::display_path_with_home(&path);
        Self { path, label }
    }
}

fn row_path(folder: &Folder) -> PathBuf {
    folder.path.clone()
}

/// The band's own state.
#[derive(Debug, Clone)]
pub struct ComposerState {
    /// The folders on offer, most recently used first.
    pub folders: Vec<Folder>,
    /// Which of them is on show. `None` means there are none yet, which is the
    /// only state in which the task field has nowhere to send anything.
    pub folder: Option<usize>,
    /// The agents this machine can start, read from `PATH` once. Reading it is
    /// a stat of every row against every entry on the path, which is far too
    /// much to do on a frame; a harness installed while herdr is running is
    /// picked up the next time herdr starts.
    harnesses: Vec<&'static Harness>,
    /// The agent on show, by name rather than by row number. The rows are what
    /// this machine can start, and a saved choice outlives the list it was
    /// made from: a name still means the same agent when the list has changed.
    pub agent: String,
    pub focus: Focus,
    /// Which dropdown is open, and which of its rows is pointed at. A dropdown
    /// is open or it is not; two cannot be.
    pub open: Option<Focus>,
    pub highlight: usize,
    /// The row currently under the mouse. This is separate from `highlight`:
    /// looking across the list should not move the keyboard's pointed row or
    /// change the value preview above it until an item is clicked.
    pub hover: Option<usize>,
    /// Whether the row pointed at is a row of the list rather than the path
    /// being typed. A path being typed is the value until the keyboard is
    /// moved down into what is offered under it, and typing another letter
    /// brings it back up: a row picked against the letters before this one was
    /// picked against a different question.
    pointed: bool,
    /// The path being typed. Opening the list copies the folder on show into
    /// it so that path can be edited; a letter on the closed control starts
    /// over. The folder on show is not replaced until the new path is settled.
    /// Every edit of it goes through [`ComposerState::edit_path`], which is
    /// what keeps `matches` answering the letters actually in it.
    path: TextField,
    /// The folders offered for what is being typed, best first. Worked out on
    /// each edit rather than where it is drawn, because working it out reads a
    /// directory and the drawing happens on every frame.
    matches: Vec<Folder>,
    pub task: TextField,
    /// True while the mouse is down in the task field, so a drag keeps
    /// extending the selection instead of falling through to the panes.
    pub selecting: bool,
    /// The letters typed at the agent control, matched against the front of
    /// the agents' names: `cl` is Claude Code, `co` is Codex. It grows a
    /// letter at a time and starts over on a letter that fits no name, so a
    /// stale run of letters never blocks a fresh one.
    typed_agent: String,
    /// Whether a named agent should start in a Herdr-managed worktree. The
    /// checkbox between Directory and Agent is the one control for it: checked
    /// is the default, because two agents editing one checkout overwrite each
    /// other.
    pub worktree: bool,
}

impl Default for ComposerState {
    fn default() -> Self {
        Self {
            folders: Vec::new(),
            folder: None,
            harnesses: crate::harness::installed(),
            agent: crate::harness::auto().name.to_string(),
            focus: Focus::Task,
            open: None,
            highlight: 0,
            hover: None,
            pointed: false,
            path: TextField::default(),
            matches: Vec::new(),
            task: TextField::default(),
            selecting: false,
            typed_agent: String::new(),
            worktree: true,
        }
    }
}

impl ComposerState {
    /// The agents this machine can start, which is what the dropdown lists.
    pub fn harnesses(&self) -> &[&'static Harness] {
        &self.harnesses
    }

    /// Restore a saved choice only when this machine can still start it. The
    /// saved value is a name, not a row, so changing the installed harness list
    /// cannot silently select a different agent.
    pub(crate) fn restore_agent(&mut self, name: &str) -> bool {
        let Some(harness) = self.harnesses.iter().find(|harness| {
            harness.name.eq_ignore_ascii_case(name)
                || harness
                    .program()
                    .is_some_and(|program| program.eq_ignore_ascii_case(name))
        }) else {
            return false;
        };
        self.agent = harness.name.to_string();
        true
    }

    /// Restore a saved folder only when it is still a directory. A path that
    /// has gone missing is dropped rather than shown as a broken choice. A
    /// linked worktree is offered as its parent, the same way a running pane
    /// in one is.
    pub(crate) fn restore_folder(&mut self, path: &Path) -> bool {
        let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        if !path.is_dir() {
            return false;
        }
        let Some(folder) = crate::workspace::composer_folder_path(&path) else {
            return false;
        };
        if !folder.is_dir() {
            return false;
        }
        self.add_folder(folder);
        true
    }

    /// A list of the tests' choosing, in place of what this machine happens to
    /// have installed.
    #[cfg(test)]
    pub(crate) fn use_harnesses(&mut self, harnesses: Vec<&'static Harness>) {
        self.harnesses = harnesses;
    }

    /// The agent on show. With nothing installed there is none, and the band
    /// says so rather than offering a row that would fail to start.
    ///
    /// A name that is no longer installed falls back to the first row that is,
    /// because a band that cannot start anything is worse than one that starts
    /// something else and says which.
    pub fn harness(&self) -> Option<&'static Harness> {
        self.harnesses
            .iter()
            .find(|harness| harness.name == self.agent)
            .or_else(|| self.harnesses.first())
            .copied()
    }

    /// Which row of the dropdown the agent on show is, for pointing at it.
    pub fn agent_row(&self) -> usize {
        self.harnesses
            .iter()
            .position(|harness| harness.name == self.agent)
            .unwrap_or(0)
    }

    /// What the task field writes in front of the task, given the agent on
    /// show. Only `Auto` writes anything.
    pub fn task_prefix(&self) -> &'static str {
        self.harness().map_or("", |harness| harness.prefix)
    }

    /// The path being typed. It is read in several places and written in one,
    /// which is [`ComposerState::edit_path`].
    pub fn path(&self) -> &TextField {
        &self.path
    }

    /// Change the path being typed. Every edit goes through here so that what
    /// is offered under the field can never be answering older letters than the
    /// ones on show, and so that a fresh letter takes the keyboard back out of
    /// the list it may have been moved into.
    pub fn edit_path<T>(&mut self, edit: impl FnOnce(&mut TextField) -> T) -> T {
        let before = self.path.text();
        let outcome = edit(&mut self.path);
        if self.path.text() == before {
            return outcome;
        }
        self.recompute_matches();
        self.pointed = false;
        self.highlight = if self.path.is_empty() {
            self.folder.unwrap_or(0)
        } else {
            0
        };
        outcome
    }

    fn recompute_matches(&mut self) {
        self.matches = suggest::matching(&self.path.text(), &self.folders);
    }

    /// The rows the folder list is showing: what is offered for the path being
    /// typed, or the folders already known when nothing is being typed — and
    /// when the field still holds the folder already on show, because opening
    /// the list to edit that path is not a search yet.
    pub fn folder_rows(&self) -> &[Folder] {
        if self.path.is_empty() || self.path_holds_the_chosen_folder() {
            &self.folders
        } else {
            &self.matches
        }
    }

    /// How many of those rows the box itself holds. A path being typed keeps
    /// five on show so the list can be read without looking away; the rest are
    /// reached by pointing past the last visible row. The remembered folders
    /// still fill the box, because that list is the whole answer.
    pub fn folder_visible_rows(&self) -> usize {
        let count = self.folder_rows().len();
        if self.path.is_empty() || self.path_holds_the_chosen_folder() {
            count
        } else {
            count.min(suggest::MOST)
        }
    }

    /// Whether the path field still holds the folder already chosen. Opening
    /// the list copies that label in so it can be edited, and until it changes
    /// the remembered folders are the list, not a search for those letters.
    pub fn path_holds_the_chosen_folder(&self) -> bool {
        self.folder_label()
            .is_some_and(|label| self.path.text() == label)
    }

    /// The letters of the best match that have not been typed yet. Drawn after
    /// the cursor, muted, and not in the field: Tab takes them, Enter does not.
    pub fn ghost_suffix(&self) -> Option<String> {
        if self.open != Some(Focus::Folder) || self.pointing() || !self.path.at_end() {
            return None;
        }
        let typed = self.path.text();
        if typed.is_empty() {
            return None;
        }
        let top = self.matches.first()?;
        ghost_after(&typed, &top.label)
    }

    /// Whether a row of the open list is the thing being pointed at. It is not,
    /// while a path is being typed and the keyboard has not been moved down
    /// into what is offered — there the typed text is the value, and lighting a
    /// row would say otherwise.
    pub fn pointing(&self) -> bool {
        match self.open {
            Some(Focus::Folder) => self.path.is_empty() || self.pointed,
            Some(_) => true,
            None => false,
        }
    }

    pub fn folder_path(&self) -> Option<&Path> {
        self.folder
            .and_then(|index| self.folders.get(index))
            .map(|folder| folder.path.as_path())
    }

    pub fn folder_label(&self) -> Option<&str> {
        self.folder
            .and_then(|index| self.folders.get(index))
            .map(|folder| folder.label.as_str())
    }

    /// The width the widest folder row wants, which is what the folder control
    /// is drawn at when there is room for it.
    pub fn folder_row_width(&self) -> usize {
        self.folder_rows()
            .iter()
            .map(|folder| folder.label.chars().count())
            .max()
            .unwrap_or(0)
    }

    /// Add a folder and put it on show, or move it to the front if it is
    /// already listed. A folder listed twice would be two ways to say the same
    /// place, and the list is short enough that the second one would be found
    /// as often as the first.
    pub fn add_folder(&mut self, path: PathBuf) {
        if let Some(at) = self.folders.iter().position(|folder| folder.path == path) {
            let folder = self.folders.remove(at);
            self.folders.insert(0, folder);
        } else {
            self.folders.insert(0, Folder::new(path));
        }
        self.folder = Some(0);
    }

    /// Replace the list, keeping whichever folder was on show if it survives.
    /// The list is rebuilt from the running panes whenever they change, and a
    /// rebuild that moved the chosen folder would retarget a half-typed task.
    pub fn set_folders(&mut self, paths: Vec<PathBuf>) {
        let showing = self.folder_path().map(Path::to_path_buf);
        self.folders = paths.into_iter().map(Folder::new).collect();
        self.folder = showing
            .and_then(|path| self.folders.iter().position(|folder| folder.path == path))
            .or_else(|| (!self.folders.is_empty()).then_some(0));
        self.recompute_matches();
    }

    /// The keyboard starts wherever the first thing left to do is. With no
    /// folder there is nothing a task could be sent to, so a task typed first
    /// would be a directory named after a sentence.
    pub fn start_where_the_work_is(&mut self) {
        self.focus = if self.folder.is_some() {
            Focus::Task
        } else {
            Focus::Folder
        };
    }

    /// How many rows the open dropdown has.
    pub fn item_count(&self) -> usize {
        match self.open {
            Some(Focus::Folder) => self.folder_rows().len(),
            Some(Focus::Agent) => self.harnesses.len(),
            _ => 0,
        }
    }

    pub fn open_dropdown(&mut self, which: Focus) {
        if which == Focus::Task {
            return;
        }
        self.focus = which;
        self.open = Some(which);
        self.hover = None;
        self.pointed = false;
        self.typed_agent.clear();
        self.highlight = match which {
            Focus::Folder => self.folder.unwrap_or(0),
            _ => self.agent_row(),
        };
        if which == Focus::Folder {
            if let Some(label) = self.folder_label().map(str::to_string) {
                self.path.set_text(&label);
                self.pointed = true;
            }
        }
    }

    pub fn close_dropdown(&mut self) {
        self.open = None;
        self.hover = None;
        self.pointed = false;
        self.edit_path(TextField::clear);
        self.typed_agent.clear();
    }

    /// Point at another row of the open dropdown, stopping at either end.
    ///
    /// While a path is being typed the field itself is above the first row, so
    /// the first press downward steps into the list and a press upward off the
    /// first row steps back out of it into the text.
    pub fn point(&mut self, delta: isize) {
        let count = self.item_count();
        if count == 0 {
            return;
        }
        let last = count as isize - 1;
        if self.open == Some(Focus::Folder) && !self.path.is_empty() {
            if !self.pointed {
                if delta > 0 {
                    self.pointed = true;
                    self.highlight = 0;
                }
                return;
            }
            let at = self.highlight as isize + delta;
            if at < 0 {
                self.pointed = false;
                self.highlight = 0;
            } else {
                self.highlight = at.min(last) as usize;
            }
            return;
        }
        self.highlight = (self.highlight as isize + delta).clamp(0, last) as usize;
    }

    /// Point at one row outright, which is what a click on it does.
    pub fn point_at(&mut self, index: usize) {
        self.highlight = index;
        self.pointed = true;
    }

    /// Take the row pointed at, close the dropdown, and hand the keyboard on to
    /// the next control — settling one is only ever the step before settling
    /// the next, and the last of them is the task.
    pub fn take_pointed(&mut self) {
        match self.open {
            Some(Focus::Folder) => {
                let _ = self.take_folder();
            }
            Some(Focus::Agent) => {
                if let Some(harness) = self.harnesses.get(self.highlight) {
                    self.agent = harness.name.to_string();
                }
                self.close_dropdown();
                self.focus = Focus::Task;
            }
            _ => {}
        }
    }

    /// Settle the directory: the row pointed at, or else the path typed, or
    /// else — for a path that names nothing yet — the best folder offered under
    /// it, so that a few letters and `Tab` are a whole answer.
    pub fn take_folder(&mut self) -> Result<(), PathError> {
        self.settle_pointed_or_typed(true)
    }

    /// Settle the directory from what was typed, ignoring the ghost. A row
    /// pointed at is still taken, because that is a choice, not a suggestion.
    pub fn take_typed_folder(&mut self) -> Result<(), PathError> {
        self.settle_pointed_or_typed(false)
    }

    fn settle_pointed_or_typed(&mut self, guess: bool) -> Result<(), PathError> {
        if self.pointing() {
            if let Some(path) = self.folder_rows().get(self.highlight).map(row_path) {
                self.settle_folder(path);
                return Ok(());
            }
        }
        match self.take_typed_path() {
            Ok(()) => Ok(()),
            Err(PathError::Empty) => Err(PathError::Empty),
            Err(PathError::NotADirectory(typed)) => {
                if guess {
                    if let Some(path) = self.matches.first().map(row_path) {
                        self.settle_folder(path);
                        return Ok(());
                    }
                }
                Err(PathError::NotADirectory(typed))
            }
        }
    }

    /// Put a folder on show and hand the keyboard to the agent, which is what
    /// every way of choosing one ends in. The path is resolved here, in the one
    /// place all of them pass through, so that a folder reached by typing it, by
    /// pointing at it, and by a link to it are the same folder rather than three.
    fn settle_folder(&mut self, path: PathBuf) {
        let path = path.canonicalize().unwrap_or(path);
        self.add_folder(path);
        self.close_dropdown();
        self.focus = Focus::Agent;
    }

    /// Change the value of a closed dropdown outright, without opening it,
    /// stopping at either end of the list.
    pub fn step(&mut self, which: Focus, delta: isize) {
        match which {
            Focus::Folder => {
                if self.folders.is_empty() {
                    return;
                }
                let last = self.folders.len() as isize - 1;
                let at = self.folder.unwrap_or(0) as isize + delta;
                self.folder = Some(at.clamp(0, last) as usize);
            }
            Focus::Agent => {
                if self.harnesses.is_empty() {
                    return;
                }
                self.typed_agent.clear();
                let last = self.harnesses.len() as isize - 1;
                let at = (self.agent_row() as isize + delta).clamp(0, last) as usize;
                self.agent = self.harnesses[at].name.to_string();
            }
            Focus::Task => {}
        }
    }

    /// Take a letter typed at the agent control and point at the agent it
    /// names: the letters typed so far, then this one, matched against the
    /// front of each name without regard to case. A letter the grown prefix
    /// cannot place starts a fresh one, and a letter no name starts with at
    /// all changes nothing. While the list is open the match moves the row
    /// pointed at; while it is closed the match is the value itself.
    pub fn type_agent(&mut self, letter: char) {
        let letter = letter.to_lowercase().to_string();
        let grown = format!("{}{letter}", self.typed_agent);
        for candidate in [grown, letter] {
            if let Some(row) = self.agent_starting_with(&candidate) {
                self.typed_agent = candidate;
                if self.open == Some(Focus::Agent) {
                    self.highlight = row;
                } else {
                    self.agent = self.harnesses[row].name.to_string();
                }
                return;
            }
        }
    }

    fn agent_starting_with(&self, prefix: &str) -> Option<usize> {
        self.harnesses
            .iter()
            .position(|harness| harness.name.to_lowercase().starts_with(prefix))
    }

    /// Settle the path that has been typed: it becomes a folder, goes on show,
    /// and the keyboard moves on to the agent, the next control on the row. A
    /// path that names nothing is left where it is, because the alternative is
    /// a folder that cannot be started in and no sign of why.
    pub fn take_typed_path(&mut self) -> Result<(), PathError> {
        let typed = self.path.text();
        let typed = typed.trim();
        if typed.is_empty() {
            return Err(PathError::Empty);
        }
        let path = expand_home(typed);
        if !path.is_dir() {
            return Err(PathError::NotADirectory(path));
        }
        self.settle_folder(path);
        Ok(())
    }

    /// What would be sent, prefix and all. An empty task still starts the
    /// chosen agent in the chosen folder, with nothing typed into it. `None`
    /// when there is nowhere to send it.
    pub fn pending(&self) -> Option<Pending> {
        Some(Pending {
            cwd: self.folder_path()?.to_path_buf(),
            harness: self.harness()?,
            task: self.task.text().trim().to_string(),
            worktree: self.worktree,
        })
    }
}

/// A task ready to be sent, with everything the sending needs settled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pending {
    pub cwd: PathBuf,
    pub harness: &'static Harness,
    /// What was typed, without the agent's prefix. The prefix belongs to the
    /// agent rather than to the task, so it is put on once, where the command
    /// line is built, instead of being carried around already attached.
    pub task: String,
    /// Whether this start should create a Herdr-managed worktree. Auto still
    /// does not, because Auto is a router rather than a new checkout.
    pub worktree: bool,
}

impl Pending {
    /// The text that actually reaches the agent, prefix and all.
    pub fn message(&self) -> String {
        self.harness.message(&self.task)
    }

    /// What starts for this task: an agent process or an interactive terminal.
    ///
    /// Herdr owns worktree creation, so harness-specific branching flags are
    /// never added here.
    pub fn launch(&self) -> crate::harness::Launch {
        self.harness.launch(&self.task, false)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathError {
    Empty,
    NotADirectory(PathBuf),
}

impl PathError {
    pub fn message(&self) -> String {
        match self {
            PathError::Empty => "type a path first".to_string(),
            PathError::NotADirectory(path) => format!("{} is not a folder", path.display()),
        }
    }
}

/// The untyped tail of `label` once `typed` is a prefix of it, or of its last
/// folder name. The last-name path is what makes `~/lab/h` complete to `erdr`
/// even when the field still holds a home-relative prefix the label does not.
fn ghost_after(typed: &str, label: &str) -> Option<String> {
    if let Some(rest) = strip_prefix_ignore_ascii_case(label, typed) {
        return (!rest.is_empty()).then(|| rest.to_string());
    }
    let typed_name = typed.rsplit('/').next().unwrap_or(typed);
    let label_name = label.rsplit('/').next().unwrap_or(label);
    let rest = strip_prefix_ignore_ascii_case(label_name, typed_name)?;
    (!rest.is_empty()).then(|| rest.to_string())
}

fn strip_prefix_ignore_ascii_case<'a>(full: &'a str, prefix: &str) -> Option<&'a str> {
    let mut consumed = 0usize;
    let mut chars = full.chars();
    for want in prefix.chars() {
        let got = chars.next()?;
        if !got.eq_ignore_ascii_case(&want) {
            return None;
        }
        consumed += got.len_utf8();
    }
    Some(&full[consumed..])
}

/// A leading `~` stands for the home directory, because a path field that does
/// not understand the character every shell does is a field that has to be
/// typed around.
fn expand_home(typed: &str) -> PathBuf {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    match (typed, home) {
        ("~", Some(home)) => home,
        (typed, Some(home)) => match typed.strip_prefix("~/") {
            Some(rest) => home.join(rest),
            None => PathBuf::from(typed),
        },
        (typed, None) => PathBuf::from(typed),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn composer_with_folders(paths: &[&str]) -> ComposerState {
        let mut composer = ComposerState::default();
        composer.set_folders(paths.iter().map(PathBuf::from).collect());
        composer
    }

    #[test]
    fn the_keyboard_starts_at_the_task_when_a_folder_is_already_chosen() {
        let mut composer = composer_with_folders(&["/tmp"]);
        composer.start_where_the_work_is();
        assert_eq!(composer.focus, Focus::Task);
    }

    #[test]
    fn the_keyboard_starts_at_the_folder_when_there_is_none() {
        let mut composer = ComposerState::default();
        composer.start_where_the_work_is();
        assert_eq!(composer.focus, Focus::Folder);
    }

    #[test]
    fn adding_a_folder_that_is_already_listed_moves_it_to_the_front() {
        let mut composer = composer_with_folders(&["/tmp", "/usr", "/var"]);
        composer.add_folder(PathBuf::from("/var"));
        let paths: Vec<_> = composer
            .folders
            .iter()
            .map(|folder| folder.path.display().to_string())
            .collect();
        assert_eq!(paths, ["/var", "/tmp", "/usr"]);
        assert_eq!(composer.folder, Some(0));
        assert_eq!(composer.folders.len(), 3, "and is not listed twice");
    }

    #[test]
    fn rebuilding_the_list_keeps_whichever_folder_was_on_show() {
        let mut composer = composer_with_folders(&["/tmp", "/usr"]);
        composer.folder = Some(1);
        composer.set_folders(vec![
            PathBuf::from("/var"),
            PathBuf::from("/tmp"),
            PathBuf::from("/usr"),
        ]);
        assert_eq!(composer.folder_path(), Some(Path::new("/usr")));
    }

    #[test]
    fn rebuilding_the_list_falls_back_to_the_first_when_the_folder_has_gone() {
        let mut composer = composer_with_folders(&["/tmp", "/usr"]);
        composer.folder = Some(1);
        composer.set_folders(vec![PathBuf::from("/var")]);
        assert_eq!(composer.folder_path(), Some(Path::new("/var")));
    }

    #[test]
    fn an_empty_list_leaves_nothing_on_show() {
        let mut composer = composer_with_folders(&["/tmp"]);
        composer.set_folders(Vec::new());
        assert_eq!(composer.folder, None);
        assert_eq!(composer.folder_path(), None);
    }

    #[test]
    fn pointing_stops_at_either_end_of_the_open_dropdown() {
        let mut composer = composer_with_folders(&["/tmp", "/usr", "/var"]);
        composer.open_dropdown(Focus::Folder);
        assert_eq!(composer.highlight, 0);
        composer.point(-1);
        assert!(
            !composer.pointing(),
            "up off the first row is back to the path"
        );
        composer.point(5);
        assert!(composer.pointing());
        assert_eq!(
            composer.highlight, 0,
            "down from the field enters the first row"
        );
        composer.point(5);
        assert_eq!(composer.highlight, 2, "and the last row is the floor");
    }

    #[test]
    fn taking_a_folder_row_settles_it_and_hands_the_keyboard_to_the_agent() {
        let mut composer = composer_with_folders(&["/tmp", "/usr"]);
        composer.open_dropdown(Focus::Folder);
        composer.point(1);
        composer.take_pointed();
        assert_eq!(composer.folder_path(), Some(Path::new("/usr")));
        assert_eq!(composer.open, None);
        assert_eq!(composer.focus, Focus::Agent);
    }

    #[test]
    fn taking_an_agent_row_hands_the_keyboard_to_the_task() {
        let mut composer = composer_with_folders(&["/tmp"]);
        composer.open_dropdown(Focus::Agent);
        composer.take_pointed();
        assert_eq!(composer.open, None);
        assert_eq!(composer.focus, Focus::Task);
    }

    fn composer_with_named_agents() -> ComposerState {
        let mut composer = ComposerState::default();
        composer.use_harnesses(
            ["Auto", "Claude Code", "Codex", "Cursor"]
                .into_iter()
                .map(|name| crate::harness::named(name).unwrap())
                .collect(),
        );
        composer.focus = Focus::Agent;
        composer
    }

    fn unique_temp_dir(name: &str) -> PathBuf {
        let unique = format!(
            "herdr-composer-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let path = std::env::temp_dir().join(unique);
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn restoring_an_agent_choice_uses_its_name_not_its_old_row() {
        let mut composer = composer_with_named_agents();

        assert!(composer.restore_agent("Codex"));

        assert_eq!(composer.agent, "Codex");
        assert_eq!(composer.agent_row(), 2);
    }

    #[test]
    fn restoring_an_unavailable_agent_keeps_the_current_choice() {
        let mut composer = composer_with_named_agents();

        assert!(!composer.restore_agent("Gemini"));

        assert_eq!(composer.agent, "Auto");
    }

    #[test]
    fn restoring_a_folder_puts_it_on_show() {
        let folder = unique_temp_dir("restore-folder");
        let mut composer = ComposerState::default();

        assert!(composer.restore_folder(&folder));

        assert_eq!(
            composer
                .folder_path()
                .and_then(|path| path.canonicalize().ok()),
            Some(folder.canonicalize().unwrap())
        );
        let _ = std::fs::remove_dir_all(folder);
    }

    #[test]
    fn restoring_a_missing_folder_keeps_nothing_on_show() {
        let mut composer = ComposerState::default();

        assert!(
            !composer.restore_folder(Path::new("/this/does/not/exist-for-herdr-composer-restore"))
        );

        assert_eq!(composer.folder_path(), None);
        assert_eq!(composer.folder, None);
    }

    #[test]
    fn typing_at_the_closed_agent_control_selects_by_the_front_of_the_name() {
        let mut composer = composer_with_named_agents();
        composer.type_agent('c');
        assert_eq!(composer.agent, "Claude Code");
        composer.type_agent('o');
        assert_eq!(composer.agent, "Codex", "the letters grow the prefix");
        assert_eq!(composer.open, None, "and the list stays closed");
    }

    #[test]
    fn capital_letters_select_the_same_agent() {
        let mut composer = composer_with_named_agents();
        composer.type_agent('C');
        composer.type_agent('U');
        assert_eq!(composer.agent, "Cursor");
    }

    #[test]
    fn a_letter_the_grown_prefix_cannot_place_starts_a_fresh_one() {
        let mut composer = composer_with_named_agents();
        composer.type_agent('c');
        composer.type_agent('l');
        assert_eq!(composer.agent, "Claude Code");
        composer.type_agent('c');
        composer.type_agent('o');
        assert_eq!(composer.agent, "Codex", "the stale letters did not block");
    }

    #[test]
    fn a_letter_no_name_starts_with_changes_nothing() {
        let mut composer = composer_with_named_agents();
        composer.type_agent('c');
        composer.type_agent('z');
        assert_eq!(composer.agent, "Claude Code");
    }

    #[test]
    fn typing_at_the_open_agent_list_points_rather_than_settles() {
        let mut composer = composer_with_named_agents();
        composer.open_dropdown(Focus::Agent);
        composer.type_agent('c');
        composer.type_agent('o');
        assert_eq!(composer.harnesses()[composer.highlight].name, "Codex");
        assert_eq!(
            composer.agent, "Auto",
            "nothing is settled until it is taken"
        );
        composer.take_pointed();
        assert_eq!(composer.agent, "Codex");
    }

    #[test]
    fn stepping_the_agent_forgets_the_letters_typed_so_far() {
        let mut composer = composer_with_named_agents();
        composer.type_agent('c');
        composer.type_agent('l');
        composer.step(Focus::Agent, 1);
        composer.type_agent('a');
        assert_eq!(
            composer.agent, "Auto",
            "the a began a prefix of its own rather than growing cla"
        );
    }

    #[test]
    fn a_closed_dropdown_changes_its_value_outright() {
        let mut composer = composer_with_folders(&["/tmp", "/usr"]);
        composer.step(Focus::Folder, 1);
        assert_eq!(composer.folder_path(), Some(Path::new("/usr")));
        assert_eq!(composer.open, None, "and stays closed");
    }

    #[test]
    fn a_typed_path_has_to_name_a_folder() {
        let mut composer = ComposerState::default();
        composer.edit_path(|path| path.set_text("/definitely/not/here"));
        assert!(matches!(
            composer.take_typed_path(),
            Err(PathError::NotADirectory(_))
        ));
        assert_eq!(composer.folder, None, "and nothing is added");

        composer.edit_path(TextField::clear);
        assert_eq!(composer.take_typed_path(), Err(PathError::Empty));
    }

    #[test]
    fn a_typed_path_becomes_the_folder_on_show() {
        let mut composer = ComposerState::default();
        composer.edit_path(|path| path.set_text("/tmp"));
        assert_eq!(composer.take_typed_path(), Ok(()));
        assert!(composer.folder_path().is_some());
        assert_eq!(composer.focus, Focus::Agent);
    }

    #[test]
    fn a_leading_tilde_stands_for_home() {
        let home = std::env::var_os("HOME").map(PathBuf::from);
        assert_eq!(expand_home("~"), home.clone().unwrap_or_default());
        assert_eq!(
            expand_home("~/lab"),
            home.unwrap_or_default().join("lab"),
            "and joins what follows it"
        );
        assert_eq!(expand_home("/tmp"), PathBuf::from("/tmp"));
        assert_eq!(
            expand_home("~notauser/x"),
            PathBuf::from("~notauser/x"),
            "another user's home is not something to guess at"
        );
    }

    fn folders_on_disk(name: &str) -> PathBuf {
        let unique = format!(
            "herdr-composer-tests-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let root = std::env::temp_dir().join(unique);
        for child in ["herdr", "herdr-old", "notes"] {
            std::fs::create_dir_all(root.join(child)).unwrap();
        }
        root.canonicalize().unwrap()
    }

    fn typing_a_path(name: &str, tail: &str) -> (PathBuf, ComposerState) {
        let root = folders_on_disk(name);
        let mut composer = ComposerState::default();
        composer.open_dropdown(Focus::Folder);
        composer.edit_path(|path| path.set_text(&format!("{}/{tail}", root.display())));
        (root, composer)
    }

    #[test]
    fn the_best_match_fills_in_the_letters_not_yet_typed() {
        let (_, composer) = typing_a_path("ghost", "h");
        assert_eq!(composer.ghost_suffix().as_deref(), Some("erdr"));
    }

    #[test]
    fn a_cursor_that_is_not_at_the_end_shows_no_ghost() {
        let (_, mut composer) = typing_a_path("midghost", "h");
        composer.edit_path(|path| path.left());
        assert_eq!(composer.ghost_suffix(), None);
    }

    #[test]
    fn pointing_at_a_row_hides_the_ghost() {
        let (_, mut composer) = typing_a_path("pointghost", "h");
        assert!(composer.ghost_suffix().is_some());
        composer.point(1);
        assert_eq!(composer.ghost_suffix(), None);
    }

    #[test]
    fn editing_the_chosen_path_stops_listing_the_remembered_folders() {
        let mut composer = composer_with_folders(&["/tmp", "/usr"]);
        composer.open_dropdown(Focus::Folder);
        assert_eq!(composer.folder_rows().len(), 2);
        composer.edit_path(|path| path.backspace());
        assert!(
            !composer.path_holds_the_chosen_folder(),
            "the letters are a search now"
        );
        assert!(!composer.pointing());
    }

    #[test]
    fn opening_with_no_folder_leaves_the_field_empty() {
        let mut composer = ComposerState::default();
        composer.open_dropdown(Focus::Folder);
        assert!(composer.path().is_empty());
    }

    #[test]
    fn a_path_that_already_names_the_match_shows_no_ghost() {
        let (_, composer) = typing_a_path("fullghost", "herdr");
        assert_eq!(composer.ghost_suffix(), None);
    }

    #[test]
    fn typing_a_path_offers_the_folders_under_it() {
        let (_, composer) = typing_a_path("offers", "her");
        let labels: Vec<_> = composer
            .folder_rows()
            .iter()
            .map(|folder| {
                folder
                    .path
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .to_string()
            })
            .collect();
        assert_eq!(labels, ["herdr", "herdr-old"]);
        assert!(
            !composer.pointing(),
            "the letters are the value until the list is stepped into"
        );
    }

    #[test]
    fn stepping_down_enters_the_offered_list_and_stepping_up_leaves_it() {
        let (_, mut composer) = typing_a_path("stepping", "her");
        composer.point(-1);
        assert!(!composer.pointing(), "there is nothing above the field");

        composer.point(1);
        assert!(composer.pointing());
        assert_eq!(composer.highlight, 0);

        composer.point(1);
        assert_eq!(composer.highlight, 1);
        composer.point(1);
        assert_eq!(composer.highlight, 1, "and the last row is the floor");

        composer.point(-1);
        composer.point(-1);
        assert!(
            !composer.pointing(),
            "stepping off the top is back to typing"
        );
    }

    #[test]
    fn a_long_match_list_is_pointed_through_to_the_last_folder() {
        let unique = format!(
            "herdr-composer-tests-long-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let root = std::env::temp_dir().join(unique);
        for index in 0..12 {
            std::fs::create_dir_all(root.join(format!("dir{index:02}"))).unwrap();
        }
        let root = root.canonicalize().unwrap();
        let mut composer = ComposerState::default();
        composer.open_dropdown(Focus::Folder);
        composer.edit_path(|path| path.set_text(&format!("{}/", root.display())));
        assert_eq!(composer.folder_rows().len(), 12);
        assert_eq!(composer.folder_visible_rows(), suggest::MOST);
        for _ in 0..20 {
            composer.point(1);
        }
        assert_eq!(composer.highlight, 11, "the last match is the floor");
        assert!(composer.pointing());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn another_letter_takes_the_keyboard_back_out_of_the_list() {
        let (_, mut composer) = typing_a_path("letter", "her");
        composer.point(1);
        composer.point(1);
        composer.edit_path(|path| path.insert('d'));
        assert!(!composer.pointing());
        assert_eq!(composer.highlight, 0);
    }

    #[test]
    fn taking_a_pointed_row_settles_the_folder_it_names() {
        let (root, mut composer) = typing_a_path("pointed", "her");
        composer.point(1);
        composer.point(1);
        assert_eq!(composer.take_folder(), Ok(()));
        assert_eq!(
            composer.folder_path(),
            Some(root.join("herdr-old").as_path())
        );
        assert_eq!(composer.focus, Focus::Agent);
        assert!(composer.path().is_empty(), "and the field is spent");
    }

    #[test]
    fn a_path_that_names_no_folder_yet_settles_the_best_one_offered() {
        let (root, mut composer) = typing_a_path("best", "her");
        assert_eq!(composer.take_folder(), Ok(()));
        assert_eq!(composer.folder_path(), Some(root.join("herdr").as_path()));
    }

    #[test]
    fn a_typed_path_that_is_not_a_folder_is_not_replaced_by_the_guess() {
        let (_, mut composer) = typing_a_path("noguess", "her");
        assert!(matches!(
            composer.take_typed_folder(),
            Err(PathError::NotADirectory(_))
        ));
        assert_eq!(composer.folder, None, "and nothing is settled");
        assert_eq!(composer.open, Some(Focus::Folder), "the list stays open");
    }

    #[test]
    fn a_path_that_names_a_folder_outright_is_the_one_taken() {
        let (root, mut composer) = typing_a_path("outright", "herdr");
        assert_eq!(composer.take_folder(), Ok(()));
        assert_eq!(
            composer.folder_path(),
            Some(root.join("herdr").as_path()),
            "not herdr-old, which is only offered under it"
        );
    }

    #[test]
    fn a_path_that_names_nothing_at_all_still_says_so() {
        let mut composer = ComposerState::default();
        composer.open_dropdown(Focus::Folder);
        composer.edit_path(|path| path.set_text("/definitely/not/here"));
        assert!(matches!(
            composer.take_folder(),
            Err(PathError::NotADirectory(_))
        ));
    }

    #[test]
    fn an_untyped_folder_control_still_lists_the_folders_it_knows() {
        let mut composer = composer_with_folders(&["/tmp", "/usr"]);
        composer.open_dropdown(Focus::Folder);
        assert_eq!(composer.folder_rows().len(), 2);
        assert!(composer.pointing(), "the chosen row is still the value");
    }

    #[test]
    fn opening_the_folder_list_keeps_the_chosen_path_in_the_field() {
        let mut composer = composer_with_folders(&["/tmp", "/usr"]);
        composer.open_dropdown(Focus::Folder);
        let label = composer.folder_label().unwrap().to_string();
        assert_eq!(composer.path().text(), label);
        assert_eq!(
            composer.path().cursor_row(),
            (0, label.chars().count()),
            "the cursor sits at the end so the path can be edited"
        );
        assert_eq!(composer.folder_rows().len(), 2, "the remembered list stays");
        assert!(composer.pointing(), "the chosen row is still the value");
    }

    #[test]
    fn an_empty_task_is_still_pending_when_a_folder_is_chosen() {
        let mut composer = composer_with_folders(&["/tmp"]);
        let pending = composer
            .pending()
            .expect("an empty task still starts in the chosen folder");
        assert!(pending.task.is_empty());
        assert_eq!(pending.cwd, PathBuf::from("/tmp"));

        composer.task.set_text("   ");
        let pending = composer.pending().expect("whitespace is still no prompt");
        assert!(pending.task.is_empty());
    }

    #[test]
    fn nothing_is_pending_without_a_folder() {
        let mut nowhere = ComposerState::default();
        nowhere.task.set_text("fix the tests");
        assert_eq!(nowhere.pending(), None, "nowhere to send it");
        nowhere.task.clear();
        assert_eq!(
            nowhere.pending(),
            None,
            "and an empty task has nowhere either"
        );
    }

    #[test]
    fn a_pending_task_asks_for_a_worktree_until_the_box_is_cleared() {
        let mut composer = composer_with_folders(&["/tmp"]);
        composer.task.set_text("fix the tests");
        assert!(
            composer.pending().unwrap().worktree,
            "a new band starts agents in a worktree"
        );

        composer.worktree = false;
        assert!(
            !composer.pending().unwrap().worktree,
            "clearing the box is what keeps the chosen folder"
        );
    }
}
