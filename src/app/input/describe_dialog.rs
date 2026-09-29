//! Key handling for the describe dialog, which asks what a space or a tab is
//! for.
//!
//! The keyboard starts in the text: the purpose of a space or the goal of a
//! tab. `Tab` walks the fields the dialog has, in the order they are drawn. A
//! new tab has an optional name row; a new space has a directory row, which
//! takes the same keys the launcher's directory control takes: typing opens
//! its list and searches it, the arrows point, `Tab` completes and then
//! settles, and `Enter` settles what was typed. Settling a directory hands the
//! keyboard back to the text. In the text, `Enter` finishes and `Shift+Enter`
//! breaks a line; in the name, `Enter` finishes.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;

use super::composer::edit_field;
use super::modal::leave_modal;
use crate::app::state::{AppState, DescribeDialogState, DescribeField, DescribeTarget};
use crate::composer::Focus;

fn open(state: &mut AppState, dialog: DescribeDialogState) {
    state.describe_dialog = Some(dialog);
    state.hovered_space = None;
    state.hovered_tab = None;
    state.mode = crate::app::Mode::DescribeDialog;
}

/// Open the dialog on an existing space, to edit only its purpose.
pub(crate) fn open_space_purpose_editor(state: &mut AppState, ws_idx: usize) {
    let Some(ws) = state.workspaces.get(ws_idx) else {
        return;
    };
    let mut dialog = DescribeDialogState::new(DescribeTarget::Space { id: ws.id.clone() });
    dialog
        .text
        .set_text(ws.purpose.as_deref().unwrap_or_default());
    open(state, dialog);
}

/// Open the dialog on an existing tab, to edit only its goal.
pub(crate) fn open_tab_goal_editor(state: &mut AppState, ws_idx: usize, tab_idx: usize) {
    let Some(ws) = state.workspaces.get(ws_idx) else {
        return;
    };
    let Some(tab) = ws.tabs.get(tab_idx) else {
        return;
    };
    let mut dialog = DescribeDialogState::new(DescribeTarget::Tab {
        space_id: ws.id.clone(),
        tab_idx,
    });
    dialog
        .text
        .set_text(tab.goal.as_deref().unwrap_or_default());
    open(state, dialog);
}

/// Ask for a new tab in the active space: a goal and an optional name.
pub(crate) fn open_new_tab_dialog(state: &mut AppState) {
    if state.active.is_none() {
        return;
    }
    open(state, DescribeDialogState::new(DescribeTarget::NewTab));
}

pub(crate) fn cancel_describe_dialog(state: &mut AppState) {
    state.describe_dialog = None;
    leave_modal(state);
}

/// Finish the dialog. Edits are applied here. A new tab or space needs a
/// pane, which only the event loop can start, so it is asked for.
pub(crate) fn submit_describe_dialog(state: &mut AppState) {
    let Some(dialog) = state.describe_dialog.as_mut() else {
        return;
    };
    // A folder typed but not yet settled is what the space should start in,
    // so finishing settles it the way Enter in the list would. A path that is
    // not a folder keeps the dialog open on the error.
    if !settle_typed_directory(dialog) {
        return;
    }
    let text = dialog.text.text();
    match dialog.target.clone() {
        DescribeTarget::NewSpace => {
            state.request_submit_describe_dialog = true;
            return;
        }
        DescribeTarget::NewTab => {
            let name = dialog
                .name
                .as_ref()
                .map(|name| name.text().trim().to_string())
                .filter(|name| !name.is_empty());
            let goal = text.trim();
            state.requested_new_tab_name = name;
            state.requested_new_tab_goal = (!goal.is_empty()).then(|| goal.to_string());
            state.request_new_tab = true;
        }
        DescribeTarget::Space { id } => {
            if let Some(ws) = state.workspaces.iter_mut().find(|ws| ws.id == id) {
                ws.set_purpose(&text);
                state.mark_session_dirty();
            }
        }
        DescribeTarget::Tab { space_id, tab_idx } => {
            if let Some(tab) = state
                .workspaces
                .iter_mut()
                .find(|ws| ws.id == space_id)
                .and_then(|ws| ws.tabs.get_mut(tab_idx))
            {
                tab.set_goal(&text);
                state.mark_session_dirty();
            }
        }
    }
    cancel_describe_dialog(state);
}

/// Settle the directory from the open folder list when something was typed
/// into it. Reports whether the dialog can go on: false means the typed path
/// is not a folder, and the dialog now says so.
fn settle_typed_directory(dialog: &mut DescribeDialogState) -> bool {
    let Some(folders) = dialog
        .folders
        .as_mut()
        .filter(|folders| folders.open == Some(Focus::Folder) && !folders.path().is_empty())
    else {
        return true;
    };
    match folders.take_typed_folder() {
        Ok(()) => {
            dialog.error = None;
            true
        }
        Err(err) => {
            dialog.error = Some(err.message());
            dialog.field = DescribeField::Directory;
            false
        }
    }
}

