use ratatui::layout::Rect;

use crate::app::state::{AppState, Mode};

use super::ScrollbarClickTarget;

const HERDPLAYER_LABEL: &str = "herdplayer";

/// The herdplay daemon shell (`shell.js`) is a sibling project, not part of
/// this repo. Its location is resolved rather than hardcoded so this works
/// on any machine that has it checked out: `HERDPLAYD_SHELL_PATH` overrides
/// it explicitly, otherwise it falls back to the conventional sibling
/// workspace layout under the user's home directory.
fn herdplayer_shell_path() -> Option<std::path::PathBuf> {
    if let Ok(p) = std::env::var("HERDPLAYD_SHELL_PATH") {
        return Some(std::path::PathBuf::from(p));
    }
    let home = std::env::var("HOME").ok()?;
    Some(
        std::path::Path::new(&home)
            .join("herdplay-workspace")
            .join("herdplay")
            .join("daemon")
            .join("shell.js"),
    )
}

fn herdplayer_launch_command() -> Option<String> {
    let path = herdplayer_shell_path()?;
    if !path.is_file() {
        return None;
    }
    Some(format!("node '{}'\n", path.display()))
}

impl AppState {
    pub(super) fn workspace_list_rect(&self) -> Rect {
        let sidebar = self.view.sidebar_rect;
        if self.sidebar_collapsed || sidebar.width <= 1 || sidebar.height == 0 {
            return Rect::default();
        }
        crate::ui::workspace_list_rect(self, sidebar)
    }

    pub(super) fn workspace_list_scrollbar_target_at(
        &self,
        col: u16,
        row: u16,
    ) -> Option<ScrollbarClickTarget> {
        let area = self.workspace_list_rect();
        let metrics = crate::ui::workspace_list_scroll_metrics(self, area);
        let track = crate::ui::workspace_list_scrollbar_rect(self, area)?;
        if col < track.x
            || col >= track.x + track.width
            || row < track.y
            || row >= track.y + track.height
        {
            return None;
        }
        if let Some(grab_row_offset) = crate::ui::scrollbar_thumb_grab_offset(metrics, track, row) {
            Some(ScrollbarClickTarget::Thumb { grab_row_offset })
        } else {
            Some(ScrollbarClickTarget::Track {
                offset_from_bottom: crate::ui::scrollbar_offset_from_row(metrics, track, row),
            })
        }
    }

    pub(super) fn workspace_list_offset_for_drag_row(
        &self,
        row: u16,
        grab_row_offset: u16,
    ) -> Option<usize> {
        let area = self.workspace_list_rect();
        let metrics = crate::ui::workspace_list_scroll_metrics(self, area);
        let track = crate::ui::workspace_list_scrollbar_rect(self, area)?;
        Some(crate::ui::scrollbar_offset_from_drag_row(
            metrics,
            track,
            row,
            grab_row_offset,
        ))
    }

    pub(super) fn set_workspace_list_offset_from_bottom(&mut self, offset_from_bottom: usize) {
        let area = self.workspace_list_rect();
        let metrics = crate::ui::workspace_list_scroll_metrics(self, area);
        self.workspace_scroll = metrics
            .max_offset_from_bottom
            .saturating_sub(offset_from_bottom);
        self.workspace_scroll = crate::ui::normalized_workspace_scroll(
            self,
            self.view.sidebar_rect,
            self.workspace_scroll,
        );
    }

    pub(super) fn scroll_workspace_list(&mut self, delta: i16) {
        if delta.is_negative() {
            self.workspace_scroll = self
                .workspace_scroll
                .saturating_sub(delta.unsigned_abs() as usize);
            self.workspace_scroll = crate::ui::normalized_workspace_scroll(
                self,
                self.view.sidebar_rect,
                self.workspace_scroll,
            );
            return;
        }

        let area = self.workspace_list_rect();
        let metrics = crate::ui::workspace_list_scroll_metrics(self, area);
        self.workspace_scroll = self
            .workspace_scroll
            .saturating_add(delta as usize)
            .min(metrics.max_offset_from_bottom);
        self.workspace_scroll = crate::ui::normalized_workspace_scroll(
            self,
            self.view.sidebar_rect,
            self.workspace_scroll,
        );
    }

    /// The `+ new` button, which trails the last list entry instead of sitting
    /// at a fixed offset from the sidebar's bottom.
    pub(crate) fn sidebar_footer_rect(&self) -> Rect {
        if self.sidebar_collapsed || crate::ui::spaces_section_collapsed(self) {
            return Rect::default();
        }
        crate::ui::new_workspace_button_rect(self, self.view.sidebar_rect)
    }

    pub(crate) fn sidebar_new_button_rect(&self) -> Rect {
        self.sidebar_footer_rect()
    }

    pub(super) fn on_sidebar_divider(&self, col: u16, row: u16) -> bool {
        if self.sidebar_collapsed {
            return false;
        }
        let sidebar = self.view.sidebar_rect;
        let toggle = crate::ui::expanded_sidebar_toggle_rect(sidebar);
        let on_toggle = toggle.width > 0
            && col >= toggle.x
            && col < toggle.x + toggle.width
            && row >= toggle.y
            && row < toggle.y + toggle.height;
        sidebar.width > 0
            && !on_toggle
            && col == sidebar.x + sidebar.width.saturating_sub(1)
            && row >= sidebar.y
            && row < sidebar.y + sidebar.height
    }

    pub(super) fn on_sidebar_toggle(&self, col: u16, row: u16) -> bool {
        if self.view.sidebar_rect == Rect::default() {
            return false;
        }
        let rect = if self.sidebar_collapsed {
            crate::ui::collapsed_sidebar_toggle_rect(self.view.sidebar_rect)
        } else {
            crate::ui::expanded_sidebar_toggle_rect(self.view.sidebar_rect)
        };
        rect.width > 0
            && col >= rect.x
            && col < rect.x + rect.width
            && row >= rect.y
            && row < rect.y + rect.height
    }

    pub(super) fn on_spaces_section_header(&self, col: u16, row: u16) -> bool {
        if self.sidebar_collapsed || self.view.sidebar_rect == Rect::default() {
            return false;
        }
        let rect = crate::ui::spaces_section_header_rect(self.view.sidebar_rect);
        rect.width > 0
            && col >= rect.x
            && col < rect.x + rect.width
            && row >= rect.y
            && row < rect.y + rect.height
    }

    pub(super) fn handle_player_pointer(
        &mut self,
        terminal_runtimes: &mut crate::terminal::TerminalRuntimeRegistry,
        col: u16,
        row: u16,
    ) -> bool {
        let Some(action) = crate::ui::player::player_action_at(self, col, row) else {
            self.player_bg_click = None;
            crate::ui::player::unfocus_player_input(self);
            return false;
        };
        match action {
            crate::ui::player::PlayerAction::Prev => {
                crate::ui::player::unfocus_player_input(self);
                self.player_bg_click = None;
                crate::ui::player::post_prev();
            }
            crate::ui::player::PlayerAction::PlayPause => {
                crate::ui::player::unfocus_player_input(self);
                self.player_bg_click = None;
                crate::ui::player::play_pause();
            }
            crate::ui::player::PlayerAction::Next => {
                crate::ui::player::unfocus_player_input(self);
                self.player_bg_click = None;
                crate::ui::player::post_next();
            }
            crate::ui::player::PlayerAction::Loop => {
                crate::ui::player::unfocus_player_input(self);
                self.player_bg_click = None;
                crate::ui::player::post_loop();
            }
            crate::ui::player::PlayerAction::Shuffle => {
                crate::ui::player::unfocus_player_input(self);
                self.player_bg_click = None;
                crate::ui::player::post_shuffle();
            }
            crate::ui::player::PlayerAction::Add => {
                self.player_bg_click = None;
                crate::ui::player::submit_player_add(self);
            }
            crate::ui::player::PlayerAction::Load => {
                self.player_bg_click = None;
                crate::ui::player::submit_player_load(self);
            }
            crate::ui::player::PlayerAction::VolumeDown => {
                crate::ui::player::unfocus_player_input(self);
                self.player_bg_click = None;
                crate::ui::player::nudge_volume(-0.1);
            }
            crate::ui::player::PlayerAction::VolumeUp => {
                crate::ui::player::unfocus_player_input(self);
                self.player_bg_click = None;
                crate::ui::player::nudge_volume(0.1);
            }
            crate::ui::player::PlayerAction::VolumeSet => {
                crate::ui::player::unfocus_player_input(self);
                self.player_bg_click = None;
                let hits = crate::ui::player::player_hit_areas(self);
                crate::ui::player::post_volume_at_bar(hits.vol_bar, col);
            }
            crate::ui::player::PlayerAction::VolumeIdle => {
                crate::ui::player::unfocus_player_input(self);
                self.player_bg_click = None;
            }
            crate::ui::player::PlayerAction::Seek => {
                crate::ui::player::unfocus_player_input(self);
                self.player_bg_click = None;
                let hits = crate::ui::player::player_hit_areas(self);
                crate::ui::player::post_seek_at_bar(hits.scrub, col);
            }
            crate::ui::player::PlayerAction::ScrubIdle => {
                crate::ui::player::unfocus_player_input(self);
                self.player_bg_click = None;
            }
            crate::ui::player::PlayerAction::PlaylistLoad(index) => {
                crate::ui::player::unfocus_player_input(self);
                self.player_bg_click = None;
                crate::ui::player::post_playlist_load(index);
            }
            crate::ui::player::PlayerAction::PlaylistRemove(index) => {
                crate::ui::player::unfocus_player_input(self);
                self.player_bg_click = None;
                crate::ui::player::post_playlist_remove(index);
            }
            crate::ui::player::PlayerAction::FocusInput => {
                self.player_bg_click = None;
                crate::ui::player::focus_player_input(self, col);
            }
            crate::ui::player::PlayerAction::Toggle => {
                self.player_bg_click = None;
                crate::ui::player::unfocus_player_input(self);
                self.player_expanded = !self.player_expanded;
            }
            crate::ui::player::PlayerAction::Background => {
                crate::ui::player::unfocus_player_input(self);
                let now = std::time::Instant::now();
                let double = self.player_bg_click.is_some_and(|(x, y, at)| {
                    now.duration_since(at) <= std::time::Duration::from_millis(350)
                        && col.abs_diff(x) <= 1
                        && row.abs_diff(y) <= 1
                });
                if double {
                    self.player_bg_click = None;
                    self.open_or_focus_herdplayer(terminal_runtimes);
                } else {
                    self.player_bg_click = Some((col, row, now));
                }
            }
        }
        true
    }

