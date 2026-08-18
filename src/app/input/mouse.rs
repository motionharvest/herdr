use bytes::Bytes;
use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::{Direction, Rect};
use tracing::warn;

use crate::{
    app::state::{
        AgentPressState, AgentTableFocus, AppState, ContextMenuKind, ContextMenuState, DragState,
        DragTarget, MenuListState, Mode, PanePressState, RightClickPassthroughGesture, ViewLayout,
    },
    layout::{PaneInfo, SplitBorder},
    selection::Selection,
    terminal::TerminalRuntimeRegistry,
};

#[cfg(test)]
use super::WheelRouting;
use super::{
    modal::{
        apply_context_menu_action, apply_global_menu_action, apply_rename_action,
        confirm_close_accept, confirm_close_cancel, global_menu_actions, leave_modal,
        modal_action_from_buttons, open_global_menu, ModalAction,
    },
    settings::SettingsAction,
    ScrollbarClickTarget, PANE_DRAG_THRESHOLD,
};

impl AppState {
    pub(crate) fn handle_pane_mouse_only(
        &mut self,
        terminal_runtimes: &TerminalRuntimeRegistry,
        mouse: MouseEvent,
    ) {
        if self.mode != Mode::Terminal {
            return;
        }
        let Some(info) = self.pane_at(mouse.column, mouse.row).cloned() else {
            return;
        };

        match mouse.kind {
            MouseEventKind::ScrollUp
            | MouseEventKind::ScrollDown
            | MouseEventKind::ScrollLeft
            | MouseEventKind::ScrollRight => {
                self.forward_pane_reported_wheel(terminal_runtimes, &info, mouse);
            }
            MouseEventKind::Down(_) | MouseEventKind::Up(_) | MouseEventKind::Drag(_) => {
                self.forward_pane_mouse_button(terminal_runtimes, &info, mouse);
            }
            MouseEventKind::Moved => {
                self.forward_pane_mouse_motion(terminal_runtimes, &info, mouse);
            }
        }
    }

