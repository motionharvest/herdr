use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Direction, Rect};

use crate::{
    app::state::{
        is_land_menu_item, land_prompt_text, AppState, ContextMenuKind, ContextMenuState,
        MenuListState, Mode, NavigatorStateFilter,
    },
    input::TerminalKey,
    layout::NavDirection,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ModalAction {
    Continue,
    Save,
    Clear,
    Cancel,
    Confirm,
    Apply,
    Close,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ModalKeyBinding {
    Enter,
    Esc,
    CtrlC,
}

impl ModalKeyBinding {
    fn matches(self, key: &KeyEvent) -> bool {
        match self {
            Self::Enter => key.code == KeyCode::Enter,
            Self::Esc => key.code == KeyCode::Esc,
            Self::CtrlC => {
                key.code == KeyCode::Char('c')
                    && key.modifiers == crossterm::event::KeyModifiers::CONTROL
            }
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) struct ModalActionSpec<A> {
    pub action: A,
    pub bindings: &'static [ModalKeyBinding],
}

pub(super) fn modal_action_from_key<A: Copy>(
    key: &KeyEvent,
    specs: &[ModalActionSpec<A>],
) -> Option<A> {
    specs
        .iter()
        .find(|spec| spec.bindings.iter().any(|binding| binding.matches(key)))
        .map(|spec| spec.action)
}

pub(super) fn modal_action_from_buttons<A: Copy>(
    col: u16,
    row: u16,
    buttons: &[(Rect, A)],
) -> Option<A> {
    buttons.iter().find_map(|(rect, action)| {
        (col >= rect.x && col < rect.x + rect.width && row >= rect.y && row < rect.y + rect.height)
            .then_some(*action)
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GlobalMenuAction {
    Detach,
    WhatsNew,
    Keybinds,
    ReloadConfig,
    Settings,
}

pub(super) fn global_menu_actions(state: &AppState) -> Vec<GlobalMenuAction> {
    let mut actions = vec![
        GlobalMenuAction::Settings,
        GlobalMenuAction::Keybinds,
        GlobalMenuAction::ReloadConfig,
    ];
    if state.update_available.is_some() || state.latest_release_notes_available {
        actions.push(GlobalMenuAction::WhatsNew);
    }
    actions.push(GlobalMenuAction::Detach);
    actions
}

pub(super) fn open_global_menu(state: &mut AppState) {
    state.composer.close_dropdown();
    state.global_menu = MenuListState::new(0);
    state.mode = Mode::GlobalMenu;
}

pub(super) fn open_keybind_help(state: &mut AppState) {
    state.keybind_help.scroll = 0;
    state.mode = Mode::KeybindHelp;
}

fn open_update_release_notes(state: &mut AppState) {
    let Some(notes) = crate::release_notes::load_latest() else {
        return;
    };

    state.release_notes = Some(crate::app::state::ReleaseNotesState {
        version: notes.version,
        body: notes.body,
        scroll: 0,
        preview: notes.preview,
    });
    state.mode = Mode::ReleaseNotes;
}

pub(super) fn request_detach(state: &mut AppState) {
    if state.detach_exits {
        state.should_quit = true;
    } else {
        state.detach_requested = true;
    }
}

pub(super) fn apply_global_menu_action(state: &mut AppState, action: GlobalMenuAction) {
    match action {
        GlobalMenuAction::Detach => {
            leave_modal(state);
            request_detach(state);
        }
        GlobalMenuAction::WhatsNew => open_update_release_notes(state),
        GlobalMenuAction::Keybinds => open_keybind_help(state),
        GlobalMenuAction::ReloadConfig => {
            state.request_reload_config = true;
            leave_modal(state);
        }
        GlobalMenuAction::Settings => super::settings::open_settings(state),
    }
}

pub(crate) fn handle_global_menu_key(state: &mut AppState, key: KeyEvent) {
    let actions = global_menu_actions(state);
    match key.code {
        KeyCode::Esc => leave_modal(state),
        KeyCode::Up | KeyCode::Char('k') => state.global_menu.move_prev(),
        KeyCode::Down | KeyCode::Char('j') => state.global_menu.move_next(actions.len()),
        KeyCode::Enter => {
            if let Some(action) = actions.get(state.global_menu.highlighted).copied() {
                apply_global_menu_action(state, action);
            }
        }
        _ => {}
    }
}

pub(crate) fn handle_navigator_key(
    state: &mut AppState,
    terminal_runtimes: &crate::terminal::TerminalRuntimeRegistry,
    key: KeyEvent,
) {
    if state.navigator.search_focused {
        match key.code {
            KeyCode::Esc => {
                if state.navigator.query.is_empty() {
                    state.navigator.search_focused = false;
                    leave_modal(state);
                } else {
                    state.navigator.query.clear();
                    state.navigator.state_filter = None;
                    state.navigator.search_focused = false;
                    state.clamp_navigator_selection_from(terminal_runtimes);
                }
            }
            KeyCode::Enter => {
                state.accept_navigator_selection_from(terminal_runtimes);
            }
            KeyCode::Backspace => {
                state.navigator.state_filter = None;
                state.navigator.query.pop();
                state.clamp_navigator_selection_from(terminal_runtimes);
            }
            KeyCode::Up => state.move_navigator_selection_from(terminal_runtimes, -1),
            KeyCode::Down => state.move_navigator_selection_from(terminal_runtimes, 1),
            KeyCode::Char('n') if key.modifiers == KeyModifiers::CONTROL => {
                state.move_navigator_selection_from(terminal_runtimes, 1)
            }
            KeyCode::Char('p') if key.modifiers == KeyModifiers::CONTROL => {
                state.move_navigator_selection_from(terminal_runtimes, -1)
            }
            KeyCode::Char('u') if key.modifiers == KeyModifiers::CONTROL => {
                state.navigator.query.clear();
                state.navigator.state_filter = None;
                state.clamp_navigator_selection_from(terminal_runtimes);
            }
            KeyCode::Char(c)
                if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT =>
            {
                state.navigator.state_filter = None;
                state.navigator.query.push(c);
                state.clamp_navigator_selection_from(terminal_runtimes);
            }
            _ => {}
        }
        return;
    }

    match key.code {
        KeyCode::Esc => {
            if state.navigator.query.is_empty() && state.navigator.state_filter.is_none() {
                leave_modal(state);
            } else {
                state.navigator.query.clear();
                state.navigator.state_filter = None;
                state.clamp_navigator_selection_from(terminal_runtimes);
            }
        }
        KeyCode::Enter => {
            state.accept_navigator_selection_from(terminal_runtimes);
        }
        KeyCode::Char('/') => {
            state.navigator.query.clear();
            state.navigator.state_filter = None;
            state.navigator.search_focused = true;
            state.clamp_navigator_selection_from(terminal_runtimes);
        }
        KeyCode::Backspace if state.navigator.state_filter.is_some() => {
            state.navigator.state_filter = None;
            state.clamp_navigator_selection_from(terminal_runtimes);
        }
        KeyCode::Char('a') if key.modifiers.is_empty() => {
            state.navigator.query.clear();
            state.navigator.state_filter = None;
            state.clamp_navigator_selection_from(terminal_runtimes);
        }
        KeyCode::Char('b') if key.modifiers.is_empty() => {
            state.navigator.query.clear();
            state.navigator.state_filter = Some(NavigatorStateFilter::Blocked);
            state.clamp_navigator_selection_from(terminal_runtimes);
        }
        KeyCode::Char('w') if key.modifiers.is_empty() => {
            state.navigator.query.clear();
            state.navigator.state_filter = Some(NavigatorStateFilter::Working);
            state.clamp_navigator_selection_from(terminal_runtimes);
        }
        KeyCode::Char('i') if key.modifiers.is_empty() => {
            state.navigator.query.clear();
            state.navigator.state_filter = Some(NavigatorStateFilter::Idle);
            state.clamp_navigator_selection_from(terminal_runtimes);
        }
        KeyCode::Char('d') if key.modifiers.is_empty() => {
            state.navigator.query.clear();
            state.navigator.state_filter = Some(NavigatorStateFilter::Done);
            state.clamp_navigator_selection_from(terminal_runtimes);
        }
        KeyCode::Char('j') | KeyCode::Down => {
            state.move_navigator_selection_from(terminal_runtimes, 1)
        }
        KeyCode::Char('k') | KeyCode::Up => {
            state.move_navigator_selection_from(terminal_runtimes, -1)
        }
        KeyCode::Char('d') if key.modifiers == KeyModifiers::CONTROL => state
            .move_navigator_selection_from(
                terminal_runtimes,
                (state.navigator_body_rect().height / 2).max(1) as isize,
            ),
        KeyCode::Char('u') if key.modifiers == KeyModifiers::CONTROL => state
            .move_navigator_selection_from(
                terminal_runtimes,
                -((state.navigator_body_rect().height / 2).max(1) as isize),
            ),
        KeyCode::Char(' ') => state.toggle_selected_navigator_workspace_from(terminal_runtimes),
        KeyCode::Home => {
            state.navigator.selected = 0;
            state.ensure_navigator_selection_visible_from(terminal_runtimes);
        }
        KeyCode::End | KeyCode::Char('G') => {
            state.navigator.selected = state
                .navigator_rows_from(terminal_runtimes)
                .len()
                .saturating_sub(1);
            state.ensure_navigator_selection_visible_from(terminal_runtimes);
        }
        _ => {}
    }
}

pub(crate) fn handle_keybind_help_key(state: &mut AppState, key: KeyEvent) {
    match key.code {
        KeyCode::Up | KeyCode::Char('k') => state.scroll_keybind_help(-1),
        KeyCode::Down | KeyCode::Char('j') => state.scroll_keybind_help(1),
        KeyCode::PageUp => state.scroll_keybind_help(-8),
        KeyCode::PageDown => state.scroll_keybind_help(8),
        KeyCode::Home => state.keybind_help.scroll = 0,
        KeyCode::End => state.keybind_help.scroll = state.keybind_help_max_scroll(),
        KeyCode::Esc | KeyCode::Enter | KeyCode::Char('?') => leave_modal(state),
        _ => {}
    }
}

pub(super) fn open_rename_workspace(
    state: &mut AppState,
    terminal_runtimes: &crate::terminal::TerminalRuntimeRegistry,
    ws_idx: usize,
) {
    state.selected = ws_idx;
    state.rename_pane_target = None;
    state.name_input =
        state.workspaces[ws_idx].display_name_from(&state.terminals, terminal_runtimes);
    state.name_input_replace_on_type = false;
    state.mode = Mode::RenameWorkspace;
}

pub(super) fn open_rename_pane(state: &mut AppState, pane_id: crate::layout::PaneId) {
    let Some(terminal_id) = terminal_id_for_pane(state, pane_id) else {
        return;
    };
    let terminal = state.terminals.get(&terminal_id);
    state.rename_pane_target = Some(pane_id);
    state.name_input = terminal
        .and_then(|t| t.manual_label.clone())
        .unwrap_or_default();
    state.name_input_replace_on_type = terminal.and_then(|t| t.manual_label.as_ref()).is_none();
    state.mode = Mode::RenamePane;
}

fn land_agent_prompt_target(state: &AppState, pane_id: crate::layout::PaneId) -> String {
    crate::ui::agent_panel_entries(state)
        .into_iter()
        .find(|entry| entry.pane_id == pane_id)
        .map(|entry| entry.name)
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| format!("p_{}", pane_id.raw()))
}

pub(super) fn leave_modal(state: &mut AppState) {
    if state.active.is_some() {
        state.mode = Mode::Terminal;
    } else {
        state.mode = Mode::Navigate;
    }
}

pub(super) const ONBOARDING_WELCOME_ACTIONS: &[ModalActionSpec<ModalAction>] = &[ModalActionSpec {
    action: ModalAction::Continue,
    bindings: &[ModalKeyBinding::Enter],
}];

pub(super) const RELEASE_NOTES_ACTIONS: &[ModalActionSpec<ModalAction>] = &[ModalActionSpec {
    action: ModalAction::Close,
    bindings: &[ModalKeyBinding::Enter, ModalKeyBinding::Esc],
}];

pub(super) const RENAME_ACTIONS: &[ModalActionSpec<ModalAction>] = &[
    ModalActionSpec {
        action: ModalAction::Save,
        bindings: &[ModalKeyBinding::Enter],
    },
    ModalActionSpec {
        action: ModalAction::Clear,
        bindings: &[ModalKeyBinding::CtrlC],
    },
    ModalActionSpec {
        action: ModalAction::Cancel,
        bindings: &[ModalKeyBinding::Esc],
    },
];

pub(super) const CONFIRM_CLOSE_ACTIONS: &[ModalActionSpec<ModalAction>] = &[
    ModalActionSpec {
        action: ModalAction::Confirm,
        bindings: &[ModalKeyBinding::Enter],
    },
    ModalActionSpec {
        action: ModalAction::Cancel,
        bindings: &[ModalKeyBinding::Esc],
    },
];

pub(super) const SETTINGS_ACTIONS: &[ModalActionSpec<ModalAction>] = &[
    ModalActionSpec {
        action: ModalAction::Apply,
        bindings: &[ModalKeyBinding::Enter],
    },
    ModalActionSpec {
        action: ModalAction::Close,
        bindings: &[ModalKeyBinding::Esc],
    },
];

pub(super) fn apply_rename_action(state: &mut AppState, action: ModalAction) {
    match action {
        ModalAction::Save => {
            let new_name = if state.name_input.trim().is_empty() {
                state.name_input.clone()
            } else {
                state.name_input.trim().to_string()
            };
            match state.mode {
                Mode::RenameWorkspace if !state.workspaces.is_empty() && !new_name.is_empty() => {
                    let workspace_id = state.workspaces[state.selected].id.clone();
                    state.workspaces[state.selected].set_custom_name(new_name);
                    crate::logging::workspace_renamed(&workspace_id);
                    state.mark_session_dirty();
                }
                Mode::RenamePane => {
                    if let Some(pane_id) = state.rename_pane_target {
                        if let Some(terminal_id) = terminal_id_for_pane(state, pane_id) {
                            if let Some(terminal) = state.terminals.get_mut(&terminal_id) {
                                terminal.set_manual_label(new_name);
                                state.mark_session_dirty();
                            }
                        }
                    }
                }
                _ => {}
            }
            state.creating_new_tab = false;
            state.rename_pane_target = None;
            state.name_input.clear();
            state.name_input_replace_on_type = false;
            leave_modal(state);
        }
        ModalAction::Clear => {
            state.name_input.clear();
            state.name_input_replace_on_type = false;
        }
        ModalAction::Cancel => {
            state.creating_new_tab = false;
            state.requested_new_tab_name = None;
            state.rename_pane_target = None;
            state.name_input.clear();
            state.name_input_replace_on_type = false;
            leave_modal(state);
        }
        _ => {}
    }
}

fn terminal_id_for_pane(
    state: &AppState,
    pane_id: crate::layout::PaneId,
) -> Option<crate::terminal::TerminalId> {
    state
        .workspaces
        .iter()
        .find_map(|workspace| workspace.pane_state(pane_id))
        .map(|pane| pane.attached_terminal_id.clone())
        .or_else(|| {
            state
                .detached_agents
                .iter()
                .find(|agent| agent.pane_id == pane_id)
                .map(|agent| agent.pane.attached_terminal_id.clone())
        })
}

fn clear_rename_input(state: &mut AppState) {
    state.name_input.clear();
    state.name_input_replace_on_type = false;
}

fn delete_rename_input_char(state: &mut AppState) {
    if state.name_input_replace_on_type {
        clear_rename_input(state);
    } else {
        state.name_input.pop();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RenameWordDeleteClass {
    Word,
    Separator,
}

fn rename_word_delete_class(ch: char) -> RenameWordDeleteClass {
    if ch.is_alphanumeric() || ch == '_' {
        RenameWordDeleteClass::Word
    } else {
        RenameWordDeleteClass::Separator
    }
}

fn delete_rename_input_word(state: &mut AppState) {
    if state.name_input_replace_on_type {
        clear_rename_input(state);
        return;
    }

    while state
        .name_input
        .chars()
        .last()
        .is_some_and(char::is_whitespace)
    {
        state.name_input.pop();
    }

    let Some(class) = state
        .name_input
        .chars()
        .last()
        .map(rename_word_delete_class)
    else {
        return;
    };

    while state
        .name_input
        .chars()
        .last()
        .is_some_and(|ch| !ch.is_whitespace() && rename_word_delete_class(ch) == class)
    {
        state.name_input.pop();
    }
}

pub(crate) fn handle_rename_key(state: &mut AppState, key: KeyEvent) {
    if let Some(action) = modal_action_from_key(&key, RENAME_ACTIONS) {
        apply_rename_action(state, action);
        return;
    }

    match key.code {
        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            clear_rename_input(state);
        }
        KeyCode::Backspace if key.modifiers.contains(KeyModifiers::SUPER) => {
            clear_rename_input(state);
        }
        KeyCode::Backspace
            if key.modifiers.contains(KeyModifiers::CONTROL)
                || key.modifiers.contains(KeyModifiers::ALT) =>
        {
            delete_rename_input_word(state);
        }
        KeyCode::Char('h' | 'w') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            delete_rename_input_word(state);
        }
        KeyCode::Backspace => delete_rename_input_char(state),
        KeyCode::Char(c) if key.modifiers.difference(KeyModifiers::SHIFT).is_empty() => {
            if state.name_input_replace_on_type {
                clear_rename_input(state);
            }
            state.name_input.push(c);
        }
        _ => {}
    }
}

pub(crate) fn handle_resize_key(state: &mut AppState, raw_key: TerminalKey) {
    let key = raw_key.as_key_event();
    if key.code == KeyCode::Esc
        || key.code == KeyCode::Enter
        || state.keybinds.resize_mode.matches_prefix_key(raw_key)
        || state.keybinds.resize_mode.matches_direct_key(raw_key)
    {
        if state.active.is_some() {
            state.mode = Mode::Terminal;
        } else {
            state.mode = Mode::Navigate;
        }
        return;
    }

    match key.code {
        KeyCode::Char('h') | KeyCode::Left => state.resize_pane(NavDirection::Left),
        KeyCode::Char('l') | KeyCode::Right => state.resize_pane(NavDirection::Right),
        KeyCode::Char('j') | KeyCode::Down => state.resize_pane(NavDirection::Down),
        KeyCode::Char('k') | KeyCode::Up => state.resize_pane(NavDirection::Up),
        _ => {}
    }
}

pub(super) fn open_confirm_close(state: &mut AppState) {
    state.mode = Mode::ConfirmClose;
}

pub(super) fn confirm_close_accept(state: &mut AppState) {
    state.close_selected_workspace();
    if state.workspaces.is_empty() {
        state.mode = Mode::Navigate;
    } else {
        state.mode = Mode::Terminal;
    }
}

pub(super) fn confirm_close_cancel(state: &mut AppState) {
    state.mode = Mode::Navigate;
}

pub(crate) fn handle_confirm_close_key(state: &mut AppState, key: KeyEvent) {
    match modal_action_from_key(&key, CONFIRM_CLOSE_ACTIONS) {
        Some(ModalAction::Confirm) => confirm_close_accept(state),
        Some(ModalAction::Cancel) => confirm_close_cancel(state),
        _ => {}
    }
}

pub(super) fn apply_context_menu_action(
    state: &mut AppState,
    terminal_runtimes: &mut crate::terminal::TerminalRuntimeRegistry,
    menu: ContextMenuState,
    idx: usize,
) {
    if !menu.item_enabled(idx) {
        state.context_menu = Some(menu);
        return;
    }
    let items = menu.items();
    let item = items.get(idx).map(String::as_str);
    match (menu.kind, item) {
        (ContextMenuKind::DetachedAgent { pane_id }, Some("Delete agent" | "Close agent")) => {
            state.close_detached_agent(pane_id);
            leave_modal(state);
        }
        (ContextMenuKind::Agent { ws_idx, .. }, Some("New worktree")) => {
            state.request_new_linked_worktree = Some(ws_idx);
            leave_modal(state);
        }
        (ContextMenuKind::Agent { ws_idx, .. }, Some("Delete agent / worktree...")) => {
            state.request_remove_linked_worktree = Some(ws_idx);
            leave_modal(state);
        }
        (ContextMenuKind::Agent { pane_id, .. }, Some(item)) if is_land_menu_item(item) => {
            state.request_land_agent_prompt =
                Some((land_agent_prompt_target(state, pane_id), land_prompt_text()));
            leave_modal(state);
        }
        (ContextMenuKind::Agent { ws_idx, .. }, Some("Open worktree...")) => {
            state.request_open_existing_worktree = Some(ws_idx);
            leave_modal(state);
        }
        (ContextMenuKind::Agent { pane_id, .. }, Some("Rename agent"))
        | (ContextMenuKind::Pane { pane_id, .. }, Some("Rename pane")) => {
            open_rename_pane(state, pane_id);
        }
        (ContextMenuKind::Pane { pane_id, .. }, Some("Clear pane name")) => {
            if let Some(ws_idx) = state.active {
                if let Some(ws) = state.workspaces.get(ws_idx) {
                    if let Some(pane) = ws.pane_state(pane_id) {
                        let terminal_id = pane.attached_terminal_id.clone();
                        if let Some(terminal) = state.terminals.get_mut(&terminal_id) {
                            terminal.clear_manual_label();
                            state.mark_session_dirty();
                        }
                    }
                }
            }
            state.mode = Mode::Terminal;
        }
        (ContextMenuKind::Pane { .. }, Some("Split vertically")) => {
            state.split_pane(terminal_runtimes, Direction::Vertical);
            state.mode = Mode::Terminal;
        }
        (ContextMenuKind::Pane { .. }, Some("Split horizontally")) => {
            state.split_pane(terminal_runtimes, Direction::Horizontal);
            state.mode = Mode::Terminal;
        }
        (ContextMenuKind::Pane { .. }, Some("Zoom")) => {
            state.toggle_zoom();
            state.mode = Mode::Terminal;
        }
        (ContextMenuKind::Pane { pane_id, .. }, Some("Dim" | "Undim")) => {
            state.toggle_pane_dimmed(pane_id);
            state.mode = Mode::Terminal;
        }
        (ContextMenuKind::Pane { pane_id, .. }, Some("Reset agent")) => {
            state.reset_agent_in_pane(terminal_runtimes, pane_id);
            leave_modal(state);
        }
        (
            ContextMenuKind::Agent { pane_id, .. } | ContextMenuKind::Pane { pane_id, .. },
            Some("Delete agent" | "Close agent"),
        ) => {
            state.close_agent_in_pane(terminal_runtimes, pane_id);
            leave_modal(state);
        }
        (ContextMenuKind::Pane { .. }, Some("Close pane")) => {
            if !state.close_pane() {
                state.mode = if state.active.is_some() {
                    Mode::Terminal
                } else {
                    Mode::Navigate
                };
            }
        }
        _ => leave_modal(state),
    }
}

pub(crate) fn handle_context_menu_key(
    state: &mut AppState,
    terminal_runtimes: &mut crate::terminal::TerminalRuntimeRegistry,
    key: KeyEvent,
) {
    match key.code {
        KeyCode::Esc => {
            state.context_menu = None;
            leave_modal(state);
        }
        KeyCode::Up => {
            if let Some(menu) = &mut state.context_menu {
                menu.list.move_prev();
            }
        }
        KeyCode::Down => {
            if let Some(menu) = &mut state.context_menu {
                menu.list.move_next(menu.items().len());
            }
        }
        KeyCode::Enter => {
            if let Some(menu) = state.context_menu.take() {
                let idx = menu.list.highlighted;
                apply_context_menu_action(state, terminal_runtimes, menu, idx);
            }
        }
        _ => {}
    }
}

impl AppState {
    pub(super) fn global_menu_item_at(&self, col: u16, row: u16) -> Option<GlobalMenuAction> {
        let rect = self.global_menu_rect();
        if col <= rect.x
            || col >= rect.x + rect.width.saturating_sub(1)
            || row <= rect.y
            || row >= rect.y + rect.height.saturating_sub(1)
        {
            return None;
        }
        let idx = (row - rect.y - 1) as usize;
        global_menu_actions(self).get(idx).copied()
    }
}

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    use super::super::state_with_workspaces;
    use super::*;
    use crate::app::state::SpaceMenuKind;

    fn config_env_lock() -> &'static std::sync::Mutex<()> {
        crate::config::test_config_env_lock()
    }

    fn temp_config_path(name: &str) -> std::path::PathBuf {
        let unique = format!(
            "herdr-modal-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        std::env::temp_dir().join(unique).join("config.toml")
    }

    #[test]
    fn custom_resize_key_exits_resize_mode() {
        let mut state = state_with_workspaces(&["test"]);
        state.mode = Mode::Resize;
        state.keybinds.resize_mode = crate::config::ActionKeybinds::prefix("g");

        handle_resize_key(
            &mut state,
            TerminalKey::new(KeyCode::Char('g'), KeyModifiers::empty()),
        );

        assert_eq!(state.mode, Mode::Terminal);
    }

    #[test]
    fn direct_resize_key_exits_resize_mode() {
        let mut state = state_with_workspaces(&["test"]);
        state.mode = Mode::Resize;
        state.keybinds.resize_mode = crate::config::ActionKeybinds::direct("ctrl+alt+r");

        handle_resize_key(
            &mut state,
            TerminalKey::new(
                KeyCode::Char('r'),
                KeyModifiers::CONTROL | KeyModifiers::ALT,
            ),
        );

        assert_eq!(state.mode, Mode::Terminal);
    }

    #[test]
    fn resize_key_exit_matches_enhanced_shifted_punctuation() {
        let mut state = state_with_workspaces(&["test"]);
        state.mode = Mode::Resize;
        state.keybinds.resize_mode = crate::config::ActionKeybinds::prefix("?");

        handle_resize_key(
            &mut state,
            TerminalKey::new(KeyCode::Char('/'), KeyModifiers::SHIFT)
                .with_shifted_codepoint('?' as u32),
        );

        assert_eq!(state.mode, Mode::Terminal);
    }

    #[test]
    fn detach_requests_client_detach_in_persistence_mode() {
        let mut state = state_with_workspaces(&["test"]);
        state.detach_exits = false;

        request_detach(&mut state);

        assert!(state.detach_requested);
        assert!(!state.should_quit);
    }

    #[test]
    fn detach_exits_in_no_session_mode() {
        let mut state = state_with_workspaces(&["test"]);
        state.detach_exits = true;

        request_detach(&mut state);

        assert!(state.should_quit);
        assert!(!state.detach_requested);
    }

    #[test]
    fn global_menu_whats_new_opens_saved_release_notes() {
        let _guard = config_env_lock().lock().unwrap();
        let path = temp_config_path("whats-new-saved-release-notes");
        std::env::set_var(crate::config::CONFIG_PATH_ENV_VAR, &path);
        crate::release_notes::save_pending(env!("CARGO_PKG_VERSION"), "### Changed\n- Menu")
            .unwrap();

        let mut state = state_with_workspaces(&["test"]);
        state.latest_release_notes_available = true;

        assert!(global_menu_actions(&state).contains(&GlobalMenuAction::WhatsNew));

        apply_global_menu_action(&mut state, GlobalMenuAction::WhatsNew);

        assert_eq!(state.mode, Mode::ReleaseNotes);
        assert_eq!(
            state
                .release_notes
                .as_ref()
                .map(|notes| notes.body.as_str()),
            Some("### Changed\n- Menu")
        );

        std::env::remove_var(crate::config::CONFIG_PATH_ENV_VAR);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }
    #[test]
    fn rename_modal_handles_line_editing_shortcuts() {
        let mut state = state_with_workspaces(&["test"]);
        state.mode = Mode::RenameWorkspace;
        state.name_input = "website zero".into();

        handle_rename_key(
            &mut state,
            KeyEvent::new(KeyCode::Backspace, KeyModifiers::empty()),
        );
        assert_eq!(state.name_input, "website zer");

        handle_rename_key(
            &mut state,
            KeyEvent::new(KeyCode::Backspace, KeyModifiers::CONTROL),
        );
        assert_eq!(state.name_input, "website ");

        state.name_input = "website-zero".into();
        handle_rename_key(
            &mut state,
            KeyEvent::new(KeyCode::Backspace, KeyModifiers::ALT),
        );
        assert_eq!(state.name_input, "website-");

        state.name_input = "website-zero".into();
        handle_rename_key(
            &mut state,
            KeyEvent::new(KeyCode::Char('h'), KeyModifiers::CONTROL),
        );
        assert_eq!(state.name_input, "website-");

        state.name_input = "website-zero".into();
        handle_rename_key(
            &mut state,
            KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL),
        );
        assert_eq!(state.name_input, "website-");

        handle_rename_key(
            &mut state,
            KeyEvent::new(KeyCode::Backspace, KeyModifiers::SUPER),
        );
        assert!(state.name_input.is_empty());

        state.name_input = "website zero".into();
        handle_rename_key(
            &mut state,
            KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL),
        );
        assert!(state.name_input.is_empty());
    }

    #[test]
    fn rename_modal_does_not_insert_modified_shortcut_chars() {
        let mut state = state_with_workspaces(&["test"]);
        state.mode = Mode::RenameWorkspace;
        state.name_input = "website".into();

        handle_rename_key(
            &mut state,
            KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL),
        );
        assert_eq!(state.name_input, "website");

        handle_rename_key(
            &mut state,
            KeyEvent::new(KeyCode::Char('Z'), KeyModifiers::SHIFT),
        );
        assert_eq!(state.name_input, "websiteZ");
    }
    #[test]
    fn confirm_close_keyboard_actions_are_direct_not_focused() {
        let mut state = state_with_workspaces(&["a", "b"]);
        state.mode = Mode::ConfirmClose;
        state.selected = 1;

        handle_confirm_close_key(
            &mut state,
            KeyEvent::new(KeyCode::Esc, KeyModifiers::empty()),
        );
        assert_eq!(state.mode, Mode::Navigate);
        assert_eq!(state.workspaces.len(), 2);

        state.mode = Mode::ConfirmClose;
        handle_confirm_close_key(
            &mut state,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::empty()),
        );
        assert_eq!(state.workspaces.len(), 1);
    }

    #[test]
    fn confirm_close_for_linked_worktree_closes_workspace_only() {
        let mut state = state_with_workspaces(&["main", "issue"]);
        state.mode = Mode::ConfirmClose;
        state.selected = 1;
        state.workspaces[1].worktree_space = Some(crate::workspace::WorktreeSpaceMembership {
            key: "repo-key".into(),
            label: "herdr".into(),
            repo_root: "/repo/herdr".into(),
            checkout_path: "/repo/herdr-issue".into(),
            is_linked_worktree: true,
        });

        handle_confirm_close_key(
            &mut state,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::empty()),
        );

        assert_eq!(state.request_remove_linked_worktree, None);
        assert_eq!(state.workspaces.len(), 1);
        assert_eq!(state.workspaces[0].display_name(), "main");
        assert_eq!(state.mode, Mode::Terminal);
    }
    #[test]
    fn context_menu_close_pane_last_parent_group_pane_keeps_confirmation_mode() {
        let mut state = state_with_workspaces(&["main", "issue"]);
        state.active = Some(0);
        state.selected = 1;
        state.workspaces[0].worktree_space = Some(crate::workspace::WorktreeSpaceMembership {
            key: "repo-key".into(),
            label: "herdr".into(),
            repo_root: "/repo/herdr".into(),
            checkout_path: "/repo/herdr".into(),
            is_linked_worktree: false,
        });
        state.workspaces[1].worktree_space = Some(crate::workspace::WorktreeSpaceMembership {
            key: "repo-key".into(),
            label: "herdr".into(),
            repo_root: "/repo/herdr".into(),
            checkout_path: "/repo/herdr-issue".into(),
            is_linked_worktree: true,
        });
        let pane_id = state.workspaces[0].tabs[0].root_pane;
        let menu = ContextMenuState {
            kind: ContextMenuKind::Pane {
                pane_id,
                has_manual_label: false,
                dimmed: false,
                has_agent: false,
                can_reset: false,
            },
            x: 0,
            y: 0,
            list: MenuListState::new(0),
        };
        let idx = menu
            .items()
            .iter()
            .position(|item| item == "Close pane")
            .expect("close pane item");
        let mut terminal_runtimes = crate::terminal::TerminalRuntimeRegistry::new();

        apply_context_menu_action(&mut state, &mut terminal_runtimes, menu, idx);

        assert_eq!(state.selected, 0);
        assert_eq!(state.mode, Mode::ConfirmClose);
        assert_eq!(state.workspaces.len(), 2);
    }

    #[test]
    fn landed_worktree_menu_does_not_start_land() {
        let mut state = state_with_workspaces(&["main"]);
        state.mode = Mode::ContextMenu;
        let menu = ContextMenuState {
            kind: ContextMenuKind::Agent {
                ws_idx: 0,
                pane_id: state.workspaces[0].tabs[0].root_pane,
                space: SpaceMenuKind::LinkedWorktree {
                    parent_branch: Some("main".into()),
                    already_landed: true,
                    in_worktree_directory: true,
                },
            },
            x: 0,
            y: 0,
            list: MenuListState::new(0),
        };
        let land_idx = menu
            .items()
            .iter()
            .position(|item| is_land_menu_item(item))
            .expect("land item");
        let mut terminal_runtimes = crate::terminal::TerminalRuntimeRegistry::new();

        apply_context_menu_action(&mut state, &mut terminal_runtimes, menu, land_idx);

        assert_eq!(state.request_land_worktree, None);
        assert!(state.request_land_agent_prompt.is_none());
        assert!(state.context_menu.is_some());
        assert_eq!(state.mode, Mode::ContextMenu);
    }

    #[test]
    fn land_menu_prompts_the_agent_instead_of_running_git() {
        let mut state = state_with_workspaces(&["main"]);
        state.ensure_test_terminals();
        state.mode = Mode::ContextMenu;
        let pane_id = state.workspaces[0].tabs[0].root_pane;
        let terminal_id = state.workspaces[0]
            .pane_state(pane_id)
            .expect("agent pane")
            .attached_terminal_id
            .clone();
        state
            .terminals
            .get_mut(&terminal_id)
            .expect("agent terminal")
            .set_agent_name("grok".into());
        let name = crate::ui::agent_panel_entries(&state)
            .into_iter()
            .find(|entry| entry.pane_id == pane_id)
            .expect("table row")
            .name;
        let menu = ContextMenuState {
            kind: ContextMenuKind::Agent {
                ws_idx: 0,
                pane_id,
                space: SpaceMenuKind::LinkedWorktree {
                    parent_branch: Some("main".into()),
                    already_landed: false,
                    in_worktree_directory: true,
                },
            },
            x: 0,
            y: 0,
            list: MenuListState::new(0),
        };
        let land_idx = menu
            .items()
            .iter()
            .position(|item| is_land_menu_item(item))
            .expect("land item");
        let mut terminal_runtimes = crate::terminal::TerminalRuntimeRegistry::new();

        apply_context_menu_action(&mut state, &mut terminal_runtimes, menu, land_idx);

        assert_eq!(state.request_land_worktree, None);
        assert_eq!(
            state.request_land_agent_prompt,
            Some((name, land_prompt_text()))
        );
        assert_eq!(state.mode, Mode::Terminal);
    }

    #[test]
    fn delete_agent_from_the_row_menu_closes_the_pane() {
        let mut state = state_with_workspaces(&["test"]);
        state.mode = Mode::ContextMenu;
        let pane_id = state.workspaces[0].tabs[0].root_pane;
        let sibling = state.workspaces[0].test_split(ratatui::layout::Direction::Horizontal);
        state.ensure_test_terminals();
        let terminal_id = state.terminal_id_for_pane(0, pane_id).unwrap();
        state
            .terminals
            .get_mut(&terminal_id)
            .expect("agent terminal")
            .set_detected_state(
                Some(crate::detect::Agent::Pi),
                crate::detect::AgentState::Idle,
            );
        let menu = ContextMenuState {
            kind: ContextMenuKind::Agent {
                ws_idx: 0,
                pane_id,
                space: SpaceMenuKind::Plain,
            },
            x: 0,
            y: 0,
            list: MenuListState::new(0),
        };
        let delete_idx = menu
            .items()
            .iter()
            .position(|item| item == "Delete agent")
            .expect("delete agent item");
        let mut terminal_runtimes = crate::terminal::TerminalRuntimeRegistry::new();

        apply_context_menu_action(&mut state, &mut terminal_runtimes, menu, delete_idx);

        assert!(state.workspaces[0].pane_state(pane_id).is_none());
        assert!(state.workspaces[0].pane_state(sibling).is_some());
        assert!(state.detached_agents.is_empty());
        assert!(!state.terminals.contains_key(&terminal_id));
        assert!(state.terminal_runtime_shutdowns.contains(&terminal_id));
        assert_eq!(state.mode, Mode::Terminal);
    }

    #[test]
    fn worktree_delete_outside_a_worktree_directory_does_not_start_removal() {
        let mut state = state_with_workspaces(&["main"]);
        state.mode = Mode::ContextMenu;
        let menu = ContextMenuState {
            kind: ContextMenuKind::Agent {
                ws_idx: 0,
                pane_id: state.workspaces[0].tabs[0].root_pane,
                space: SpaceMenuKind::LinkedWorktree {
                    parent_branch: Some("main".into()),
                    already_landed: false,
                    in_worktree_directory: false,
                },
            },
            x: 0,
            y: 0,
            list: MenuListState::new(0),
        };
        let delete_idx = menu
            .items()
            .iter()
            .position(|item| item == "Delete agent / worktree...")
            .expect("delete worktree item");
        let mut terminal_runtimes = crate::terminal::TerminalRuntimeRegistry::new();

        apply_context_menu_action(&mut state, &mut terminal_runtimes, menu, delete_idx);

        assert_eq!(state.request_remove_linked_worktree, None);
        assert!(state.context_menu.is_some());
        assert_eq!(state.mode, Mode::ContextMenu);
    }
}
