//! Key handling for the space dialog.
//!
//! The keyboard starts in the purpose, because the directory already holds the
//! folder the new space would have started in anyway. `Tab` moves between the
//! two fields. The directory field takes the same keys the launcher's
//! directory control takes: typing opens its list and searches it, the arrows
//! point, `Tab` completes and then settles, and `Enter` settles what was typed.
//! Settling a directory hands the keyboard back to the purpose. In the
//! purpose, `Enter` makes the space and `Shift+Enter` breaks a line.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;

use super::composer::edit_field;
use super::modal::leave_modal;
use crate::app::state::{AppState, SpaceDialogField, SpaceDialogState};
use crate::composer::Focus;

/// Open the dialog on an existing space, to edit only its purpose.
pub(crate) fn open_space_purpose_editor(state: &mut AppState, ws_idx: usize) {
    let Some(ws) = state.workspaces.get(ws_idx) else {
        return;
    };
    let mut purpose = crate::composer::TextField::default();
    purpose.set_text(ws.purpose.as_deref().unwrap_or_default());
    state.space_dialog = Some(SpaceDialogState {
        editing: Some(ws.id.clone()),
        folders: None,
        purpose,
        field: SpaceDialogField::Purpose,
        error: None,
    });
    state.hovered_space = None;
    state.mode = crate::app::Mode::SpaceDialog;
}

pub(crate) fn cancel_space_dialog(state: &mut AppState) {
    state.space_dialog = None;
    leave_modal(state);
}

/// Finish the dialog. An edit is applied here; a new space needs a pane, which
/// only the event loop can start, so it is asked for.
pub(crate) fn submit_space_dialog(state: &mut AppState) {
    let Some(dialog) = state.space_dialog.as_ref() else {
        return;
    };
    let Some(id) = dialog.editing.clone() else {
        state.request_submit_space_dialog = true;
        return;
    };
    let purpose = dialog.purpose.text();
    if let Some(ws) = state.workspaces.iter_mut().find(|ws| ws.id == id) {
        ws.set_purpose(&purpose);
        state.mark_session_dirty();
    }
    cancel_space_dialog(state);
}

pub(crate) fn handle_space_dialog_key(state: &mut AppState, key: KeyEvent) {
    let Some(dialog) = state.space_dialog.as_mut() else {
        leave_modal(state);
        return;
    };
    let plain = key.modifiers.difference(KeyModifiers::SHIFT).is_empty();

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
                dialog.field = SpaceDialogField::Purpose;
            }
            Some(Err(err)) => dialog.error = Some(err.message()),
            None => {}
        }
        return;
    }

    match key.code {
        KeyCode::Esc => return cancel_space_dialog(state),
        KeyCode::Tab | KeyCode::BackTab => {
            if dialog.folders.is_some() {
                dialog.field = match dialog.field {
                    SpaceDialogField::Purpose => SpaceDialogField::Directory,
                    SpaceDialogField::Directory => SpaceDialogField::Purpose,
                };
            }
            return;
        }
        _ => {}
    }

    match dialog.field {
        SpaceDialogField::Directory => {
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
        SpaceDialogField::Purpose => match key.code {
            // A purpose may run over several lines, so breaking a line and
            // finishing are different keys: the same pair the launcher's task
            // field uses.
            KeyCode::Enter if !key.modifiers.is_empty() => dialog.purpose.newline(),
            KeyCode::Char('j') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                dialog.purpose.newline()
            }
            KeyCode::Char('a') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                dialog.purpose.select_all()
            }
            KeyCode::Enter => submit_space_dialog(state),
            _ => {
                edit_field(&mut dialog.purpose, key.code, key.modifiers, plain);
            }
        },
    }
}

fn contains(rect: Rect, col: u16, row: u16) -> bool {
    col >= rect.x && col < rect.right() && row >= rect.y && row < rect.bottom()
}

/// The folder row under the pointer in the open list, if any.
fn list_index_at(state: &AppState, col: u16, row: u16) -> Option<usize> {
    let dialog = state.space_dialog.as_ref()?;
    let layout = crate::ui::space_dialog_layout(dialog, state.screen_rect())?;
    let list = layout.list.filter(|list| contains(*list, col, row))?;
    let index = crate::ui::space_dialog_list_first(dialog, list.height) + (row - list.y) as usize;
    let count = dialog.folders.as_ref()?.folder_rows().len();
    (index < count).then_some(index)
}

