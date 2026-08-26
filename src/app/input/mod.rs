//! Input handling — translates crossterm key/mouse events into state mutations.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use tracing::warn;

use crate::app::PaneClickState;
use crate::input::TerminalKey;
use ratatui::layout::Direction;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScrollbarClickTarget {
    Thumb { grab_row_offset: u16 },
    Track { offset_from_bottom: usize },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg(test)]
enum WheelRouting {
    HostScroll,
    MouseReport,
    AlternateScroll,
}

const PANE_DRAG_THRESHOLD: u16 = 1;

mod agent_table;
mod composer;
mod copy_mode;
mod modal;
mod mouse;
mod navigate;
mod overlays;
mod selection;
mod settings;
mod sidebar;
mod terminal;

pub(crate) use self::agent_table::{
    agent_table_delete_intercept, confirm_close_agent_accept, confirm_close_agent_cancel,
    handle_confirm_close_agent_key,
};
pub(crate) use self::composer::{
    enter_composer_mode, handle_composer_key, handle_player_link_key, leave_composer_mode,
    ComposerKeyOutcome,
};
pub(crate) use self::{
    modal::{
        handle_confirm_close_key, handle_context_menu_key, handle_global_menu_key,
        handle_keybind_help_key, handle_navigator_key, handle_rename_key, handle_resize_key,
    },
    navigate::terminal_direct_navigation_action,
    settings::open_settings_at,
};
use self::{
    modal::{
        modal_action_from_key, ModalAction, ONBOARDING_WELCOME_ACTIONS, RELEASE_NOTES_ACTIONS,
    },
    settings::SettingsAction,
};
use super::state::{AppState, Mode};
use super::App;

// ---------------------------------------------------------------------------
// Key handling
// ---------------------------------------------------------------------------

impl App {
    pub(super) async fn handle_key(&mut self, key: TerminalKey) {
        if agent_table_delete_intercept(&mut self.state, key) {
            return;
        }
        match self.state.mode {
            Mode::Terminal => {
                if handle_player_link_key(&mut self.state, key) {
                    return;
                }
                if key.as_key_event().code == crossterm::event::KeyCode::Esc
                    && crate::ui::player::handle_player_queue_esc(&mut self.state)
                {
                    return;
                }
                self.handle_terminal_key(key).await
            }
            Mode::PlayerInput => {
                if handle_player_link_key(&mut self.state, key) {
                    return;
                }
                if self.state.mode == Mode::Terminal {
                    self.handle_terminal_key(key).await
                }
            }
            Mode::Prefix => self.handle_prefix_key(key),
            Mode::Navigate => self.handle_navigate_key(key),
            Mode::Copy => self.handle_copy_mode_key(key),
            Mode::Composer => {
                match handle_composer_key(&mut self.state, &self.terminal_runtimes, key) {
                    ComposerKeyOutcome::Submit(pending) => self.submit_composer(*pending),
                    ComposerKeyOutcome::Trouble(reason) => self.show_composer_trouble(reason),
                    ComposerKeyOutcome::Edited => {}
                }
            }
            _ => {
                let key_event = key.as_key_event();
                match self.state.mode {
                    Mode::Onboarding => self.handle_onboarding_key(key_event),
                    Mode::ReleaseNotes => self.handle_release_notes_key(key_event),
                    Mode::ProductAnnouncement => self.handle_product_announcement_key(key_event),
                    Mode::Prefix
                    | Mode::Navigate
                    | Mode::Copy
                    | Mode::Composer
                    | Mode::PlayerInput => unreachable!(),
                    Mode::RenameWorkspace | Mode::RenamePane => {
                        handle_rename_key(&mut self.state, key_event)
                    }
                    Mode::NewLinkedWorktree => self.handle_worktree_create_key(key_event),
                    Mode::OpenExistingWorktree => self.handle_worktree_open_key(key_event),
                    Mode::ConfirmRemoveWorktree => self.handle_worktree_remove_key(key_event),
                    Mode::WorktreeLand => self.handle_worktree_land_key(key_event),
                    Mode::Resize => handle_resize_key(&mut self.state, key),
                    Mode::ConfirmClose => handle_confirm_close_key(&mut self.state, key_event),
                    Mode::ConfirmCloseAgent => handle_confirm_close_agent_key(
                        &mut self.state,
                        &self.terminal_runtimes,
                        key_event,
                    ),
                    Mode::ContextMenu => {
                        handle_context_menu_key(
                            &mut self.state,
                            &mut self.terminal_runtimes,
                            key_event,
                        );
                    }
                    Mode::Settings => self.handle_settings_key(key_event),
                    Mode::GlobalMenu => handle_global_menu_key(&mut self.state, key_event),
                    Mode::KeybindHelp => handle_keybind_help_key(&mut self.state, key_event),
                    Mode::Navigator => {
                        handle_navigator_key(&mut self.state, &self.terminal_runtimes, key_event)
                    }
                    Mode::Terminal => unreachable!(),
                }
            }
        }
    }

