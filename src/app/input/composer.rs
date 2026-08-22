//! Key handling for the composer band.
//!
//! The keyboard moves itself. Settling a directory or an agent hands it on to
//! the task, so the ordinary way through the band — a path, an agent, a task —
//! never asks for `Tab`. `Tab` is here anyway, because a band you can only
//! walk forwards through is one you have to leave and re-enter to correct.

use crossterm::event::{KeyCode, KeyModifiers};

use crate::app::state::{AppState, Mode};
use crate::composer::{Focus, Pending};
use crate::input::TerminalKey;

/// What a key did to the band.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ComposerKeyOutcome {
    /// The band absorbed the key.
    Edited,
    /// The band is ready to start an agent: a task if one was typed, or the
    /// chosen agent idle in the chosen folder if the task is empty.
    Submit(Box<Pending>),
    /// The key could not do what it asked for, and this says why.
    Trouble(String),
}

/// Reach the band. The folder list is freshened on the way in, because this is
/// one of the two moments anybody looks at it.
pub(crate) fn enter_composer_mode(
    state: &mut AppState,
    terminal_runtimes: &crate::terminal::TerminalRuntimeRegistry,
) {
    state.refresh_composer_folders(terminal_runtimes);
    state.composer.start_where_the_work_is();
    state.mode = Mode::Composer;
}

pub(crate) fn leave_composer_mode(state: &mut AppState) {
    state.composer.close_dropdown();
    state.mode = if state.active.is_some() {
        Mode::Terminal
    } else {
        Mode::Navigate
    };
}

pub(crate) fn handle_composer_key(
    state: &mut AppState,
    terminal_runtimes: &crate::terminal::TerminalRuntimeRegistry,
    raw_key: TerminalKey,
) -> ComposerKeyOutcome {
    // The prefix works from the band the way it works from a pane, so the rest
    // of herdr stays one keystroke away from a half-typed task.
    if state.is_prefix_key(raw_key) {
        state.mode = Mode::Prefix;
        return ComposerKeyOutcome::Edited;
    }

    let key = raw_key.as_key_event();
    let plain = key.modifiers.difference(KeyModifiers::SHIFT).is_empty();

    // An open dropdown owns the arrows and Enter, whichever control opened it.
    // Tab commits a pointed row the way Enter does. On the folder field they
    // split: Tab takes the guessed directory, Enter takes only what was typed.
    if let Some(open) = state.composer.open {
        match key.code {
            KeyCode::Esc => {
                state.composer.close_dropdown();
                return ComposerKeyOutcome::Edited;
            }
            KeyCode::BackTab => {
                state.composer.close_dropdown();
                state.composer.focus = match open {
                    Focus::Folder => Focus::Task,
                    Focus::Agent => Focus::Folder,
                    Focus::Task => Focus::Agent,
                };
                return ComposerKeyOutcome::Edited;
            }
            KeyCode::Up => {
                state.composer.point(-1);
                return ComposerKeyOutcome::Edited;
            }
            KeyCode::Down => {
                state.composer.point(1);
                return ComposerKeyOutcome::Edited;
            }
            KeyCode::Tab if open == Focus::Folder => {
                return match state.composer.take_folder() {
                    Ok(()) => ComposerKeyOutcome::Edited,
                    Err(err) => ComposerKeyOutcome::Trouble(err.message()),
                };
            }
            KeyCode::Enter if open == Focus::Folder => {
                return match state.composer.take_typed_folder() {
                    Ok(()) => ComposerKeyOutcome::Edited,
                    Err(err) => ComposerKeyOutcome::Trouble(err.message()),
                };
            }
            KeyCode::Enter | KeyCode::Tab => {
                state.composer.take_pointed();
                return ComposerKeyOutcome::Edited;
            }
            KeyCode::Char(' ') if open == Focus::Agent => {
                state.composer.take_pointed();
                return ComposerKeyOutcome::Edited;
            }
            KeyCode::Char(c) if open == Focus::Agent && plain => {
                state.composer.type_agent(c);
                return ComposerKeyOutcome::Edited;
            }
            _ => {}
        }
        // Only the folder dropdown is typed into: a path is added by writing
        // one, and the agent list is a fixed set of rows with nothing to add.
        if open == Focus::Folder
            && state
                .composer
                .edit_path(|path| edit_field(path, key.code, key.modifiers, plain))
        {
            return ComposerKeyOutcome::Edited;
        }
        return ComposerKeyOutcome::Edited;
    }

    match key.code {
        KeyCode::Esc => {
            leave_composer_mode(state);
            ComposerKeyOutcome::Edited
        }
        KeyCode::Tab => {
            state.composer.focus = match state.composer.focus {
                Focus::Folder => Focus::Agent,
                Focus::Agent => Focus::Task,
                Focus::Task => Focus::Folder,
            };
            ComposerKeyOutcome::Edited
        }
        KeyCode::BackTab => {
            state.composer.focus = match state.composer.focus {
                Focus::Folder => Focus::Task,
                Focus::Agent => Focus::Folder,
                Focus::Task => Focus::Agent,
            };
            ComposerKeyOutcome::Edited
        }
        _ => match state.composer.focus {
            Focus::Folder | Focus::Agent => {
                closed_dropdown_key(state, terminal_runtimes, key.code, plain)
            }
            Focus::Task => task_key(state, key.code, key.modifiers, plain),
        },
    }
}