pub(crate) fn handle_space_dialog_mouse(state: &mut AppState, mouse: MouseEvent) {
    let (col, row) = (mouse.column, mouse.row);
    let hovered = list_index_at(state, col, row);
    let Some(dialog) = state.space_dialog.as_ref() else {
        return;
    };
    let Some(layout) = crate::ui::space_dialog_layout(dialog, state.screen_rect()) else {
        return;
    };
    let Some(dialog) = state.space_dialog.as_mut() else {
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
                        dialog.field = SpaceDialogField::Purpose;
                    }
                    Err(err) => dialog.error = Some(err.message()),
                }
                return;
            }
            if layout
                .directory
                .is_some_and(|rect| contains(rect, col, row))
            {
                dialog.field = SpaceDialogField::Directory;
                if let Some(folders) = dialog.folders.as_mut() {
                    if list_open {
                        folders.close_dropdown();
                    } else {
                        folders.open_dropdown(Focus::Folder);
                    }
                }
                return;
            }
            // Anything else closes an open list before it does its own thing.
            if let Some(folders) = dialog.folders.as_mut() {
                folders.close_dropdown();
            }
            if contains(layout.purpose, col, row) {
                dialog.field = SpaceDialogField::Purpose;
                let first = crate::ui::space_dialog_purpose_first(dialog);
                dialog.purpose.click(
                    first + (row - layout.purpose.y) as usize,
                    (col - layout.purpose.x) as usize,
                );
                dialog.purpose.clear_selection();
            } else if contains(layout.create, col, row) {
                submit_space_dialog(state);
            } else if contains(layout.cancel, col, row) {
                cancel_space_dialog(state);
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
            handle_space_dialog_key(state, key(KeyCode::Char(c)));
        }
    }

    fn new_space_state() -> AppState {
        let mut state = AppState::test_new();
        state.workspaces = vec![Workspace::test_new("one")];
        state.active = Some(0);
        let mut folders = state.composer.clone();
        folders.add_folder(std::path::PathBuf::from("/tmp"));
        state.space_dialog = Some(SpaceDialogState {
            editing: None,
            folders: Some(folders),
            purpose: Default::default(),
            field: SpaceDialogField::Purpose,
            error: None,
        });
        state.mode = Mode::SpaceDialog;
        state
    }

    #[test]
    fn enter_in_the_purpose_asks_for_a_new_space() {
        let mut state = new_space_state();
        type_text(&mut state, "ship it");
        handle_space_dialog_key(&mut state, key(KeyCode::Enter));
        assert!(state.request_submit_space_dialog);
        let dialog = state.space_dialog.as_ref().unwrap();
        assert_eq!(dialog.purpose.text(), "ship it");
    }

    #[test]
    fn shift_enter_breaks_a_line_in_the_purpose() {
        let mut state = new_space_state();
        type_text(&mut state, "a");
        handle_space_dialog_key(
            &mut state,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT),
        );
        type_text(&mut state, "b");
        assert!(!state.request_submit_space_dialog);
        assert_eq!(state.space_dialog.unwrap().purpose.text(), "a\nb");
    }

    #[test]
    fn typing_a_directory_settles_it_and_returns_to_the_purpose() {
        let mut state = new_space_state();
        handle_space_dialog_key(&mut state, key(KeyCode::Tab));
        type_text(&mut state, "/");
        handle_space_dialog_key(&mut state, key(KeyCode::Enter));
        let dialog = state.space_dialog.as_ref().unwrap();
        assert_eq!(dialog.field, SpaceDialogField::Purpose);
        assert_eq!(
            dialog.folders.as_ref().unwrap().folder_path(),
            Some(std::path::Path::new("/"))
        );
    }

    #[test]
    fn a_path_that_is_not_a_folder_says_so() {
        let mut state = new_space_state();
        handle_space_dialog_key(&mut state, key(KeyCode::Tab));
        type_text(&mut state, "/definitely/not/here");
        handle_space_dialog_key(&mut state, key(KeyCode::Enter));
        let dialog = state.space_dialog.as_ref().unwrap();
        assert_eq!(dialog.field, SpaceDialogField::Directory);
        assert!(dialog.error.as_deref().unwrap().contains("is not a folder"));
    }

    #[test]
    fn escape_closes_an_open_list_before_the_dialog() {
        let mut state = new_space_state();
        handle_space_dialog_key(&mut state, key(KeyCode::Tab));
        handle_space_dialog_key(&mut state, key(KeyCode::Enter));
        handle_space_dialog_key(&mut state, key(KeyCode::Esc));
        assert_eq!(state.mode, Mode::SpaceDialog);
        handle_space_dialog_key(&mut state, key(KeyCode::Esc));
        assert!(state.space_dialog.is_none());
        assert_eq!(state.mode, Mode::Terminal);
    }

    #[test]
    fn editing_saves_the_purpose_and_blank_clears_it() {
        let mut state = AppState::test_new();
        state.workspaces = vec![Workspace::test_new("one")];
        state.active = Some(0);
        open_space_purpose_editor(&mut state, 0);
        type_text(&mut state, "  fix the build  ");
        handle_space_dialog_key(&mut state, key(KeyCode::Enter));
        assert_eq!(
            state.workspaces[0].purpose.as_deref(),
            Some("fix the build")
        );
        assert!(state.space_dialog.is_none());

        open_space_purpose_editor(&mut state, 0);
        handle_space_dialog_key(
            &mut state,
            KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL),
        );
        handle_space_dialog_key(&mut state, key(KeyCode::Enter));
        assert_eq!(state.workspaces[0].purpose, None);
    }
}