    pub(super) async fn handle_paste(&mut self, text: String) {
        if self.state.mode == Mode::PlayerInput || self.state.player_input_focused {
            let first = text.lines().next().unwrap_or_default();
            self.state.player_link.insert_str(first);
            return;
        }

        if self.state.mode == Mode::Composer {
            match self.state.composer.focus {
                // A pasted task keeps its lines: the field holds as many as it
                // takes, and flattening them would join two thoughts into one.
                crate::composer::Focus::Task => self.state.composer.task.insert_str(&text),
                // A pasted path is the reason the path field exists, and typing
                // into the folder control is what opens it — so a paste into it
                // opens it too. A path is one line, so a paste of several
                // contributes its first.
                crate::composer::Focus::Folder => {
                    if self.state.composer.open != Some(crate::composer::Focus::Folder) {
                        self.state.refresh_composer_folders(&self.terminal_runtimes);
                        self.state
                            .composer
                            .open_dropdown(crate::composer::Focus::Folder);
                    }
                    let first = text.lines().next().unwrap_or_default().to_string();
                    self.state
                        .composer
                        .edit_path(|path| path.insert_str(&first));
                }
                crate::composer::Focus::Agent => {}
            }
            return;
        }

        if self.state.mode != Mode::Terminal {
            return;
        }

        if text.is_empty() && self.try_bridge_clipboard_image_paste().await {
            return;
        }

        if let Some((_, _, rt)) = self.state.terminal_input_target(&self.terminal_runtimes) {
            let _ = rt.send_paste(text).await;
        }
    }

    pub(super) async fn try_bridge_clipboard_image_from_key(&mut self, key: TerminalKey) -> bool {
        if self.state.mode != Mode::Terminal {
            return false;
        }

        if key.kind != crossterm::event::KeyEventKind::Press
            || !key
                .modifiers
                .intersects(crossterm::event::KeyModifiers::CONTROL)
            || !matches!(key.code, crossterm::event::KeyCode::Char('v' | 'V'))
        {
            return false;
        }

        self.try_bridge_clipboard_image_paste().await
    }

    async fn try_bridge_clipboard_image_paste(&mut self) -> bool {
        if self
            .state
            .terminal_input_target(&self.terminal_runtimes)
            .is_none()
        {
            return false;
        }

        let Some(image) = crate::platform::read_clipboard_image() else {
            self.show_copy_feedback_message("clipboard image paste: no image on clipboard");
            return false;
        };

        if image.bytes.len() > crate::protocol::MAX_CLIPBOARD_IMAGE_PAYLOAD {
            self.show_copy_feedback_message("clipboard image paste: image is too large");
            return true;
        }

        let paste_text =
            match crate::server::clipboard_image::stage(0, image.extension, &image.bytes) {
                Ok(staged) => {
                    self.show_copy_feedback_message("clipboard image paste: sending image");
                    staged.paste_text
                }
                Err(err) => {
                    warn!(err = %err, "failed to stage local clipboard image for paste");
                    self.show_copy_feedback_message("clipboard image paste: failed to stage image");
                    return true;
                }
            };

        if let Some((_, _, rt)) = self.state.terminal_input_target(&self.terminal_runtimes) {
            let _ = rt.send_paste(paste_text).await;
        }
        true
    }

