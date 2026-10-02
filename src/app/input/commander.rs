//! Key handling for the Commander, and carrying out what it was told.
//!
//! The box holds one field. `Enter` carries out the line, `Shift-Enter` or
//! `Alt-Enter` starts a new line in it, `Esc` or the hotkey again closes it,
//! and every other editing key is the composer's. Each edit re-reads the line,
//! so the reading drawn under the field is always the one `Enter` would act on.

use crossterm::event::{KeyCode, KeyEventKind, KeyModifiers};

use super::navigate::{execute_navigate_action_in_context, ActionContext, NavigateAction};
use crate::app::state::{AppState, Mode};
use crate::commander::intent::{self, Action, Command, Entry, Kind};
use crate::commander::{trail::V2, Delivery, Launch};
use crate::input::TerminalKey;
use crate::terminal::TerminalRuntimeRegistry;

/// What a key did to the Commander.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CommanderKeyOutcome {
    Edited,
    Closed,
    Submit,
}

pub(crate) fn open_commander(state: &mut AppState) {
    state.commander.field.clear();
    state.commander.reading = None;
    state.mode = Mode::Commander;
}

pub(crate) fn close_commander(state: &mut AppState) {
    state.mode = if state.active.is_some() {
        Mode::Terminal
    } else {
        Mode::Navigate
    };
}

pub(crate) fn handle_commander_key(
    state: &mut AppState,
    terminal_runtimes: &TerminalRuntimeRegistry,
    raw_key: TerminalKey,
) -> CommanderKeyOutcome {
    let key = raw_key.as_key_event();
    if key.kind == KeyEventKind::Release {
        return CommanderKeyOutcome::Edited;
    }
    if key.code == KeyCode::Esc || state.keybinds.commander.matches_direct_key(raw_key) {
        close_commander(state);
        return CommanderKeyOutcome::Closed;
    }
    let newline =
        key.modifiers.contains(KeyModifiers::SHIFT) || key.modifiers.contains(KeyModifiers::ALT);
    match key.code {
        KeyCode::Enter if newline => state.commander.field.newline(),
        KeyCode::Enter => return CommanderKeyOutcome::Submit,
        code => {
            let plain = key.modifiers.difference(KeyModifiers::SHIFT).is_empty();
            if !super::composer::edit_field(&mut state.commander.field, code, key.modifiers, plain)
            {
                return CommanderKeyOutcome::Edited;
            }
        }
    }
    refresh_reading(state, terminal_runtimes);
    CommanderKeyOutcome::Edited
}

/// Read the field again and keep the sentence saying what it would do.
pub(crate) fn refresh_reading(state: &mut AppState, terminal_runtimes: &TerminalRuntimeRegistry) {
    let text = state.commander.field.text();
    state.commander.reading = if text.trim().is_empty() {
        None
    } else {
        Some(intent::interpret(&text, &catalog(state, terminal_runtimes)).map(|c| c.describe()))
    };
}

/// Carry out the line in the field. A message for a pane is not delivered
/// here: the pane is brought on screen and a star is readied to fly there, and
/// the message goes when it lands. A line that cannot be read leaves the box
/// open with the reason under it.
pub(crate) fn submit_commander(
    state: &mut AppState,
    terminal_runtimes: &mut TerminalRuntimeRegistry,
) {
    let text = state.commander.field.text();
    let command = match intent::interpret(&text, &catalog(state, terminal_runtimes)) {
        Ok(command) => command,
        Err(reason) => {
            state.commander.reading = (!reason.is_empty()).then_some(Err(reason));
            return;
        }
    };
    let origin = state.commander.area;
    close_commander(state);
    state.commander.field.clear();
    state.commander.reading = None;

    match command {
        Command::Go(entry) => {
            go(state, &entry);
        }
        Command::Act(action) => {
            execute_navigate_action_in_context(
                state,
                terminal_runtimes,
                navigate_action(action),
                ActionContext::Direct,
            );
        }
        Command::Send { to, text } => {
            let Some(pane_id) = to.pane else {
                return;
            };
            let Some(ws) = state.workspaces.get(to.ws) else {
                return;
            };
            let workspace_id = ws.id.clone();
            let tab = ws.find_tab_index_for_pane(pane_id).unwrap_or(to.tab);
            // The star has to land on something the user can see.
            let on_screen = state.active == Some(to.ws) && ws.active_tab_index() == tab;
            if !on_screen {
                state.switch_workspace_tab(to.ws, tab);
            }
            // The star leaves from the top edge of the box, or from the
            // bottom of the panes if the box was never laid out.
            let origin = if origin.width == 0 {
                let area = state.view.terminal_area;
                ratatui::layout::Rect::new(area.x, area.bottom().saturating_sub(1), area.width, 1)
            } else {
                origin
            };
            state.commander.launch = Some(Launch {
                from: V2::from_cell(origin.x + origin.width / 2, origin.y),
                delivery: Delivery {
                    workspace_id,
                    pane_id,
                    text,
                    label: to.label,
                },
            });
        }
    }
}