/// A dropdown that is closed changes its value outright, because the value is
/// the thing being chosen and opening the list to choose it is a step that
/// answers nothing.
fn closed_dropdown_key(
    state: &mut AppState,
    terminal_runtimes: &crate::terminal::TerminalRuntimeRegistry,
    code: KeyCode,
    plain: bool,
) -> ComposerKeyOutcome {
    let which = state.composer.focus;
    match code {
        KeyCode::Up => state.composer.step(which, -1),
        KeyCode::Down => state.composer.step(which, 1),
        KeyCode::Char(' ') if which == Focus::Agent => {
            state.composer.open_dropdown(Focus::Agent);
        }
        KeyCode::Char(c) if which == Focus::Agent && plain => {
            state.composer.type_agent(c);
        }
        KeyCode::Enter => {
            // The list is read from the running panes when it is opened, which
            // is the only moment it is looked at.
            if which == Focus::Folder {
                state.refresh_composer_folders(terminal_runtimes);
            }
            state.composer.open_dropdown(which);
        }
        // Typing at the folder control is how a path is added, so it opens the
        // list and lands in the path field rather than being dropped.
        KeyCode::Char(c) if which == Focus::Folder && plain => {
            state.refresh_composer_folders(terminal_runtimes);
            state.composer.open_dropdown(Focus::Folder);
            state.composer.edit_path(|path| {
                path.clear();
                path.insert(c);
            });
        }
        _ => {}
    }
    ComposerKeyOutcome::Edited
}

fn task_key(
    state: &mut AppState,
    code: KeyCode,
    modifiers: KeyModifiers,
    plain: bool,
) -> ComposerKeyOutcome {
    match code {
        // A task written over several lines is still one task, so the key that
        // sends it and the key that breaks a line have to be different keys. A
        // terminal that reports modifiers separately sends Shift+Enter as Enter
        // with Shift; one that does not sends Ctrl-J as a control character.
        // Both mean the same thing here.
        KeyCode::Enter if !modifiers.is_empty() => {
            state.composer.task.newline();
            ComposerKeyOutcome::Edited
        }
        KeyCode::Char('j') if modifiers.contains(KeyModifiers::CONTROL) => {
            state.composer.task.newline();
            ComposerKeyOutcome::Edited
        }
        KeyCode::Enter => match state.composer.pending() {
            Some(pending) => ComposerKeyOutcome::Submit(Box::new(pending)),
            None if state.composer.folder.is_none() => {
                state.composer.focus = Focus::Folder;
                ComposerKeyOutcome::Trouble("add a folder first".to_string())
            }
            None => ComposerKeyOutcome::Edited,
        },
        _ => {
            edit_field(&mut state.composer.task, code, modifiers, plain);
            ComposerKeyOutcome::Edited
        }
    }
}