    fn show_copy_feedback_message(&mut self, message: &str) {
        self.state.copy_feedback = Some(crate::app::state::CopyFeedback {
            message: message.to_string(),
        });
        self.copy_feedback_deadline =
            Some(std::time::Instant::now() + super::COPY_FEEDBACK_DURATION);
    }

    pub(crate) fn handle_onboarding_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Right | KeyCode::Char('l') => self.open_settings_from_onboarding(),
            _ => {
                if let Some(ModalAction::Continue) =
                    modal_action_from_key(&key, ONBOARDING_WELCOME_ACTIONS)
                {
                    self.open_settings_from_onboarding();
                }
            }
        }
    }

    pub(crate) fn handle_release_notes_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => self.scroll_release_notes(-1),
            KeyCode::Down | KeyCode::Char('j') => self.scroll_release_notes(1),
            KeyCode::PageUp => self.scroll_release_notes(-8),
            KeyCode::PageDown => self.scroll_release_notes(8),
            KeyCode::Home => {
                if let Some(notes) = &mut self.state.release_notes {
                    notes.scroll = 0;
                }
            }
            KeyCode::End => {
                let max_scroll = self.state.release_notes_max_scroll();
                if let Some(notes) = &mut self.state.release_notes {
                    notes.scroll = max_scroll;
                }
            }
            _ => {
                if let Some(ModalAction::Close) = modal_action_from_key(&key, RELEASE_NOTES_ACTIONS)
                {
                    self.dismiss_release_notes();
                }
            }
        }
    }

    pub(crate) fn handle_product_announcement_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => self.scroll_product_announcement(-1),
            KeyCode::Down | KeyCode::Char('j') => self.scroll_product_announcement(1),
            KeyCode::PageUp => self.scroll_product_announcement(-8),
            KeyCode::PageDown => self.scroll_product_announcement(8),
            KeyCode::Home => {
                if let Some(announcement) = &mut self.state.product_announcement {
                    announcement.scroll = 0;
                }
            }
            KeyCode::End => {
                let max_scroll = self.state.product_announcement_max_scroll();
                if let Some(announcement) = &mut self.state.product_announcement {
                    announcement.scroll = max_scroll;
                }
            }
            _ => {
                if let Some(ModalAction::Close) = modal_action_from_key(&key, RELEASE_NOTES_ACTIONS)
                {
                    self.dismiss_product_announcement();
                }
            }
        }
    }

    pub(super) fn handle_mouse(&mut self, mouse: MouseEvent) {
        if self.dismiss_config_diagnostic_at(mouse) {
            return;
        }

        if matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left)) {
            if let Some(reason) = self
                .state
                .commit_typed_folder_on_away_click(mouse.column, mouse.row)
            {
                self.show_composer_trouble(reason);
                return;
            }
        }

        if self.handle_overlay_mouse(mouse) {
            return;
        }

        if self.handle_agent_name_double_click(mouse) {
            return;
        }

        if self.handle_modified_url_click(mouse) {
            return;
        }

        let handled_pane_double_click = self.handle_pane_double_click(mouse);

        let previous_settings_section = self.state.settings.section;
        if !handled_pane_double_click {
            if let Some(action) = self.state.handle_mouse(&mut self.terminal_runtimes, mouse) {
                match action {
                    SettingsAction::SaveTheme(name) => self.save_theme(&name),
                    SettingsAction::SaveSound(enabled) => self.save_sound(enabled),
                    SettingsAction::SaveDoneSound(choice) => self.save_done_sound(&choice),
                    SettingsAction::SaveToastDelivery(delivery) => {
                        self.save_toast_delivery(delivery)
                    }
                    SettingsAction::SaveAgentBorderLabels(enabled) => {
                        self.save_agent_border_labels(enabled)
                    }
                    SettingsAction::SavePaneHistory(enabled) => {
                        self.save_pane_history_persistence(enabled)
                    }
                    SettingsAction::SaveSwitchAsciiInputSourceInPrefix(enabled) => {
                        self.save_switch_ascii_input_source_in_prefix(enabled)
                    }
                    SettingsAction::InstallRecommendedIntegrations => {
                        self.install_recommended_integrations()
                    }
                }
            }
        }
        if previous_settings_section != crate::app::state::SettingsSection::Integrations
            && self.state.settings.section == crate::app::state::SettingsSection::Integrations
        {
            self.refresh_integration_recommendations();
        }
        if let Some(content) = self.state.request_clipboard_write.take() {
            if self
                .event_tx
                .try_send(crate::events::AppEvent::ClipboardWrite { content })
                .is_err()
            {
                tracing::warn!("failed to queue clipboard write event");
            }
        }

        // Sync autoscroll deadline with state (mouse handler may have
        // set or cleared selection_autoscroll during handle_mouse).
        if self.state.selection_autoscroll.is_none() {
            self.selection_autoscroll_deadline = None;
        } else if self.selection_autoscroll_deadline.is_none() {
            self.selection_autoscroll_deadline =
                Some(std::time::Instant::now() + super::SELECTION_AUTOSCROLL_INTERVAL);
        }
    }

    fn dismiss_config_diagnostic_at(&mut self, mouse: MouseEvent) -> bool {
        if !matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left)) {
            return false;
        }
        let Some(message) = self.state.config_diagnostic.as_deref() else {
            return false;
        };
        let Some(rect) =
            crate::ui::config_diagnostic_dismiss_rect(self.state.view.terminal_area, message)
        else {
            return false;
        };
        if mouse.column < rect.x
            || mouse.column >= rect.x + rect.width
            || mouse.row < rect.y
            || mouse.row >= rect.y + rect.height
        {
            return false;
        }
        self.state.config_diagnostic = None;
        self.config_diagnostic_deadline = None;
        true
    }

    fn handle_agent_name_double_click(&mut self, mouse: MouseEvent) -> bool {
        if matches!(mouse.kind, MouseEventKind::Drag(MouseButton::Left)) {
            self.last_agent_name_click = None;
            return false;
        }

        if !matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left)) {
            return false;
        }

        if !mouse.modifiers.is_empty() || self.state.mode != Mode::Terminal {
            self.last_agent_name_click = None;
            return false;
        }

        let Some(hit) = self.state.agent_name_target_at(mouse.column, mouse.row) else {
            self.last_agent_name_click = None;
            return false;
        };
        let click = super::AgentNameClickState {
            pane_id: hit.pane_id,
            row: mouse.row,
            col: mouse.column,
            at: std::time::Instant::now(),
        };
        if !self
            .last_agent_name_click
            .is_some_and(|last| last.is_double_click_for(click))
        {
            self.last_agent_name_click = Some(click);
            return false;
        }

        self.last_agent_name_click = None;
        self.state.agent_press = None;
        self.state.drag = None;
        modal::open_rename_pane(&mut self.state, hit.pane_id);
        self.state.mode == Mode::RenamePane
    }

    fn handle_modified_url_click(&mut self, mouse: MouseEvent) -> bool {
        if self.state.mode != Mode::Terminal
            || !matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left))
            || !mouse.modifiers.contains(KeyModifiers::CONTROL)
        {
            return false;
        }

        let Some(info) = self.state.pane_at(mouse.column, mouse.row).cloned() else {
            return false;
        };
        let viewport_row = mouse.row.saturating_sub(info.inner_rect.y);
        let col = mouse.column.saturating_sub(info.inner_rect.x);
        let Some(url) =
            self.state
                .url_at_pane_cell(&self.terminal_runtimes, info.id, viewport_row, col)
        else {
            return false;
        };

        self.last_pane_click = None;
        if let Err(err) = crate::platform::open_url(&url) {
            tracing::warn!(err = %err, url = %url, "failed to open pane URL");
        }
        true
    }

    fn handle_pane_double_click(&mut self, mouse: MouseEvent) -> bool {
        // A pane press stops being a double-click candidate once it becomes
        // a drag or completes as a real text selection.
        match mouse.kind {
            MouseEventKind::Drag(MouseButton::Left) => {
                self.last_pane_click = None;
                return false;
            }
            MouseEventKind::Up(MouseButton::Left)
                if self
                    .state
                    .selection
                    .as_ref()
                    .is_some_and(|selection| selection.is_visible()) =>
            {
                self.last_pane_click = None;
                return false;
            }
            _ => {}
        }

        // Only terminal-pane left-clicks can start this gesture; other clicks
        // should keep their existing mouse behavior and clear stale candidates.
        let Some(click) = self.pane_click_candidate(mouse) else {
            return false;
        };

        // Require the second click to land near the first click in the same pane
        // and within the double-click window so adjacent interactions do not copy.
        if !self.take_pane_double_click(click) {
            return false;
        }

        // Preserve a short highlight after copying so the user gets visible
        // confirmation without leaving a persistent selection behind.
        self.copy_double_clicked_word(click)
    }

    fn pane_click_candidate(&mut self, mouse: MouseEvent) -> Option<PaneClickState> {
        if !matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left)) {
            return None;
        }

        if !mouse.modifiers.is_empty() {
            self.last_pane_click = None;
            return None;
        }

        if self.state.mode != Mode::Terminal {
            self.last_pane_click = None;
            return None;
        }

        let Some(info) = self.state.pane_at(mouse.column, mouse.row).cloned() else {
            self.last_pane_click = None;
            return None;
        };

        Some(PaneClickState {
            pane_id: info.id,
            viewport_row: mouse.row - info.inner_rect.y,
            col: mouse.column - info.inner_rect.x,
            at: std::time::Instant::now(),
        })
    }

    fn take_pane_double_click(&mut self, click: PaneClickState) -> bool {
        if !self
            .last_pane_click
            .is_some_and(|last| last.is_double_click_for(click))
        {
            self.last_pane_click = Some(click);
            return false;
        }

        self.last_pane_click = None;
        true
    }

    fn copy_double_clicked_word(&mut self, click: PaneClickState) -> bool {
        let copied = self.state.copy_word_at_pane_cell(
            &self.terminal_runtimes,
            click.pane_id,
            click.viewport_row,
            click.col,
        );
        if copied {
            self.selection_highlight_clear_deadline =
                Some(std::time::Instant::now() + super::PANE_COPY_HIGHLIGHT_DURATION);
        }
        copied
    }
}