pub(crate) fn handle_describe_dialog_key(state: &mut AppState, key: KeyEvent) {
    let Some(dialog) = state.describe_dialog.as_mut() else {
        leave_modal(state);
        return;
    };
    let plain = key.modifiers.difference(KeyModifiers::SHIFT).is_empty();
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);

    // An open folder list owns the keyboard until it is settled or closed.
    if let Some(folders) = dialog
        .folders
        .as_mut()
        .filter(|folders| folders.open == Some(Focus::Folder))
    {
        let settled = match key.code {
            KeyCode::Esc | KeyCode::BackTab => {
                folders.close_dropdown();
                None
            }
            KeyCode::Up => {
                folders.point(-1);
                None
            }
            KeyCode::Down => {
                folders.point(1);
                None
            }
            KeyCode::Tab if folders.fill_in_guess() => None,
            KeyCode::Tab => Some(folders.take_folder()),
            KeyCode::Enter => Some(folders.take_typed_folder()),
            _ => {
                folders.edit_path(|path| edit_field(path, key.code, key.modifiers, plain));
                None
            }
        };
        match settled {
            Some(Ok(())) => {
                dialog.error = None;
                dialog.field = DescribeField::Text;
            }
            Some(Err(err)) => dialog.error = Some(err.message()),
            None => {}
        }
        return;
    }

    match key.code {
        KeyCode::Esc => return cancel_describe_dialog(state),
        KeyCode::Tab => return dialog.step_field(1),
        KeyCode::BackTab => return dialog.step_field(-1),
        _ => {}
    }

    match dialog.field {
        DescribeField::Directory => {
            let Some(folders) = dialog.folders.as_mut() else {
                return;
            };
            match key.code {
                KeyCode::Up => folders.step(Focus::Folder, -1),
                KeyCode::Down => folders.step(Focus::Folder, 1),
                KeyCode::Enter => folders.open_dropdown(Focus::Folder),
                // Typing at the closed control starts a fresh path, the way
                // it does in the launcher.
                KeyCode::Char(c) if plain => {
                    folders.open_dropdown(Focus::Folder);
                    folders.edit_path(|path| {
                        path.clear();
                        path.insert(c);
                    });
                }
                _ => {}
            }
        }
        DescribeField::Name => match key.code {
            KeyCode::Enter => submit_describe_dialog(state),
            KeyCode::Char('a') if ctrl => {
                if let Some(name) = dialog.name.as_mut() {
                    name.select_all();
                }
            }
            _ => {
                if let Some(name) = dialog.name.as_mut() {
                    edit_field(name, key.code, key.modifiers, plain);
                }
            }
        },
        DescribeField::Text => match key.code {
            // The text may run over several lines, so breaking a line and
            // finishing are different keys: the same pair the launcher's task
            // field uses.
            KeyCode::Enter if !key.modifiers.is_empty() => dialog.text.newline(),
            KeyCode::Char('j') if ctrl => dialog.text.newline(),
            KeyCode::Char('a') if ctrl => dialog.text.select_all(),
            KeyCode::Enter => submit_describe_dialog(state),
            _ => {
                edit_field(&mut dialog.text, key.code, key.modifiers, plain);
            }
        },
    }
}

fn contains(rect: Rect, col: u16, row: u16) -> bool {
    col >= rect.x && col < rect.right() && row >= rect.y && row < rect.bottom()
}

/// The folder row under the pointer in the open list, if any.
fn list_index_at(state: &AppState, col: u16, row: u16) -> Option<usize> {
    let dialog = state.describe_dialog.as_ref()?;
    let layout = crate::ui::describe_dialog_layout(dialog, state.screen_rect())?;
    let list = layout.list.filter(|list| contains(*list, col, row))?;
    let index =
        crate::ui::describe_dialog_list_first(dialog, list.height) + (row - list.y) as usize;
    let count = dialog.folders.as_ref()?.folder_rows().len();
    (index < count).then_some(index)
}