    pub(crate) fn find_herdplayer_pane(&self) -> Option<(usize, crate::layout::PaneId)> {
        for (ws_idx, ws) in self.workspaces.iter().enumerate() {
            for tab in &ws.tabs {
                for (pane_id, pane) in &tab.panes {
                    let Some(terminal) = self.terminals.get(&pane.attached_terminal_id) else {
                        continue;
                    };
                    if terminal.manual_label.as_deref() == Some(HERDPLAYER_LABEL) {
                        return Some((ws_idx, *pane_id));
                    }
                }
            }
        }
        None
    }

    pub(crate) fn open_or_focus_herdplayer(
        &mut self,
        terminal_runtimes: &mut crate::terminal::TerminalRuntimeRegistry,
    ) {
        if let Some((ws_idx, pane_id)) = self.find_herdplayer_pane() {
            self.focus_pane_in_workspace(ws_idx, pane_id);
            self.mode = crate::app::state::Mode::Terminal;
            return;
        }

        let previous = self.current_pane_focus_target();
        self.split_pane_with_placement(
            terminal_runtimes,
            ratatui::layout::Direction::Vertical,
            crate::layout::SplitPlacement::After,
        );
        if self.current_pane_focus_target() == previous {
            return;
        }
        let Some(ws_idx) = self.active else {
            return;
        };
        let Some(pane_id) = self.workspaces.get(ws_idx).and_then(|ws| ws.focused_pane_id()) else {
            return;
        };
        let Some(terminal_id) = self
            .workspaces
            .get(ws_idx)
            .and_then(|ws| ws.terminal_id(pane_id).cloned())
        else {
            return;
        };
        if let Some(terminal) = self.terminals.get_mut(&terminal_id) {
            terminal.set_manual_label(HERDPLAYER_LABEL.to_string());
        }
        if let Some(command) = herdplayer_launch_command() {
            if let Some(runtime) = terminal_runtimes.get(&terminal_id) {
                let _ = runtime.try_send_bytes(bytes::Bytes::from(command));
            }
        }
    }

    pub(super) fn set_manual_sidebar_width(&mut self, divider_col: u16) {
        let sidebar = self.view.sidebar_rect;
        if sidebar == Rect::default() {
            return;
        }
        let width = divider_col.saturating_sub(sidebar.x).saturating_add(1);
        self.sidebar_width = width.clamp(self.sidebar_min_width, self.sidebar_max_width);
        self.sidebar_width_source = crate::app::state::SidebarWidthSource::Manual;
        self.mark_session_dirty();
    }

    pub(super) fn workspace_at_row(&self, row: u16) -> Option<usize> {
        let footer = self.sidebar_footer_rect();
        if footer == Rect::default() {
            return None;
        }

        let cards = if self.view.workspace_card_areas.is_empty() {
            crate::ui::compute_workspace_card_areas(self, self.view.sidebar_rect)
        } else {
            self.view.workspace_card_areas.clone()
        };

        cards.iter().find_map(|card| {
            (row >= card.rect.y && row < card.rect.y + card.rect.height).then_some(card.ws_idx)
        })
    }

    pub(super) fn collapsed_workspace_at_row(&self, row: u16) -> Option<usize> {
        if !self.sidebar_collapsed {
            return None;
        }

        let (ws_area, _, _) = crate::ui::collapsed_sidebar_sections(self.view.sidebar_rect);
        if ws_area == Rect::default() || row < ws_area.y || row >= ws_area.y + ws_area.height {
            return None;
        }

        let idx = (row - ws_area.y) as usize;
        (idx < self.workspaces.len()).then_some(idx)
    }

    fn collapsed_detail_workspace_idx(&self) -> Option<usize> {
        if matches!(
            self.mode,
            Mode::Navigate
                | Mode::RenameWorkspace
                | Mode::Resize
                | Mode::ConfirmClose
                | Mode::ContextMenu
                | Mode::Settings
                | Mode::GlobalMenu
                | Mode::KeybindHelp
        ) {
            Some(self.selected)
        } else {
            self.active
        }
    }

    pub(super) fn collapsed_agent_detail_target_at(
        &self,
        row: u16,
    ) -> Option<(usize, usize, crate::layout::PaneId)> {
        if !self.sidebar_collapsed {
            return None;
        }

        let (_, _, detail_area) = crate::ui::collapsed_sidebar_sections(self.view.sidebar_rect);
        let detail_content_area = Rect::new(
            detail_area.x,
            detail_area.y,
            detail_area.width,
            detail_area.height.saturating_sub(1),
        );
        if detail_content_area == Rect::default()
            || row < detail_content_area.y
            || row >= detail_content_area.y + detail_content_area.height
        {
            return None;
        }

        let ws_idx = self.collapsed_detail_workspace_idx()?;
        let ws = self.workspaces.get(ws_idx)?;
        let detail_idx = (row - detail_content_area.y) as usize;
        let details = ws.pane_details(&self.terminals);
        let detail = details.get(detail_idx)?;
        Some((ws_idx, detail.tab_idx, detail.pane_id))
    }

    pub(super) fn workspace_drop_index_at_row(&self, row: u16) -> Option<usize> {
        let area = self.workspace_list_rect();
        let footer = self.sidebar_footer_rect();
        if area == Rect::default() || row < area.y || row >= area.y + area.height {
            return None;
        }
        // The `+ new` button floats up with the list, so only its own rows are
        // off limits; empty space below it still targets the bottom slot.
        if footer != Rect::default() && row >= footer.y && row < footer.y + footer.height {
            return None;
        }

        let (cards, agent_rows) = if self.view.workspace_card_areas.is_empty() {
            let (cards, agent_rows, _) =
                crate::ui::compute_workspace_list_areas(self, self.view.sidebar_rect);
            (cards, agent_rows)
        } else {
            (
                self.view.workspace_card_areas.clone(),
                self.view.agent_row_areas.clone(),
            )
        };
        if cards.is_empty() {
            return Some(0);
        }

        let mut insert_indices = Vec::with_capacity(cards.len() + 1);
        for (idx, card) in cards.iter().enumerate() {
            let card_group = self
                .workspaces
                .get(card.ws_idx)
                .and_then(|ws| ws.worktree_space())
                .map(|space| space.key.as_str());
            let previous_group = idx.checked_sub(1).and_then(|prev_idx| {
                self.workspaces
                    .get(cards[prev_idx].ws_idx)
                    .and_then(|ws| ws.worktree_space())
                    .map(|space| space.key.as_str())
            });
            let inside_group_gap = card_group.is_some() && card_group == previous_group;
            if !inside_group_gap {
                insert_indices.push(card.ws_idx);
            }
        }
        insert_indices.push(cards.last().map(|card| card.ws_idx + 1).unwrap_or(0));

        let mut best: Option<(usize, u16)> = None;
        for insert_idx in insert_indices {
            let Some(slot_row) =
                crate::ui::workspace_drop_indicator_row(&cards, &agent_rows, area, insert_idx)
            else {
                continue;
            };
            let distance = row.abs_diff(slot_row);
            match best {
                Some((best_idx, best_distance))
                    if distance > best_distance
                        || (distance == best_distance && insert_idx < best_idx) => {}
                _ => best = Some((insert_idx, distance)),
            }
        }

        best.map(|(insert_idx, _)| insert_idx)
    }

    /// Fold a space's agent entries away, or bring them back. Clicking the
    /// space card drives this.
    pub(crate) fn toggle_workspace_agents(&mut self, ws_idx: usize) {
        let Some(id) = self.workspaces.get(ws_idx).map(|ws| ws.id.clone()) else {
            return;
        };
        if crate::ui::workspace_agents_expanded(self, ws_idx) {
            self.collapsed_agent_space_ids.insert(id);
        } else {
            self.collapsed_agent_space_ids.remove(&id);
        }
        self.mark_session_dirty();
        self.workspace_scroll = crate::ui::normalized_workspace_scroll(
            self,
            self.view.sidebar_rect,
            self.workspace_scroll,
        );
    }

    /// Agent rows currently drawn for `ws_idx`, top to bottom.
    fn agent_rows_for_workspace(&self, ws_idx: usize) -> Vec<crate::app::state::AgentRowArea> {
        self.drawn_agent_rows()
            .into_iter()
            .filter(|area| area.ws_idx == ws_idx)
            .collect()
    }

    fn drawn_agent_rows(&self) -> Vec<crate::app::state::AgentRowArea> {
        if self.view.workspace_card_areas.is_empty() {
            crate::ui::compute_workspace_list_areas(self, self.view.sidebar_rect).1
        } else {
            self.view.agent_row_areas.clone()
        }
    }

    /// Folder rows currently drawn for `ws_idx`, top to bottom.
    pub(super) fn agent_folder_rows_for_workspace(
        &self,
        ws_idx: usize,
    ) -> Vec<crate::app::state::AgentFolderArea> {
        let folder_rows = if self.view.workspace_card_areas.is_empty() {
            crate::ui::compute_workspace_list_areas(self, self.view.sidebar_rect).2
        } else {
            self.view.agent_folder_areas.clone()
        };
        folder_rows
            .into_iter()
            .filter(|area| area.ws_idx == ws_idx)
            .collect()
    }