// ---------------------------------------------------------------------------
// Mouse handling
// ---------------------------------------------------------------------------

// Note: split_pane needs runtime (event_tx for PTY spawn), so it lives on App
impl AppState {
    pub(crate) fn split_pane(
        &mut self,
        terminal_runtimes: &mut crate::terminal::TerminalRuntimeRegistry,
        direction: Direction,
    ) {
        self.split_pane_with_placement(
            terminal_runtimes,
            direction,
            crate::layout::SplitPlacement::After,
        );
    }

    pub(crate) fn split_pane_with_placement(
        &mut self,
        terminal_runtimes: &mut crate::terminal::TerminalRuntimeRegistry,
        direction: Direction,
        placement: crate::layout::SplitPlacement,
    ) {
        // Actual PTY spawning happens in Workspace::split_focused
        // which needs events channel — this is called from navigate_key
        // where we don't have async context, so the workspace handles it
        let (rows, cols) = self.estimate_pane_size();
        let new_rows = (rows / 2).max(4);
        let new_cols = (cols / 2).max(10);

        let focused_runtime = self
            .active
            .and_then(|i| self.workspaces.get(i))
            .and_then(|ws| {
                let tab = ws.active_tab()?;
                let terminal_id = tab.terminal_id(tab.layout.focused())?;
                terminal_runtimes.get(terminal_id)
            });
        let follow_cwd = self
            .active
            .and_then(|i| self.workspaces.get(i))
            .and_then(|ws| {
                let tab = ws.active_tab()?;
                tab.cwd_for_pane(tab.layout.focused(), &self.terminals, terminal_runtimes)
            });
        // Splitting a pane that is sitting in a Windows shell should land in
        // that same shell, not drop back to the Linux one.
        let inherited_shell =
            focused_runtime.and_then(|runtime| runtime.foreground_interop_shell());
        let cwd = Some(super::creation::resolve_new_terminal_cwd(
            &self.new_terminal_cwd,
            follow_cwd,
        ));

        let previous_focus = self.current_pane_focus_target();
        if let Some(ws_idx) = self.active {
            let Some(ws) = self.workspaces.get_mut(ws_idx) else {
                return;
            };
            if let Ok(new_pane) = ws.split_focused_with_placement(
                direction,
                placement,
                new_rows,
                new_cols,
                cwd,
                self.pane_scrollback_limit_bytes,
                self.host_terminal_theme,
                crate::pane::PaneShellConfig::new(&self.default_shell, self.shell_mode)
                    .with_program_override(inherited_shell.as_deref()),
            ) {
                let new_id = new_pane.pane_id;
                terminal_runtimes.insert(new_pane.terminal.id.clone(), new_pane.runtime);
                self.remove_alias_shadowed_by_new_pane(new_id);
                self.terminals
                    .insert(new_pane.terminal.id.clone(), new_pane.terminal);
                self.record_pane_focus_change(previous_focus, ws_idx, new_id);
                self.mark_session_dirty();
                self.mode = Mode::Terminal;
            }
        }
    }