/// The editing keys every field in the band shares. Returns whether the key was
/// one of them.
fn edit_field(
    field: &mut crate::composer::TextField,
    code: KeyCode,
    modifiers: KeyModifiers,
    plain: bool,
) -> bool {
    match code {
        KeyCode::Char('u') if modifiers.contains(KeyModifiers::CONTROL) => field.clear(),
        KeyCode::Backspace if modifiers.contains(KeyModifiers::SUPER) => field.clear(),
        KeyCode::Backspace
            if modifiers.contains(KeyModifiers::CONTROL)
                || modifiers.contains(KeyModifiers::ALT) =>
        {
            delete_word(field)
        }
        KeyCode::Char('h' | 'w') if modifiers.contains(KeyModifiers::CONTROL) => delete_word(field),
        KeyCode::Backspace => field.backspace(),
        KeyCode::Delete => field.delete(),
        KeyCode::Left => field.left(),
        KeyCode::Right => field.right(),
        KeyCode::Up => {
            field.up();
        }
        KeyCode::Down => {
            field.down();
        }
        KeyCode::Home => field.home(),
        KeyCode::End => field.end(),
        KeyCode::Char(c) if plain => field.insert(c),
        _ => return false,
    }
    true
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WordClass {
    Word,
    Separator,
}

fn word_class(ch: char) -> WordClass {
    if ch.is_alphanumeric() || ch == '_' {
        WordClass::Word
    } else {
        WordClass::Separator
    }
}

/// Rub out the word before the cursor: the whitespace it sits behind, then the
/// run of one kind of character before that. A path is a run of words and
/// separators, so deleting by class stops at the slash rather than eating the
/// whole path.
fn delete_word(field: &mut crate::composer::TextField) {
    let text = field.text();
    let mut chars: Vec<char> = text.chars().collect();
    while chars.last().is_some_and(|ch| ch.is_whitespace()) {
        chars.pop();
    }
    if let Some(class) = chars.last().copied().map(word_class) {
        while chars
            .last()
            .is_some_and(|ch| !ch.is_whitespace() && word_class(*ch) == class)
        {
            chars.pop();
        }
    }
    field.set_text(&chars.into_iter().collect::<String>());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> TerminalKey {
        crossterm::event::KeyEvent::new(code, KeyModifiers::empty()).into()
    }

    fn ctrl(code: KeyCode) -> TerminalKey {
        crossterm::event::KeyEvent::new(code, KeyModifiers::CONTROL).into()
    }

    fn shift(code: KeyCode) -> TerminalKey {
        crossterm::event::KeyEvent::new(code, KeyModifiers::SHIFT).into()
    }

    fn composer_state() -> AppState {
        let mut state = AppState::test_new();
        state.workspaces = vec![crate::workspace::Workspace::test_new("space")];
        state.active = Some(0);
        state.mode = Mode::Composer;
        state.composer.add_folder(std::path::PathBuf::from("/tmp"));
        state.composer.focus = Focus::Task;
        state
    }

    fn runtimes() -> crate::terminal::TerminalRuntimeRegistry {
        crate::terminal::TerminalRuntimeRegistry::default()
    }

    fn press(state: &mut AppState, code: KeyCode) -> ComposerKeyOutcome {
        handle_composer_key(state, &runtimes(), key(code))
    }

    fn press_key(state: &mut AppState, raw: TerminalKey) -> ComposerKeyOutcome {
        handle_composer_key(state, &runtimes(), raw)
    }

    fn type_in(state: &mut AppState, text: &str) {
        for ch in text.chars() {
            press(state, KeyCode::Char(ch));
        }
    }

    #[test]
    fn typing_builds_the_task() {
        let mut state = composer_state();
        type_in(&mut state, "fix the tab bar");
        assert_eq!(state.composer.task.text(), "fix the tab bar");
    }

    #[test]
    fn enter_submits_the_task_with_the_folder_and_agent_on_show() {
        let mut state = composer_state();
        type_in(&mut state, "  fix the tab bar  ");
        let ComposerKeyOutcome::Submit(pending) = press(&mut state, KeyCode::Enter) else {
            panic!("a task with a folder should submit");
        };
        assert_eq!(pending.cwd, std::path::PathBuf::from("/tmp"));
        assert!(pending.message().ends_with("fix the tab bar"));
    }

    #[test]
    fn enter_on_an_empty_task_starts_the_agent_with_no_prompt() {
        let mut state = composer_state();
        state
            .composer
            .use_harnesses(vec![crate::harness::named("Claude Code").unwrap()]);
        let ComposerKeyOutcome::Submit(pending) = press(&mut state, KeyCode::Enter) else {
            panic!("an empty task with a folder should start the agent");
        };
        assert_eq!(pending.cwd, std::path::PathBuf::from("/tmp"));
        assert!(pending.task.is_empty());
        assert_eq!(
            pending.launch(),
            crate::harness::Launch::Agent {
                agent: crate::detect::Agent::Claude,
                argv: vec!["claude".into()],
            }
        );
    }

    #[test]
    fn a_task_with_nowhere_to_go_sends_the_keyboard_to_the_folder() {
        let mut state = composer_state();
        state.composer.folders.clear();
        state.composer.folder = None;
        type_in(&mut state, "fix the tab bar");
        assert!(matches!(
            press(&mut state, KeyCode::Enter),
            ComposerKeyOutcome::Trouble(_)
        ));
        assert_eq!(state.composer.focus, Focus::Folder);
    }

    #[test]
    fn shift_enter_writes_a_line_rather_than_sending() {
        let mut state = composer_state();
        type_in(&mut state, "first");
        assert_eq!(
            press_key(&mut state, shift(KeyCode::Enter)),
            ComposerKeyOutcome::Edited
        );
        type_in(&mut state, "second");
        assert_eq!(state.composer.task.text(), "first\nsecond");
    }

    #[test]
    fn ctrl_j_writes_a_line_too() {
        let mut state = composer_state();
        type_in(&mut state, "first");
        press_key(&mut state, ctrl(KeyCode::Char('j')));
        type_in(&mut state, "second");
        assert_eq!(state.composer.task.text(), "first\nsecond");
    }

    #[test]
    fn escape_leaves_the_band_and_keeps_the_task() {
        let mut state = composer_state();
        type_in(&mut state, "half a thought");
        press(&mut state, KeyCode::Esc);
        assert_eq!(state.mode, Mode::Terminal);
        assert_eq!(state.composer.task.text(), "half a thought");
    }

    #[test]
    fn ctrl_w_deletes_the_last_word() {
        let mut state = composer_state();
        type_in(&mut state, "rename the route bar");
        press_key(&mut state, ctrl(KeyCode::Char('w')));
        assert_eq!(state.composer.task.text(), "rename the route ");
    }

    fn with_named_agents(state: &mut AppState) {
        state.composer.use_harnesses(
            ["Auto", "Claude Code", "Codex"]
                .into_iter()
                .map(|name| crate::harness::named(name).unwrap())
                .collect(),
        );
        state.composer.focus = Focus::Agent;
    }

    #[test]
    fn typing_at_the_closed_agent_control_picks_the_agent_the_letters_name() {
        let mut state = composer_state();
        with_named_agents(&mut state);
        type_in(&mut state, "co");
        assert_eq!(state.composer.agent, "Codex");
        assert_eq!(state.composer.open, None, "and the list stayed closed");
    }

    #[test]
    fn typing_at_the_open_agent_list_points_and_enter_takes_it() {
        let mut state = composer_state();
        with_named_agents(&mut state);
        press(&mut state, KeyCode::Enter);
        assert_eq!(state.composer.open, Some(Focus::Agent));
        type_in(&mut state, "cl");
        press(&mut state, KeyCode::Enter);
        assert_eq!(state.composer.agent, "Claude Code");
        assert_eq!(state.composer.open, None);
        assert_eq!(state.composer.focus, Focus::Task);
    }

    #[test]
    fn a_closed_agent_control_changes_the_agent_outright() {
        let mut state = composer_state();
        state.composer.focus = Focus::Agent;
        let before = state.composer.agent.clone();
        press(&mut state, KeyCode::Down);
        if state.composer.harnesses().len() > 1 {
            assert_ne!(state.composer.agent, before);
        }
        assert_eq!(state.composer.open, None, "and does not open the list");
    }

    #[test]
    fn typing_at_the_folder_control_opens_it_and_starts_a_path() {
        let mut state = composer_state();
        state.composer.focus = Focus::Folder;
        press(&mut state, KeyCode::Char('/'));
        assert_eq!(state.composer.open, Some(Focus::Folder));
        assert_eq!(state.composer.path().text(), "/");
    }

    #[test]
    fn a_typed_path_that_names_nothing_says_so_and_keeps_the_list_open() {
        let mut state = composer_state();
        state.composer.focus = Focus::Folder;
        type_in(&mut state, "/definitely/not/here");
        assert!(matches!(
            press(&mut state, KeyCode::Enter),
            ComposerKeyOutcome::Trouble(_)
        ));
        assert_eq!(state.composer.open, Some(Focus::Folder));
    }

    fn folders_on_disk(name: &str) -> std::path::PathBuf {
        let unique = format!(
            "herdr-composer-key-tests-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let root = std::env::temp_dir().join(unique);
        for child in ["herdr", "herdr-old"] {
            std::fs::create_dir_all(root.join(child)).unwrap();
        }
        root.canonicalize().unwrap()
    }

    #[test]
    fn typing_a_path_lists_the_folders_it_could_mean() {
        let root = folders_on_disk("lists");
        let mut state = composer_state();
        state.composer.focus = Focus::Folder;
        type_in(&mut state, &format!("{}/her", root.display()));
        assert_eq!(state.composer.folder_rows().len(), 2);
        assert!(
            !state.composer.pointing(),
            "the typing still holds the value"
        );
    }

    #[test]
    fn the_down_arrow_picks_a_listed_folder_and_enter_takes_it() {
        let root = folders_on_disk("down");
        let mut state = composer_state();
        state.composer.focus = Focus::Folder;
        type_in(&mut state, &format!("{}/her", root.display()));

        press(&mut state, KeyCode::Down);
        press(&mut state, KeyCode::Down);
        assert!(state.composer.pointing());

        press(&mut state, KeyCode::Enter);
        assert_eq!(
            state.composer.folder_path(),
            Some(root.join("herdr-old").as_path())
        );
        assert_eq!(state.composer.focus, Focus::Agent);
    }

    #[test]
    fn enter_on_a_half_typed_folder_ignores_the_guess() {
        let root = folders_on_disk("half");
        let mut state = composer_state();
        state.composer.focus = Focus::Folder;
        type_in(&mut state, &format!("{}/her", root.display()));

        assert!(matches!(
            press(&mut state, KeyCode::Enter),
            ComposerKeyOutcome::Trouble(_)
        ));
        assert_eq!(
            state.composer.folder_path(),
            Some(std::path::Path::new("/tmp")),
            "the folder on show is unchanged"
        );
        assert_eq!(state.composer.open, Some(Focus::Folder));
        let _ = root;
    }

    #[test]
    fn tab_on_a_half_typed_folder_takes_the_one_at_the_top_of_the_list() {
        let root = folders_on_disk("tabhalf");
        let mut state = composer_state();
        state.composer.focus = Focus::Folder;
        type_in(&mut state, &format!("{}/her", root.display()));

        assert_eq!(press(&mut state, KeyCode::Tab), ComposerKeyOutcome::Edited);
        assert_eq!(
            state.composer.folder_path(),
            Some(root.join("herdr").as_path())
        );
        assert_eq!(state.composer.focus, Focus::Agent);
    }

    #[test]
    fn escape_closes_an_open_dropdown_before_it_leaves_the_band() {
        let mut state = composer_state();
        state.composer.focus = Focus::Agent;
        press(&mut state, KeyCode::Enter);
        assert_eq!(state.composer.open, Some(Focus::Agent));

        press(&mut state, KeyCode::Esc);
        assert_eq!(state.composer.open, None);
        assert_eq!(state.mode, Mode::Composer, "and stays in the band");

        press(&mut state, KeyCode::Esc);
        assert_eq!(state.mode, Mode::Terminal);
    }

    #[test]
    fn settling_an_agent_hands_the_keyboard_to_the_task() {
        let mut state = composer_state();
        state.composer.focus = Focus::Agent;
        press(&mut state, KeyCode::Enter);
        press(&mut state, KeyCode::Enter);
        assert_eq!(state.composer.focus, Focus::Task);
    }

    #[test]
    fn the_top_row_of_the_task_keeps_the_keyboard() {
        let mut state = composer_state();
        type_in(&mut state, "one line");
        press(&mut state, KeyCode::Up);
        assert_eq!(state.composer.focus, Focus::Task);
    }

    #[test]
    fn tab_commits_an_open_dropdown() {
        let mut state = composer_state();
        state.composer.focus = Focus::Agent;
        press(&mut state, KeyCode::Enter);
        assert_eq!(state.composer.open, Some(Focus::Agent));
        press(&mut state, KeyCode::Tab);
        assert_eq!(state.composer.open, None);
        assert_eq!(state.composer.focus, Focus::Task);
    }

    #[test]
    fn settling_a_folder_hands_the_keyboard_to_the_agent() {
        let mut state = composer_state();
        state.composer.focus = Focus::Folder;
        press(&mut state, KeyCode::Enter);
        press(&mut state, KeyCode::Enter);
        assert_eq!(state.composer.focus, Focus::Agent);
    }
}