fn go(state: &mut AppState, entry: &Entry) {
    match entry.kind {
        Kind::Space => state.switch_workspace(entry.ws),
        Kind::Tab => {
            state.switch_workspace_tab(entry.ws, entry.tab);
        }
        Kind::Pane => {
            if let Some(pane_id) = entry.pane {
                state.focus_pane_in_workspace(entry.ws, pane_id);
            }
        }
    }
}

fn navigate_action(action: Action) -> NavigateAction {
    match action {
        Action::NewTab => NavigateAction::NewTab,
        Action::NewSpace => NavigateAction::NewWorkspace,
        Action::SplitRight => NavigateAction::SplitRight,
        Action::SplitLeft => NavigateAction::SplitLeft,
        Action::SplitDown => NavigateAction::SplitDown,
        Action::SplitUp => NavigateAction::SplitUp,
        Action::ClosePane => NavigateAction::ClosePane,
        Action::CloseTab => NavigateAction::CloseTab,
        Action::CloseSpace => NavigateAction::CloseWorkspace,
        Action::Zoom => NavigateAction::Zoom,
        Action::NextTab => NavigateAction::NextTab,
        Action::PreviousTab => NavigateAction::PreviousTab,
        Action::NextSpace => NavigateAction::NextWorkspace,
        Action::PreviousSpace => NavigateAction::PreviousWorkspace,
        Action::NextAgent => NavigateAction::NextAgent,
        Action::PreviousAgent => NavigateAction::PreviousAgent,
        Action::LastPane => NavigateAction::LastPane,
        Action::FocusLeft => NavigateAction::FocusPaneLeft,
        Action::FocusRight => NavigateAction::FocusPaneRight,
        Action::FocusUp => NavigateAction::FocusPaneUp,
        Action::FocusDown => NavigateAction::FocusPaneDown,
        Action::ToggleSidebar => NavigateAction::ToggleSidebar,
        Action::ToggleAgentTable => NavigateAction::ToggleAgentTable,
        Action::RenameTab => NavigateAction::RenameTab,
        Action::RenameSpace => NavigateAction::RenameWorkspace,
        Action::RenamePane => NavigateAction::RenamePane,
        Action::Settings => NavigateAction::Settings,
        Action::Help => NavigateAction::Help,
        Action::Navigator => NavigateAction::OpenNavigator,
        Action::Composer => NavigateAction::OpenComposer,
        Action::CopyMode => NavigateAction::CopyMode,
        Action::Resize => NavigateAction::EnterResizeMode,
        Action::ReloadConfig => NavigateAction::ReloadConfig,
    }
}