    pub(super) fn handle_mouse(
        &mut self,
        terminal_runtimes: &mut TerminalRuntimeRegistry,
        mouse: MouseEvent,
    ) -> Option<SettingsAction> {
        if self.mode == Mode::Onboarding {
            self.handle_onboarding_mouse(mouse);
            return None;
        }

        if self.mode == Mode::Terminal
            && self.clickable_toast_at(mouse.column, mouse.row)
            && matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left))
        {
            self.focus_toast_target();
            return None;
        }

        if self.mode == Mode::Terminal
            && self.clickable_toast_at(mouse.column, mouse.row)
            && matches!(mouse.kind, MouseEventKind::Up(MouseButton::Left))
        {
            return None;
        }

        if self.mode == Mode::Settings {
            return self.handle_settings_mouse(mouse);
        }

        // The launcher shares the composer's caption row, so it gets first
        // claim on that small piece of chrome. This also lets it take focus
        // from a selected task field instead of the composer swallowing the
        // click as empty space in its band.
        let launcher_enabled = self.view.layout != ViewLayout::Mobile
            && matches!(
                self.mode,
                Mode::Terminal
                    | Mode::Navigate
                    | Mode::Resize
                    | Mode::Composer
                    | Mode::GlobalMenu
                    | Mode::KeybindHelp
            );
        let launcher = self.global_launcher_rect();
        let launcher_hit = launcher_enabled
            && mouse.column >= launcher.x
            && mouse.column < launcher.x + launcher.width
            && mouse.row >= launcher.y
            && mouse.row < launcher.y + launcher.height;

        if matches!(mouse.kind, MouseEventKind::Moved) && self.mode == Mode::GlobalMenu {
            let actions = global_menu_actions(self);
            let hovered = self
                .global_menu_item_at(mouse.column, mouse.row)
                .and_then(|action| actions.iter().position(|item| *item == action));
            self.global_menu.hover(hovered);
            return None;
        }

        if matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left)) && launcher_hit {
            if self.mode == Mode::GlobalMenu {
                leave_modal(self);
            } else {
                open_global_menu(self);
            }
            return None;
        }

        if self.mode == Mode::GlobalMenu {
            if matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left)) {
                if let Some(action) = self.global_menu_item_at(mouse.column, mouse.row) {
                    apply_global_menu_action(self, action);
                } else {
                    leave_modal(self);
                }
            }
            return None;
        }

        if matches!(mouse.kind, MouseEventKind::Moved) && self.composer.open.is_some() {
            self.composer.hover = self.composer_dropdown_item_at(mouse.column, mouse.row);
            if rect_contains(self.view.composer.dropdown, mouse.column, mouse.row) {
                return None;
            }
        }

        // An open dropdown's rows live inside its control's box, so they are
        // claimed before the band is: a click on a row is the row being taken,
        // not the box being pointed at.
        if matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left))
            && self.composer.open.is_some()
            && rect_contains(self.view.composer.dropdown, mouse.column, mouse.row)
        {
            self.click_composer_dropdown(mouse.row);
            return None;
        }

        if matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left))
            && matches!(self.mode, Mode::Terminal | Mode::Navigate | Mode::Composer)
            && rect_contains(self.view.composer.worktree, mouse.column, mouse.row)
        {
            self.composer.worktree = !self.composer.worktree;
            return None;
        }

        // The composer is chrome above every surface, so a click in it takes
        // focus from whatever had it, the same way clicking a pane does — and
        // it takes the keyboard to the control that was actually clicked.
        if matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left))
            && matches!(self.mode, Mode::Terminal | Mode::Navigate | Mode::Composer)
            && rect_contains(self.view.composer.area, mouse.column, mouse.row)
        {
            self.click_composer(terminal_runtimes, mouse.column, mouse.row);
            return None;
        }

        if self.mode == Mode::KeybindHelp {
            return None;
        }

        if self.view.layout == ViewLayout::Mobile && self.handle_mobile_mouse(mouse) {
            return None;
        }

        let in_table = self.in_agent_table(mouse.column, mouse.row);

        if self.handle_right_click_passthrough(terminal_runtimes, mouse, in_table) {
            return None;
        }

        if self.mode == Mode::OpenExistingWorktree {
            match mouse.kind {
                MouseEventKind::ScrollUp => {
                    if let Some(open) = &mut self.worktree_open {
                        open.select_previous_filtered();
                    }
                    return None;
                }
                MouseEventKind::ScrollDown => {
                    if let Some(open) = &mut self.worktree_open {
                        open.select_next_filtered();
                    }
                    return None;
                }
                _ => {}
            }
        }

        if matches!(
            self.mode,
            Mode::NewLinkedWorktree
                | Mode::OpenExistingWorktree
                | Mode::ConfirmRemoveWorktree
                | Mode::WorktreeLand
        ) && !matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left))
        {
            return None;
        }

        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                self.selection = None;
                self.selection_autoscroll = None;
                self.agent_press = None;
                self.agent_table_focus = None;

                if self.mode == Mode::ConfirmClose {
                    let popup = self.confirm_close_rect();
                    let inner = Rect::new(
                        popup.x + 1,
                        popup.y + 1,
                        popup.width.saturating_sub(2),
                        popup.height.saturating_sub(2),
                    );
                    let (confirm, cancel) = crate::ui::confirm_close_button_rects(inner);
                    match modal_action_from_buttons(
                        mouse.column,
                        mouse.row,
                        &[
                            (confirm, ModalAction::Confirm),
                            (cancel, ModalAction::Cancel),
                        ],
                    ) {
                        Some(ModalAction::Confirm) => confirm_close_accept(self),
                        Some(ModalAction::Cancel) | None => confirm_close_cancel(self),
                        _ => {}
                    }
                    return None;
                }

                if self.mode == Mode::ConfirmCloseAgent {
                    let popup = self.confirm_close_agent_rect();
                    let inner = Rect::new(
                        popup.x + 1,
                        popup.y + 1,
                        popup.width.saturating_sub(2),
                        popup.height.saturating_sub(2),
                    );
                    let (confirm, cancel) = crate::ui::confirm_close_agent_button_rects(inner);
                    match modal_action_from_buttons(
                        mouse.column,
                        mouse.row,
                        &[
                            (confirm, ModalAction::Confirm),
                            (cancel, ModalAction::Cancel),
                        ],
                    ) {
                        Some(ModalAction::Confirm) => {
                            super::confirm_close_agent_accept(self, terminal_runtimes)
                        }
                        Some(ModalAction::Cancel) | None => super::confirm_close_agent_cancel(self),
                        _ => {}
                    }
                    return None;
                }

                if self.mode == Mode::NewLinkedWorktree {
                    if let Some(inner) =
                        crate::ui::new_linked_worktree_inner_rect(self.screen_rect())
                    {
                        let (create, cancel) = crate::ui::new_linked_worktree_button_rects(inner);
                        match modal_action_from_buttons(
                            mouse.column,
                            mouse.row,
                            &[
                                (create, ModalAction::Confirm),
                                (cancel, ModalAction::Cancel),
                            ],
                        ) {
                            Some(ModalAction::Confirm) => {
                                self.request_submit_worktree_create = true;
                            }
                            Some(ModalAction::Cancel)
                                if !self
                                    .worktree_create
                                    .as_ref()
                                    .is_some_and(|create| create.creating) =>
                            {
                                self.worktree_create = None;
                                self.name_input.clear();
                                self.name_input_replace_on_type = false;
                                leave_modal(self);
                            }
                            _ => {}
                        }
                    }
                    return None;
                }

                if self.mode == Mode::OpenExistingWorktree {
                    if let Some(open) = self.worktree_open.as_ref() {
                        if let Some(inner) = crate::ui::open_existing_worktree_inner_rect(
                            self.screen_rect(),
                            open.entries.len(),
                        ) {
                            let filtered = open.filtered_indices();
                            let max_rows =
                                crate::ui::open_existing_worktree_max_visible_rows(inner);
                            let start =
                                crate::ui::open_existing_worktree_visible_start(open, max_rows);
                            if mouse.row == inner.y.saturating_add(1)
                                && mouse.column >= inner.x
                                && mouse.column < inner.x.saturating_add(inner.width)
                            {
                                if let Some(open) = &mut self.worktree_open {
                                    open.search_focused = true;
                                }
                                return None;
                            }
                            let row_idx = if rect_contains(inner, mouse.column, mouse.row) {
                                mouse
                                    .row
                                    .checked_sub(inner.y.saturating_add(3))
                                    .map(usize::from)
                                    .map(|row| row / 2)
                                    .filter(|row| *row < max_rows)
                                    .and_then(|row| filtered.get(start + row).copied())
                            } else {
                                None
                            };
                            if let Some(entry_idx) = row_idx {
                                if let Some(open) = &mut self.worktree_open {
                                    open.selected = entry_idx;
                                }
                                self.request_submit_worktree_open = true;
                                return None;
                            }

                            let (open_button, cancel) =
                                crate::ui::open_existing_worktree_button_rects(inner);
                            match modal_action_from_buttons(
                                mouse.column,
                                mouse.row,
                                &[
                                    (open_button, ModalAction::Confirm),
                                    (cancel, ModalAction::Cancel),
                                ],
                            ) {
                                Some(ModalAction::Confirm) => {
                                    self.request_submit_worktree_open = true;
                                }
                                Some(ModalAction::Cancel) => {
                                    self.worktree_open = None;
                                    leave_modal(self);
                                }
                                _ => {}
                            }
                        }
                    }
                    return None;
                }

                if self.mode == Mode::ConfirmRemoveWorktree {
                    if let Some(popup) = crate::ui::remove_worktree_popup_rect(self.screen_rect()) {
                        let inner = Rect::new(
                            popup.x + 1,
                            popup.y + 1,
                            popup.width.saturating_sub(2),
                            popup.height.saturating_sub(2),
                        );
                        let force_confirmation = self
                            .worktree_remove
                            .as_ref()
                            .is_some_and(|remove| remove.force_confirmation);
                        let (remove, cancel) =
                            crate::ui::remove_worktree_button_rects(inner, force_confirmation);
                        match modal_action_from_buttons(
                            mouse.column,
                            mouse.row,
                            &[
                                (remove, ModalAction::Confirm),
                                (cancel, ModalAction::Cancel),
                            ],
                        ) {
                            Some(ModalAction::Confirm) => {
                                self.request_submit_worktree_remove = true;
                            }
                            Some(ModalAction::Cancel)
                                if !self
                                    .worktree_remove
                                    .as_ref()
                                    .is_some_and(|remove| remove.removing) =>
                            {
                                self.worktree_remove = None;
                                leave_modal(self);
                            }
                            _ => {}
                        }
                    }
                    return None;
                }

                if self.mode == Mode::WorktreeLand {
                    if let Some(popup) = crate::ui::land_worktree_popup_rect(self.screen_rect()) {
                        let inner = Rect::new(
                            popup.x + 1,
                            popup.y + 1,
                            popup.width.saturating_sub(2),
                            popup.height.saturating_sub(2),
                        );
                        let close = crate::ui::land_worktree_close_rect(inner);
                        if modal_action_from_buttons(
                            mouse.column,
                            mouse.row,
                            &[(close, ModalAction::Cancel)],
                        ) == Some(ModalAction::Cancel)
                            && !self.worktree_land.as_ref().is_some_and(|land| land.landing)
                        {
                            self.worktree_land = None;
                            leave_modal(self);
                        }
                    }
                    return None;
                }

                if matches!(self.mode, Mode::RenameWorkspace | Mode::RenamePane) {
                    let action = self
                        .rename_modal_inner()
                        .map(crate::ui::rename_button_rects)
                        .and_then(|(save, clear, cancel)| {
                            modal_action_from_buttons(
                                mouse.column,
                                mouse.row,
                                &[
                                    (save, ModalAction::Save),
                                    (clear, ModalAction::Clear),
                                    (cancel, ModalAction::Cancel),
                                ],
                            )
                        })
                        .unwrap_or(ModalAction::Cancel);
                    apply_rename_action(self, action);
                    return None;
                }

                if self.mode == Mode::ContextMenu {
                    let item_idx = self.context_menu_item_at(mouse.column, mouse.row);
                    if let Some(menu) = self.context_menu.take() {
                        if let Some(idx) = item_idx {
                            apply_context_menu_action(self, terminal_runtimes, menu, idx);
                        } else {
                            leave_modal(self);
                        }
                    }
                    return None;
                }

                if !in_table {
                    if let Some(control) = self.pane_chrome_control_at(mouse.column, mouse.row) {
                        self.focus_pane(control.pane_id);
                        match control.action {
                            crate::app::state::PaneChromeAction::Focus => self.toggle_zoom(),
                            crate::app::state::PaneChromeAction::Close => {
                                self.close_pane();
                            }
                        }
                        return None;
                    }

                    if let Some(hit) = self
                        .pane_title_hit_at(mouse.column, mouse.row)
                        .filter(|_| self.pane_swap_enabled())
                    {
                        self.focus_pane(hit.pane_id);
                        self.pane_press = Some(PanePressState {
                            pane_id: hit.pane_id,
                            start_col: mouse.column,
                            start_row: mouse.row,
                        });
                        return None;
                    }

                    if let Some(border) = self.find_border_at(mouse.column, mouse.row) {
                        self.drag = Some(DragState {
                            target: DragTarget::PaneSplit {
                                path: border.path.clone(),
                                direction: border.direction,
                                area: border.area,
                            },
                        });
                        return None;
                    }

                    if let Some((pane_id, target)) =
                        self.scrollbar_target_at(terminal_runtimes, mouse.column, mouse.row)
                    {
                        self.focus_pane(pane_id);
                        match target {
                            ScrollbarClickTarget::Thumb { grab_row_offset } => {
                                self.drag = Some(DragState {
                                    target: DragTarget::PaneScrollbar {
                                        pane_id,
                                        grab_row_offset,
                                    },
                                });
                            }
                            ScrollbarClickTarget::Track { offset_from_bottom } => {
                                self.set_pane_scroll_offset(
                                    terminal_runtimes,
                                    pane_id,
                                    offset_from_bottom,
                                );
                            }
                        }
                        if self.mode != Mode::Terminal {
                            self.mode = Mode::Terminal;
                        }
                        return None;
                    }
                }

                if in_table {
                    // The done marker is a button before it is part of a row:
                    // clicking it takes the marker off and nothing else, so the
                    // click that acknowledges an agent is not also the click
                    // that jumps to it.
                    if let Some(hit) = self.agent_marker_target_at(mouse.column, mouse.row) {
                        if self.acknowledge_agent_completion(hit.pane_id) {
                            return None;
                        }
                    }
                    if let Some(hit) = self.agent_table_target_at(mouse.column, mouse.row) {
                        // Hold the press so a drag can carry the agent; a
                        // plain click on a docked row still focuses its pane
                        // right away.
                        self.agent_press = Some(AgentPressState {
                            docked: hit.docked,
                            pane_id: hit.pane_id,
                            start_col: mouse.column,
                            start_row: mouse.row,
                        });
                        // The row now holds the keyboard for one key, which is
                        // what makes delete mean this agent.
                        self.agent_table_focus = Some(AgentTableFocus {
                            docked: hit.docked,
                            pane_id: hit.pane_id,
                        });
                        if hit.docked {
                            self.focus_pane_in_workspace(hit.ws_idx, hit.pane_id);
                            self.mode = Mode::Terminal;
                        }
                    }
                    return None;
                } else if let Some(info) = self.pane_at(mouse.column, mouse.row).cloned() {
                    self.focus_pane(info.id);
                    if self.mode != Mode::Terminal {
                        self.mode = Mode::Terminal;
                    }

                    if self.forward_pane_mouse_button(terminal_runtimes, &info, mouse) {
                        self.selection = None;
                        self.selection_autoscroll = None;
                        return None;
                    }

                    let (row, col) = (
                        mouse.row - info.inner_rect.y,
                        mouse.column - info.inner_rect.x,
                    );
                    self.selection = Some(Selection::anchor(
                        info.id,
                        row,
                        col,
                        self.pane_scroll_metrics(terminal_runtimes, info.id),
                    ));
                } else if let Some(info) = self.view.pane_infos.iter().find(|p| {
                    mouse.column >= p.rect.x
                        && mouse.column < p.rect.x + p.rect.width
                        && mouse.row >= p.rect.y
                        && mouse.row < p.rect.y + p.rect.height
                }) {
                    let id = info.id;
                    self.focus_pane(id);
                    if self.mode != Mode::Terminal {
                        self.mode = Mode::Terminal;
                    }
                }
            }

            MouseEventKind::Drag(MouseButton::Left) => {
                if self.selection.is_some() {
                    self.update_selection_drag(terminal_runtimes, mouse.column, mouse.row);
                    return None;
                }

                if self.drag.is_none() && !self.chrome_press_active() {
                    if let Some(info) = self.pane_mouse_target(mouse.column, mouse.row).cloned() {
                        if self.forward_pane_mouse_button(terminal_runtimes, &info, mouse) {
                            self.selection = None;
                            self.selection_autoscroll = None;
                            return None;
                        }
                    }
                }

                if self.drag.is_none() {
                    if let Some(press) = &self.pane_press {
                        let delta_col = mouse.column.abs_diff(press.start_col);
                        let delta_row = mouse.row.abs_diff(press.start_row);
                        if self.pane_swap_enabled()
                            && delta_col.max(delta_row) >= PANE_DRAG_THRESHOLD
                        {
                            self.drag = Some(DragState {
                                target: DragTarget::PaneSwap {
                                    source_pane_id: press.pane_id,
                                    hovered_pane_id: None,
                                    drop_zone: crate::layout::DropZone::Over,
                                    moved: false,
                                },
                            });
                        }
                    } else if let Some(press) = &self.agent_press {
                        let delta_col = mouse.column.abs_diff(press.start_col);
                        let delta_row = mouse.row.abs_diff(press.start_row);
                        if delta_col.max(delta_row) >= PANE_DRAG_THRESHOLD {
                            self.drag = Some(DragState {
                                target: DragTarget::AgentReorder {
                                    source_pane_id: press.pane_id,
                                    insert_idx: self.agent_drop_index_at(
                                        mouse.column,
                                        mouse.row,
                                        press.pane_id,
                                    ),
                                },
                            });
                        }
                    }
                }

                // A table-row drag rearranges rows, never pane geometry. A
                // set-down agent may still be carried out of the table to dock
                // on a pane, preserving that distinct interaction.
                if let Some(press) = self.agent_press.as_ref() {
                    let pane_id = press.pane_id;
                    let docked = press.docked;
                    if in_table {
                        let insert_idx = self.agent_drop_index_at(mouse.column, mouse.row, pane_id);
                        if matches!(
                            self.drag.as_ref().map(|drag| &drag.target),
                            Some(DragTarget::AgentReorder { .. })
                                | Some(DragTarget::AgentDock { .. })
                        ) {
                            self.drag = Some(DragState {
                                target: DragTarget::AgentReorder {
                                    source_pane_id: pane_id,
                                    insert_idx,
                                },
                            });
                        }
                    } else if !docked
                        && matches!(
                            self.drag.as_ref().map(|drag| &drag.target),
                            Some(DragTarget::AgentReorder { .. })
                        )
                    {
                        self.drag = Some(DragState {
                            target: DragTarget::AgentDock {
                                pane_id,
                                hovered_pane_id: None,
                                drop_zone: crate::layout::DropZone::Over,
                            },
                        });
                    } else if let Some(DragState {
                        target: DragTarget::AgentReorder { insert_idx, .. },
                    }) = &mut self.drag
                    {
                        *insert_idx = None;
                    }
                }

                let agent_dock_source = self.drag.as_ref().and_then(|drag| {
                    let DragTarget::AgentDock { pane_id, .. } = &drag.target else {
                        return None;
                    };
                    Some(*pane_id)
                });
                if let Some(source_pane_id) = agent_dock_source {
                    let hovered =
                        self.pane_swap_hover_target(mouse.column, mouse.row, source_pane_id);
                    let zone = self.pane_drop_zone(hovered, mouse.column, mouse.row);
                    if let Some(DragState {
                        target:
                            DragTarget::AgentDock {
                                hovered_pane_id,
                                drop_zone,
                                ..
                            },
                    }) = &mut self.drag
                    {
                        *hovered_pane_id = hovered;
                        *drop_zone = zone;
                    }
                }

                let pane_swap_source = self.drag.as_ref().and_then(|drag| {
                    let DragTarget::PaneSwap { source_pane_id, .. } = &drag.target else {
                        return None;
                    };
                    Some(*source_pane_id)
                });
                if let Some(source_pane_id) = pane_swap_source {
                    let hovered =
                        self.pane_swap_hover_target(mouse.column, mouse.row, source_pane_id);
                    let zone = self.pane_drop_zone(hovered, mouse.column, mouse.row);
                    if let Some(DragState {
                        target:
                            DragTarget::PaneSwap {
                                hovered_pane_id,
                                drop_zone,
                                moved,
                                ..
                            },
                    }) = &mut self.drag
                    {
                        *moved = true;
                        *hovered_pane_id = hovered;
                        *drop_zone = zone;
                    }
                }

                if let Some(drag) = &self.drag {
                    match &drag.target {
                        DragTarget::PaneSwap { .. }
                        | DragTarget::AgentDock { .. }
                        | DragTarget::AgentReorder { .. } => {}
                        DragTarget::PaneSplit {
                            path,
                            direction,
                            area,
                        } => {
                            let ratio = match direction {
                                Direction::Horizontal => {
                                    (mouse.column.saturating_sub(area.x)) as f32
                                        / area.width.max(1) as f32
                                }
                                Direction::Vertical => {
                                    (mouse.row.saturating_sub(area.y)) as f32
                                        / area.height.max(1) as f32
                                }
                            };
                            let ratio = ratio.clamp(0.1, 0.9);
                            let path = path.clone();
                            if let Some(ws) = self.active.and_then(|i| self.workspaces.get_mut(i)) {
                                ws.layout.set_ratio_at(&path, ratio);
                                self.mark_session_dirty();
                            }
                        }
                        DragTarget::PaneScrollbar {
                            pane_id,
                            grab_row_offset,
                        } => {
                            if let Some(offset_from_bottom) = self.scrollbar_offset_for_pane_row(
                                terminal_runtimes,
                                *pane_id,
                                mouse.row,
                                *grab_row_offset,
                            ) {
                                self.set_pane_scroll_offset(
                                    terminal_runtimes,
                                    *pane_id,
                                    offset_from_bottom,
                                );
                            }
                        }
                        DragTarget::ReleaseNotesScrollbar { .. }
                        | DragTarget::ProductAnnouncementScrollbar { .. }
                        | DragTarget::KeybindHelpScrollbar { .. } => {}
                    }
                }
            }

            MouseEventKind::Up(MouseButton::Left) => {
                // Mouse-up either finishes a drag selection or releases after a
                // double-click copy; the latter is already copied.
                if let Some(selection) = self.selection.as_ref() {
                    let was_click = selection.was_just_click();
                    let was_already_copied = selection.is_done();

                    self.pane_press = None;
                    self.drag = None;
                    self.selection_autoscroll = None;
                    if was_click {
                        self.selection = None;
                    } else if !was_already_copied {
                        self.copy_selection(terminal_runtimes);
                    }
                    return None;
                }

                if self.drag.is_none() && !self.chrome_press_active() {
                    if let Some(info) = self.pane_mouse_target(mouse.column, mouse.row).cloned() {
                        if self.forward_pane_mouse_button(terminal_runtimes, &info, mouse) {
                            self.selection = None;
                            self.selection_autoscroll = None;
                            self.pane_press = None;
                            self.drag = None;
                            return None;
                        }
                    }
                }

                let pane_press = self.pane_press.take();
                let agent_press = self.agent_press.take();
                let drag = self.drag.take();
                let dragged_agent = matches!(
                    drag.as_ref().map(|drag| &drag.target),
                    Some(DragTarget::AgentReorder { .. }) | Some(DragTarget::AgentDock { .. })
                );
                match drag {
                    Some(DragState {
                        target:
                            DragTarget::AgentReorder {
                                source_pane_id,
                                insert_idx: Some(insert_idx),
                            },
                    }) => {
                        self.move_agent_to_index(source_pane_id, insert_idx);
                    }
                    Some(DragState {
                        target:
                            DragTarget::AgentDock {
                                pane_id,
                                hovered_pane_id: Some(target_pane_id),
                                drop_zone,
                            },
                    }) => {
                        self.dock_detached_agent(pane_id, target_pane_id, drop_zone);
                    }
                    Some(DragState {
                        target:
                            DragTarget::PaneSwap {
                                source_pane_id,
                                hovered_pane_id: Some(target_pane_id),
                                drop_zone,
                                moved: true,
                            },
                    }) => {
                        self.drop_pane_on_pane(source_pane_id, target_pane_id, drop_zone);
                        self.mode = Mode::Terminal;
                    }
                    Some(DragState {
                        target: DragTarget::PaneSwap { source_pane_id, .. },
                    }) => {
                        self.focus_pane(source_pane_id);
                        self.mode = Mode::Terminal;
                    }
                    Some(_) => {}
                    None => {
                        if let Some(AgentPressState {
                            docked: false,
                            pane_id,
                            ..
                        }) = agent_press.filter(|_| !dragged_agent)
                        {
                            self.open_detached_agent(pane_id);
                        }
                        if let Some(press) = pane_press {
                            self.focus_pane(press.pane_id);
                            self.mode = Mode::Terminal;
                            return None;
                        }
                    }
                }
            }

            MouseEventKind::Up(MouseButton::Middle) | MouseEventKind::Drag(MouseButton::Middle)
                if !in_table =>
            {
                if let Some(info) = self.pane_mouse_target(mouse.column, mouse.row).cloned() {
                    let _ = self.forward_pane_mouse_button(terminal_runtimes, &info, mouse);
                }
            }

            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown
                if !in_table && self.scroll_selection_with_wheel(terminal_runtimes, mouse) => {}

            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown if !in_table => {
                self.selection = None;
                self.selection_autoscroll = None;
                self.handle_terminal_wheel(terminal_runtimes, mouse);
            }

            MouseEventKind::ScrollUp if in_table => self.scroll_agent_table(-1),
            MouseEventKind::ScrollDown if in_table => self.scroll_agent_table(1),

            MouseEventKind::Moved if self.mode == Mode::ContextMenu => {
                let hovered = self.context_menu_item_at(mouse.column, mouse.row);
                if let Some(menu) = &mut self.context_menu {
                    menu.list.hover(hovered);
                }
            }

            MouseEventKind::Moved if self.mode == Mode::Terminal && !in_table => {
                if let Some(info) = self.pane_at(mouse.column, mouse.row).cloned() {
                    let _ = self.forward_pane_mouse_motion(terminal_runtimes, &info, mouse);
                }
            }

            MouseEventKind::Down(MouseButton::Right) if in_table => {
                self.pane_press = None;
                self.agent_press = None;
                self.agent_table_focus = None;
                if let Some(hit) = self.agent_table_target_at(mouse.column, mouse.row) {
                    let kind = if hit.docked {
                        // Focus the row's agent first, the way right-clicking
                        // the pane itself does, so the menu acts on what was
                        // clicked.
                        self.focus_pane_in_workspace(hit.ws_idx, hit.pane_id);
                        self.agent_menu_kind(terminal_runtimes, hit.ws_idx, hit.pane_id)
                    } else {
                        ContextMenuKind::DetachedAgent {
                            pane_id: hit.pane_id,
                        }
                    };
                    self.context_menu = Some(ContextMenuState {
                        kind,
                        x: mouse.column,
                        y: mouse.row,
                        list: MenuListState::new(0),
                    });
                    self.mode = Mode::ContextMenu;
                }
            }

            MouseEventKind::Down(MouseButton::Right) if !in_table => {
                if let Some(info) = self.pane_mouse_target(mouse.column, mouse.row).cloned() {
                    self.focus_pane(info.id);
                    let pane = self
                        .active
                        .and_then(|ws_idx| self.workspaces.get(ws_idx))
                        .and_then(|ws| ws.pane_state(info.id));
                    let dimmed = pane.is_some_and(|pane| pane.dimmed);
                    let terminal =
                        pane.and_then(|pane| self.terminals.get(&pane.attached_terminal_id));
                    let has_manual_label = terminal
                        .and_then(|terminal| terminal.manual_label.as_ref())
                        .is_some();
                    let has_agent = terminal.is_some_and(|terminal| terminal.is_agent_terminal());
                    let can_reset = self.agent_reset_command_for_pane(info.id).is_some();
                    self.context_menu = Some(ContextMenuState {
                        kind: ContextMenuKind::Pane {
                            pane_id: info.id,
                            has_manual_label,
                            dimmed,
                            has_agent,
                            can_reset,
                        },
                        x: mouse.column,
                        y: mouse.row,
                        list: MenuListState::new(0),
                    });
                    self.mode = Mode::ContextMenu;
                }
            }

            _ => {}
        }

        None
    }

    fn handle_mobile_mouse(&mut self, mouse: MouseEvent) -> bool {
        if self.mode == Mode::Navigate {
            match mouse.kind {
                MouseEventKind::ScrollUp => {
                    self.scroll_mobile_switcher_at(mouse.column, mouse.row, -1);
                    return true;
                }
                MouseEventKind::ScrollDown => {
                    self.scroll_mobile_switcher_at(mouse.column, mouse.row, 1);
                    return true;
                }
                MouseEventKind::Down(MouseButton::Left) => {}
                _ => return true,
            }
        } else if !matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left)) {
            return false;
        }

        if self.mode != Mode::Navigate {
            if !matches!(self.mode, Mode::Terminal | Mode::Resize) {
                return false;
            }
            if rect_contains(self.view.mobile_menu_hit_area, mouse.column, mouse.row) {
                self.mobile_switcher_scroll = 0;
                self.mode = Mode::Navigate;
                return true;
            }
            return false;
        }

        let areas = crate::ui::mobile_switcher_areas(self);
        if rect_contains(areas.close, mouse.column, mouse.row) {
            self.mode = Mode::Terminal;
            return true;
        }

        match crate::ui::mobile_switcher_target_at(self, mouse.column, mouse.row) {
            Some(crate::ui::MobileSwitcherTarget::NewWorkspace) => {
                self.request_new_workspace = true;
            }
            Some(crate::ui::MobileSwitcherTarget::Workspace(ws_idx)) => {
                self.switch_workspace(ws_idx);
                self.mode = Mode::Terminal;
            }
            Some(crate::ui::MobileSwitcherTarget::Agent {
                ws_idx,
                tab_idx: _,
                pane_id,
            }) => {
                self.focus_pane_in_workspace(ws_idx, pane_id);
                self.mode = Mode::Terminal;
            }
            Some(crate::ui::MobileSwitcherTarget::AcknowledgeAgent { pane_id }) => {
                self.acknowledge_agent_completion(pane_id);
            }
            Some(crate::ui::MobileSwitcherTarget::Menu(action_idx)) => {
                let actions = global_menu_actions(self);
                if let Some(action) = actions.get(action_idx).copied() {
                    apply_global_menu_action(self, action);
                }
            }
            None => {}
        }

        true
    }

    fn scroll_mobile_switcher_at(&mut self, _col: u16, _row: u16, delta: i16) {
        let max_scroll = crate::ui::mobile_switcher_max_scroll(self);
        apply_scroll(
            &mut self.mobile_switcher_scroll,
            delta.saturating_mul(2),
            max_scroll,
        );
    }

    /// The whole frame, worked back out of the surfaces laid out on it. Modals
    /// centre themselves in this rather than in the pane area, so a dialog does
    /// not shift as the table above the panes grows and shrinks.
    pub(super) fn screen_rect(&self) -> Rect {
        let table = self.view.agent_table.area;
        let composer = self.view.composer.area;
        let terminal = self.view.terminal_area;
        let bands = [composer, table, terminal];
        let placed: Vec<Rect> = bands
            .into_iter()
            .filter(|rect| rect.width > 0 && rect.height > 0)
            .collect();
        if placed.is_empty() {
            return Rect::default();
        }
        let x = placed.iter().map(|rect| rect.x).min().unwrap_or(0);
        let y = placed.iter().map(|rect| rect.y).min().unwrap_or(0);
        let right = placed
            .iter()
            .map(|rect| rect.x + rect.width)
            .max()
            .unwrap_or(0);
        let bottom = placed
            .iter()
            .map(|rect| rect.y + rect.height)
            .max()
            .unwrap_or(0);
        Rect::new(x, y, right.saturating_sub(x), bottom.saturating_sub(y))
    }

    pub(crate) fn context_menu_rect(&self) -> Option<Rect> {
        let menu = self.context_menu.as_ref()?;
        let screen = self.screen_rect();
        let max_item_w = menu
            .items()
            .iter()
            .map(|item| item.len() as u16)
            .max()
            .unwrap_or(0);
        let menu_w = (max_item_w + 4).max(14).min(screen.width.max(1));
        let menu_h = (menu.items().len() as u16 + 2).min(screen.height.max(1));
        let x = menu.x.min(screen.x + screen.width.saturating_sub(menu_w));
        let y = menu.y.min(screen.y + screen.height.saturating_sub(menu_h));
        Some(Rect::new(x, y, menu_w, menu_h))
    }

    pub(crate) fn confirm_close_rect(&self) -> Rect {
        crate::ui::confirm_close_popup_rect(self.view.terminal_area).unwrap_or_default()
    }

    pub(crate) fn confirm_close_agent_rect(&self) -> Rect {
        crate::ui::confirm_close_agent_popup_rect(self.view.terminal_area).unwrap_or_default()
    }

    fn context_menu_item_at(&self, col: u16, row: u16) -> Option<usize> {
        let menu_rect = self.context_menu_rect()?;
        let inner_x = menu_rect.x + 1;
        let inner_y = menu_rect.y + 1;
        let inner_w = menu_rect.width.saturating_sub(2);
        let inner_h = menu_rect.height.saturating_sub(2);
        let item_count = self
            .context_menu
            .as_ref()
            .map(|menu| menu.items().len() as u16)
            .unwrap_or(0);
        if col >= inner_x
            && col < inner_x + inner_w
            && row >= inner_y
            && row < inner_y + inner_h.min(item_count)
        {
            Some((row - inner_y) as usize)
        } else {
            None
        }
    }

    pub(super) fn find_border_at(&self, col: u16, row: u16) -> Option<&SplitBorder> {
        if self.pane_title_hit_at(col, row).is_some() {
            return None;
        }

        self.view.split_borders.iter().find(|b| match b.direction {
            Direction::Horizontal => {
                col >= b.pos.saturating_sub(1)
                    && col <= b.pos
                    && row >= b.area.y
                    && row < b.area.y + b.area.height
            }
            Direction::Vertical => {
                row >= b.pos.saturating_sub(1)
                    && row <= b.pos
                    && col >= b.area.x
                    && col < b.area.x + b.area.width
            }
        })
    }

    pub(super) fn pane_at(&self, col: u16, row: u16) -> Option<&PaneInfo> {
        self.view.pane_infos.iter().find(|p| {
            col >= p.inner_rect.x
                && col < p.inner_rect.x + p.inner_rect.width
                && row >= p.inner_rect.y
                && row < p.inner_rect.y + p.inner_rect.height
        })
    }

    pub(super) fn pane_mouse_target(&self, col: u16, row: u16) -> Option<&PaneInfo> {
        self.pane_at(col, row)
            .or_else(|| self.pane_frame_at(col, row))
    }

    pub(crate) fn pane_info_by_id(&self, pane_id: crate::layout::PaneId) -> Option<&PaneInfo> {
        self.view.pane_infos.iter().find(|info| info.id == pane_id)
    }

    fn pane_chrome_control_at(
        &self,
        col: u16,
        row: u16,
    ) -> Option<crate::app::state::PaneChromeControl> {
        self.view
            .pane_chrome_controls
            .iter()
            .copied()
            .find(|control| rect_contains(control.rect, col, row))
    }

    fn pane_title_hit_at(&self, col: u16, row: u16) -> Option<crate::app::state::PaneTitleHitArea> {
        self.view
            .pane_title_hit_areas
            .iter()
            .copied()
            .find(|hit| rect_contains(hit.rect, col, row))
    }

    /// A press that landed on herdr's own chrome owns the gesture until
    /// release: the motion after it belongs to the drag it may still become,
    /// never to whatever pane sits under the cursor.
    fn chrome_press_active(&self) -> bool {
        self.pane_press.is_some() || self.agent_press.is_some()
    }

    fn pane_swap_enabled(&self) -> bool {
        let Some(ws_idx) = self.active else {
            return false;
        };
        let Some(ws) = self.workspaces.get(ws_idx) else {
            return false;
        };
        let Some(tab) = ws.active_tab() else {
            return false;
        };
        !tab.zoomed && tab.layout.pane_count() > 1
    }

    fn pane_swap_hover_target(
        &self,
        col: u16,
        row: u16,
        source_pane_id: crate::layout::PaneId,
    ) -> Option<crate::layout::PaneId> {
        self.pane_frame_at(col, row)
            .map(|pane| pane.id)
            .filter(|pane_id| *pane_id != source_pane_id)
    }

    /// A click outside the open folder box, while a path is being typed, settles
    /// that path the same way Enter does. Returns the reason the path could not
    /// be settled, which is the click being refused so the field stays as it is.
    pub(super) fn commit_typed_folder_on_away_click(
        &mut self,
        col: u16,
        row: u16,
    ) -> Option<String> {
        if self.composer.open != Some(crate::composer::Focus::Folder) {
            return None;
        }
        if self.composer.path().is_empty() {
            return None;
        }
        if rect_contains(self.view.composer.folder, col, row) {
            return None;
        }
        match self.composer.take_folder() {
            Ok(()) => None,
            Err(err) => Some(err.message()),
        }
    }

    /// A click in the band takes the keyboard to the control it landed on, and
    /// a click on the folder or the agent opens its list there and then — the
    /// click is the asking to see the choices. A click on a control whose list
    /// is already open leaves it as it is. A click in the task puts the cursor
    /// on the character clicked, the way any text field would.
    fn click_composer(&mut self, terminal_runtimes: &TerminalRuntimeRegistry, col: u16, row: u16) {
        let was = self.mode;
        self.mode = Mode::Composer;
        let layout = &self.view.composer;
        let which = if rect_contains(layout.folder, col, row) {
            Some(crate::composer::Focus::Folder)
        } else if rect_contains(layout.agent, col, row) {
            Some(crate::composer::Focus::Agent)
        } else if rect_contains(layout.task, col, row) {
            Some(crate::composer::Focus::Task)
        } else {
            None
        };
        let Some(which) = which else {
            self.composer.close_dropdown();
            if was != Mode::Composer {
                self.composer.start_where_the_work_is();
            }
            return;
        };
        if self.composer.open == Some(which) {
            return;
        }
        self.composer.close_dropdown();
        self.composer.focus = which;
        if which == crate::composer::Focus::Task {
            let task = self.view.composer.task;
            let lead = self.view.composer.task_lead;
            self.composer.task.click(
                row.saturating_sub(task.y + 1) as usize,
                col.saturating_sub(task.x + lead) as usize,
            );
            return;
        }
        if which == crate::composer::Focus::Folder {
            self.refresh_composer_folders(terminal_runtimes);
        }
        self.composer.open_dropdown(which);
    }

    fn click_composer_dropdown(&mut self, row: u16) {
        let Some(index) = self.composer_dropdown_item_at(self.view.composer.dropdown.x, row) else {
            return;
        };
        self.composer.point_at(index);
        self.composer.take_pointed();
    }

    fn composer_dropdown_item_at(&self, col: u16, row: u16) -> Option<usize> {
        if !rect_contains(self.view.composer.dropdown, col, row) {
            return None;
        }
        let offset = self
            .view
            .composer
            .dropdown_rows
            .iter()
            .position(|dropdown_row| *dropdown_row == row)?;
        // The rows drawn are a window onto the list, so the row clicked is the
        // one the window starts at plus how far down it was.
        let rows = self.view.composer.dropdown_rows.len();
        let count = self.composer.item_count();
        let first = self
            .composer
            .highlight
            .saturating_sub(rows.saturating_sub(1))
            .min(count.saturating_sub(rows));
        Some((first + offset).min(count.saturating_sub(1)))
    }

    /// Put one pane where the drag ended: over another pane, which trades their
    /// places, or against one of its edges, which cuts that pane in two.
    fn drop_pane_on_pane(
        &mut self,
        source: crate::layout::PaneId,
        target: crate::layout::PaneId,
        zone: crate::layout::DropZone,
    ) {
        match zone {
            crate::layout::DropZone::Over => {
                self.swap_panes(source, target);
            }
            crate::layout::DropZone::Edge(side) => {
                self.pin_pane_to_edge(source, target, side);
            }
        }
    }

    /// Where on the hovered pane the pointer is. With nothing hovered there is
    /// nothing to cut, so the answer is the middle: a drop that lands nowhere
    /// should not be remembered as a drop against an edge.
    fn pane_drop_zone(
        &self,
        hovered: Option<crate::layout::PaneId>,
        col: u16,
        row: u16,
    ) -> crate::layout::DropZone {
        hovered
            .and_then(|pane_id| self.view.pane_infos.iter().find(|p| p.id == pane_id))
            .map_or(crate::layout::DropZone::Over, |pane| {
                crate::layout::drop_zone_at(pane.rect, col, row)
            })
    }

    pub(super) fn pane_frame_at(&self, col: u16, row: u16) -> Option<&PaneInfo> {
        self.view.pane_infos.iter().find(|p| {
            col >= p.rect.x
                && col < p.rect.x + p.rect.width
                && row >= p.rect.y
                && row < p.rect.y + p.rect.height
        })
    }

    pub(super) fn focus_pane(&mut self, pane_id: crate::layout::PaneId) {
        if let Some(ws_idx) = self.active {
            self.focus_pane_in_workspace(ws_idx, pane_id);
        }
    }

    fn clickable_toast_at(&self, col: u16, row: u16) -> bool {
        self.toast
            .as_ref()
            .is_some_and(|toast| toast.target.is_some())
            && rect_contains(self.view.toast_hit_area, col, row)
    }

    pub(crate) fn focus_toast_target(&mut self) {
        let Some(target) = self.toast.as_ref().and_then(|toast| toast.target.clone()) else {
            return;
        };
        let Some(ws_idx) = self
            .workspaces
            .iter()
            .position(|workspace| workspace.id == target.workspace_id)
        else {
            return;
        };
        let Some(_tab_idx) = self.workspaces[ws_idx].find_tab_index_for_pane(target.pane_id) else {
            return;
        };

        self.focus_pane_in_workspace(ws_idx, target.pane_id);
        self.toast = None;
        self.mode = Mode::Terminal;
    }

    pub(crate) fn scroll_pane_up(
        &self,
        terminal_runtimes: &TerminalRuntimeRegistry,
        pane_id: crate::layout::PaneId,
        lines: usize,
    ) {
        if let Some(ws_idx) = self.active {
            if let Some(rt) = self.runtime_for_pane_in_workspace(terminal_runtimes, ws_idx, pane_id)
            {
                rt.scroll_up(lines);
            }
        }
    }

    pub(crate) fn scroll_pane_down(
        &self,
        terminal_runtimes: &TerminalRuntimeRegistry,
        pane_id: crate::layout::PaneId,
        lines: usize,
    ) {
        if let Some(ws_idx) = self.active {
            if let Some(rt) = self.runtime_for_pane_in_workspace(terminal_runtimes, ws_idx, pane_id)
            {
                rt.scroll_down(lines);
            }
        }
    }

    pub(crate) fn pane_scroll_metrics(
        &self,
        terminal_runtimes: &TerminalRuntimeRegistry,
        pane_id: crate::layout::PaneId,
    ) -> Option<crate::pane::ScrollMetrics> {
        self.active
            .and_then(|i| self.runtime_for_pane_in_workspace(terminal_runtimes, i, pane_id))
            .and_then(crate::terminal::TerminalRuntime::scroll_metrics)
    }

    fn handle_right_click_passthrough(
        &mut self,
        terminal_runtimes: &TerminalRuntimeRegistry,
        mouse: MouseEvent,
        in_table: bool,
    ) -> bool {
        if let Some(gesture) = self.right_click_passthrough.clone() {
            match mouse.kind {
                MouseEventKind::Drag(MouseButton::Right)
                | MouseEventKind::Up(MouseButton::Right) => {
                    let forwarded_mouse =
                        self.strip_right_click_passthrough_modifiers(mouse, gesture.modifiers);
                    let _ = self.forward_pane_mouse_button(
                        terminal_runtimes,
                        &gesture.pane_info,
                        forwarded_mouse,
                    );
                    if matches!(mouse.kind, MouseEventKind::Up(MouseButton::Right)) {
                        self.right_click_passthrough = None;
                    }
                    return true;
                }
                _ => {
                    self.right_click_passthrough = None;
                }
            }
        }

        if self.mode != Mode::Terminal
            || in_table
            || !matches!(mouse.kind, MouseEventKind::Down(MouseButton::Right))
        {
            return false;
        }

        let Some(modifiers) = self.right_click_passthrough_modifiers else {
            return false;
        };
        if mouse.modifiers != modifiers {
            return false;
        }

        let Some(info) = self.pane_at(mouse.column, mouse.row).cloned() else {
            return false;
        };

        self.focus_pane(info.id);
        let forwarded_mouse = self.strip_right_click_passthrough_modifiers(mouse, modifiers);
        if !self.forward_pane_mouse_button(terminal_runtimes, &info, forwarded_mouse) {
            return false;
        }

        self.selection = None;
        self.selection_autoscroll = None;
        self.drag = None;
        self.context_menu = None;
        self.right_click_passthrough = Some(RightClickPassthroughGesture {
            pane_info: info,
            modifiers,
        });
        true
    }

    fn strip_right_click_passthrough_modifiers(
        &self,
        mouse: MouseEvent,
        modifiers: crossterm::event::KeyModifiers,
    ) -> MouseEvent {
        MouseEvent {
            modifiers: mouse.modifiers.difference(modifiers),
            ..mouse
        }
    }

    pub(super) fn handle_terminal_wheel(
        &mut self,
        terminal_runtimes: &TerminalRuntimeRegistry,
        mouse: MouseEvent,
    ) {
        let lines_per_notch = self.mouse_scroll_lines;

        if let Some(info) = self.pane_at(mouse.column, mouse.row).cloned() {
            self.focus_pane(info.id);
            if self.forward_pane_wheel(terminal_runtimes, &info, mouse) {
                return;
            }
            match mouse.kind {
                MouseEventKind::ScrollUp => {
                    self.scroll_pane_up(terminal_runtimes, info.id, lines_per_notch)
                }
                MouseEventKind::ScrollDown => {
                    self.scroll_pane_down(terminal_runtimes, info.id, lines_per_notch)
                }
                _ => {}
            }
            return;
        }

        if let Some(info) = self.pane_frame_at(mouse.column, mouse.row).cloned() {
            self.focus_pane(info.id);
            match mouse.kind {
                MouseEventKind::ScrollUp => {
                    self.scroll_pane_up(terminal_runtimes, info.id, lines_per_notch)
                }
                MouseEventKind::ScrollDown => {
                    self.scroll_pane_down(terminal_runtimes, info.id, lines_per_notch)
                }
                _ => {}
            }
            return;
        }

        if let Some(ws_idx) = self.active {
            if let Some(rt) = self.focused_runtime_in_workspace(terminal_runtimes, ws_idx) {
                match mouse.kind {
                    MouseEventKind::ScrollUp => rt.scroll_up(lines_per_notch),
                    MouseEventKind::ScrollDown => rt.scroll_down(lines_per_notch),
                    _ => {}
                }
            }
        }
    }

    pub(super) fn forward_pane_mouse_button(
        &self,
        terminal_runtimes: &TerminalRuntimeRegistry,
        info: &PaneInfo,
        mouse: MouseEvent,
    ) -> bool {
        let Some(ws_idx) = self.active else {
            return false;
        };
        let Some(rt) = self.runtime_for_pane_in_workspace(terminal_runtimes, ws_idx, info.id)
        else {
            return false;
        };
        let column = mouse.column.saturating_sub(info.inner_rect.x);
        let row = mouse.row.saturating_sub(info.inner_rect.y);
        let Some(bytes) = rt.encode_mouse_button(mouse.kind, column, row, mouse.modifiers) else {
            return false;
        };
        rt.scroll_reset();
        if let Err(err) = rt.try_send_bytes(Bytes::from(bytes)) {
            warn!(pane = info.id.raw(), err = %err, kind = ?mouse.kind, "failed to forward mouse button event");
        }
        true
    }

    pub(super) fn forward_pane_mouse_motion(
        &self,
        terminal_runtimes: &TerminalRuntimeRegistry,
        info: &PaneInfo,
        mouse: MouseEvent,
    ) -> bool {
        let Some(ws_idx) = self.active else {
            return false;
        };
        let Some(rt) = self.runtime_for_pane_in_workspace(terminal_runtimes, ws_idx, info.id)
        else {
            return false;
        };
        let column = mouse.column.saturating_sub(info.inner_rect.x);
        let row = mouse.row.saturating_sub(info.inner_rect.y);
        let Some(bytes) = rt.encode_mouse_motion(mouse.kind, column, row, mouse.modifiers) else {
            return false;
        };
        if let Err(err) = rt.try_send_bytes(Bytes::from(bytes)) {
            warn!(pane = info.id.raw(), err = %err, kind = ?mouse.kind, "failed to forward mouse motion event");
        }
        true
    }

    fn forward_pane_reported_wheel(
        &self,
        terminal_runtimes: &TerminalRuntimeRegistry,
        info: &PaneInfo,
        mouse: MouseEvent,
    ) -> bool {
        let Some(ws_idx) = self.active else {
            return false;
        };
        let Some(rt) = self.runtime_for_pane_in_workspace(terminal_runtimes, ws_idx, info.id)
        else {
            return false;
        };
        if !rt
            .input_state()
            .is_some_and(crate::pane::InputState::mouse_reporting_enabled)
        {
            return false;
        }
        rt.scroll_reset();
        let column = mouse.column.saturating_sub(info.inner_rect.x);
        let row = mouse.row.saturating_sub(info.inner_rect.y);
        let Some(bytes) = rt.encode_mouse_wheel(mouse.kind, column, row, mouse.modifiers) else {
            warn!(pane = info.id.raw(), kind = ?mouse.kind, "failed to encode mouse wheel event");
            return true;
        };
        if let Err(err) = rt.try_send_bytes(Bytes::from(bytes)) {
            warn!(pane = info.id.raw(), err = %err, "failed to forward mouse wheel event");
        }
        true
    }

    pub(super) fn forward_pane_wheel(
        &self,
        terminal_runtimes: &TerminalRuntimeRegistry,
        info: &PaneInfo,
        mouse: MouseEvent,
    ) -> bool {
        let Some(ws_idx) = self.active else {
            return false;
        };
        let Some(rt) = self.runtime_for_pane_in_workspace(terminal_runtimes, ws_idx, info.id)
        else {
            return false;
        };
        match rt.wheel_routing() {
            Some(crate::pane::WheelRouting::HostScroll) | None => false,
            Some(crate::pane::WheelRouting::MouseReport) => {
                rt.scroll_reset();
                let column = mouse.column.saturating_sub(info.inner_rect.x);
                let row = mouse.row.saturating_sub(info.inner_rect.y);
                let Some(bytes) = rt.encode_mouse_wheel(mouse.kind, column, row, mouse.modifiers)
                else {
                    warn!(pane = info.id.raw(), kind = ?mouse.kind, "failed to encode mouse wheel event");
                    return true;
                };
                if let Err(err) = rt.try_send_bytes(Bytes::from(bytes)) {
                    warn!(pane = info.id.raw(), err = %err, "failed to forward mouse wheel event");
                }
                true
            }
            Some(crate::pane::WheelRouting::AlternateScroll) => {
                rt.scroll_reset();
                let Some(bytes) = rt.encode_alternate_scroll(mouse.kind) else {
                    return true;
                };
                if let Err(err) = rt.try_send_bytes(Bytes::from(bytes)) {
                    warn!(pane = info.id.raw(), err = %err, "failed to forward alternate-scroll key");
                }
                true
            }
        }
    }

    pub(super) fn set_pane_scroll_offset(
        &self,
        terminal_runtimes: &TerminalRuntimeRegistry,
        pane_id: crate::layout::PaneId,
        offset_from_bottom: usize,
    ) {
        if let Some(ws_idx) = self.active {
            if let Some(rt) = self.runtime_for_pane_in_workspace(terminal_runtimes, ws_idx, pane_id)
            {
                rt.set_scroll_offset_from_bottom(offset_from_bottom);
            }
        }
    }

    pub(super) fn scrollbar_target_at(
        &self,
        terminal_runtimes: &TerminalRuntimeRegistry,
        col: u16,
        row: u16,
    ) -> Option<(crate::layout::PaneId, ScrollbarClickTarget)> {
        let ws_idx = self.active?;
        let info = self.view.pane_infos.iter().find(|info| {
            crate::ui::pane_scrollbar_rect(info).is_some_and(|track| {
                col >= track.x
                    && col < track.x + track.width
                    && row >= track.y
                    && row < track.y + track.height
            })
        })?;
        let rt = self.runtime_for_pane_in_workspace(terminal_runtimes, ws_idx, info.id)?;
        let metrics = rt.scroll_metrics()?;
        if metrics.max_offset_from_bottom == 0 {
            return None;
        }
        let track = crate::ui::pane_scrollbar_rect(info)?;
        if let Some(grab_row_offset) = crate::ui::scrollbar_thumb_grab_offset(metrics, track, row) {
            Some((info.id, ScrollbarClickTarget::Thumb { grab_row_offset }))
        } else {
            Some((
                info.id,
                ScrollbarClickTarget::Track {
                    offset_from_bottom: crate::ui::scrollbar_offset_from_row(metrics, track, row),
                },
            ))
        }
    }

    pub(super) fn scrollbar_offset_for_pane_row(
        &self,
        terminal_runtimes: &TerminalRuntimeRegistry,
        pane_id: crate::layout::PaneId,
        row: u16,
        grab_row_offset: u16,
    ) -> Option<usize> {
        let ws_idx = self.active?;
        let info = self
            .view
            .pane_infos
            .iter()
            .find(|info| info.id == pane_id)?;
        let track = crate::ui::pane_scrollbar_rect(info)?;
        let rt = self.runtime_for_pane_in_workspace(terminal_runtimes, ws_idx, pane_id)?;
        let metrics = rt.scroll_metrics()?;
        if metrics.max_offset_from_bottom == 0 {
            return None;
        }
        Some(crate::ui::scrollbar_offset_from_drag_row(
            metrics,
            track,
            row,
            grab_row_offset,
        ))
    }
}