    /// The rows drawn for one folder: its agents, in the order they are listed.
    fn agent_rows_in_folder(
        &self,
        ws_idx: usize,
        key: &str,
    ) -> Vec<crate::app::state::AgentRowArea> {
        let members = crate::ui::workspace_agent_groups(self, ws_idx)
            .into_iter()
            .find(|group| group.key == key)
            .map(|group| {
                group
                    .agents
                    .into_iter()
                    .map(|member| member.pane_id)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let rows = self.agent_rows_for_workspace(ws_idx);
        members
            .into_iter()
            .filter_map(|pane_id| rows.iter().find(|area| area.pane_id == pane_id).cloned())
            .collect()
    }

    /// Where a dragged agent would land among the agents it shares a folder
    /// with: an insert-before position in that folder's list. An agent cannot
    /// leave the folder it is working in, so the cursor pins to the folder's
    /// ends rather than escaping it.
    pub(super) fn agent_drop_index_at_row(
        &self,
        ws_idx: usize,
        source_pane_id: crate::layout::PaneId,
        row: u16,
    ) -> Option<usize> {
        let (key, _) = crate::ui::agent_folder_position(self, ws_idx, source_pane_id)?;
        let rows = self.agent_rows_in_folder(ws_idx, &key);
        let first = rows.first()?;
        let last = rows.last()?;
        if row < first.rect.y {
            return Some(0);
        }
        if row >= last.rect.y + last.rect.height {
            return Some(rows.len());
        }

        match rows
            .iter()
            .position(|area| row >= area.rect.y && row < area.rect.y + area.rect.height)
        {
            // Top half of a row drops above it, bottom half below.
            Some(position) => {
                let hovered = &rows[position];
                let past_midpoint = row >= hovered.rect.y + hovered.rect.height / 2;
                Some(position + usize::from(past_midpoint))
            }
            // A gap row belongs to the entry below it, so it is that entry's
            // insert-above slot.
            None => rows.iter().position(|area| area.rect.y > row),
        }
    }

    /// Where a dragged folder would land among its space's folders: an
    /// insert-before position in that space's folder list.
    pub(super) fn agent_folder_drop_index_at_row(&self, ws_idx: usize, row: u16) -> Option<usize> {
        let folders = self.agent_folder_rows_for_workspace(ws_idx);
        let first = folders.first()?;
        let rows = self.agent_rows_for_workspace(ws_idx);
        let last_agent = rows.last()?;
        if row < first.rect.y {
            return Some(0);
        }
        if row >= last_agent.rect.y + last_agent.rect.height {
            return Some(folders.len());
        }

        // A folder owns every row from its own down to the row before the next
        // folder, so the block a cursor is in decides which slot it means, and
        // its midpoint decides which side of that block.
        let position = folders
            .iter()
            .rposition(|area| area.rect.y <= row)
            .unwrap_or(0);
        let block_top = folders[position].rect.y;
        let block_bottom = folders
            .get(position + 1)
            .map(|next| next.rect.y)
            .unwrap_or_else(|| last_agent.rect.y + last_agent.rect.height);
        let past_midpoint = row >= block_top + (block_bottom.saturating_sub(block_top)) / 2;
        Some(position + usize::from(past_midpoint))
    }

    pub(super) fn agent_detail_target_at(
        &self,
        row: u16,
    ) -> Option<(usize, usize, crate::layout::PaneId)> {
        if self.sidebar_collapsed {
            return None;
        }

        self.drawn_agent_rows()
            .iter()
            .find(|area| row >= area.rect.y && row < area.rect.y + area.rect.height)
            .map(|area| (area.ws_idx, area.tab_idx, area.pane_id))
    }

    /// The folder row under `row`, which a press can pick up and drag.
    pub(super) fn agent_folder_target_at(&self, row: u16) -> Option<(usize, String)> {
        if self.sidebar_collapsed {
            return None;
        }

        let folder_rows = if self.view.workspace_card_areas.is_empty() {
            crate::ui::compute_workspace_list_areas(self, self.view.sidebar_rect).2
        } else {
            self.view.agent_folder_areas.clone()
        };
        folder_rows
            .into_iter()
            .find(|area| row >= area.rect.y && row < area.rect.y + area.rect.height)
            .map(|area| (area.ws_idx, area.key))
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use crossterm::event::{MouseButton, MouseEventKind};
    use ratatui::layout::Rect;

    use super::super::{app_for_mouse_test, capture_snapshot, mouse, unique_temp_path};
    use crate::{
        app::state::{AgentPanelScope, ContextMenuKind, DragTarget, Mode},
        detect::Agent,
        workspace::Workspace,
    };

    #[test]
    fn clicking_launcher_opens_global_menu() {
        let mut app = app_for_mouse_test();
        let rect = app.state.global_launcher_rect();

        app.handle_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            rect.x + rect.width.saturating_sub(1),
            rect.y,
        ));

        assert_eq!(app.state.mode, Mode::GlobalMenu);
    }

    #[test]
    fn hovering_global_menu_updates_highlight() {
        let mut app = app_for_mouse_test();
        let launcher = app.state.global_launcher_rect();
        app.handle_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            launcher.x,
            launcher.y,
        ));

        let menu = app.state.global_menu_rect();
        app.handle_mouse(mouse(MouseEventKind::Moved, menu.x + 2, menu.y + 2));