    /// Split a specific leaf, not whichever pane currently has keyboard focus.
    pub(crate) fn split_given_pane_with_placement(
        &mut self,
        terminal_runtimes: &mut crate::terminal::TerminalRuntimeRegistry,
        pane_id: crate::layout::PaneId,
        direction: Direction,
        placement: crate::layout::SplitPlacement,
    ) {
        if let Some(ws_idx) = self.active {
            if let Some(tab) = self
                .workspaces
                .get_mut(ws_idx)
                .and_then(|ws| ws.active_tab_mut())
            {
                tab.layout.focus_pane(pane_id);
            }
        }
        self.split_pane_with_placement(terminal_runtimes, direction, placement);
    }
}

#[cfg(test)]
fn state_with_workspaces(names: &[&str]) -> AppState {
    let mut state = AppState::test_new();
    state.workspaces = names
        .iter()
        .map(|name| crate::workspace::Workspace::test_new(name))
        .collect();
    if !state.workspaces.is_empty() {
        state.active = Some(0);
        state.selected = 0;
        state.mode = Mode::Navigate;
    }
    state
}

#[cfg(test)]
fn app_for_mouse_test() -> App {
    let (_api_tx, api_rx) = tokio::sync::mpsc::unbounded_channel();
    let mut app = App::new(
        &crate::config::Config::default(),
        true,
        None,
        api_rx,
        crate::api::EventHub::default(),
    );
    app.state.mode = Mode::Terminal;
    app.state.update_available = None;
    app.state.latest_release_notes_available = false;
    app.state.view.terminal_area = ratatui::layout::Rect::new(0, 1, 106, 19);
    app
}