#[cfg(test)]
pub(super) fn wheel_routing(input_state: crate::pane::InputState) -> WheelRouting {
    if input_state.mouse_protocol_mode.reporting_enabled() {
        WheelRouting::MouseReport
    } else if input_state.alternate_screen && input_state.mouse_alternate_scroll {
        WheelRouting::AlternateScroll
    } else {
        WheelRouting::HostScroll
    }
}

fn rect_contains(rect: Rect, col: u16, row: u16) -> bool {
    rect.width > 0
        && rect.height > 0
        && col >= rect.x
        && col < rect.x + rect.width
        && row >= rect.y
        && row < rect.y + rect.height
}

fn apply_scroll(scroll: &mut usize, delta: i16, max_scroll: usize) {
    if delta.is_negative() {
        *scroll = scroll.saturating_sub(delta.unsigned_abs() as usize);
    } else {
        *scroll = scroll.saturating_add(delta as usize).min(max_scroll);
    }
}

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEventKind};
    use ratatui::layout::{Direction, Rect};

    use super::super::{
        app_for_mouse_test, capture_snapshot, handle_context_menu_key, mouse, numbered_lines_bytes,
        root_layout_ratio,
    };
    use super::*;
    use crate::{
        app::state::{ContextMenuKind, ContextMenuState, MenuListState, Mode, ViewLayout},
        detect::{Agent, AgentState},
        workspace::Workspace,
    };

    #[tokio::test]
    async fn terminal_wheel_uses_configured_mouse_scroll_lines() {
        let mut app = app_for_mouse_test();
        let mut ws = Workspace::test_new("test");
        let pane_id = ws.tabs[0].root_pane;
        let pane_infos = ws.tabs[0].layout.panes(Rect::new(26, 2, 80, 18));
        let info = pane_infos[0].clone();
        ws.tabs[0].runtimes.insert(
            pane_id,
            crate::terminal::TerminalRuntime::test_with_scrollback_bytes(
                info.inner_rect.width,
                info.inner_rect.height,
                16 * 1024,
                &numbered_lines_bytes(64),
            ),
        );

        app.state.workspaces = vec![ws];
        app.state.active = Some(0);
        app.state.selected = 0;
        app.state.mode = Mode::Terminal;
        app.state.view.pane_infos = pane_infos;
        app.state.mouse_scroll_lines = 7;

        app.handle_mouse(mouse(
            MouseEventKind::ScrollUp,
            info.inner_rect.x + 1,
            info.inner_rect.y + 1,
        ));

        let metrics = app
            .state
            .runtime_for_pane_in_workspace(&app.terminal_runtimes, 0, pane_id)
            .and_then(crate::terminal::TerminalRuntime::scroll_metrics)
            .expect("scroll metrics after wheel");
        assert_eq!(metrics.offset_from_bottom, 7);
    }

    #[tokio::test]
    async fn configured_right_click_passthrough_forwards_full_gesture_to_pane() {
        let mut app = app_for_mouse_test();
        let mut ws = Workspace::test_new("test");
        let pane_id = ws.tabs[0].root_pane;
        let pane_infos = ws.tabs[0].layout.panes(Rect::new(26, 2, 80, 18));
        let info = pane_infos[0].clone();
        let (runtime, mut input_rx) =
            crate::terminal::TerminalRuntime::test_with_channel_and_scrollback_bytes(
                info.inner_rect.width,
                info.inner_rect.height,
                0,
                b"\x1b[?1002h\x1b[?1006h",
                4,
            );
        ws.insert_test_runtime(pane_id, runtime);

        app.state.workspaces = vec![ws];
        app.state.active = Some(0);
        app.state.selected = 0;
        app.state.mode = Mode::Terminal;
        app.state.view.pane_infos = pane_infos;
        app.state.right_click_passthrough_modifiers = Some(KeyModifiers::CONTROL);

        let col = info.inner_rect.x + 2;
        let row = info.inner_rect.y + 3;
        app.handle_mouse(MouseEvent {
            modifiers: KeyModifiers::CONTROL,
            ..mouse(MouseEventKind::Down(MouseButton::Right), col, row)
        });
        app.handle_mouse(MouseEvent {
            modifiers: KeyModifiers::CONTROL,
            ..mouse(MouseEventKind::Drag(MouseButton::Right), col + 1, row + 1)
        });
        app.handle_mouse(MouseEvent {
            modifiers: KeyModifiers::CONTROL,
            ..mouse(MouseEventKind::Up(MouseButton::Right), col + 1, row + 1)
        });

        assert_eq!(app.state.mode, Mode::Terminal);
        assert!(app.state.context_menu.is_none());
        assert!(app.state.right_click_passthrough.is_none());
        assert_eq!(
            input_rx.try_recv().expect("forwarded right mouse down"),
            Bytes::from_static(b"\x1b[<2;3;4M")
        );
        assert_eq!(
            input_rx.try_recv().expect("forwarded right mouse drag"),
            Bytes::from_static(b"\x1b[<34;4;5M")
        );
        assert_eq!(
            input_rx.try_recv().expect("forwarded right mouse up"),
            Bytes::from_static(b"\x1b[<2;4;5m")
        );
        assert!(input_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn pane_mouse_only_forwards_moved_events_for_any_motion_apps() {
        let mut app = app_for_mouse_test();
        let mut ws = Workspace::test_new("test");
        let pane_id = ws.tabs[0].root_pane;
        let pane_infos = ws.tabs[0].layout.panes(Rect::new(26, 2, 80, 18));
        let info = pane_infos[0].clone();
        let (runtime, mut input_rx) =
            crate::terminal::TerminalRuntime::test_with_channel_and_scrollback_bytes(
                info.inner_rect.width,
                info.inner_rect.height,
                0,
                b"\x1b[?1003h\x1b[?1006h",
                4,
            );
        ws.insert_test_runtime(pane_id, runtime);

        app.state.workspaces = vec![ws];
        app.state.active = Some(0);
        app.state.selected = 0;
        app.state.mode = Mode::Terminal;
        app.state.view.pane_infos = pane_infos;

        app.state.handle_pane_mouse_only(
            &app.terminal_runtimes,
            mouse(
                MouseEventKind::Moved,
                info.inner_rect.x + 2,
                info.inner_rect.y + 3,
            ),
        );

        assert_eq!(
            input_rx.try_recv().expect("forwarded mouse motion"),
            Bytes::from_static(b"\x1b[<35;3;4M")
        );
        assert!(input_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn pane_mouse_motion_uses_computed_inner_rect_offsets() {
        let mut app = app_for_mouse_test();
        let mut ws = Workspace::test_new("test");
        let pane_id = ws.tabs[0].root_pane;
        let (runtime, mut input_rx) =
            crate::terminal::TerminalRuntime::test_with_channel_and_scrollback_bytes(
                80,
                18,
                0,
                b"\x1b[?1003h\x1b[?1006h",
                4,
            );
        ws.insert_test_runtime(pane_id, runtime);

        app.state.workspaces = vec![ws];
        app.state.active = Some(0);
        app.state.selected = 0;
        app.state.mode = Mode::Terminal;
        crate::ui::compute_view(&mut app.state, Rect::new(0, 0, 106, 20));
        let info = app.state.view.pane_infos[0].clone();
        assert_eq!(
            info.inner_rect.x,
            info.rect.x + 1,
            "desktop pane input starts inside the rounded panel border"
        );
        assert!(info.inner_rect.y > 0, "tab bar offset should be present");

        app.state.handle_pane_mouse_only(
            &app.terminal_runtimes,
            mouse(
                MouseEventKind::Moved,
                info.inner_rect.x + 2,
                info.inner_rect.y + 3,
            ),
        );

        assert_eq!(
            input_rx.try_recv().expect("forwarded mouse motion"),
            Bytes::from_static(b"\x1b[<35;3;4M")
        );
        assert!(input_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn mouse_dispatcher_downgrades_sgr_pixel_motion_to_cell_coordinates() {
        let mut app = app_for_mouse_test();
        let mut ws = Workspace::test_new("test");
        let pane_id = ws.tabs[0].root_pane;
        let (runtime, mut input_rx) =
            crate::terminal::TerminalRuntime::test_with_channel_and_scrollback_bytes(
                80,
                18,
                0,
                b"\x1b[?1003h\x1b[?1006h\x1b[?1016h",
                4,
            );
        ws.insert_test_runtime(pane_id, runtime);

        app.state.workspaces = vec![ws];
        app.state.active = Some(0);
        app.state.selected = 0;
        app.state.mode = Mode::Terminal;
        crate::ui::compute_view(&mut app.state, Rect::new(0, 0, 106, 20));
        let info = app.state.view.pane_infos[0].clone();
        assert_eq!(
            info.inner_rect.x,
            info.rect.x + 1,
            "desktop pane input starts inside the rounded panel border"
        );
        assert!(info.inner_rect.y > 0, "tab bar offset should be present");

        app.handle_mouse(mouse(
            MouseEventKind::Moved,
            info.inner_rect.x + 2,
            info.inner_rect.y + 3,
        ));

        assert_eq!(
            input_rx.try_recv().expect("forwarded mouse motion"),
            Bytes::from_static(b"\x1b[<35;3;4M")
        );
        assert!(input_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn mouse_dispatcher_does_not_forward_motion_behind_herdr_modes() {
        let mut app = app_for_mouse_test();
        let mut ws = Workspace::test_new("test");
        let pane_id = ws.tabs[0].root_pane;
        let (runtime, mut input_rx) =
            crate::terminal::TerminalRuntime::test_with_channel_and_scrollback_bytes(
                80,
                18,
                0,
                b"\x1b[?1003h\x1b[?1006h",
                4,
            );
        ws.insert_test_runtime(pane_id, runtime);

        app.state.workspaces = vec![ws];
        app.state.active = Some(0);
        app.state.selected = 0;
        app.state.mode = Mode::Navigate;
        crate::ui::compute_view(&mut app.state, Rect::new(0, 0, 106, 20));
        let info = app.state.view.pane_infos[0].clone();

        app.handle_mouse(mouse(
            MouseEventKind::Moved,
            info.inner_rect.x + 2,
            info.inner_rect.y + 3,
        ));

        assert!(input_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn unset_right_click_passthrough_keeps_modified_right_click_as_herdr_menu() {
        let mut app = app_for_mouse_test();
        let mut ws = Workspace::test_new("test");
        let pane_id = ws.tabs[0].root_pane;
        let pane_infos = ws.tabs[0].layout.panes(Rect::new(26, 2, 80, 18));
        let info = pane_infos[0].clone();
        let (runtime, mut input_rx) =
            crate::terminal::TerminalRuntime::test_with_channel_and_scrollback_bytes(
                info.inner_rect.width,
                info.inner_rect.height,
                0,
                b"\x1b[?1002h\x1b[?1006h",
                4,
            );
        ws.insert_test_runtime(pane_id, runtime);

        app.state.workspaces = vec![ws];
        app.state.active = Some(0);
        app.state.selected = 0;
        app.state.mode = Mode::Terminal;
        app.state.view.pane_infos = pane_infos;
        app.state.right_click_passthrough_modifiers = None;

        app.handle_mouse(MouseEvent {
            modifiers: KeyModifiers::CONTROL,
            ..mouse(
                MouseEventKind::Down(MouseButton::Right),
                info.inner_rect.x + 2,
                info.inner_rect.y + 3,
            )
        });

        assert_eq!(app.state.mode, Mode::ContextMenu);
        assert!(app.state.context_menu.is_some());
        assert!(app.state.right_click_passthrough.is_none());
        assert!(input_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn right_click_passthrough_requires_exact_modifier_match() {
        let mut app = app_for_mouse_test();
        let mut ws = Workspace::test_new("test");
        let pane_id = ws.tabs[0].root_pane;
        let pane_infos = ws.tabs[0].layout.panes(Rect::new(26, 2, 80, 18));
        let info = pane_infos[0].clone();
        let (runtime, mut input_rx) =
            crate::terminal::TerminalRuntime::test_with_channel_and_scrollback_bytes(
                info.inner_rect.width,
                info.inner_rect.height,
                0,
                b"\x1b[?1002h\x1b[?1006h",
                4,
            );
        ws.insert_test_runtime(pane_id, runtime);

        app.state.workspaces = vec![ws];
        app.state.active = Some(0);
        app.state.selected = 0;
        app.state.mode = Mode::Terminal;
        app.state.view.pane_infos = pane_infos;
        app.state.right_click_passthrough_modifiers = Some(KeyModifiers::CONTROL);

        let col = info.inner_rect.x + 2;
        let row = info.inner_rect.y + 3;
        app.handle_mouse(MouseEvent {
            modifiers: KeyModifiers::CONTROL | KeyModifiers::SHIFT,
            ..mouse(MouseEventKind::Down(MouseButton::Right), col, row)
        });

        assert_eq!(app.state.mode, Mode::ContextMenu);
        assert!(app.state.context_menu.is_some());
        assert!(app.state.right_click_passthrough.is_none());
        assert!(input_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn right_click_passthrough_does_not_forward_pane_frame_clicks() {
        let mut app = app_for_mouse_test();
        let mut ws = Workspace::test_new("test");
        let pane_id = ws.tabs[0].root_pane;
        let other_pane = ws.test_split(Direction::Vertical);
        app.state.workspaces = vec![ws];
        app.state.active = Some(0);
        app.state.selected = 0;
        app.state.mode = Mode::Terminal;
        app.state.right_click_passthrough_modifiers = Some(KeyModifiers::CONTROL);
        crate::ui::compute_view(&mut app.state, Rect::new(0, 0, 106, 20));

        let info = app
            .state
            .view
            .pane_infos
            .iter()
            .find(|info| info.id == pane_id)
            .expect("pane info")
            .clone();
        let (runtime, mut input_rx) =
            crate::terminal::TerminalRuntime::test_with_channel_and_scrollback_bytes(
                info.inner_rect.width,
                info.inner_rect.height,
                0,
                b"\x1b[?1002h\x1b[?1006h",
                4,
            );
        app.state.insert_test_runtime(pane_id, runtime);
        app.state.insert_test_runtime(
            other_pane,
            crate::terminal::TerminalRuntime::test_with_screen_bytes(10, 5, b""),
        );

        assert!(app.state.pane_at(info.rect.x, info.rect.y).is_none());
        assert!(app
            .state
            .pane_mouse_target(info.rect.x, info.rect.y)
            .is_some());
        app.handle_mouse(MouseEvent {
            modifiers: KeyModifiers::CONTROL,
            ..mouse(
                MouseEventKind::Down(MouseButton::Right),
                info.rect.x,
                info.rect.y,
            )
        });

        assert_eq!(app.state.mode, Mode::ContextMenu);
        assert!(app.state.context_menu.is_some());
        assert!(app.state.right_click_passthrough.is_none());
        assert!(input_rx.try_recv().is_err());
    }

    fn sample_worktree_open_state() -> crate::app::state::WorktreeOpenState {
        crate::app::state::WorktreeOpenState {
            source_workspace_id: "source".into(),
            source_existing_membership: None,
            source_checkout_path: "/repo/herdr".into(),
            source_repo_root: "/repo/herdr".into(),
            repo_key: "repo-key".into(),
            repo_name: "herdr".into(),
            entries: vec![
                crate::app::state::WorktreeOpenEntry {
                    path: "/repo/herdr".into(),
                    branch: Some("main".into()),
                    is_linked_worktree: false,
                    already_open_ws_idx: Some(0),
                },
                crate::app::state::WorktreeOpenEntry {
                    path: "/repo/herdr-issue".into(),
                    branch: Some("worktree/issue".into()),
                    is_linked_worktree: true,
                    already_open_ws_idx: None,
                },
            ],
            selected: 0,
            query: String::new(),
            search_focused: false,
            error: None,
        }
    }
    #[test]
    fn clicking_config_warning_dismiss_clears_the_warning() {
        let mut app = app_for_mouse_test();
        app.state.mode = Mode::Navigate;
        app.state.config_diagnostic =
            Some("This workspace is not a Herdr-managed worktree checkout.".into());
        crate::ui::compute_view(&mut app.state, Rect::new(0, 0, 120, 24));

        let dismiss = crate::ui::config_diagnostic_dismiss_rect(
            app.state.view.terminal_area,
            app.state.config_diagnostic.as_deref().unwrap(),
        )
        .expect("dismiss control");

        app.handle_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            dismiss.x + dismiss.width.saturating_sub(2),
            dismiss.y,
        ));

        assert!(app.state.config_diagnostic.is_none());
    }

    #[test]
    fn clicking_config_warning_text_does_not_dismiss() {
        let mut app = app_for_mouse_test();
        app.state.mode = Mode::Navigate;
        let message = "This workspace is not a Herdr-managed worktree checkout.";
        app.state.config_diagnostic = Some(message.into());
        crate::ui::compute_view(&mut app.state, Rect::new(0, 0, 120, 24));

        let dismiss =
            crate::ui::config_diagnostic_dismiss_rect(app.state.view.terminal_area, message)
                .expect("dismiss control");

        app.handle_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            dismiss.x.saturating_sub(4),
            dismiss.y,
        ));

        assert_eq!(app.state.config_diagnostic.as_deref(), Some(message));
    }

    #[test]
    fn clicking_agent_toast_focuses_target_pane() {
        let mut app = app_for_mouse_test();
        let active = Workspace::test_new("active");
        let mut background = Workspace::test_new("background");
        let first_pane = background.tabs[0].root_pane;
        let target_pane = background.test_split(Direction::Horizontal);
        background.tabs[0].layout.focus_pane(first_pane);

        app.state.workspaces = vec![active, background];
        app.state.ensure_test_terminals();
        app.state.active = Some(0);
        app.state.selected = 0;
        app.state.toast_config.delivery = crate::config::ToastDelivery::Herdr;
        let target_terminal_id = app.state.workspaces[1]
            .panes
            .get(&target_pane)
            .unwrap()
            .attached_terminal_id
            .clone();
        app.state
            .terminals
            .get_mut(&target_terminal_id)
            .unwrap()
            .state = AgentState::Working;

        app.state
            .handle_app_event(crate::events::AppEvent::StateChanged {
                pane_id: target_pane,
                agent: Some(Agent::Pi),
                state: AgentState::Idle,
                visible_blocker: false,
                visible_idle: false,
                visible_working: false,
                process_exited: false,
                observed_at: std::time::Instant::now(),
            });
        crate::ui::compute_view(&mut app.state, Rect::new(0, 0, 106, 20));

        let hit = app.state.view.toast_hit_area;
        app.handle_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            hit.x + 1,
            hit.y + 1,
        ));

        assert_eq!(app.state.active, Some(1));
        assert_eq!(app.state.workspaces[1].focused_pane_id(), Some(target_pane));
        assert!(app.state.toast.is_none());
        assert_eq!(app.state.mode, Mode::Terminal);

        app.state.last_pane();

        assert_eq!(app.state.active, Some(0));
        assert_eq!(
            app.state.workspaces[0].focused_pane_id(),
            Some(app.state.workspaces[0].tabs[0].root_pane)
        );
    }

    #[test]
    fn toast_click_does_not_steal_mouse_from_settings_overlay() {
        let mut app = app_for_mouse_test();
        let active = Workspace::test_new("active");
        let background = Workspace::test_new("background");
        let target_pane = background.tabs[0].root_pane;
        let workspace_id = background.id.clone();

        app.state.workspaces = vec![active, background];
        app.state.active = Some(0);
        app.state.selected = 0;
        app.state.toast = Some(crate::app::state::ToastNotification {
            kind: crate::app::state::ToastKind::Finished,
            title: "pi finished".into(),
            context: "background · 2".into(),
            target: Some(crate::app::state::ToastTarget {
                workspace_id,
                pane_id: target_pane,
            }),
        });
        app.state.mode = Mode::Settings;
        crate::ui::compute_view(&mut app.state, Rect::new(0, 0, 106, 20));

        let hit = app.state.view.toast_hit_area;
        app.handle_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            hit.x + 1,
            hit.y + 1,
        ));

        assert_eq!(app.state.active, Some(0));
        assert!(app.state.toast.is_some());
    }

    #[test]
    fn clicking_confirm_close_accepts_workspace_close() {
        let mut app = app_for_mouse_test();
        app.state.workspaces = vec![Workspace::test_new("a"), Workspace::test_new("b")];
        app.state.active = Some(0);
        app.state.selected = 1;
        app.state.mode = Mode::ConfirmClose;

        let popup = app.state.confirm_close_rect();
        let inner = Rect::new(
            popup.x + 1,
            popup.y + 1,
            popup.width.saturating_sub(2),
            popup.height.saturating_sub(2),
        );
        let (confirm, _) = crate::ui::confirm_close_button_rects(inner);

        app.handle_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            confirm.x,
            confirm.y,
        ));

        assert_eq!(app.state.workspaces.len(), 1);
        assert_eq!(app.state.mode, Mode::Terminal);
    }

    #[test]
    fn clicking_open_worktree_row_selects_and_requests_open() {
        let mut app = app_for_mouse_test();
        app.state.mode = Mode::OpenExistingWorktree;
        app.state.worktree_open = Some(sample_worktree_open_state());
        let inner =
            crate::ui::open_existing_worktree_inner_rect(app.state.screen_rect(), 2).unwrap();

        app.handle_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            inner.x + 1,
            inner.y + 5,
        ));

        assert_eq!(app.state.worktree_open.as_ref().unwrap().selected, 1);
        assert!(app.state.request_submit_worktree_open);
    }

    #[test]
    fn clicking_open_worktree_buttons_requests_open_or_cancels() {
        let mut app = app_for_mouse_test();
        app.state.mode = Mode::OpenExistingWorktree;
        app.state.worktree_open = Some(sample_worktree_open_state());
        let inner =
            crate::ui::open_existing_worktree_inner_rect(app.state.screen_rect(), 2).unwrap();
        let (open, _) = crate::ui::open_existing_worktree_button_rects(inner);

        app.handle_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            open.x,
            open.y,
        ));

        assert!(app.state.worktree_open.is_some());
        assert!(app.state.request_submit_worktree_open);

        let mut app = app_for_mouse_test();
        app.state.mode = Mode::OpenExistingWorktree;
        app.state.worktree_open = Some(sample_worktree_open_state());
        let inner =
            crate::ui::open_existing_worktree_inner_rect(app.state.screen_rect(), 2).unwrap();
        let (_, cancel) = crate::ui::open_existing_worktree_button_rects(inner);

        app.handle_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            cancel.x,
            cancel.y,
        ));

        assert!(app.state.worktree_open.is_none());
        assert_eq!(app.state.mode, Mode::Navigate);
    }

    #[test]
    fn scrolling_open_worktree_picker_moves_selection() {
        let mut app = app_for_mouse_test();
        app.state.mode = Mode::OpenExistingWorktree;
        app.state.worktree_open = Some(sample_worktree_open_state());

        app.handle_mouse(mouse(MouseEventKind::ScrollDown, 1, 1));
        assert_eq!(app.state.worktree_open.as_ref().unwrap().selected, 1);

        app.handle_mouse(mouse(MouseEventKind::ScrollUp, 1, 1));
        assert_eq!(app.state.worktree_open.as_ref().unwrap().selected, 0);
    }

    #[test]
    fn clicking_remove_worktree_buttons_requests_remove_or_cancels() {
        let mut app = app_for_mouse_test();
        app.state.mode = Mode::ConfirmRemoveWorktree;
        app.state.worktree_remove = Some(crate::app::state::WorktreeRemoveState {
            workspace_id: "issue".into(),
            repo_root: "/repo/herdr".into(),
            path: "/repo/herdr-issue".into(),
            error: None,
            removing: false,
            force_confirmation: false,
        });
        let popup = crate::ui::remove_worktree_popup_rect(app.state.screen_rect()).unwrap();
        let inner = Rect::new(
            popup.x + 1,
            popup.y + 1,
            popup.width.saturating_sub(2),
            popup.height.saturating_sub(2),
        );
        let (remove, _) = crate::ui::remove_worktree_button_rects(inner, false);

        app.handle_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            remove.x,
            remove.y,
        ));

        assert!(app.state.worktree_remove.is_some());
        assert!(app.state.request_submit_worktree_remove);

        let mut app = app_for_mouse_test();
        app.state.mode = Mode::ConfirmRemoveWorktree;
        app.state.worktree_remove = Some(crate::app::state::WorktreeRemoveState {
            workspace_id: "issue".into(),
            repo_root: "/repo/herdr".into(),
            path: "/repo/herdr-issue".into(),
            error: None,
            removing: false,
            force_confirmation: false,
        });
        let popup = crate::ui::remove_worktree_popup_rect(app.state.screen_rect()).unwrap();
        let inner = Rect::new(
            popup.x + 1,
            popup.y + 1,
            popup.width.saturating_sub(2),
            popup.height.saturating_sub(2),
        );
        let (_, cancel) = crate::ui::remove_worktree_button_rects(inner, false);

        app.handle_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            cancel.x,
            cancel.y,
        ));

        assert!(app.state.worktree_remove.is_none());
        assert_eq!(app.state.mode, Mode::Navigate);
    }
    #[tokio::test]
    async fn keyboard_context_menu_split_keeps_new_runtime() {
        let mut app = app_for_mouse_test();
        app.state.default_shell = "/usr/bin/true".into();
        let (workspace, terminal, runtime) = Workspace::new(
            std::env::current_dir().unwrap_or_else(|_| "/".into()),
            24,
            80,
            app.state.pane_scrollback_limit_bytes,
            app.state.host_terminal_theme,
            crate::pane::PaneShellConfig::new(&app.state.default_shell, app.state.shell_mode),
            app.event_tx.clone(),
            app.render_notify.clone(),
            app.render_dirty.clone(),
        )
        .expect("workspace should spawn");
        app.state.workspaces = vec![workspace];
        app.terminal_runtimes.insert(terminal.id.clone(), runtime);
        app.state.terminals.insert(terminal.id.clone(), terminal);
        app.state.active = Some(0);
        app.state.selected = 0;
        let pane_id = app.state.workspaces[0].tabs[0].root_pane;
        let runtime_count = app.terminal_runtimes.len();
        app.state.context_menu = Some(ContextMenuState {
            kind: ContextMenuKind::Pane {
                pane_id,
                has_manual_label: false,
                dimmed: false,
                has_agent: false,
                can_reset: false,
            },
            x: 2,
            y: 2,
            list: MenuListState::new(1),
        });
        app.state.mode = Mode::ContextMenu;

        handle_context_menu_key(
            &mut app.state,
            &mut app.terminal_runtimes,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::empty()),
        );

        assert_eq!(app.state.mode, Mode::Terminal);
        assert_eq!(app.state.workspaces[0].tabs[0].layout.pane_count(), 2);
        assert_eq!(app.terminal_runtimes.len(), runtime_count + 1);

        let runtimes: Vec<_> = app.terminal_runtimes.drain().collect();
        for (_terminal_id, runtime) in runtimes {
            runtime.shutdown();
        }
    }

    #[test]
    fn dragging_a_pane_onto_another_pane_edge_cuts_it_in_two() {
        let mut app = app_for_mouse_test();
        app.state.workspaces = vec![Workspace::test_new("test")];
        app.state.active = Some(0);
        app.state.selected = 0;
        let left = app.state.workspaces[0].tabs[0].root_pane;
        let right = app.state.workspaces[0].test_split(Direction::Horizontal);
        let area = Rect::new(0, 0, 106, 20);
        crate::ui::compute_view(&mut app.state, area);

        let right_rect = app
            .state
            .view
            .pane_infos
            .iter()
            .find(|pane| pane.id == right)
            .unwrap()
            .rect;
        let title_hit = app
            .state
            .view
            .pane_title_hit_areas
            .iter()
            .find(|hit| hit.pane_id == left)
            .expect("left pane title hit area");
        let start_col = title_hit.rect.x + title_hit.rect.width / 2;
        let start_row = title_hit.rect.y;
        // The bottom edge of the right pane, well inside its lower quarter.
        let target_col = right_rect.x + right_rect.width / 2;
        let target_row = right_rect.y + right_rect.height - 1;

        app.handle_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            start_col,
            start_row,
        ));
        app.handle_mouse(mouse(
            MouseEventKind::Drag(MouseButton::Left),
            target_col,
            target_row,
        ));
        assert!(matches!(
            app.state.drag.as_ref().map(|drag| &drag.target),
            Some(DragTarget::PaneSwap {
                drop_zone: crate::layout::DropZone::Edge(crate::layout::SplitSide::Bottom),
                ..
            })
        ));
        app.handle_mouse(mouse(
            MouseEventKind::Up(MouseButton::Left),
            target_col,
            target_row,
        ));
        crate::ui::compute_view(&mut app.state, area);

        // The column the moved pane left behind closed up, so the two panes now
        // share one column with the moved one underneath.
        let new_left = app
            .state
            .view
            .pane_infos
            .iter()
            .find(|pane| pane.id == left)
            .unwrap()
            .rect;
        let new_right = app
            .state
            .view
            .pane_infos
            .iter()
            .find(|pane| pane.id == right)
            .unwrap()
            .rect;
        assert_eq!(app.state.workspaces[0].tabs[0].layout.pane_count(), 2);
        assert_eq!(new_left.x, new_right.x);
        assert_eq!(new_left.width, area.width - new_left.x);
        assert!(new_left.y > new_right.y);
    }

    #[test]
    fn dragging_pane_title_swaps_pane_positions() {
        let mut app = app_for_mouse_test();
        app.state.workspaces = vec![Workspace::test_new("test")];
        app.state.active = Some(0);
        app.state.selected = 0;
        let left = app.state.workspaces[0].tabs[0].root_pane;
        let right = app.state.workspaces[0].test_split(Direction::Horizontal);
        crate::ui::compute_view(&mut app.state, Rect::new(0, 0, 106, 20));

        let left_rect = app
            .state
            .view
            .pane_infos
            .iter()
            .find(|pane| pane.id == left)
            .unwrap()
            .rect;
        let right_rect = app
            .state
            .view
            .pane_infos
            .iter()
            .find(|pane| pane.id == right)
            .unwrap()
            .rect;
        assert!(left_rect.x < right_rect.x);

        let title_hit = app
            .state
            .view
            .pane_title_hit_areas
            .iter()
            .find(|hit| hit.pane_id == left)
            .expect("left pane title hit area");
        let start_col = title_hit.rect.x + title_hit.rect.width / 2;
        let start_row = title_hit.rect.y;
        let target_col = right_rect.x + right_rect.width / 2;
        let target_row = right_rect.y + right_rect.height / 2;

        app.handle_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            start_col,
            start_row,
        ));
        app.handle_mouse(mouse(
            MouseEventKind::Drag(MouseButton::Left),
            target_col,
            target_row,
        ));
        app.handle_mouse(mouse(
            MouseEventKind::Up(MouseButton::Left),
            target_col,
            target_row,
        ));
        crate::ui::compute_view(&mut app.state, Rect::new(0, 0, 106, 20));

        let new_left = app
            .state
            .view
            .pane_infos
            .iter()
            .find(|pane| pane.id == left)
            .unwrap()
            .rect;
        let new_right = app
            .state
            .view
            .pane_infos
            .iter()
            .find(|pane| pane.id == right)
            .unwrap()
            .rect;
        assert_eq!(new_left, right_rect);
        assert_eq!(new_right, left_rect);
    }

    #[tokio::test]
    async fn dragging_pane_title_still_swaps_when_pane_mouse_reporting_is_enabled() {
        let mut app = app_for_mouse_test();
        let mut ws = Workspace::test_new("test");
        let left = ws.tabs[0].root_pane;
        let right = ws.test_split(Direction::Horizontal);
        app.state.workspaces = vec![ws];
        app.state.active = Some(0);
        app.state.selected = 0;
        crate::ui::compute_view(&mut app.state, Rect::new(0, 0, 106, 20));

        for pane_id in [left, right] {
            let info = app
                .state
                .view
                .pane_infos
                .iter()
                .find(|info| info.id == pane_id)
                .expect("pane info")
                .clone();
            app.state.insert_test_runtime(
                pane_id,
                crate::terminal::TerminalRuntime::test_with_screen_bytes(
                    info.inner_rect.width.max(1),
                    info.inner_rect.height.max(1),
                    b"\x1b[?1002h\x1b[?1006h",
                ),
            );
        }

        crate::ui::compute_view(&mut app.state, Rect::new(0, 0, 106, 20));
        let left_rect = app
            .state
            .view
            .pane_infos
            .iter()
            .find(|pane| pane.id == left)
            .unwrap()
            .rect;
        let right_rect = app
            .state
            .view
            .pane_infos
            .iter()
            .find(|pane| pane.id == right)
            .unwrap()
            .rect;
        assert!(left_rect.x < right_rect.x);

        let title_hit = app
            .state
            .view
            .pane_title_hit_areas
            .iter()
            .find(|hit| hit.pane_id == left)
            .expect("left pane title hit area");
        let start_col = title_hit.rect.x + title_hit.rect.width / 2;
        let start_row = title_hit.rect.y;
        let target_col = right_rect.x + right_rect.width / 2;
        let target_row = right_rect.y + right_rect.height / 2;

        app.handle_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            start_col,
            start_row,
        ));
        // The first drag step lands a few cells along the source title, still
        // inside the source pane frame: the gesture must not be handed to the
        // pane underneath before it can become a swap.
        app.handle_mouse(mouse(
            MouseEventKind::Drag(MouseButton::Left),
            start_col + PANE_DRAG_THRESHOLD,
            start_row,
        ));
        assert!(matches!(
            app.state.drag.as_ref().map(|drag| &drag.target),
            Some(DragTarget::PaneSwap { .. })
        ));

        app.handle_mouse(mouse(
            MouseEventKind::Drag(MouseButton::Left),
            target_col,
            target_row,
        ));
        app.handle_mouse(mouse(
            MouseEventKind::Up(MouseButton::Left),
            target_col,
            target_row,
        ));
        crate::ui::compute_view(&mut app.state, Rect::new(0, 0, 106, 20));

        let new_left = app
            .state
            .view
            .pane_infos
            .iter()
            .find(|pane| pane.id == left)
            .unwrap()
            .rect;
        let new_right = app
            .state
            .view
            .pane_infos
            .iter()
            .find(|pane| pane.id == right)
            .unwrap()
            .rect;
        assert_eq!(new_left, right_rect);
        assert_eq!(new_right, left_rect);
    }
    #[test]
    fn dragging_pane_split_updates_captured_layout_ratio() {
        let mut app = app_for_mouse_test();
        app.state.workspaces = vec![Workspace::test_new("test")];
        app.state.active = Some(0);
        app.state.selected = 0;
        app.state.workspaces[0].test_split(Direction::Horizontal);
        crate::ui::compute_view(&mut app.state, Rect::new(0, 0, 106, 20));
        let border = app.state.view.split_borders[0].clone();
        let before = capture_snapshot(&app.state);
        let drag_row = border.area.y.saturating_add(1);

        app.handle_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            border.pos,
            drag_row,
        ));
        app.handle_mouse(mouse(
            MouseEventKind::Drag(MouseButton::Left),
            border.pos.saturating_add(6),
            drag_row,
        ));

        let after = capture_snapshot(&app.state);
        assert_ne!(root_layout_ratio(&before), root_layout_ratio(&after));
    }

    #[test]
    fn dragging_bottom_pane_rule_glyphs_starts_resize_not_swap() {
        let mut app = app_for_mouse_test();
        app.state.workspaces = vec![Workspace::test_new("test")];
        app.state.active = Some(0);
        app.state.selected = 0;
        app.state.workspaces[0].test_split(Direction::Vertical);
        crate::ui::compute_view(&mut app.state, Rect::new(0, 0, 106, 20));

        let border = app.state.view.split_borders[0].clone();
        let bottom_pane = app
            .state
            .view
            .pane_infos
            .iter()
            .find(|info| info.rect.y == border.pos)
            .expect("bottom pane");
        let title_hit = app
            .state
            .view
            .pane_title_hit_areas
            .iter()
            .find(|hit| hit.pane_id == bottom_pane.id)
            .expect("bottom pane title hit area");
        let rule_col = title_hit.rect.x + title_hit.rect.width;
        let before = capture_snapshot(&app.state);

        app.handle_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            rule_col,
            border.pos,
        ));
        assert!(matches!(
            app.state.drag.as_ref().map(|drag| &drag.target),
            Some(DragTarget::PaneSplit { .. })
        ));

        app.handle_mouse(mouse(
            MouseEventKind::Drag(MouseButton::Left),
            rule_col,
            border.pos.saturating_add(2),
        ));

        assert_ne!(
            root_layout_ratio(&capture_snapshot(&app.state)),
            root_layout_ratio(&before)
        );
    }

    #[test]
    fn dragging_bottom_pane_title_starts_swap_not_resize() {
        let mut app = app_for_mouse_test();
        app.state.workspaces = vec![Workspace::test_new("test")];
        app.state.active = Some(0);
        app.state.selected = 0;
        let _top = app.state.workspaces[0].tabs[0].root_pane;
        let bottom = app.state.workspaces[0].test_split(Direction::Vertical);
        crate::ui::compute_view(&mut app.state, Rect::new(0, 0, 106, 20));

        let title_hit = app
            .state
            .view
            .pane_title_hit_areas
            .iter()
            .find(|hit| hit.pane_id == bottom)
            .expect("bottom pane title hit area")
            .rect;
        let before = capture_snapshot(&app.state);

        app.handle_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            title_hit.x + 2,
            title_hit.y,
        ));
        app.handle_mouse(mouse(
            MouseEventKind::Drag(MouseButton::Left),
            title_hit.x + 4,
            title_hit.y,
        ));

        assert!(matches!(
            app.state.drag.as_ref().map(|drag| &drag.target),
            Some(DragTarget::PaneSwap { .. })
        ));
        assert_eq!(
            root_layout_ratio(&capture_snapshot(&app.state)),
            root_layout_ratio(&before)
        );
    }

    #[test]
    fn pane_split_hitbox_does_not_overlap_right_pane_content() {
        let mut app = app_for_mouse_test();
        app.state.workspaces = vec![Workspace::test_new("test")];
        app.state.active = Some(0);
        app.state.selected = 0;
        app.state.workspaces[0].test_split(Direction::Horizontal);
        crate::ui::compute_view(&mut app.state, Rect::new(0, 0, 106, 20));
        let border = app.state.view.split_borders[0].clone();
        let row = border.area.y.saturating_add(1);

        assert!(app
            .state
            .find_border_at(border.pos.saturating_sub(1), row)
            .is_some());
        assert!(app.state.find_border_at(border.pos, row).is_some());
        assert!(app
            .state
            .find_border_at(border.pos.saturating_add(1), row)
            .is_none());
    }

    #[test]
    fn pane_split_hitbox_does_not_overlap_bottom_pane_content() {
        let mut app = app_for_mouse_test();
        app.state.workspaces = vec![Workspace::test_new("test")];
        app.state.active = Some(0);
        app.state.selected = 0;
        app.state.workspaces[0].test_split(Direction::Vertical);
        crate::ui::compute_view(&mut app.state, Rect::new(0, 0, 106, 20));
        let border = app.state.view.split_borders[0].clone();
        let col = border.area.x.saturating_add(1);
        let bottom_pane = app
            .state
            .view
            .pane_infos
            .iter()
            .find(|info| info.rect.y == border.pos)
            .expect("bottom pane");

        assert!(app
            .state
            .pane_title_hit_at(col, border.pos)
            .is_some_and(|hit| hit.pane_id == bottom_pane.id));
        assert!(app.state.find_border_at(col, border.pos).is_none());

        let rule_col = app
            .state
            .view
            .pane_title_hit_areas
            .iter()
            .find(|hit| hit.pane_id == bottom_pane.id)
            .map(|hit| hit.rect.x + hit.rect.width)
            .expect("bottom pane title hit area");
        assert!(app.state.find_border_at(rule_col, border.pos).is_some());
        assert!(app.state.pane_title_hit_at(rule_col, border.pos).is_none());

        assert!(app
            .state
            .find_border_at(col, border.pos.saturating_sub(1))
            .is_some());
        assert!(app
            .state
            .find_border_at(col, border.pos.saturating_add(1))
            .is_none());
    }

    #[test]
    fn selecting_from_right_pane_first_content_column_starts_selection() {
        let mut app = app_for_mouse_test();
        let mut ws = Workspace::test_new("test");
        let second_pane = ws.test_split(Direction::Horizontal);
        app.state.workspaces = vec![ws];
        app.state.active = Some(0);
        app.state.selected = 0;
        crate::ui::compute_view(&mut app.state, Rect::new(0, 0, 106, 20));

        let second_info = app
            .state
            .view
            .pane_infos
            .iter()
            .find(|info| info.id == second_pane)
            .expect("second pane info")
            .clone();
        let col = second_info.inner_rect.x;
        let row = second_info.inner_rect.y;

        assert!(app.state.find_border_at(col, row).is_none());
        app.handle_mouse(mouse(MouseEventKind::Down(MouseButton::Left), col, row));

        assert!(app.state.drag.is_none());
        assert_eq!(
            app.state
                .selection
                .as_ref()
                .map(|selection| selection.pane_id),
            Some(second_pane)
        );
    }

    #[test]
    fn selecting_from_bottom_pane_first_content_row_starts_selection() {
        let mut app = app_for_mouse_test();
        let mut ws = Workspace::test_new("test");
        let second_pane = ws.test_split(Direction::Vertical);
        app.state.workspaces = vec![ws];
        app.state.active = Some(0);
        app.state.selected = 0;
        crate::ui::compute_view(&mut app.state, Rect::new(0, 0, 106, 20));

        let second_info = app
            .state
            .view
            .pane_infos
            .iter()
            .find(|info| info.id == second_pane)
            .expect("second pane info")
            .clone();
        let col = second_info.inner_rect.x;
        let row = second_info.inner_rect.y;

        assert!(app.state.find_border_at(col, row).is_none());
        app.handle_mouse(mouse(MouseEventKind::Down(MouseButton::Left), col, row));

        assert!(app.state.drag.is_none());
        assert_eq!(
            app.state
                .selection
                .as_ref()
                .map(|selection| selection.pane_id),
            Some(second_pane)
        );
    }

    #[tokio::test]
    async fn dragging_vertical_pane_split_still_resizes_when_pane_mouse_reporting_is_enabled() {
        let mut app = app_for_mouse_test();
        let mut ws = Workspace::test_new("test");
        let first_pane = ws.tabs[0].root_pane;
        let second_pane = ws.test_split(Direction::Vertical);

        app.state.workspaces = vec![ws];
        app.state.active = Some(0);
        app.state.selected = 0;

        crate::ui::compute_view(&mut app.state, Rect::new(0, 0, 106, 20));

        let pane_infos = app.state.view.pane_infos.clone();
        let first_info = pane_infos
            .iter()
            .find(|info| info.id == first_pane)
            .expect("first pane info")
            .clone();
        let second_info = pane_infos
            .iter()
            .find(|info| info.id == second_pane)
            .expect("second pane info")
            .clone();

        app.state.insert_test_runtime(
            first_pane,
            crate::terminal::TerminalRuntime::test_with_screen_bytes(
                first_info.inner_rect.width.max(1),
                first_info.inner_rect.height.max(1),
                b"\x1b[?1002h",
            ),
        );
        app.state.insert_test_runtime(
            second_pane,
            crate::terminal::TerminalRuntime::test_with_screen_bytes(
                second_info.inner_rect.width.max(1),
                second_info.inner_rect.height.max(1),
                b"\x1b[?1002h",
            ),
        );

        crate::ui::compute_view(&mut app.state, Rect::new(0, 0, 106, 20));
        let border = app
            .state
            .view
            .split_borders
            .iter()
            .find(|border| border.direction == Direction::Vertical)
            .expect("vertical split border")
            .clone();
        let before = capture_snapshot(&app.state);
        let drag_col = border.area.x.saturating_add(1);
        let drag_row = border.pos.saturating_sub(1);

        app.handle_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            drag_col,
            drag_row,
        ));
        app.handle_mouse(mouse(
            MouseEventKind::Drag(MouseButton::Left),
            drag_col,
            border.pos.saturating_add(4),
        ));

        let after = capture_snapshot(&app.state);
        assert_ne!(root_layout_ratio(&before), root_layout_ratio(&after));
    }

    #[tokio::test]
    async fn dragging_horizontal_pane_split_still_resizes_when_pane_mouse_reporting_is_enabled() {
        let mut app = app_for_mouse_test();
        let mut ws = Workspace::test_new("test");
        let first_pane = ws.tabs[0].root_pane;
        let second_pane = ws.test_split(Direction::Horizontal);

        app.state.workspaces = vec![ws];
        app.state.active = Some(0);
        app.state.selected = 0;

        crate::ui::compute_view(&mut app.state, Rect::new(0, 0, 106, 20));

        let pane_infos = app.state.view.pane_infos.clone();
        let first_info = pane_infos
            .iter()
            .find(|info| info.id == first_pane)
            .expect("first pane info")
            .clone();
        let second_info = pane_infos
            .iter()
            .find(|info| info.id == second_pane)
            .expect("second pane info")
            .clone();

        app.state.insert_test_runtime(
            first_pane,
            crate::terminal::TerminalRuntime::test_with_screen_bytes(
                first_info.inner_rect.width.max(1),
                first_info.inner_rect.height.max(1),
                b"\x1b[?1002h",
            ),
        );
        app.state.insert_test_runtime(
            second_pane,
            crate::terminal::TerminalRuntime::test_with_screen_bytes(
                second_info.inner_rect.width.max(1),
                second_info.inner_rect.height.max(1),
                b"\x1b[?1002h",
            ),
        );

        crate::ui::compute_view(&mut app.state, Rect::new(0, 0, 106, 20));
        let border = app
            .state
            .view
            .split_borders
            .iter()
            .find(|border| border.direction == Direction::Horizontal)
            .expect("horizontal split border")
            .clone();
        let before = capture_snapshot(&app.state);
        let drag_row = border.area.y.saturating_add(1);

        app.handle_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            border.pos,
            drag_row,
        ));
        app.handle_mouse(mouse(
            MouseEventKind::Drag(MouseButton::Left),
            border.pos.saturating_add(6),
            drag_row,
        ));

        let after = capture_snapshot(&app.state);
        assert_ne!(root_layout_ratio(&before), root_layout_ratio(&after));
    }

    #[test]
    fn wheel_routing_prefers_mouse_reporting() {
        let input_state = crate::pane::InputState {
            alternate_screen: true,
            application_cursor: false,
            bracketed_paste: false,
            focus_reporting: false,
            mouse_protocol_mode: crate::input::MouseProtocolMode::ButtonMotion,
            mouse_protocol_encoding: crate::input::MouseProtocolEncoding::Sgr,
            mouse_alternate_scroll: true,
            modify_other_keys: false,
        };

        assert_eq!(wheel_routing(input_state), WheelRouting::MouseReport);
    }
    #[test]
    fn mobile_switch_button_opens_switcher_and_workspace_row_switches_workspace() {
        let mut app = app_for_mouse_test();
        app.state.workspaces = vec![Workspace::test_new("one"), Workspace::test_new("two")];
        app.state.active = Some(0);
        app.state.selected = 0;
        app.state.mode = Mode::Terminal;

        crate::ui::compute_view(&mut app.state, Rect::new(0, 0, 44, 20));
        assert_eq!(app.state.view.layout, ViewLayout::Mobile);

        let switch = app.state.view.mobile_menu_hit_area;
        app.handle_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            switch.x + 1,
            switch.y + 1,
        ));

        assert_eq!(app.state.mode, Mode::Navigate);

        let viewport = crate::ui::mobile_switcher_areas(&app.state).viewport;
        app.handle_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            viewport.x + 2,
            viewport.y + 4,
        ));

        assert_eq!(app.state.active, Some(1));
        assert_eq!(app.state.mode, Mode::Terminal);
    }

    #[test]
    fn mobile_workspace_panel_scroll_reaches_extra_workspaces() {
        let mut app = app_for_mouse_test();
        app.state.workspaces = (0..12)
            .map(|idx| Workspace::test_new(&format!("ws-{idx}")))
            .collect();
        app.state.active = Some(0);
        app.state.selected = 0;
        app.state.mode = Mode::Terminal;

        crate::ui::compute_view(&mut app.state, Rect::new(0, 0, 44, 20));
        let switch = app.state.view.mobile_menu_hit_area;
        app.handle_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            switch.x + 1,
            switch.y + 1,
        ));
        assert_eq!(app.state.mode, Mode::Navigate);

        let viewport = crate::ui::mobile_switcher_areas(&app.state).viewport;
        app.handle_mouse(mouse(
            MouseEventKind::ScrollDown,
            viewport.x + 2,
            viewport.y,
        ));
        crate::ui::compute_view(&mut app.state, Rect::new(0, 0, 44, 20));
        assert_eq!(app.state.mobile_switcher_scroll, 2);

        app.handle_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            viewport.x + 2,
            viewport.y + 2,
        ));

        assert_eq!(app.state.active, Some(1));
        assert_eq!(app.state.mode, Mode::Terminal);
    }

    #[test]
    fn mobile_switcher_swallows_non_left_mouse_events() {
        let mut app = app_for_mouse_test();
        app.state.workspaces = vec![Workspace::test_new("one")];
        app.state.active = Some(0);
        app.state.selected = 0;
        app.state.mode = Mode::Terminal;

        crate::ui::compute_view(&mut app.state, Rect::new(0, 0, 44, 20));
        let switch = app.state.view.mobile_menu_hit_area;
        app.handle_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            switch.x + 1,
            switch.y + 1,
        ));
        assert_eq!(app.state.mode, Mode::Navigate);

        let viewport = crate::ui::mobile_switcher_areas(&app.state).viewport;
        app.handle_mouse(mouse(
            MouseEventKind::Down(MouseButton::Right),
            viewport.x + 2,
            viewport.y + 2,
        ));

        assert_eq!(app.state.mode, Mode::Navigate);
        assert!(app.state.context_menu.is_none());
    }
    #[test]
    fn mobile_switcher_close_returns_to_terminal() {
        let mut app = app_for_mouse_test();
        app.state.workspaces = vec![Workspace::test_new("one")];
        app.state.active = Some(0);
        app.state.selected = 0;
        app.state.mode = Mode::Terminal;

        crate::ui::compute_view(&mut app.state, Rect::new(0, 0, 44, 20));
        let switch = app.state.view.mobile_menu_hit_area;
        app.handle_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            switch.x + 1,
            switch.y + 1,
        ));
        assert_eq!(app.state.mode, Mode::Navigate);

        let close = crate::ui::mobile_switcher_areas(&app.state).close;
        app.handle_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            close.x + 1,
            close.y,
        ));

        assert_eq!(app.state.mode, Mode::Terminal);
    }

    #[test]
    fn double_clicking_agent_name_opens_rename_dialog() {
        let mut app = app_for_mouse_test();
        let workspace = Workspace::test_new("space");
        let pane_id = workspace.tabs[0].root_pane;
        app.state.workspaces = vec![workspace];
        app.state.ensure_test_terminals();
        app.state.active = Some(0);
        app.state.selected = 0;
        let terminal_id = app.state.workspaces[0]
            .pane_state(pane_id)
            .expect("agent pane")
            .attached_terminal_id
            .clone();
        let terminal = app
            .state
            .terminals
            .get_mut(&terminal_id)
            .expect("agent terminal");
        terminal.set_agent_name("codex".into());
        terminal.set_manual_label("worker".into());
        crate::ui::compute_view(&mut app.state, Rect::new(0, 0, 106, 20));

        let row = app.state.view.agent_table.rows[0].clone();
        let name = app.state.view.agent_table.groups[row.group].columns[0];
        let col = name.x;
        app.handle_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            col,
            row.rect.y,
        ));
        app.handle_mouse(mouse(
            MouseEventKind::Up(MouseButton::Left),
            col,
            row.rect.y,
        ));
        app.handle_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            col,
            row.rect.y,
        ));

        assert_eq!(app.state.mode, Mode::RenamePane);
        assert_eq!(app.state.rename_pane_target, Some(pane_id));
        assert_eq!(app.state.name_input, "worker");
    }

    #[test]
    fn double_clicking_another_agent_column_does_not_rename() {
        let mut app = app_for_mouse_test();
        let workspace = Workspace::test_new("space");
        let pane_id = workspace.tabs[0].root_pane;
        app.state.workspaces = vec![workspace];
        app.state.ensure_test_terminals();
        app.state.active = Some(0);
        app.state.selected = 0;
        let terminal_id = app.state.workspaces[0]
            .pane_state(pane_id)
            .expect("agent pane")
            .attached_terminal_id
            .clone();
        app.state
            .terminals
            .get_mut(&terminal_id)
            .expect("agent terminal")
            .set_agent_name("codex".into());
        crate::ui::compute_view(&mut app.state, Rect::new(0, 0, 106, 20));

        let row = app.state.view.agent_table.rows[0].clone();
        let directory = app.state.view.agent_table.groups[row.group].columns[1];
        app.handle_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            directory.x,
            row.rect.y,
        ));
        app.handle_mouse(mouse(
            MouseEventKind::Up(MouseButton::Left),
            directory.x,
            row.rect.y,
        ));
        app.handle_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            directory.x,
            row.rect.y,
        ));

        assert_eq!(app.state.mode, Mode::Terminal);
        assert_eq!(app.state.rename_pane_target, None);
    }

    #[test]
    fn clicking_the_done_marker_acknowledges_that_agent_and_nothing_else() {
        let mut app = app_for_mouse_test();
        let mut workspace = Workspace::test_new("space");
        let focused = workspace.tabs[0].root_pane;
        let finished = workspace.test_split(Direction::Horizontal);
        app.state.workspaces = vec![workspace];
        app.state.ensure_test_terminals();
        app.state.active = Some(0);
        app.state.selected = 0;
        for pane_id in [focused, finished] {
            let terminal_id = app.state.workspaces[0]
                .pane_state(pane_id)
                .expect("agent pane")
                .attached_terminal_id
                .clone();
            app.state
                .terminals
                .get_mut(&terminal_id)
                .expect("agent terminal")
                .set_agent_name("codex".into());
        }
        app.state.workspaces[0].tabs[0].layout.focus_pane(focused);
        let pane = app.state.workspaces[0].tabs[0]
            .panes
            .get_mut(&finished)
            .expect("finished pane");
        pane.seen = false;
        pane.completed = true;
        crate::ui::compute_view(&mut app.state, Rect::new(0, 0, 106, 20));

        let row = app
            .state
            .view
            .agent_table
            .rows
            .iter()
            .find(|row| row.pane_id == finished)
            .expect("finished agent row")
            .clone();
        app.handle_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            row.rect.x,
            row.rect.y,
        ));

        assert!(app.state.workspaces[0].tabs[0].panes[&finished].seen);
        assert!(app.state.workspaces[0].tabs[0].panes[&finished].completed);
        assert_eq!(
            app.state.workspaces[0].focused_pane_id(),
            Some(focused),
            "acknowledging a finish should not also jump to that agent"
        );
    }

    #[test]
    fn clicking_a_row_with_no_done_marker_still_focuses_its_agent() {
        let mut app = app_for_mouse_test();
        let mut workspace = Workspace::test_new("space");
        let focused = workspace.tabs[0].root_pane;
        let other = workspace.test_split(Direction::Horizontal);
        app.state.workspaces = vec![workspace];
        app.state.ensure_test_terminals();
        app.state.active = Some(0);
        app.state.selected = 0;
        for pane_id in [focused, other] {
            let terminal_id = app.state.workspaces[0]
                .pane_state(pane_id)
                .expect("agent pane")
                .attached_terminal_id
                .clone();
            app.state
                .terminals
                .get_mut(&terminal_id)
                .expect("agent terminal")
                .set_agent_name("codex".into());
        }
        app.state.workspaces[0].tabs[0].layout.focus_pane(focused);
        crate::ui::compute_view(&mut app.state, Rect::new(0, 0, 106, 20));

        let row = app
            .state
            .view
            .agent_table
            .rows
            .iter()
            .find(|row| row.pane_id == other)
            .expect("agent row")
            .clone();
        app.handle_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            row.rect.x,
            row.rect.y,
        ));

        assert_eq!(app.state.workspaces[0].focused_pane_id(), Some(other));
    }

    #[test]
    fn clicking_a_set_down_agent_reopens_it_in_the_focused_pane() {
        let mut app = app_for_mouse_test();
        let mut workspace = Workspace::test_new("space");
        let target = workspace.tabs[0].root_pane;
        let pane_id = workspace.test_split(Direction::Horizontal);
        app.state.workspaces = vec![workspace];
        app.state.ensure_test_terminals();
        app.state.active = Some(0);
        app.state.selected = 0;
        let terminal_id = app.state.workspaces[0]
            .pane_state(pane_id)
            .expect("agent pane")
            .attached_terminal_id
            .clone();
        app.state
            .terminals
            .get_mut(&terminal_id)
            .expect("agent terminal")
            .set_agent_name("codex".into());
        app.state.workspaces[0].layout.focus_pane(pane_id);
        app.state.close_pane();
        app.state.workspaces[0].layout.focus_pane(target);
        crate::ui::compute_view(&mut app.state, Rect::new(0, 0, 106, 20));

        let row = app
            .state
            .view
            .agent_table
            .rows
            .iter()
            .find(|row| row.pane_id == pane_id)
            .expect("set-down agent row")
            .clone();
        app.handle_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            row.rect.x + 1,
            row.rect.y,
        ));
        app.handle_mouse(mouse(
            MouseEventKind::Up(MouseButton::Left),
            row.rect.x + 1,
            row.rect.y,
        ));

        assert!(app.state.detached_agents.is_empty());
        assert_eq!(app.state.workspaces[0].focused_pane_id(), Some(pane_id));
        assert_eq!(
            app.state.workspaces[0]
                .pane_state(pane_id)
                .map(|pane| pane.attached_terminal_id.clone()),
            Some(terminal_id)
        );
    }

    #[test]
    fn dragging_an_agent_row_reorders_rows_without_moving_panes() {
        let mut app = app_for_mouse_test();
        let mut workspace = Workspace::test_new("space");
        let first = workspace.tabs[0].root_pane;
        let second = workspace.test_split(Direction::Horizontal);
        let pane_layout_before = workspace.natural_pane_order();
        app.state.workspaces = vec![workspace];
        app.state.ensure_test_terminals();
        app.state.active = Some(0);
        app.state.selected = 0;
        for pane_id in [first, second] {
            let terminal_id = app.state.workspaces[0]
                .pane_state(pane_id)
                .expect("agent pane")
                .attached_terminal_id
                .clone();
            app.state
                .terminals
                .get_mut(&terminal_id)
                .expect("agent terminal")
                .set_agent_name("codex".into());
        }
        crate::ui::compute_view(&mut app.state, Rect::new(0, 0, 106, 20));
        let rows = app.state.view.agent_table.rows.clone();

        app.handle_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            rows[0].rect.x + 2,
            rows[0].rect.y,
        ));
        app.handle_mouse(mouse(
            MouseEventKind::Drag(MouseButton::Left),
            rows[1].rect.x + 2,
            rows[1].rect.y,
        ));
        assert!(matches!(
            app.state.drag.as_ref().map(|drag| &drag.target),
            Some(DragTarget::AgentReorder {
                insert_idx: Some(2),
                ..
            })
        ));
        app.handle_mouse(mouse(
            MouseEventKind::Up(MouseButton::Left),
            rows[1].rect.x + 2,
            rows[1].rect.y,
        ));

        assert_eq!(
            crate::ui::agent_panel_entries(&app.state)
                .iter()
                .map(|entry| entry.pane_id)
                .collect::<Vec<_>>(),
            vec![second, first]
        );
        assert_eq!(
            app.state.workspaces[0].natural_pane_order(),
            pane_layout_before
        );
    }

    #[test]
    fn wheel_routing_uses_alternate_scroll_in_fullscreen_without_mouse_reporting() {
        let input_state = crate::pane::InputState {
            alternate_screen: true,
            application_cursor: false,
            bracketed_paste: false,
            focus_reporting: false,
            mouse_protocol_mode: crate::input::MouseProtocolMode::None,
            mouse_protocol_encoding: crate::input::MouseProtocolEncoding::Default,
            mouse_alternate_scroll: true,
            modify_other_keys: false,
        };

        assert_eq!(wheel_routing(input_state), WheelRouting::AlternateScroll);
    }

    #[test]
    fn wheel_routing_falls_back_to_host_scrollback() {
        let input_state = crate::pane::InputState {
            alternate_screen: false,
            application_cursor: false,
            bracketed_paste: false,
            focus_reporting: false,
            mouse_protocol_mode: crate::input::MouseProtocolMode::None,
            mouse_protocol_encoding: crate::input::MouseProtocolEncoding::Default,
            mouse_alternate_scroll: true,
            modify_other_keys: false,
        };

        assert_eq!(wheel_routing(input_state), WheelRouting::HostScroll);
    }

    #[test]
    fn clicking_away_from_a_typed_directory_settles_it() {
        let mut app = app_for_mouse_test();
        app.state.workspaces = vec![Workspace::test_new("space")];
        app.state.active = Some(0);
        app.state.mode = Mode::Composer;
        app.state
            .composer
            .add_folder(std::path::PathBuf::from("/tmp"));
        app.state
            .composer
            .open_dropdown(crate::composer::Focus::Folder);
        app.state.composer.edit_path(|path| path.set_text("/usr"));
        crate::ui::compute_view(&mut app.state, Rect::new(0, 0, 120, 30));

        let task = app.state.view.composer.task;
        app.handle_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            task.x + 4,
            task.y,
        ));

        assert_eq!(
            app.state.composer.folder_path(),
            Some(std::path::Path::new("/usr")),
            "the typed path should be kept the way Enter keeps it"
        );
        assert_eq!(app.state.composer.open, None);
        assert_eq!(app.state.composer.focus, crate::composer::Focus::Task);
    }

    #[test]
    fn clicking_a_pane_while_typing_a_directory_settles_it() {
        let mut app = app_for_mouse_test();
        app.state.workspaces = vec![Workspace::test_new("space")];
        app.state.active = Some(0);
        app.state.mode = Mode::Composer;
        app.state
            .composer
            .add_folder(std::path::PathBuf::from("/tmp"));
        app.state
            .composer
            .open_dropdown(crate::composer::Focus::Folder);
        app.state.composer.edit_path(|path| path.set_text("/usr"));
        crate::ui::compute_view(&mut app.state, Rect::new(0, 0, 120, 30));

        let pane = app.state.view.pane_infos[0].inner_rect;
        app.handle_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            pane.x + 1,
            pane.y + 1,
        ));

        assert_eq!(
            app.state.composer.folder_path(),
            Some(std::path::Path::new("/usr"))
        );
        assert_eq!(app.state.composer.open, None);
        assert_eq!(app.state.mode, Mode::Terminal);
    }

    #[test]
    fn clicking_away_from_an_unsettled_path_keeps_the_list_open() {
        let mut app = app_for_mouse_test();
        app.state.workspaces = vec![Workspace::test_new("space")];
        app.state.active = Some(0);
        app.state.mode = Mode::Composer;
        app.state
            .composer
            .add_folder(std::path::PathBuf::from("/tmp"));
        app.state
            .composer
            .open_dropdown(crate::composer::Focus::Folder);
        app.state
            .composer
            .edit_path(|path| path.set_text("/definitely/not/here"));
        crate::ui::compute_view(&mut app.state, Rect::new(0, 0, 120, 30));

        let task = app.state.view.composer.task;
        app.handle_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            task.x + 4,
            task.y,
        ));

        assert_eq!(
            app.state.composer.folder_path(),
            Some(std::path::Path::new("/tmp")),
            "an unusable path is not kept"
        );
        assert_eq!(
            app.state.composer.open,
            Some(crate::composer::Focus::Folder)
        );
        assert_eq!(app.state.composer.path().text(), "/definitely/not/here");
    }

    #[test]
    fn clicking_a_listed_folder_still_takes_that_row() {
        let unique = format!(
            "herdr-composer-click-row-{}-{}",
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
        let root = root.canonicalize().unwrap();

        let mut app = app_for_mouse_test();
        app.state.workspaces = vec![Workspace::test_new("space")];
        app.state.active = Some(0);
        app.state.mode = Mode::Composer;
        app.state
            .composer
            .open_dropdown(crate::composer::Focus::Folder);
        app.state.composer.edit_path(|path| {
            path.set_text(&format!("{}/her", root.display()));
        });
        crate::ui::compute_view(&mut app.state, Rect::new(0, 0, 120, 30));

        let row = app.state.view.composer.dropdown_rows[1];
        let col = app.state.view.composer.dropdown.x;
        app.handle_mouse(mouse(MouseEventKind::Down(MouseButton::Left), col, row));

        assert_eq!(
            app.state.composer.folder_path(),
            Some(root.join("herdr-old").as_path())
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn a_click_in_the_task_field_moves_the_cursor_to_the_character_clicked() {
        let mut app = app_for_mouse_test();
        app.state.workspaces = vec![Workspace::test_new("space")];
        app.state.active = Some(0);
        app.state.mode = Mode::Composer;
        app.state
            .composer
            .add_folder(std::path::PathBuf::from("/tmp"));
        app.state.composer.task.set_text("fix the tests");
        crate::ui::compute_view(&mut app.state, Rect::new(0, 0, 120, 30));

        let task = app.state.view.composer.task;
        let lead = app.state.view.composer.task_lead;
        app.handle_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            task.x + lead + 4,
            task.y,
        ));

        assert_eq!(app.state.composer.focus, crate::composer::Focus::Task);
        assert_eq!(
            app.state.composer.task.cursor_row(),
            (0, 4),
            "the cursor left the end of the text for the character clicked"
        );
    }

    #[test]
    fn moving_over_composer_dropdown_rows_sets_hover_without_selecting() {
        let mut app = app_for_mouse_test();
        app.state.workspaces = vec![Workspace::test_new("space")];
        app.state.active = Some(0);
        app.state.mode = Mode::Composer;
        app.state
            .composer
            .add_folder(std::path::PathBuf::from("/first"));
        app.state
            .composer
            .add_folder(std::path::PathBuf::from("/second"));
        app.state
            .composer
            .open_dropdown(crate::composer::Focus::Folder);
        crate::ui::compute_view(&mut app.state, Rect::new(0, 0, 120, 30));

        let folder_row = app.state.view.composer.dropdown_rows[1];
        let dropdown_col = app.state.view.composer.dropdown.x;
        app.handle_mouse(mouse(MouseEventKind::Moved, dropdown_col, folder_row));

        assert_eq!(app.state.composer.hover, Some(1));
        assert_eq!(app.state.composer.highlight, 0);
        assert_eq!(
            app.state.composer.open,
            Some(crate::composer::Focus::Folder)
        );

        app.state
            .composer
            .use_harnesses(vec![&crate::harness::ALL[0], &crate::harness::ALL[1]]);
        app.state
            .composer
            .open_dropdown(crate::composer::Focus::Agent);
        crate::ui::compute_view(&mut app.state, Rect::new(0, 0, 120, 30));
        let agent_row = app.state.view.composer.dropdown_rows[1];
        let dropdown_col = app.state.view.composer.dropdown.x;
        app.handle_mouse(mouse(MouseEventKind::Moved, dropdown_col, agent_row));

        assert_eq!(app.state.composer.hover, Some(1));
        assert_eq!(app.state.composer.highlight, 0);
        assert_eq!(app.state.composer.open, Some(crate::composer::Focus::Agent));
    }

    #[test]
    fn clicking_the_worktree_box_flips_it_and_leaves_the_keyboard() {
        let mut app = app_for_mouse_test();
        app.state.workspaces = vec![Workspace::test_new("space")];
        app.state.active = Some(0);
        app.state.mode = Mode::Composer;
        app.state.composer.focus = crate::composer::Focus::Task;
        crate::ui::compute_view(&mut app.state, Rect::new(0, 0, 120, 30));

        let worktree = app.state.view.composer.worktree;
        assert!(worktree.width > 0, "the box has to be on the caption row");
        assert!(app.state.composer.worktree);

        app.handle_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            worktree.x,
            worktree.y,
        ));

        assert!(!app.state.composer.worktree);
        assert_eq!(app.state.mode, Mode::Composer);
        assert_eq!(app.state.composer.focus, crate::composer::Focus::Task);

        app.handle_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            worktree.x + worktree.width - 1,
            worktree.y,
        ));
        assert!(app.state.composer.worktree);
    }

    #[test]
    fn top_row_menu_opens_while_task_field_is_selected() {
        let mut app = app_for_mouse_test();
        app.state.workspaces = vec![Workspace::test_new("space")];
        app.state.active = Some(0);
        app.state.mode = Mode::Composer;
        app.state.composer.focus = crate::composer::Focus::Task;
        crate::ui::compute_view(&mut app.state, Rect::new(0, 0, 120, 30));

        let launcher = app.state.global_launcher_rect();
        assert_eq!(launcher.y, 0);
        app.handle_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            launcher.x + launcher.width.saturating_sub(1),
            launcher.y,
        ));

        assert_eq!(app.state.mode, Mode::GlobalMenu);
    }
}