pub(crate) fn handle_describe_dialog_mouse(state: &mut AppState, mouse: MouseEvent) {
    let (col, row) = (mouse.column, mouse.row);
    let hovered = list_index_at(state, col, row);
    let Some(dialog) = state.describe_dialog.as_ref() else {
        return;
    };
    let Some(layout) = crate::ui::describe_dialog_layout(dialog, state.screen_rect()) else {
        return;
    };
    let Some(dialog) = state.describe_dialog.as_mut() else {
        return;
    };
    let list_open = layout.list.is_some();

    match mouse.kind {
        MouseEventKind::Moved => {
            if let Some(folders) = dialog.folders.as_mut() {
                folders.hover = hovered;
            }
        }
        MouseEventKind::ScrollUp | MouseEventKind::ScrollDown if list_open => {
            if let Some(folders) = dialog.folders.as_mut() {
                let delta = if mouse.kind == MouseEventKind::ScrollUp {
                    -1
                } else {
                    1
                };
                folders.point(delta);
            }
        }
        MouseEventKind::Down(MouseButton::Left) => {
            // A click on a row of the open list takes that folder.
            if let (Some(index), Some(folders)) = (hovered, dialog.folders.as_mut()) {
                folders.point_at(index);
                match folders.take_folder() {
                    Ok(()) => {
                        dialog.error = None;
                        dialog.field = DescribeField::Text;
                    }
                    Err(err) => dialog.error = Some(err.message()),
                }
                return;
            }
            if layout
                .directory
                .is_some_and(|rect| contains(rect, col, row))
            {
                dialog.field = DescribeField::Directory;
                if let Some(folders) = dialog.folders.as_mut() {
                    if list_open {
                        folders.close_dropdown();
                    } else {
                        folders.open_dropdown(Focus::Folder);
                    }
                }
                return;
            }
            // Create finishes with whatever was typed into an open list, so it
            // goes before the list is closed and that text thrown away.
            if contains(layout.create, col, row) {
                submit_describe_dialog(state);
                return;
            }
            // Anything else closes an open list before it does its own thing.
            if let Some(folders) = dialog.folders.as_mut() {
                folders.close_dropdown();
            }
            if contains(layout.text, col, row) {
                dialog.field = DescribeField::Text;
                let first = crate::ui::describe_dialog_text_first(dialog);
                dialog.text.click(
                    first + (row - layout.text.y) as usize,
                    (col - layout.text.x) as usize,
                );
                dialog.text.clear_selection();
            } else if layout.name.is_some_and(|rect| contains(rect, col, row)) {
                dialog.field = DescribeField::Name;
            } else if contains(layout.cancel, col, row) {
                cancel_describe_dialog(state);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::Mode;
    use crate::workspace::Workspace;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::empty())
    }

    fn type_text(state: &mut AppState, text: &str) {
        for c in text.chars() {
            handle_describe_dialog_key(state, key(KeyCode::Char(c)));
        }
    }

    fn new_space_state() -> AppState {
        let mut state = AppState::test_new();
        state.workspaces = vec![Workspace::test_new("one")];
        state.active = Some(0);
        let mut folders = state.composer.clone();
        folders.add_folder(std::path::PathBuf::from("/tmp"));
        let mut dialog = DescribeDialogState::new(DescribeTarget::NewSpace);
        dialog.folders = Some(folders);
        state.describe_dialog = Some(dialog);
        state.mode = Mode::DescribeDialog;
        state
    }

    #[test]
    fn enter_in_the_purpose_asks_for_a_new_space() {
        let mut state = new_space_state();
        type_text(&mut state, "ship it");
        handle_describe_dialog_key(&mut state, key(KeyCode::Enter));
        assert!(state.request_submit_describe_dialog);
        let dialog = state.describe_dialog.as_ref().unwrap();
        assert_eq!(dialog.text.text(), "ship it");
    }

    #[test]
    fn shift_enter_breaks_a_line_in_the_purpose() {
        let mut state = new_space_state();
        type_text(&mut state, "a");
        handle_describe_dialog_key(
            &mut state,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT),
        );
        type_text(&mut state, "b");
        assert!(!state.request_submit_describe_dialog);
        assert_eq!(state.describe_dialog.unwrap().text.text(), "a\nb");
    }

    #[test]
    fn typing_a_directory_settles_it_and_returns_to_the_purpose() {
        let mut state = new_space_state();
        handle_describe_dialog_key(&mut state, key(KeyCode::Tab));
        type_text(&mut state, "/");
        handle_describe_dialog_key(&mut state, key(KeyCode::Enter));
        let dialog = state.describe_dialog.as_ref().unwrap();
        assert_eq!(dialog.field, DescribeField::Text);
        assert_eq!(
            dialog.folders.as_ref().unwrap().folder_path(),
            Some(std::path::Path::new("/"))
        );
    }

    #[test]
    fn a_path_that_is_not_a_folder_says_so() {
        let mut state = new_space_state();
        handle_describe_dialog_key(&mut state, key(KeyCode::Tab));
        type_text(&mut state, "/definitely/not/here");
        handle_describe_dialog_key(&mut state, key(KeyCode::Enter));
        let dialog = state.describe_dialog.as_ref().unwrap();
        assert_eq!(dialog.field, DescribeField::Directory);
        assert!(dialog.error.as_deref().unwrap().contains("is not a folder"));
    }

    #[test]
    fn finishing_takes_a_directory_typed_but_not_settled() {
        let mut state = new_space_state();
        handle_describe_dialog_key(&mut state, key(KeyCode::Tab));
        type_text(&mut state, "/");
        submit_describe_dialog(&mut state);
        assert!(state.request_submit_describe_dialog);
        let dialog = state.describe_dialog.as_ref().unwrap();
        assert_eq!(
            dialog.folders.as_ref().unwrap().folder_path(),
            Some(std::path::Path::new("/"))
        );
    }

    #[test]
    fn finishing_with_a_typed_path_that_is_not_a_folder_stays_open() {
        let mut state = new_space_state();
        handle_describe_dialog_key(&mut state, key(KeyCode::Tab));
        type_text(&mut state, "/definitely/not/here");
        submit_describe_dialog(&mut state);
        assert!(!state.request_submit_describe_dialog);
        let dialog = state.describe_dialog.as_ref().unwrap();
        assert_eq!(dialog.field, DescribeField::Directory);
        assert!(dialog.error.as_deref().unwrap().contains("is not a folder"));
    }

    #[test]
    fn escape_closes_an_open_list_before_the_dialog() {
        let mut state = new_space_state();
        handle_describe_dialog_key(&mut state, key(KeyCode::Tab));
        handle_describe_dialog_key(&mut state, key(KeyCode::Enter));
        handle_describe_dialog_key(&mut state, key(KeyCode::Esc));
        assert_eq!(state.mode, Mode::DescribeDialog);
        handle_describe_dialog_key(&mut state, key(KeyCode::Esc));
        assert!(state.describe_dialog.is_none());
        assert_eq!(state.mode, Mode::Terminal);
    }

    #[test]
    fn editing_saves_the_purpose_and_blank_clears_it() {
        let mut state = AppState::test_new();
        state.workspaces = vec![Workspace::test_new("one")];
        state.active = Some(0);
        open_space_purpose_editor(&mut state, 0);
        type_text(&mut state, "  fix the build  ");
        handle_describe_dialog_key(&mut state, key(KeyCode::Enter));
        assert_eq!(
            state.workspaces[0].purpose.as_deref(),
            Some("fix the build")
        );
        assert!(state.describe_dialog.is_none());

        open_space_purpose_editor(&mut state, 0);
        handle_describe_dialog_key(
            &mut state,
            KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL),
        );
        handle_describe_dialog_key(&mut state, key(KeyCode::Enter));
        assert_eq!(state.workspaces[0].purpose, None);
    }

    fn tab_state() -> AppState {
        let mut state = AppState::test_new();
        state.workspaces = vec![Workspace::test_new("one")];
        state.active = Some(0);
        state
    }

    #[test]
    fn a_new_tab_asks_for_a_goal_and_an_optional_name() {
        let mut state = tab_state();
        open_new_tab_dialog(&mut state);
        type_text(&mut state, "reproduce the crash");
        handle_describe_dialog_key(&mut state, key(KeyCode::Tab));
        assert_eq!(
            state.describe_dialog.as_ref().unwrap().field,
            DescribeField::Name
        );
        type_text(&mut state, "crash");
        handle_describe_dialog_key(&mut state, key(KeyCode::Enter));
        assert!(state.request_new_tab);
        assert_eq!(state.requested_new_tab_name.as_deref(), Some("crash"));
        assert_eq!(
            state.requested_new_tab_goal.as_deref(),
            Some("reproduce the crash")
        );
        assert!(state.describe_dialog.is_none());
    }

    #[test]
    fn a_new_tab_without_a_name_keeps_its_number() {
        let mut state = tab_state();
        open_new_tab_dialog(&mut state);
        handle_describe_dialog_key(&mut state, key(KeyCode::Enter));
        assert!(state.request_new_tab);
        assert_eq!(state.requested_new_tab_name, None);
        assert_eq!(state.requested_new_tab_goal, None);
    }

    #[test]
    fn tab_walks_only_the_fields_the_dialog_has() {
        let mut state = tab_state();
        open_new_tab_dialog(&mut state);
        handle_describe_dialog_key(&mut state, key(KeyCode::Tab));
        handle_describe_dialog_key(&mut state, key(KeyCode::Tab));
        assert_eq!(
            state.describe_dialog.as_ref().unwrap().field,
            DescribeField::Text
        );
    }

    #[test]
    fn editing_a_tab_saves_its_goal() {
        let mut state = tab_state();
        open_tab_goal_editor(&mut state, 0, 0);
        type_text(&mut state, "land the fix");
        handle_describe_dialog_key(&mut state, key(KeyCode::Enter));
        assert_eq!(
            state.workspaces[0].tabs[0].goal.as_deref(),
            Some("land the fix")
        );
    }
}