/// Everything a name can point at right now, with every name each answers
/// to: what the sidebar, tab bar and pane headers call it.
pub(crate) fn catalog(state: &AppState, terminal_runtimes: &TerminalRuntimeRegistry) -> Vec<Entry> {
    let assigned = crate::pane_names::assigned_names(&state.terminals);
    let mut entries = Vec::new();
    for (ws_idx, ws) in state.workspaces.iter().enumerate() {
        let space_name = ws.display_name_from(&state.terminals, terminal_runtimes);
        let space_visible = state.active == Some(ws_idx);
        let active_tab = ws.active_tab_index();
        let mut names = vec![intent::normalize(&space_name), (ws_idx + 1).to_string()];
        if let Some(custom) = &ws.custom_name {
            names.push(intent::normalize(custom));
        }
        entries.push(Entry {
            kind: Kind::Space,
            ws: ws_idx,
            tab: active_tab,
            pane: ws.focused_pane_id(),
            label: space_name.clone(),
            context: None,
            names,
            locality: u8::from(space_visible),
        });

        for (tab_idx, tab) in ws.tabs.iter().enumerate() {
            let tab_visible = space_visible && tab_idx == active_tab;
            let tab_name = tab.display_name();
            entries.push(Entry {
                kind: Kind::Tab,
                ws: ws_idx,
                tab: tab_idx,
                pane: Some(tab.layout.focused()),
                label: tab_name.clone(),
                context: Some(space_name.clone()),
                names: vec![intent::normalize(&tab_name), format!("tab {}", tab_idx + 1)],
                locality: u8::from(space_visible) + u8::from(tab_visible),
            });

            for pane_id in tab.layout.pane_ids() {
                let Some(pane) = tab.panes.get(&pane_id) else {
                    continue;
                };
                let terminal = state.terminals.get(&pane.attached_terminal_id);
                let assigned_name = assigned.get(&pane.attached_terminal_id).cloned();
                let number = ws.public_pane_number(pane_id).unwrap_or(0);
                let mut names = vec![format!("pane {number}")];
                let mut label = None;
                if let Some(terminal) = terminal {
                    for name in [
                        terminal.manual_label.clone(),
                        assigned_name.clone(),
                        terminal.agent_name.clone(),
                        terminal.effective_agent_label().map(str::to_string),
                        terminal.effective_title(),
                    ]
                    .into_iter()
                    .flatten()
                    {
                        label.get_or_insert_with(|| name.clone());
                        names.push(intent::normalize(&name));
                    }
                }
                let label = label.unwrap_or_else(|| format!("pane {number}"));
                let agent = terminal.and_then(|t| t.effective_agent_label().map(str::to_string));
                let label = match agent {
                    Some(agent) if !label.eq_ignore_ascii_case(&agent) => {
                        format!("{label} ({agent})")
                    }
                    _ => label,
                };
                entries.push(Entry {
                    kind: Kind::Pane,
                    ws: ws_idx,
                    tab: tab_idx,
                    pane: Some(pane_id),
                    label,
                    context: Some(space_name.clone()),
                    names,
                    locality: u8::from(space_visible) + u8::from(tab_visible),
                });
            }
        }
    }
    entries
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> TerminalKey {
        TerminalKey::new(code, KeyModifiers::empty())
    }

    fn type_line(state: &mut AppState, line: &str) {
        let runtimes = TerminalRuntimeRegistry::new();
        for c in line.chars() {
            handle_commander_key(state, &runtimes, key(KeyCode::Char(c)));
        }
    }

    #[test]
    fn typing_reads_the_line_as_it_goes() {
        let mut state = AppState::test_new();
        open_commander(&mut state);
        type_line(&mut state, "new tab");
        assert_eq!(
            state.commander.reading,
            Some(Ok("open a new tab".to_string()))
        );
    }

    #[test]
    fn escape_closes_and_keeps_nothing_running() {
        let mut state = AppState::test_new();
        open_commander(&mut state);
        let runtimes = TerminalRuntimeRegistry::new();
        let outcome = handle_commander_key(&mut state, &runtimes, key(KeyCode::Esc));
        assert_eq!(outcome, CommanderKeyOutcome::Closed);
        assert_ne!(state.mode, Mode::Commander);
    }

    #[test]
    fn an_unreadable_line_stays_open_with_the_reason() {
        let mut state = AppState::test_new();
        open_commander(&mut state);
        type_line(&mut state, "frobnicate the widgets");
        let mut runtimes = TerminalRuntimeRegistry::new();
        submit_commander(&mut state, &mut runtimes);
        assert_eq!(state.mode, Mode::Commander);
        assert!(matches!(state.commander.reading, Some(Err(_))));
    }

    #[test]
    fn shift_enter_starts_a_new_line() {
        let mut state = AppState::test_new();
        open_commander(&mut state);
        let runtimes = TerminalRuntimeRegistry::new();
        type_line(&mut state, "a");
        let outcome = handle_commander_key(
            &mut state,
            &runtimes,
            TerminalKey::new(KeyCode::Enter, KeyModifiers::SHIFT),
        );
        assert_eq!(outcome, CommanderKeyOutcome::Edited);
        assert_eq!(state.commander.field.text(), "a\n");
    }
}