        assert_eq!(app.state.global_menu.highlighted, 1);
    }

    #[test]
    fn clicking_keybinds_menu_item_opens_help() {
        let mut app = app_for_mouse_test();
        let launcher = app.state.global_launcher_rect();
        app.handle_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            launcher.x,
            launcher.y,
        ));

        let menu = app.state.global_menu_rect();
        app.handle_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            menu.x + 2,
            menu.y + 2,
        ));

        assert_eq!(app.state.mode, Mode::KeybindHelp);
    }

    #[test]
    fn clicking_settings_menu_item_opens_settings() {
        let mut app = app_for_mouse_test();
        let launcher = app.state.global_launcher_rect();
        app.handle_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            launcher.x,
            launcher.y,
        ));

        let menu = app.state.global_menu_rect();
        app.handle_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            menu.x + 2,
            menu.y + 1,
        ));

        assert_eq!(app.state.mode, Mode::Settings);
    }

    #[test]
    fn clicking_reload_config_menu_item_requests_reload() {
        let mut app = app_for_mouse_test();
        let launcher = app.state.global_launcher_rect();
        app.handle_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            launcher.x,
            launcher.y,
        ));

        let menu = app.state.global_menu_rect();
        app.handle_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            menu.x + 2,
            menu.y + 3,
        ));

        assert!(app.state.request_reload_config);
        assert_eq!(app.state.mode, Mode::Navigate);
    }

    #[test]
    fn update_pending_menu_uses_whats_new_entry_when_release_notes_exist() {
        let mut app = app_for_mouse_test();
        app.state.update_available = Some("0.3.2".into());
        app.state.latest_release_notes_available = true;

        let launcher = app.state.global_launcher_rect();
        app.handle_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            launcher.x,
            launcher.y,
        ));

        assert_eq!(
            app.state.global_menu_labels(),
            vec![
                "settings",
                "keybinds",
                "reload config",
                "what's new",
                "detach"
            ]
        );
        assert!(!app.state.should_quit);
    }

    #[test]
    fn persistence_mode_menu_surfaces_detach_action() {
        let mut app = app_for_mouse_test();
        app.state.detach_exits = false;

        let launcher = app.state.global_launcher_rect();
        app.handle_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            launcher.x,
            launcher.y,
        ));

        assert_eq!(
            app.state.global_menu_labels(),
            vec!["settings", "keybinds", "reload config", "detach"]
        );

        let menu = app.state.global_menu_rect();
        app.handle_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            menu.x + 2,
            menu.y + 4,
        ));

        assert!(app.state.detach_requested);
        assert!(!app.state.should_quit);
        assert_ne!(app.state.mode, Mode::GlobalMenu);
    }

    #[test]
    fn whats_new_remains_in_menu_for_latest_installed_release_notes() {
        let mut app = app_for_mouse_test();
        app.state.latest_release_notes_available = true;

        assert_eq!(
            app.state.global_menu_labels(),
            vec![
                "settings",
                "keybinds",
                "reload config",
                "what's new",
                "detach"
            ]
        );
    }

    #[test]
    fn clicking_agent_detail_row_switches_to_correct_tab_and_pane() {
        let mut app = app_for_mouse_test();
        let mut ws = Workspace::test_new("test");
        ws.tabs[0].set_custom_name("main".into());
        let first_pane = ws.tabs[0].root_pane;
        let first_tab = ws.test_add_tab(Some("logs"));
        let second_pane = ws.tabs[first_tab].root_pane;
        app.state.workspaces = vec![ws];
        app.state.ensure_test_terminals();
        let first_terminal_id = app.state.workspaces[0].tabs[0].panes[&first_pane]
            .attached_terminal_id
            .clone();
        app.state
            .terminals
            .get_mut(&first_terminal_id)
            .unwrap()
            .detected_agent = Some(Agent::Pi);
        let second_terminal_id = app.state.workspaces[0].tabs[first_tab].panes[&second_pane]
            .attached_terminal_id
            .clone();
        app.state
            .terminals
            .get_mut(&second_terminal_id)
            .unwrap()
            .detected_agent = Some(Agent::Claude);
        app.state.active = Some(0);
        app.state.selected = 0;
        app.state.mode = Mode::Terminal;
        app.state.view.sidebar_rect = Rect::new(0, 0, 26, 30);
        let (cards, agent_rows, folder_rows) =
            crate::ui::compute_workspace_list_areas(&app.state, app.state.view.sidebar_rect);
        app.state.view.workspace_card_areas = cards;
        app.state.view.agent_row_areas = agent_rows;
        app.state.view.agent_folder_areas = folder_rows;

        let target = app
            .state
            .view
            .agent_row_areas
            .iter()
            .find(|row| row.pane_id == first_pane)
            .expect("first pane should have an agent row")
            .rect;

        app.handle_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            target.x + 2,
            target.y,
        ));

        assert_eq!(app.state.workspaces[0].active_tab, 0);
        assert_eq!(app.state.workspaces[0].tabs[0].layout.focused(), first_pane);
        assert_eq!(app.state.mode, Mode::Terminal);
    }

    #[test]
    fn clicking_an_inactive_space_switches_without_folding_its_agents() {
        let mut app = app_for_mouse_test();
        app.state.workspaces = vec![Workspace::test_new("one"), Workspace::test_new("two")];
        app.state.ensure_test_terminals();
        app.state.active = Some(0);
        app.state.selected = 0;
        app.state.mode = Mode::Terminal;
        crate::ui::compute_view(&mut app.state, Rect::new(0, 0, 106, 24));
        app.state.view.sidebar_rect = Rect::new(0, 0, 26, 24);
        app.state.view.workspace_card_areas =
            crate::ui::compute_workspace_card_areas(&app.state, app.state.view.sidebar_rect);
        // The second space is folded away and must stay that way when the user
        // merely switches to it.
        let second_id = app.state.workspaces[1].id.clone();
        app.state.collapsed_agent_space_ids.insert(second_id);
        let card = app.state.view.workspace_card_areas[1].rect;

        app.handle_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            card.x + 2,
            card.y,
        ));
        app.handle_mouse(mouse(
            MouseEventKind::Up(MouseButton::Left),
            card.x + 2,
            card.y,
        ));

        assert_eq!(app.state.active, Some(1));
        assert!(!crate::ui::workspace_agents_expanded(&app.state, 1));
        // The space that lost focus keeps its own agents listed.
        assert!(crate::ui::workspace_agents_expanded(&app.state, 0));
    }

    #[test]
    fn clicking_the_active_space_card_toggles_its_agent_entries() {
        let mut app = app_for_mouse_test();
        app.state.workspaces = vec![Workspace::test_new("one"), Workspace::test_new("two")];
        app.state.ensure_test_terminals();
        app.state.active = Some(0);
        app.state.selected = 0;
        app.state.mode = Mode::Terminal;
        crate::ui::compute_view(&mut app.state, Rect::new(0, 0, 106, 24));
        app.state.view.sidebar_rect = Rect::new(0, 0, 26, 24);
        app.state.view.workspace_card_areas =
            crate::ui::compute_workspace_card_areas(&app.state, app.state.view.sidebar_rect);
        let card = app.state.view.workspace_card_areas[0].rect;
        let second_id = app.state.workspaces[1].id.clone();
        assert!(crate::ui::workspace_agents_expanded(&app.state, 0));

        app.handle_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            card.x + 2,
            card.y,
        ));
        app.handle_mouse(mouse(
            MouseEventKind::Up(MouseButton::Left),
            card.x + 2,
            card.y,
        ));

        assert_eq!(app.state.active, Some(0));
        assert!(!crate::ui::workspace_agents_expanded(&app.state, 0));
        // Only the clicked space folds; siblings keep their agents listed.
        assert!(!app.state.collapsed_agent_space_ids.contains(&second_id));

        app.handle_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            card.x + 2,
            card.y,
        ));
        app.handle_mouse(mouse(
            MouseEventKind::Up(MouseButton::Left),
            card.x + 2,
            card.y,
        ));

        assert!(crate::ui::workspace_agents_expanded(&app.state, 0));
    }

    #[test]
    fn collapsed_space_drops_its_agent_rows_from_the_list() {
        let mut app = app_for_mouse_test();
        let ws = Workspace::test_new("one");
        let root_pane = ws.tabs[0].root_pane;
        app.state.workspaces = vec![ws];
        app.state.ensure_test_terminals();
        let terminal_id = app.state.workspaces[0].tabs[0].panes[&root_pane]
            .attached_terminal_id
            .clone();
        app.state
            .terminals
            .get_mut(&terminal_id)
            .unwrap()
            .detected_agent = Some(crate::detect::Agent::Claude);
        app.state.active = Some(0);
        app.state.selected = 0;
        app.state.mode = Mode::Terminal;
        crate::ui::compute_view(&mut app.state, Rect::new(0, 0, 106, 24));
        let sidebar = app.state.view.sidebar_rect;

        let expanded = crate::ui::compute_workspace_list_areas(&app.state, sidebar);
        assert!(!expanded.1.is_empty());

        app.state.toggle_workspace_agents(0);

        let collapsed = crate::ui::compute_workspace_list_areas(&app.state, sidebar);
        assert_eq!(collapsed.0.len(), 1);
        assert!(collapsed.1.is_empty());
    }

    #[test]
    fn clicking_all_workspaces_agent_row_switches_to_correct_workspace() {
        let mut app = app_for_mouse_test();
        let first = Workspace::test_new("one");
        let first_pane = first.tabs[0].root_pane;

        let second = Workspace::test_new("two");
        let second_pane = second.tabs[0].root_pane;

        app.state.workspaces = vec![first, second];
        app.state.ensure_test_terminals();
        let first_terminal_id = app.state.workspaces[0].tabs[0].panes[&first_pane]
            .attached_terminal_id
            .clone();
        app.state
            .terminals
            .get_mut(&first_terminal_id)
            .unwrap()
            .detected_agent = Some(Agent::Pi);
        let second_terminal_id = app.state.workspaces[1].tabs[0].panes[&second_pane]
            .attached_terminal_id
            .clone();
        app.state
            .terminals
            .get_mut(&second_terminal_id)
            .unwrap()
            .detected_agent = Some(Agent::Claude);
        app.state.active = Some(0);
        app.state.selected = 0;
        app.state.mode = Mode::Terminal;
        app.state.agent_panel_scope = AgentPanelScope::AllWorkspaces;
        app.state.view.sidebar_rect = Rect::new(0, 0, 26, 40);
        let (cards, agent_rows, folder_rows) =
            crate::ui::compute_workspace_list_areas(&app.state, app.state.view.sidebar_rect);
        app.state.view.workspace_card_areas = cards;
        app.state.view.agent_row_areas = agent_rows;
        app.state.view.agent_folder_areas = folder_rows;

        let target = app
            .state
            .view
            .agent_row_areas
            .iter()
            .find(|row| row.ws_idx == 1)
            .expect("second workspace should list its agent")
            .rect;

        app.handle_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            target.x + 2,
            target.y,
        ));

        assert_eq!(app.state.active, Some(1));
        assert_eq!(app.state.selected, 1);
        assert_eq!(app.state.workspaces[1].active_tab, 0);
    }

    #[test]
    fn scrolling_sidebar_with_wheel_without_scrollbar_keeps_selection() {
        let mut app = app_for_mouse_test();
        let mut ws = Workspace::test_new("test");
        let first_pane = ws.tabs[0].root_pane;

        let mut tabs = Vec::new();
        for (tab_name, agent) in [
            ("logs", Agent::Claude),
            ("review", Agent::Codex),
            ("ops", Agent::Gemini),
        ] {
            let tab_idx = ws.test_add_tab(Some(tab_name));
            let pane_id = ws.tabs[tab_idx].root_pane;
            tabs.push((tab_idx, pane_id, agent));
        }

        app.state.workspaces = vec![ws];
        app.state.ensure_test_terminals();
        let first_terminal_id = app.state.workspaces[0].tabs[0].panes[&first_pane]
            .attached_terminal_id
            .clone();
        app.state
            .terminals
            .get_mut(&first_terminal_id)
            .unwrap()
            .detected_agent = Some(Agent::Pi);
        for (tab_idx, pane_id, agent) in tabs {
            let terminal_id = app.state.workspaces[0].tabs[tab_idx].panes[&pane_id]
                .attached_terminal_id
                .clone();
            app.state
                .terminals
                .get_mut(&terminal_id)
                .unwrap()
                .detected_agent = Some(agent);
        }
        app.state.active = Some(0);
        app.state.selected = 0;
        app.state.mode = Mode::Terminal;

        app.handle_mouse(mouse(MouseEventKind::ScrollDown, 2, 16));

        assert_eq!(app.state.workspace_scroll, 0);
        assert_eq!(app.state.selected, 0);
    }

    #[test]
    fn clicking_scrolled_agent_detail_row_switches_to_correct_tab_and_pane() {
        let mut app = app_for_mouse_test();
        let mut ws = Workspace::test_new("test");
        let first_pane = ws.tabs[0].root_pane;
        let second_tab = ws.test_add_tab(Some("logs"));
        let second_pane = ws.tabs[second_tab].root_pane;
        let mut extra_tabs = Vec::new();
        for (tab_name, agent) in [("review", Agent::Codex), ("ops", Agent::Gemini)] {
            let tab_idx = ws.test_add_tab(Some(tab_name));
            let pane_id = ws.tabs[tab_idx].root_pane;
            extra_tabs.push((tab_idx, pane_id, agent));
        }

        app.state.workspaces = vec![ws];
        app.state.ensure_test_terminals();
        let first_terminal_id = app.state.workspaces[0].tabs[0].panes[&first_pane]
            .attached_terminal_id
            .clone();
        app.state
            .terminals
            .get_mut(&first_terminal_id)
            .unwrap()
            .detected_agent = Some(Agent::Pi);
        let second_terminal_id = app.state.workspaces[0].tabs[second_tab].panes[&second_pane]
            .attached_terminal_id
            .clone();
        app.state
            .terminals
            .get_mut(&second_terminal_id)
            .unwrap()
            .detected_agent = Some(Agent::Claude);
        for (tab_idx, pane_id, agent) in extra_tabs {
            let terminal_id = app.state.workspaces[0].tabs[tab_idx].panes[&pane_id]
                .attached_terminal_id
                .clone();
            app.state
                .terminals
                .get_mut(&terminal_id)
                .unwrap()
                .detected_agent = Some(agent);
        }
        app.state.active = Some(0);
        app.state.selected = 0;
        app.state.mode = Mode::Terminal;
        // Scroll the merged list past the workspace card so agent rows shift up.
        app.state.view.sidebar_rect = Rect::new(0, 0, 26, 20);
        app.state.workspace_scroll = 1;
        let (cards, agent_rows, folder_rows) =
            crate::ui::compute_workspace_list_areas(&app.state, app.state.view.sidebar_rect);
        app.state.view.workspace_card_areas = cards;
        app.state.view.agent_row_areas = agent_rows;
        app.state.view.agent_folder_areas = folder_rows;

        let target = app
            .state
            .view
            .agent_row_areas
            .iter()
            .find(|row| row.pane_id == second_pane)
            .expect("scrolled list should still expose the second pane's row")
            .rect;

        app.handle_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            target.x + 2,
            target.y,
        ));

        assert_eq!(app.state.workspaces[0].active_tab, second_tab);
        assert_eq!(
            app.state.workspaces[0].tabs[second_tab].layout.focused(),
            second_pane
        );
        assert_eq!(app.state.mode, Mode::Terminal);
    }

    #[test]
    fn clicking_removed_collapsed_agent_row_is_noop() {
        let mut app = app_for_mouse_test();
        let mut ws = Workspace::test_new("test");
        let first_pane = ws.tabs[0].root_pane;
        let second_tab = ws.test_add_tab(Some("logs"));
        let second_pane = ws.tabs[second_tab].root_pane;
        app.state.workspaces = vec![ws];
        app.state.ensure_test_terminals();
        let first_terminal_id = app.state.workspaces[0].tabs[0].panes[&first_pane]
            .attached_terminal_id
            .clone();
        app.state
            .terminals
            .get_mut(&first_terminal_id)
            .unwrap()
            .detected_agent = Some(Agent::Pi);
        let second_terminal_id = app.state.workspaces[0].tabs[second_tab].panes[&second_pane]
            .attached_terminal_id
            .clone();
        app.state
            .terminals
            .get_mut(&second_terminal_id)
            .unwrap()
            .detected_agent = Some(Agent::Claude);
        app.state.active = Some(0);
        app.state.selected = 0;
        app.state.mode = Mode::Terminal;
        app.state.sidebar_collapsed = true;

        app.handle_mouse(mouse(MouseEventKind::Down(MouseButton::Left), 2, 16));

        assert_eq!(app.state.workspaces[0].active_tab, 0);
        assert_eq!(
            app.state.workspaces[0].tabs[1].layout.focused(),
            second_pane
        );
        assert_eq!(app.state.mode, Mode::Terminal);
    }

    #[test]
    fn clicking_removed_collapsed_sidebar_toggle_is_noop() {
        let mut app = app_for_mouse_test();
        app.state.sidebar_collapsed = true;

        app.handle_mouse(mouse(MouseEventKind::Down(MouseButton::Left), 2, 16));

        assert!(app.state.sidebar_collapsed);
    }

    #[test]
    fn clicking_removed_expanded_sidebar_toggle_is_noop() {
        let mut app = app_for_mouse_test();
        app.state.sidebar_collapsed = false;

        app.handle_mouse(mouse(MouseEventKind::Down(MouseButton::Left), 2, 16));

        assert!(!app.state.sidebar_collapsed);
        assert!(app.state.drag.is_none());
    }

    #[test]
    fn clicking_spaces_header_toggles_spaces_section_not_sidebar() {
        let mut app = app_for_mouse_test();
        app.state.workspaces = vec![Workspace::test_new("a"), Workspace::test_new("b")];
        app.state.active = Some(0);
        app.state.selected = 0;
        app.state.mode = Mode::Terminal;
        crate::ui::compute_view(&mut app.state, Rect::new(0, 0, 106, 20));
        let sidebar = app.state.view.sidebar_rect;

        app.handle_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            sidebar.x + 2,
            sidebar.y,
        ));

        assert!(app.state.spaces_collapsed);
        assert!(!app.state.sidebar_collapsed);
        assert_eq!(app.state.sidebar_footer_rect(), Rect::default());

        app.handle_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            sidebar.x + 2,
            sidebar.y,
        ));

        assert!(!app.state.spaces_collapsed);
    }

    #[test]
    fn folding_the_spaces_section_queues_a_session_save() {
        let mut app = app_for_mouse_test();
        app.state.workspaces = vec![Workspace::test_new("a")];
        app.state.active = Some(0);
        app.state.selected = 0;
        app.state.mode = Mode::Terminal;
        crate::ui::compute_view(&mut app.state, Rect::new(0, 0, 106, 20));
        let sidebar = app.state.view.sidebar_rect;
        app.state.session_dirty = false;

        app.handle_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            sidebar.x + 2,
            sidebar.y,
        ));

        assert!(app.state.spaces_collapsed);
        assert!(app.state.session_dirty);
    }

    #[test]
    fn folding_a_space_agent_list_queues_a_session_save() {
        let mut app = app_for_mouse_test();
        app.state.workspaces = vec![Workspace::test_new("a")];
        app.state.active = Some(0);
        app.state.selected = 0;
        app.state.session_dirty = false;

        app.state.toggle_workspace_agents(0);

        assert!(!crate::ui::workspace_agents_expanded(&app.state, 0));
        assert!(app.state.session_dirty);
    }

    #[test]
    fn clicking_workspace_switches_on_mouse_up() {
        let mut app = app_for_mouse_test();
        app.state.workspaces = vec![Workspace::test_new("a"), Workspace::test_new("b")];
        app.state.active = Some(0);
        app.state.selected = 0;
        crate::ui::compute_view(&mut app.state, Rect::new(0, 0, 106, 20));
        app.state.view.sidebar_rect = Rect::new(0, 0, 26, 20);
        app.state.view.workspace_card_areas =
            crate::ui::compute_workspace_card_areas(&app.state, app.state.view.sidebar_rect);
        let target_row = app.state.view.workspace_card_areas[1].rect.y;

        app.handle_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            2,
            target_row,
        ));
        assert_eq!(app.state.active, Some(0));
        assert!(app.state.workspace_press.is_some());

        app.handle_mouse(mouse(MouseEventKind::Up(MouseButton::Left), 2, target_row));
        assert_eq!(app.state.active, Some(1));
        assert_eq!(app.state.selected, 1);
        assert!(app.state.workspace_press.is_none());
        let snapshot = capture_snapshot(&app.state);
        assert_eq!(snapshot.active, Some(1));
        assert_eq!(snapshot.selected, 1);
    }

    #[test]
    fn clicking_worktree_parent_row_focuses_workspace_without_toggling() {
        let mut app = app_for_mouse_test();
        app.state.workspaces = vec![Workspace::test_new("main"), Workspace::test_new("issue")];
        for (idx, checkout_path) in ["/repo/herdr", "/repo/herdr-issue"].into_iter().enumerate() {
            app.state.workspaces[idx].worktree_space =
                Some(crate::workspace::WorktreeSpaceMembership {
                    key: "repo-key".into(),
                    label: "herdr".into(),
                    repo_root: "/repo/herdr".into(),
                    checkout_path: checkout_path.into(),
                    is_linked_worktree: idx > 0,
                });
        }
        app.state.active = None;
        app.state.mode = Mode::Terminal;
        crate::ui::compute_view(&mut app.state, Rect::new(0, 0, 106, 20));
        app.state.view.sidebar_rect = Rect::new(0, 0, 26, 20);
        app.state.view.workspace_card_areas =
            crate::ui::compute_workspace_card_areas(&app.state, app.state.view.sidebar_rect);
        let parent = app.state.view.workspace_card_areas[0].rect;

        app.handle_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            parent.x + 2,
            parent.y,
        ));
        app.handle_mouse(mouse(
            MouseEventKind::Up(MouseButton::Left),
            parent.x + 2,
            parent.y,
        ));

        assert_eq!(app.state.active, Some(0));
        assert!(!app.state.collapsed_space_keys.contains("repo-key"));
    }

    #[test]
    fn clicking_worktree_parent_chevron_toggles_group_only() {
        let mut app = app_for_mouse_test();
        app.state.workspaces = vec![Workspace::test_new("main"), Workspace::test_new("issue")];
        for (idx, checkout_path) in ["/repo/herdr", "/repo/herdr-issue"].into_iter().enumerate() {
            app.state.workspaces[idx].worktree_space =
                Some(crate::workspace::WorktreeSpaceMembership {
                    key: "repo-key".into(),
                    label: "herdr".into(),
                    repo_root: "/repo/herdr".into(),
                    checkout_path: checkout_path.into(),
                    is_linked_worktree: idx > 0,
                });
        }
        app.state.active = None;
        app.state.mode = Mode::Terminal;
        crate::ui::compute_view(&mut app.state, Rect::new(0, 0, 106, 20));
        app.state.view.sidebar_rect = Rect::new(0, 0, 26, 20);
        app.state.view.workspace_card_areas =
            crate::ui::compute_workspace_card_areas(&app.state, app.state.view.sidebar_rect);
        let parent = app.state.view.workspace_card_areas[0].rect;

        app.handle_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            parent.x,
            parent.y,
        ));

        assert_eq!(app.state.active, None);
        assert!(app.state.workspace_press.is_none());
        assert!(app.state.collapsed_space_keys.contains("repo-key"));

        app.handle_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            parent.x,
            parent.y,
        ));

        assert!(!app.state.collapsed_space_keys.contains("repo-key"));
    }

    /// A sidebar whose spaces and agents are taller than the window.
    fn app_with_overflowing_sidebar() -> crate::app::App {
        let mut app = app_for_mouse_test();
        app.state.workspaces = (0..3)
            .map(|idx| {
                let mut ws = Workspace::test_new(&format!("space{idx}"));
                for tab in 1..4 {
                    ws.test_add_tab(Some(&format!("tab{tab}")));
                }
                ws
            })
            .collect();
        app.state.ensure_test_terminals();
        for ws_idx in 0..app.state.workspaces.len() {
            for tab_idx in 0..app.state.workspaces[ws_idx].tabs.len() {
                let pane = app.state.workspaces[ws_idx].tabs[tab_idx].root_pane;
                let terminal_id = app.state.workspaces[ws_idx].tabs[tab_idx].panes[&pane]
                    .attached_terminal_id
                    .clone();
                app.state
                    .terminals
                    .get_mut(&terminal_id)
                    .unwrap()
                    .detected_agent = Some(Agent::Claude);
            }
        }
        app.state.active = Some(0);
        app.state.selected = 0;
        app.state.mode = Mode::Terminal;
        app.state.view.sidebar_rect = Rect::new(0, 0, 26, 30);
        let (cards, agent_rows, folder_rows) =
            crate::ui::compute_workspace_list_areas(&app.state, app.state.view.sidebar_rect);
        app.state.view.workspace_card_areas = cards;
        app.state.view.agent_row_areas = agent_rows;
        app.state.view.agent_folder_areas = folder_rows;
        app
    }

    #[test]
    fn wheel_over_an_agent_row_scrolls_the_sidebar_both_ways() {
        let mut app = app_with_overflowing_sidebar();
        let row = app
            .state
            .view
            .agent_row_areas
            .first()
            .expect("the list should show agent rows")
            .rect;

        app.handle_mouse(mouse(MouseEventKind::ScrollDown, row.x + 2, row.y + 1));
        assert_eq!(app.state.workspace_scroll, 1, "wheel down should scroll");
        app.handle_mouse(mouse(MouseEventKind::ScrollDown, row.x + 2, row.y + 1));
        assert_eq!(app.state.workspace_scroll, 2);

        app.handle_mouse(mouse(MouseEventKind::ScrollUp, row.x + 2, row.y + 1));
        assert_eq!(app.state.workspace_scroll, 1, "wheel up should scroll back");

        // Scrolling the list must not double as changing which space is active.
        assert_eq!(app.state.selected, 0);
        assert_eq!(app.state.active, Some(0));
    }

    #[test]
    fn sidebar_scroll_survives_the_next_render() {
        let mut app = app_with_overflowing_sidebar();
        let area = Rect::new(0, 0, 106, 30);
        crate::ui::compute_view(&mut app.state, area);
        let row = crate::ui::compute_workspace_list_areas(&app.state, app.state.view.sidebar_rect)
            .1
            .first()
            .expect("the list should show agent rows")
            .rect;

        app.handle_mouse(mouse(MouseEventKind::ScrollDown, row.x + 2, row.y + 1));
        let scrolled = app.state.workspace_scroll;
        assert!(scrolled > 0, "the wheel should have scrolled the list");

        // Frames are drawn constantly — an animating agent alone repaints
        // several times a second — so a scroll offset that does not survive a
        // render is a scroll offset the user never sees.
        crate::ui::compute_view(&mut app.state, area);

        assert_eq!(app.state.workspace_scroll, scrolled);
    }

    #[test]
    fn wheel_over_an_agent_row_stops_at_the_end_of_the_list() {
        let mut app = app_with_overflowing_sidebar();
        let row = app
            .state
            .view
            .agent_row_areas
            .first()
            .expect("the list should show agent rows")
            .rect;
        let max = crate::ui::workspace_list_scroll_metrics(
            &app.state,
            crate::ui::workspace_list_rect(&app.state, app.state.view.sidebar_rect),
        )
        .max_offset_from_bottom;

        for _ in 0..max + 5 {
            app.handle_mouse(mouse(MouseEventKind::ScrollDown, row.x + 2, row.y + 1));
        }

        assert_eq!(app.state.workspace_scroll, max);
    }

    #[test]
    fn wheel_workspace_selection_follows_grouped_visual_order_without_scrollbar() {
        let mut app = app_for_mouse_test();
        app.state.workspaces = vec![
            Workspace::test_new("main"),
            Workspace::test_new("normal"),
            Workspace::test_new("issue"),
        ];
        for (idx, checkout_path) in [(0, "/repo/herdr"), (2, "/repo/herdr-issue")] {
            app.state.workspaces[idx].worktree_space =
                Some(crate::workspace::WorktreeSpaceMembership {
                    key: "repo-key".into(),
                    label: "herdr".into(),
                    repo_root: "/repo/herdr".into(),
                    checkout_path: checkout_path.into(),
                    is_linked_worktree: idx != 0,
                });
        }
        app.state.active = Some(0);
        app.state.selected = 0;
        app.state.mode = Mode::Navigate;
        crate::ui::compute_view(&mut app.state, Rect::new(0, 0, 106, 30));
        app.state.view.sidebar_rect = Rect::new(0, 0, 26, 30);
        app.state.view.workspace_card_areas =
            crate::ui::compute_workspace_card_areas(&app.state, app.state.view.sidebar_rect);
        let list = app.state.workspace_list_rect();
        assert!(!crate::ui::should_show_scrollbar(
            crate::ui::workspace_list_scroll_metrics(&app.state, list)
        ));

        app.handle_mouse(mouse(MouseEventKind::ScrollDown, list.x + 1, list.y + 1));

        assert_eq!(app.state.selected, 2);
    }

    #[test]
    fn dragging_workspace_reorders_without_changing_identity() {
        let mut app = app_for_mouse_test();
        app.state.workspaces = vec![
            Workspace::test_new("a"),
            Workspace::test_new("b"),
            Workspace::test_new("c"),
        ];
        let active_id = app.state.workspaces[1].id.clone();
        let selected_id = app.state.workspaces[2].id.clone();
        app.state.active = Some(1);
        app.state.selected = 2;
        crate::ui::compute_view(&mut app.state, Rect::new(0, 0, 106, 20));
        app.state.view.sidebar_rect = Rect::new(0, 0, 26, 20);
        app.state.view.workspace_card_areas =
            crate::ui::compute_workspace_card_areas(&app.state, app.state.view.sidebar_rect);
        let source_row = app.state.view.workspace_card_areas[1].rect.y;
        let target_row = crate::ui::workspace_drop_indicator_row(
            &app.state.view.workspace_card_areas,
            &app.state.view.agent_row_areas,
            app.state.workspace_list_rect(),
            0,
        )
        .unwrap();

        app.handle_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            2,
            source_row,
        ));
        app.handle_mouse(mouse(
            MouseEventKind::Drag(MouseButton::Left),
            2,
            target_row,
        ));
        assert!(matches!(
            app.state.drag.as_ref().map(|drag| &drag.target),
            Some(DragTarget::WorkspaceReorder {
                source_ws_idx: 1,
                insert_idx: Some(0),
            })
        ));
        app.handle_mouse(mouse(MouseEventKind::Up(MouseButton::Left), 2, target_row));

        let names: Vec<_> = app
            .state
            .workspaces
            .iter()
            .map(|ws| ws.display_name())
            .collect();
        assert_eq!(names, vec!["b", "a", "c"]);
        assert_eq!(app.state.active, Some(0));
        assert_eq!(app.state.selected, 2);
        assert_eq!(app.state.workspaces[0].id, active_id);
        assert_eq!(app.state.workspaces[2].id, selected_id);
        let snapshot = capture_snapshot(&app.state);
        let captured_names: Vec<_> = snapshot
            .workspaces
            .iter()
            .map(|ws| ws.custom_name.clone().unwrap())
            .collect();
        assert_eq!(captured_names, vec!["b", "a", "c"]);
    }

    /// A space with `agents.len()` agent rows, one pane per tab, laid out in
    /// the sidebar and ready for mouse events.
    fn app_with_agent_rows(agents: &[(&str, Agent)]) -> crate::app::App {
        let mut app = app_for_mouse_test();
        let mut ws = Workspace::test_new("space");
        let mut panes = vec![(0_usize, ws.tabs[0].root_pane, agents[0].1)];
        for (name, agent) in &agents[1..] {
            let tab_idx = ws.test_add_tab(Some(name));
            let pane_id = ws.tabs[tab_idx].root_pane;
            panes.push((tab_idx, pane_id, *agent));
        }

        app.state.workspaces = vec![ws];
        app.state.ensure_test_terminals();
        for (tab_idx, pane_id, agent) in panes {
            let terminal_id = app.state.workspaces[0].tabs[tab_idx].panes[&pane_id]
                .attached_terminal_id
                .clone();
            app.state
                .terminals
                .get_mut(&terminal_id)
                .unwrap()
                .detected_agent = Some(agent);
        }
        app.state.active = Some(0);
        app.state.selected = 0;
        app.state.mode = Mode::Terminal;
        app.state.view.sidebar_rect = Rect::new(0, 0, 26, 40);
        let (cards, agent_rows, folder_rows) =
            crate::ui::compute_workspace_list_areas(&app.state, app.state.view.sidebar_rect);
        app.state.view.workspace_card_areas = cards;
        app.state.view.agent_row_areas = agent_rows;
        app.state.view.agent_folder_areas = folder_rows;
        app
    }

    fn agent_row_pane_ids(app: &crate::app::App) -> Vec<crate::layout::PaneId> {
        app.state.workspaces[0]
            .pane_details(&app.state.terminals)
            .iter()
            .map(|detail| detail.pane_id)
            .collect()
    }

    /// One space whose agents work in the folders given, one agent per entry.
    fn app_with_agent_folders(agents: &[(&str, &str)]) -> crate::app::App {
        let mut app = app_for_mouse_test();
        let mut ws = Workspace::test_new("space");
        let mut panes = vec![(0_usize, ws.tabs[0].root_pane, agents[0].1)];
        for (name, cwd) in &agents[1..] {
            let tab_idx = ws.test_add_tab(Some(name));
            let pane_id = ws.tabs[tab_idx].root_pane;
            panes.push((tab_idx, pane_id, *cwd));
        }

        app.state.workspaces = vec![ws];
        app.state.ensure_test_terminals();
        for (tab_idx, pane_id, cwd) in panes {
            let terminal_id = app.state.workspaces[0].tabs[tab_idx].panes[&pane_id]
                .attached_terminal_id
                .clone();
            let terminal = app.state.terminals.get_mut(&terminal_id).unwrap();
            terminal.detected_agent = Some(Agent::Claude);
            terminal.cwd = std::path::PathBuf::from(cwd);
        }
        app.state.active = Some(0);
        app.state.selected = 0;
        app.state.mode = Mode::Terminal;
        app.state.view.sidebar_rect = Rect::new(0, 0, 26, 40);
        let (cards, agent_rows, folder_rows) =
            crate::ui::compute_workspace_list_areas(&app.state, app.state.view.sidebar_rect);
        app.state.view.workspace_card_areas = cards;
        app.state.view.agent_row_areas = agent_rows;
        app.state.view.agent_folder_areas = folder_rows;
        app
    }

    fn drag_sidebar(app: &mut crate::app::App, from: (u16, u16), to: (u16, u16)) {
        app.handle_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            from.0,
            from.1,
        ));
        app.handle_mouse(mouse(MouseEventKind::Drag(MouseButton::Left), to.0, to.1));
        app.handle_mouse(mouse(MouseEventKind::Up(MouseButton::Left), to.0, to.1));
    }

    #[test]
    fn agents_in_one_folder_are_listed_together_however_their_tabs_are_ordered() {
        let app =
            app_with_agent_folders(&[("one", "/srv/a"), ("two", "/srv/b"), ("three", "/srv/a")]);
        let natural = app.state.workspaces[0].natural_pane_order();
        let drawn = app
            .state
            .view
            .agent_row_areas
            .iter()
            .map(|area| area.pane_id)
            .collect::<Vec<_>>();

        assert_eq!(
            drawn,
            vec![natural[0], natural[2], natural[1]],
            "the two panes in /srv/a are listed together, though a /srv/b pane opened between them"
        );
        assert_eq!(app.state.view.agent_folder_areas.len(), 2);
    }

    #[test]
    fn dragging_a_folder_row_moves_every_agent_under_it() {
        let mut app =
            app_with_agent_folders(&[("one", "/srv/a"), ("two", "/srv/b"), ("three", "/srv/a")]);
        let natural = app.state.workspaces[0].natural_pane_order();
        let folders = app.state.view.agent_folder_areas.clone();
        let rows = app.state.view.agent_row_areas.clone();
        let last = rows.last().unwrap().rect;

        drag_sidebar(
            &mut app,
            (folders[0].rect.x + 2, folders[0].rect.y),
            (folders[0].rect.x + 2, last.y + last.height),
        );

        assert_eq!(
            agent_row_pane_ids(&app),
            vec![natural[1], natural[0], natural[2]],
            "the /srv/b folder now leads, and /srv/a keeps both of its agents in order"
        );
    }

    #[test]
    fn dragging_an_agent_keeps_it_inside_its_own_folder() {
        let mut app =
            app_with_agent_folders(&[("one", "/srv/a"), ("two", "/srv/a"), ("three", "/srv/b")]);
        let natural = app.state.workspaces[0].natural_pane_order();
        let rows = app.state.view.agent_row_areas.clone();
        let source = rows[0].rect;
        let below_everything = rows[2].rect.y + rows[2].rect.height + 4;

        drag_sidebar(
            &mut app,
            (source.x + 2, source.y),
            (source.x + 2, below_everything),
        );

        assert_eq!(
            agent_row_pane_ids(&app),
            vec![natural[1], natural[0], natural[2]],
            "the agent lands at the end of its own folder, not at the end of the space"
        );
    }

    #[test]
    fn dragging_agent_row_reorders_the_list_without_touching_tabs_or_layout() {
        let mut app = app_with_agent_rows(&[
            ("one", Agent::Pi),
            ("two", Agent::Claude),
            ("three", Agent::Codex),
        ]);
        let natural = agent_row_pane_ids(&app);
        assert_eq!(natural.len(), 3);
        let tab_numbers: Vec<_> = app.state.workspaces[0]
            .tabs
            .iter()
            .map(|tab| tab.number)
            .collect();

        let rows = app.state.view.agent_row_areas.clone();
        let source = rows[0].rect;
        // Past the bottom of the last row drops at the end of the list.
        let target_row = rows[2].rect.y + rows[2].rect.height;

        app.handle_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            source.x + 2,
            source.y,
        ));
        app.handle_mouse(mouse(
            MouseEventKind::Drag(MouseButton::Left),
            source.x + 2,
            target_row,
        ));
        assert!(matches!(
            app.state.drag.as_ref().map(|drag| &drag.target),
            Some(DragTarget::SidebarAgentReorder {
                ws_idx: 0,
                insert_idx: Some(3),
                ..
            })
        ));
        app.handle_mouse(mouse(
            MouseEventKind::Up(MouseButton::Left),
            source.x + 2,
            target_row,
        ));

        assert_eq!(
            agent_row_pane_ids(&app),
            vec![natural[1], natural[2], natural[0]]
        );
        // Display order only: tabs and each tab's layout are untouched.
        assert_eq!(
            app.state.workspaces[0]
                .tabs
                .iter()
                .map(|tab| tab.number)
                .collect::<Vec<_>>(),
            tab_numbers
        );
        assert_eq!(app.state.workspaces[0].natural_pane_order(), natural);
    }

    #[test]
    fn right_clicking_an_agent_row_renames_that_row_s_pane() {
        let mut app = app_with_agent_rows(&[("one", Agent::Pi), ("two", Agent::Claude)]);
        let rows = app.state.view.agent_row_areas.clone();
        // The second row lives in another tab, so the menu has to carry the
        // pane rather than lean on whatever was focused before.
        let target = &rows[1];

        app.handle_mouse(mouse(
            MouseEventKind::Down(MouseButton::Right),
            target.rect.x + 2,
            target.rect.y,
        ));

        assert_eq!(app.state.mode, Mode::ContextMenu);
        let menu = app.state.context_menu.as_ref().expect("agent context menu");
        match &menu.kind {
            ContextMenuKind::Agent { pane_id, .. } => assert_eq!(*pane_id, target.pane_id),
            other => panic!("expected agent menu, got {other:?}"),
        }
        assert_eq!(
            menu.items()[0],
            "Rename agent",
            "the first item still renames the row"
        );

        let menu_rect = app.state.context_menu_rect().expect("menu rect");
        app.handle_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            menu_rect.x + 2,
            menu_rect.y + 1,
        ));

        assert_eq!(app.state.mode, Mode::RenamePane);
        assert_eq!(app.state.rename_pane_target, Some(target.pane_id));
        assert!(app.state.context_menu.is_none());
    }

    #[test]
    fn agent_drop_slots_cover_the_gaps_between_rows() {
        let app = app_with_agent_rows(&[
            ("one", Agent::Pi),
            ("two", Agent::Claude),
            ("three", Agent::Codex),
        ]);
        let rows = app.state.view.agent_row_areas.clone();
        let dragged = rows[0].pane_id;
        let gap_above_second = rows[1].rect.y - 1;
        let above_everything = rows[0].rect.y.saturating_sub(4);
        let below_everything = rows[2].rect.y + rows[2].rect.height + 4;

        assert_eq!(
            app.state
                .agent_drop_index_at_row(0, dragged, gap_above_second),
            Some(1)
        );
        assert_eq!(
            app.state
                .agent_drop_index_at_row(0, dragged, above_everything),
            Some(0)
        );
        assert_eq!(
            app.state
                .agent_drop_index_at_row(0, dragged, below_everything),
            Some(3)
        );
    }

    #[test]
    fn clicking_agent_row_without_dragging_leaves_the_order_alone() {
        let mut app = app_with_agent_rows(&[("one", Agent::Pi), ("two", Agent::Claude)]);
        let natural = agent_row_pane_ids(&app);
        let source = app.state.view.agent_row_areas[0].rect;

        app.handle_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            source.x + 2,
            source.y,
        ));
        // A one-row wobble inside the same entry stays a click.
        app.handle_mouse(mouse(
            MouseEventKind::Drag(MouseButton::Left),
            source.x + 2,
            source.y + 1,
        ));
        app.handle_mouse(mouse(
            MouseEventKind::Up(MouseButton::Left),
            source.x + 2,
            source.y + 1,
        ));

        assert!(app.state.drag.is_none());
        assert!(app.state.workspaces[0].agent_order.is_empty());
        assert_eq!(agent_row_pane_ids(&app), natural);
    }

    #[test]
    fn reordered_agents_survive_a_session_snapshot() {
        let mut app = app_with_agent_rows(&[
            ("one", Agent::Pi),
            ("two", Agent::Claude),
            ("three", Agent::Codex),
        ]);
        let natural = agent_row_pane_ids(&app);
        assert!(app.state.move_agent_in_folder(0, natural[2], 0));

        let snapshot = capture_snapshot(&app.state);
        assert_eq!(snapshot.workspaces[0].agent_order, vec![2, 0, 1]);
    }

    #[test]
    fn closing_a_reordered_agents_pane_drops_it_from_the_order() {
        let mut app = app_with_agent_rows(&[
            ("one", Agent::Pi),
            ("two", Agent::Claude),
            ("three", Agent::Codex),
        ]);
        let natural = agent_row_pane_ids(&app);
        assert!(app.state.move_agent_in_folder(0, natural[2], 0));

        // Closing the tab that owns the moved pane takes it out of the space.
        app.state.workspaces[0].active_tab = 2;
        app.state.workspaces[0].close_active_tab();

        assert!(!app.state.workspaces[0]
            .ordered_pane_ids()
            .contains(&natural[2]));
        assert_eq!(
            agent_row_pane_ids(&app),
            vec![natural[0], natural[1]],
            "the surviving agents keep their natural order"
        );
    }

    fn temp_git_repo(branch: &str) -> std::path::PathBuf {
        let repo = unique_temp_path("sidebar-drop-slot-repo");
        fs::create_dir_all(repo.join(".git")).unwrap();
        fs::write(
            repo.join(".git/HEAD"),
            format!("ref: refs/heads/{branch}\n"),
        )
        .unwrap();
        repo
    }

    fn workspace_with_space(name: &str, key: &str) -> Workspace {
        let mut ws = Workspace::test_new(name);
        ws.worktree_space = Some(crate::workspace::WorktreeSpaceMembership {
            key: key.into(),
            label: "herdr".into(),
            repo_root: "/repo/herdr".into(),
            checkout_path: format!("/repo/{name}").into(),
            is_linked_worktree: name != "main",
        });
        ws
    }

    #[test]
    fn top_drop_slot_is_distinct_from_gap_below_first_workspace() {
        let mut app = app_for_mouse_test();
        let first_repo = temp_git_repo("main");
        let second_repo = temp_git_repo("main");

        let mut first = Workspace::test_new("a");
        let first_root = first.tabs[0].root_pane;
        first.identity_cwd = first_repo.clone();
        let _ = first.git_ahead_behind();

        let mut second = Workspace::test_new("b");
        let second_root = second.tabs[0].root_pane;
        second.identity_cwd = second_repo.clone();
        let _ = second.git_ahead_behind();

        app.state.workspaces = vec![first, second];
        app.state.ensure_test_terminals();
        let first_terminal_id = app.state.workspaces[0].tabs[0].panes[&first_root]
            .attached_terminal_id
            .clone();
        app.state.terminals.get_mut(&first_terminal_id).unwrap().cwd = first_repo.clone();
        let second_terminal_id = app.state.workspaces[1].tabs[0].panes[&second_root]
            .attached_terminal_id
            .clone();
        app.state
            .terminals
            .get_mut(&second_terminal_id)
            .unwrap()
            .cwd = second_repo.clone();
        crate::ui::compute_view(&mut app.state, Rect::new(0, 0, 106, 20));
        app.state.view.sidebar_rect = Rect::new(0, 0, 26, 20);
        app.state.view.workspace_card_areas =
            crate::ui::compute_workspace_card_areas(&app.state, app.state.view.sidebar_rect);

        assert_eq!(app.state.workspace_drop_index_at_row(0), Some(0));
        assert_eq!(app.state.workspace_drop_index_at_row(1), Some(0));
        assert_eq!(app.state.workspace_drop_index_at_row(2), Some(1));
        assert_eq!(app.state.workspace_drop_index_at_row(3), Some(1));
        assert_eq!(app.state.workspace_drop_index_at_row(4), Some(1));

        let _ = fs::remove_dir_all(first_repo);
        let _ = fs::remove_dir_all(second_repo);
    }

    #[test]
    fn bottom_drop_slot_stays_below_last_workspace_not_footer() {
        let mut app = app_for_mouse_test();
        app.state.workspaces = vec![
            Workspace::test_new("a"),
            Workspace::test_new("b"),
            Workspace::test_new("c"),
        ];
        crate::ui::compute_view(&mut app.state, Rect::new(0, 0, 106, 30));
        app.state.view.sidebar_rect = Rect::new(0, 0, 26, 30);
        app.state.view.workspace_card_areas =
            crate::ui::compute_workspace_card_areas(&app.state, app.state.view.sidebar_rect);

        // These spaces have no attached terminals, so the list is cards only and
        // the end of it is the row below the last card.
        let cards = &app.state.view.workspace_card_areas;
        let agent_rows = &app.state.view.agent_row_areas;
        assert!(agent_rows.is_empty());
        let bottom_slot = crate::ui::workspace_drop_indicator_row(
            cards,
            agent_rows,
            app.state.workspace_list_rect(),
            cards.len(),
        )
        .unwrap();

        let last = cards.last().unwrap().rect;
        assert_eq!(bottom_slot, last.y + last.height);
        assert!(bottom_slot < app.state.sidebar_footer_rect().y);
    }

    #[test]
    fn grouped_sidebar_drop_slots_do_not_land_inside_compact_group() {
        let mut app = app_for_mouse_test();
        app.state.workspaces = vec![
            workspace_with_space("main", "repo-key"),
            Workspace::test_new("normal"),
            workspace_with_space("issue", "repo-key"),
        ];
        app.state.active = Some(1);
        app.state.selected = 1;
        crate::ui::compute_view(&mut app.state, Rect::new(0, 0, 106, 40));
        app.state.view.sidebar_rect = Rect::new(0, 0, 26, 40);
        app.state.view.workspace_card_areas =
            crate::ui::compute_workspace_card_areas(&app.state, app.state.view.sidebar_rect);

        let cards = &app.state.view.workspace_card_areas;
        let order = cards.iter().map(|card| card.ws_idx).collect::<Vec<_>>();
        assert_eq!(order, vec![0, 2, 1]);
        let issue = cards.iter().find(|card| card.ws_idx == 2).unwrap();
        let normal = cards.iter().find(|card| card.ws_idx == 1).unwrap();

        assert_eq!(app.state.workspace_drop_index_at_row(issue.rect.y), Some(1));

        // Slot 2 is the end of the list, which sits below the last card and
        // below the agents listed under it.
        let agent_rows = &app.state.view.agent_row_areas;
        let end_slot = crate::ui::workspace_drop_indicator_row(
            cards,
            agent_rows,
            app.state.workspace_list_rect(),
            2,
        )
        .unwrap();
        assert!(end_slot >= normal.rect.y + normal.rect.height);
        for row in agent_rows.iter().filter(|row| row.ws_idx == normal.ws_idx) {
            assert!(end_slot >= row.rect.y + row.rect.height);
        }
    }

    #[test]
    fn dragging_worktree_space_member_does_not_reorder_workspaces() {
        let mut app = app_for_mouse_test();
        app.state.workspaces = vec![
            workspace_with_space("main", "repo-key"),
            Workspace::test_new("normal"),
            workspace_with_space("issue", "repo-key"),
        ];
        app.state.active = Some(0);
        app.state.selected = 0;
        crate::ui::compute_view(&mut app.state, Rect::new(0, 0, 106, 40));
        app.state.view.sidebar_rect = Rect::new(0, 0, 26, 40);
        app.state.view.workspace_card_areas =
            crate::ui::compute_workspace_card_areas(&app.state, app.state.view.sidebar_rect);

        let source = app
            .state
            .view
            .workspace_card_areas
            .iter()
            .find(|card| card.ws_idx == 2)
            .unwrap()
            .rect;
        let target_row = crate::ui::workspace_drop_indicator_row(
            &app.state.view.workspace_card_areas,
            &app.state.view.agent_row_areas,
            app.state.workspace_list_rect(),
            0,
        )
        .unwrap();

        app.handle_mouse(mouse(MouseEventKind::Down(MouseButton::Left), 2, source.y));
        app.handle_mouse(mouse(
            MouseEventKind::Drag(MouseButton::Left),
            2,
            target_row,
        ));
        assert!(app.state.drag.is_none());
        app.handle_mouse(mouse(MouseEventKind::Up(MouseButton::Left), 2, target_row));

        let names = app
            .state
            .workspaces
            .iter()
            .map(|ws| ws.display_name())
            .collect::<Vec<_>>();
        assert_eq!(names, vec!["main", "normal", "issue"]);
    }

    #[test]
    fn dragging_sidebar_divider_sets_manual_width() {
        let mut app = app_for_mouse_test();

        app.handle_mouse(mouse(MouseEventKind::Down(MouseButton::Left), 25, 5));
        app.handle_mouse(mouse(MouseEventKind::Drag(MouseButton::Left), 30, 5));

        assert_eq!(app.state.sidebar_width, app.state.default_sidebar_width);
    }

    #[test]
    fn dragging_removed_sidebar_bottom_divider_is_noop() {
        let mut app = app_for_mouse_test();

        app.handle_mouse(mouse(MouseEventKind::Down(MouseButton::Left), 25, 19));
        app.handle_mouse(mouse(MouseEventKind::Drag(MouseButton::Left), 30, 19));

        assert_eq!(app.state.sidebar_width, app.state.default_sidebar_width);
    }

    #[test]
    fn dragging_removed_sidebar_past_max_is_noop() {
        let mut app = app_for_mouse_test();
        app.state.sidebar_max_width = 30;

        app.handle_mouse(mouse(MouseEventKind::Down(MouseButton::Left), 25, 5));
        app.handle_mouse(mouse(MouseEventKind::Drag(MouseButton::Left), 50, 5));

        assert_eq!(app.state.sidebar_width, app.state.default_sidebar_width);
    }

    #[test]
    fn dragging_removed_sidebar_below_min_is_noop() {
        let mut app = app_for_mouse_test();
        app.state.sidebar_min_width = 22;

        app.handle_mouse(mouse(MouseEventKind::Down(MouseButton::Left), 25, 5));
        app.handle_mouse(mouse(MouseEventKind::Drag(MouseButton::Left), 5, 5));

        assert_eq!(app.state.sidebar_width, app.state.default_sidebar_width);
    }

    #[test]
    fn dragging_removed_sidebar_section_divider_is_noop() {
        let mut app = app_for_mouse_test();
        let original_split = app.state.sidebar_section_split;

        app.handle_mouse(mouse(MouseEventKind::Down(MouseButton::Left), 2, 10));
        app.handle_mouse(mouse(MouseEventKind::Drag(MouseButton::Left), 2, 14));

        assert_eq!(app.state.sidebar_section_split, original_split);
    }

    #[test]
    fn double_clicking_removed_sidebar_divider_is_noop() {
        let mut app = app_for_mouse_test();
        app.state.default_sidebar_width = 26;
        app.state.sidebar_width = 30;

        app.handle_mouse(mouse(MouseEventKind::Down(MouseButton::Left), 25, 5));
        app.handle_mouse(mouse(MouseEventKind::Up(MouseButton::Left), 25, 5));
        app.handle_mouse(mouse(MouseEventKind::Down(MouseButton::Left), 25, 5));

        assert_eq!(app.state.sidebar_width, 30);
        assert!(app.state.drag.is_none());
    }

    #[test]
    fn herdplayer_reuse_focuses_existing_named_pane() {
        let mut app = app_for_mouse_test();
        let mut workspace = Workspace::test_new("one");
        let root = workspace.tabs[0].root_pane;
        let named = workspace.test_split(ratatui::layout::Direction::Vertical);
        app.state.workspaces = vec![workspace];
        app.state.active = Some(0);
        app.state.ensure_test_terminals();
        let terminal_id = app.state.workspaces[0]
            .pane_state(named)
            .expect("named pane")
            .attached_terminal_id
            .clone();
        app.state
            .terminals
            .get_mut(&terminal_id)
            .expect("named terminal")
            .set_manual_label("herdplayer".into());
        assert!(app.state.focus_pane_in_workspace(0, root));
        let pane_count = app.state.workspaces[0].tabs[0].panes.len();

        app.state
            .open_or_focus_herdplayer(&mut app.terminal_runtimes);

        assert_eq!(
            app.state.workspaces[0].tabs[0].panes.len(),
            pane_count,
            "second open must not spawn another pane"
        );
        assert_eq!(app.state.workspaces[0].focused_pane_id(), Some(named));
        assert_eq!(app.state.find_herdplayer_pane(), Some((0, named)));
        let label = app
            .state
            .terminals
            .get(&terminal_id)
            .and_then(|t| t.manual_label.clone());
        assert_eq!(label.as_deref(), Some("herdplayer"));
    }
}