#[cfg(test)]
fn mouse(
    kind: crossterm::event::MouseEventKind,
    col: u16,
    row: u16,
) -> crossterm::event::MouseEvent {
    crossterm::event::MouseEvent {
        kind,
        column: col,
        row,
        modifiers: crossterm::event::KeyModifiers::empty(),
    }
}

#[cfg(test)]
fn numbered_lines_bytes(count: usize) -> Vec<u8> {
    (0..count)
        .map(|i| format!("{i:06}\r\n"))
        .collect::<String>()
        .into_bytes()
}

#[cfg(test)]
fn capture_snapshot(state: &AppState) -> crate::persist::SessionSnapshot {
    let terminal_runtimes = crate::terminal::TerminalRuntimeRegistry::new();
    crate::persist::capture(state, &terminal_runtimes)
}

#[cfg(test)]
fn root_layout_ratio(snapshot: &crate::persist::SessionSnapshot) -> Option<f32> {
    match &snapshot.workspaces.first()?.tabs.first()?.layout {
        crate::persist::LayoutSnapshot::Split { ratio, .. } => Some(*ratio),
        crate::persist::LayoutSnapshot::Pane(_) => None,
    }
}

#[cfg(test)]
fn unique_temp_path(name: &str) -> std::path::PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    std::env::temp_dir().join(format!("herdr-{name}-{}-{nanos}", std::process::id()))
}

#[cfg(test)]
fn wait_for_file(path: &std::path::Path) -> String {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while std::time::Instant::now() < deadline {
        if let Ok(content) = std::fs::read_to_string(path) {
            return content;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    panic!("timed out waiting for {}", path.display());
}
