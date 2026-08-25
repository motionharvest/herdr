use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::Span,
    Frame,
};

mod agent_table;
mod composer;
mod dialogs;
mod keybind_help;
mod menus;
mod mobile;
mod navigator;
mod onboarding;
mod panes;
mod release_notes;
mod scrollbar;
mod settings;
mod sidebar;
mod status;
mod widgets;

use self::agent_table::render_global_launcher;
pub(crate) use self::agent_table::{
    agent_panel_entries, agent_panel_entries_from, compute_agent_locations, render_agent_table,
    sort_agent_table_by_column, split_agent_table, AgentLocation, AgentTableLayout, AgentTableRow,
};
pub(crate) use self::composer::ComposerLayout;
use self::composer::{render_composer, render_composer_dropdown};
use self::dialogs::{
    render_confirm_close_agent_overlay, render_confirm_close_overlay, render_land_worktree_overlay,
    render_new_linked_worktree_overlay, render_open_existing_worktree_overlay,
    render_remove_worktree_overlay, render_rename_overlay,
};
use self::keybind_help::render_keybind_help_overlay;
use self::menus::{
    render_context_menu, render_copy_mode_overlay, render_global_launcher_menu,
    render_navigate_overlay, render_prefix_overlay, render_resize_overlay,
};
use self::mobile::{
    compute_mobile_header_hit_areas, is_mobile_width, mobile_switcher_max_scroll_for_height,
    mobile_toast_banner_rect, render_mobile_header, render_mobile_panel,
    render_mobile_toast_banner,
};
use self::navigator::render_navigator_overlay;
pub(crate) use self::onboarding::onboarding_welcome_continue_rect;
use self::onboarding::render_onboarding_overlay;
use self::panes::{compute_pane_infos, render_panes, resize_tab_panes};
pub(crate) use self::release_notes::{
    product_announcement_display_lines, release_notes_close_button_rect,
    release_notes_display_lines, release_notes_wrapped_line_count, PRODUCT_ANNOUNCEMENT_MODAL_SIZE,
    RELEASE_NOTES_MODAL_SIZE,
};
use self::release_notes::{render_product_announcement_overlay, render_release_notes_overlay};
pub(crate) use self::scrollbar::{
    pane_scrollbar_rect, release_notes_scrollbar_rect, scrollbar_offset_from_drag_row,
    scrollbar_offset_from_row, scrollbar_thumb_grab_offset, should_show_scrollbar,
};
use self::settings::render_settings_overlay;
pub(crate) use self::sidebar::{
    agent_folder_position, collapsed_sidebar_sections, collapsed_sidebar_toggle_rect,
    compute_workspace_card_areas, compute_workspace_list_areas, expanded_sidebar_toggle_rect,
    new_workspace_button_rect, normalized_workspace_scroll, render_sidebar,
    spaces_section_collapsed, spaces_section_header_rect, workspace_agent_groups,
    workspace_agents_expanded, workspace_drop_indicator_row, workspace_list_entries,
    workspace_list_rect, workspace_list_scroll_metrics, workspace_list_scrollbar_rect,
    workspace_parent_group_state, AgentFolderGroup, WorkspaceListEntry,
};
pub(crate) use self::status::config_diagnostic_dismiss_rect;
use self::status::{
    render_config_diagnostic, render_copy_feedback, render_toast_notification,
    toast_notification_rect,
};
pub(crate) use self::{
    composer::split_composer,
    keybind_help::keybind_help_lines,
    mobile::{
        mobile_switcher_areas, mobile_switcher_max_scroll, mobile_switcher_target_at,
        mobile_switcher_workspace_doc_range, MobileSwitcherTarget,
    },
    panes::{cursor_hidden_by_host_focus, pane_is_scrolled_back},
    widgets::{centered_popup_rect, modal_stack_areas},
};
pub(crate) use self::{
    dialogs::{
        confirm_close_agent_button_rects, confirm_close_agent_popup_rect,
        confirm_close_button_rects, confirm_close_popup_rect, land_worktree_close_rect,
        land_worktree_popup_rect, new_linked_worktree_button_rects, new_linked_worktree_inner_rect,
        open_existing_worktree_button_rects, open_existing_worktree_inner_rect,
        open_existing_worktree_max_visible_rows, open_existing_worktree_visible_start,
        remove_worktree_button_rects, remove_worktree_popup_rect, rename_button_rects,
    },
    settings::{
        settings_button_rects, settings_show_primary_action, SOUND_ALERT_ROWS_OFFSET,
        SOUND_CHOICE_ROWS_OFFSET,
    },
};
use crate::app::state::ViewLayout;
use crate::app::{AppState, Mode};
use crate::terminal::TerminalRuntimeRegistry;

/// The `arc` set from FGRibreau's spinners: an arc segment sweeping around a
/// circle. Not braille, on purpose — a braille cell is drawn small and faint in
/// many terminal fonts, which turns a spinner into a character that twitches
/// rather than a shape that turns. These six frames are quadrant arcs, which
/// every font draws at full weight.
const SPINNERS: &[&str] = &["◜", "◠", "◝", "◞", "◡", "◟"];

/// Map spinner_tick, which counts 16ms animation ticks, to a spinner frame. The
/// set holds one frame per 80ms, so every fifth tick turns it.
pub(super) fn spinner_frame(tick: u32) -> &'static str {
    SPINNERS[(tick / crate::app::ANIMATION_TICKS_PER_FRAME) as usize % SPINNERS.len()]
}

/// Compute view geometry and reconcile pane sizes.
/// Called before render to separate mutation from drawing.
#[cfg_attr(not(test), allow(dead_code))]
pub fn compute_view(app: &mut AppState, area: Rect) {
    let terminal_runtimes = TerminalRuntimeRegistry::new();
    compute_view_with_runtime_registry(app, &terminal_runtimes, area);
}

pub fn compute_view_with_runtime_registry(
    app: &mut AppState,
    terminal_runtimes: &TerminalRuntimeRegistry,
    area: Rect,
) {
    compute_view_internal(
        app,
        terminal_runtimes,
        area,
        true,
        crate::kitty_graphics::HostCellSize::default(),
    );
}

pub fn compute_view_with_cell_size(
    app: &mut AppState,
    terminal_runtimes: &TerminalRuntimeRegistry,
    area: Rect,
    cell_size: crate::kitty_graphics::HostCellSize,
) {
    compute_view_internal(app, terminal_runtimes, area, true, cell_size);
}

/// Compute view geometry for a client-sized render without resizing pane runtimes.
///
/// This is used by the headless server when a non-foreground client needs its
/// own frame size while the shared pane runtimes stay pinned to the foreground
/// client.
pub(crate) fn compute_view_without_resizing_panes(
    app: &mut AppState,
    terminal_runtimes: &TerminalRuntimeRegistry,
    area: Rect,
) {
    compute_view_internal(
        app,
        terminal_runtimes,
        area,
        false,
        crate::kitty_graphics::HostCellSize::default(),
    );
}

fn resize_background_tab_panes_to_terminal_area(
    app: &AppState,
    terminal_runtimes: &TerminalRuntimeRegistry,
    terminal_area: Rect,
    cell_size: crate::kitty_graphics::HostCellSize,
) {
    for (ws_idx, ws) in app.workspaces.iter().enumerate() {
        for (tab_idx, tab) in ws.tabs.iter().enumerate() {
            if app.active == Some(ws_idx) && tab_idx == ws.active_tab_index() {
                continue;
            }
            resize_tab_panes(app, terminal_runtimes, tab, terminal_area, cell_size);
        }
    }
}

fn compute_view_internal(
    app: &mut AppState,
    terminal_runtimes: &TerminalRuntimeRegistry,
    area: Rect,
    resize_panes: bool,
    cell_size: crate::kitty_graphics::HostCellSize,
) {
    // Agent order is session state, not geometry. Capture newcomers before
    // either desktop or mobile computes rows so later pane rearrangement can
    // never feed back into the list.
    agent_table::sync_agent_order(app);

    if is_mobile_width(area, app.mobile_width_threshold) {
        // Mobile has no sidebar, so the composer still spans the frame.
        let (composer, area) = split_composer(app, area);
        compute_mobile_view(
            app,
            terminal_runtimes,
            area,
            composer,
            resize_panes,
            cell_size,
        );
        return;
    }

    app.view.agent_locations = compute_agent_locations(app, terminal_runtimes);

    let sidebar_width = desktop_sidebar_width(app, area.width);
    let sidebar_rect = if sidebar_width > 0 {
        Rect::new(area.x, area.y, sidebar_width, area.height)
    } else {
        Rect::default()
    };
    let main_area = Rect::new(
        area.x.saturating_add(sidebar_width),
        area.y,
        area.width.saturating_sub(sidebar_width),
        area.height,
    );

    app.workspace_scroll = normalized_workspace_scroll(app, sidebar_rect, app.workspace_scroll);

    let (composer, main_area) = split_composer(app, main_area);
    let (agent_table, terminal_area) = split_agent_table(app, main_area);

    let split_borders = if app.agent_peek.is_some() {
        Vec::new()
    } else {
        app.active
            .and_then(|i| app.workspaces.get(i))
            .map(|ws| ws.layout.splits(terminal_area))
            .unwrap_or_default()
    };

    let pane_infos = compute_pane_infos(
        app,
        terminal_runtimes,
        terminal_area,
        resize_panes,
        cell_size,
    );
    if resize_panes {
        resize_background_tab_panes_to_terminal_area(
            app,
            terminal_runtimes,
            terminal_area,
            cell_size,
        );
    }

    let toast_hit_area = app
        .toast
        .as_ref()
        .map(|toast| toast_notification_rect(terminal_area, toast, app.config_diagnostic.is_some()))
        .unwrap_or_default();

    let (workspace_card_areas, agent_row_areas, agent_folder_areas) = if app.sidebar_collapsed {
        (Vec::new(), Vec::new(), Vec::new())
    } else {
        compute_workspace_list_areas(app, sidebar_rect)
    };
    let agent_locations = std::mem::take(&mut app.view.agent_locations);
    app.view = crate::app::ViewState {
        layout: ViewLayout::Desktop,
        composer,
        sidebar_rect,
        workspace_card_areas,
        agent_row_areas,
        agent_folder_areas,
        agent_table,
        agent_locations,
        terminal_area,
        mobile_header_rect: Rect::default(),
        mobile_menu_hit_area: Rect::default(),
        toast_hit_area,
        pane_infos,
        pane_chrome_controls: Vec::new(),
        pane_title_hit_areas: Vec::new(),
        split_borders,
    };
    app.view.pane_chrome_controls = self::panes::compute_pane_chrome_controls(app);
    app.view.pane_title_hit_areas = self::panes::compute_pane_title_hit_areas(app);
}

fn desktop_sidebar_width(app: &AppState, total_width: u16) -> u16 {
    if total_width == 0 {
        return 0;
    }

    const MIN_MAIN_WIDTH: u16 = 20;

    let max_width = if total_width > MIN_MAIN_WIDTH {
        total_width - MIN_MAIN_WIDTH
    } else if total_width > 1 {
        1
    } else {
        total_width
    };
    let desired_width = if app.sidebar_collapsed {
        1
    } else {
        app.sidebar_width
            .clamp(app.sidebar_min_width, app.sidebar_max_width)
    };
    desired_width.min(max_width)
}

fn compute_mobile_view(
    app: &mut AppState,
    terminal_runtimes: &TerminalRuntimeRegistry,
    area: Rect,
    composer: crate::ui::composer::ComposerLayout,
    resize_panes: bool,
    cell_size: crate::kitty_graphics::HostCellSize,
) {
    let header_h = area.height.min(2);
    let (header_rect, terminal_area) = if area.height > header_h {
        let [header_rect, terminal_area] =
            Layout::vertical([Constraint::Length(header_h), Constraint::Min(1)]).areas(area);
        (header_rect, terminal_area)
    } else {
        (area, Rect::default())
    };

    if app.mode == Mode::Navigate {
        let switcher_viewport_h = area.height.saturating_sub(header_h + 1);
        let max_scroll = mobile_switcher_max_scroll_for_height(app, switcher_viewport_h);
        app.mobile_switcher_scroll = app.mobile_switcher_scroll.min(max_scroll);
    }

    let split_borders = if app.agent_peek.is_some() {
        Vec::new()
    } else {
        app.active
            .and_then(|i| app.workspaces.get(i))
            .map(|ws| ws.layout.splits(terminal_area))
            .unwrap_or_default()
    };

    let pane_infos = compute_pane_infos(
        app,
        terminal_runtimes,
        terminal_area,
        resize_panes,
        cell_size,
    );
    if resize_panes {
        resize_background_tab_panes_to_terminal_area(
            app,
            terminal_runtimes,
            terminal_area,
            cell_size,
        );
    }
    let header_hits = compute_mobile_header_hit_areas(app, header_rect);

    let toast_hit_area = app
        .toast
        .as_ref()
        .map(|_| mobile_toast_banner_rect(area, app.config_diagnostic.is_some()))
        .unwrap_or_default();

    app.view = crate::app::ViewState {
        layout: ViewLayout::Mobile,
        composer,
        sidebar_rect: Rect::default(),
        workspace_card_areas: Vec::new(),
        agent_row_areas: Vec::new(),
        agent_folder_areas: Vec::new(),
        agent_table: crate::ui::AgentTableLayout::default(),
        agent_locations: std::collections::HashMap::new(),
        terminal_area,
        mobile_header_rect: header_rect,
        mobile_menu_hit_area: header_hits.menu,
        toast_hit_area,
        pane_infos,
        pane_chrome_controls: Vec::new(),
        pane_title_hit_areas: Vec::new(),
        split_borders,
    };
    app.view.pane_chrome_controls = self::panes::compute_pane_chrome_controls(app);
    app.view.pane_title_hit_areas = self::panes::compute_pane_title_hit_areas(app);
}

/// Render the UI — reads AppState but does not mutate it.
#[cfg_attr(not(test), allow(dead_code))]
pub fn render(app: &AppState, frame: &mut Frame) {
    let terminal_runtimes = TerminalRuntimeRegistry::new();
    render_with_runtime_registry(app, &terminal_runtimes, frame);
}

pub fn render_with_runtime_registry(
    app: &AppState,
    terminal_runtimes: &TerminalRuntimeRegistry,
    frame: &mut Frame,
) {
    let terminal_area = app.view.terminal_area;

    render_composer(app, frame, &app.view.composer);

    if app.view.layout == ViewLayout::Mobile {
        render_mobile_header(app, terminal_runtimes, frame, app.view.mobile_header_rect);
    }
    if app.view.layout != ViewLayout::Mobile {
        render_sidebar(app, terminal_runtimes, frame, app.view.sidebar_rect);
        let entries = agent_panel_entries_from(app, terminal_runtimes);
        render_agent_table(app, frame, &app.view.agent_table, &entries);
        render_global_launcher(app, frame);
    }
    render_panes(app, terminal_runtimes, frame, terminal_area);

    // Ambient notifications sit above panes, but below interactive overlays.
    render_notifications(app, frame, terminal_area);

    match app.mode {
        Mode::Onboarding => render_onboarding_overlay(app, frame, frame.area()),
        Mode::ReleaseNotes => render_release_notes_overlay(app, frame, frame.area()),
        Mode::ProductAnnouncement => render_product_announcement_overlay(app, frame, frame.area()),
        Mode::Navigate if app.view.layout == ViewLayout::Mobile => {
            render_mobile_panel(app, terminal_runtimes, frame, frame.area())
        }
        Mode::Navigate => render_navigate_overlay(app, frame, terminal_area),
        Mode::Prefix => render_prefix_overlay(app, frame, terminal_area),
        Mode::Copy => render_copy_mode_overlay(app, frame, terminal_area),
        Mode::Resize => render_resize_overlay(app, frame, terminal_area),
        Mode::ConfirmClose => render_confirm_close_overlay(app, frame, terminal_area),
        Mode::ConfirmCloseAgent => render_confirm_close_agent_overlay(app, frame, terminal_area),
        Mode::ContextMenu => {
            render_context_menu(app, frame);
        }
        Mode::Settings => render_settings_overlay(app, frame, frame.area()),
        Mode::RenameWorkspace | Mode::RenamePane | Mode::UpdateSummary => {
            render_rename_overlay(app, frame, frame.area())
        }
        Mode::NewLinkedWorktree => render_new_linked_worktree_overlay(app, frame, frame.area()),
        Mode::OpenExistingWorktree => {
            render_open_existing_worktree_overlay(app, frame, frame.area())
        }
        Mode::ConfirmRemoveWorktree => render_remove_worktree_overlay(app, frame, frame.area()),
        Mode::WorktreeLand => render_land_worktree_overlay(app, frame, frame.area()),
        Mode::GlobalMenu => render_global_launcher_menu(app, frame),
        Mode::KeybindHelp => render_keybind_help_overlay(app, frame),
        Mode::Navigator => render_navigator_overlay(app, terminal_runtimes, frame),
        // The composer band is chrome, not an overlay: it draws itself above.
        Mode::Composer | Mode::Terminal => {}
    }

    // A config warning outranks whatever is open: a broken config is the one
    // fact the user must be able to see from anywhere, so it is drawn over
    // every overlay rather than under them.
    if let Some(message) = &app.config_diagnostic {
        render_config_diagnostic(frame, terminal_area, message, &app.palette);
    }

    // Its open list is an overlay, though, and hangs over the panes — so it
    // goes on last, after everything it covers has been drawn.
    render_composer_dropdown(app, frame, &app.view.composer);
}

fn render_notifications(app: &AppState, frame: &mut Frame, terminal_area: Rect) {
    let has_config_diagnostic = app.config_diagnostic.is_some();
    let mut copy_feedback_offset = u16::from(has_config_diagnostic);
    if let Some(toast) = &app.toast {
        if app.view.layout == ViewLayout::Mobile {
            render_mobile_toast_banner(
                frame,
                frame.area(),
                toast,
                has_config_diagnostic,
                &app.palette,
            );
        } else {
            render_toast_notification(
                frame,
                terminal_area,
                toast,
                has_config_diagnostic,
                &app.palette,
            );
        }
        copy_feedback_offset =
            copy_feedback_offset.saturating_add(if app.view.layout == ViewLayout::Mobile {
                1
            } else {
                toast_notification_rect(terminal_area, toast, has_config_diagnostic).height
            });
    }
    if let Some(feedback) = &app.copy_feedback {
        let area = if app.view.layout == ViewLayout::Mobile {
            frame.area()
        } else {
            terminal_area
        };
        render_copy_feedback(frame, area, feedback, copy_feedback_offset, &app.palette);
    }
}

fn dim_background(frame: &mut Frame, area: Rect) {
    let buf = frame.buffer_mut();
    for y in area.y..area.y + area.height {
        for x in area.x..area.x + area.width {
            let cell = &mut buf[(x, y)];
            cell.set_style(cell.style().add_modifier(Modifier::DIM));
        }
    }
}

/// Floating overlay for navigate mode — appears at bottom of terminal area.
fn _build_hints(items: &[(&str, &str)], key_style: Style, dim_style: Style) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    spans.push(Span::raw(" "));
    for (i, (k, desc)) in items.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled("  ", dim_style));
        }
        spans.push(Span::styled(k.to_string(), key_style));
        spans.push(Span::styled(format!(" {desc}"), dim_style));
    }
    spans
}

#[cfg(test)]
mod tests {
    use super::keybind_help::keybind_help_groups;
    use super::scrollbar::scrollbar_thumb;
    use super::*;
    use crate::{app::state::ViewLayout, layout::PaneInfo, workspace::Workspace};
    use ratatui::{backend::TestBackend, Terminal};

    #[tokio::test]
    async fn focused_pane_cursor_wins_during_terminal_render() {
        let mut app = crate::app::state::AppState::test_new();
        let mut ws = Workspace::test_new("test");
        let first_pane = ws.tabs[0].root_pane;
        let second_pane = ws.test_split(ratatui::layout::Direction::Horizontal);

        ws.insert_test_runtime(
            first_pane,
            crate::terminal::TerminalRuntime::test_with_screen_bytes(20, 5, b"left"),
        );
        ws.insert_test_runtime(
            second_pane,
            crate::terminal::TerminalRuntime::test_with_screen_bytes(20, 5, b"r\r\nb"),
        );
        ws.tabs[0].layout.focus_pane(first_pane);

        app.workspaces = vec![ws];
        app.active = Some(0);
        app.selected = 0;
        app.mode = Mode::Terminal;

        compute_view(&mut app, Rect::new(0, 0, 80, 20));
        let focused = app
            .view
            .pane_infos
            .iter()
            .find(|info| info.id == first_pane)
            .expect("focused pane info");

        let backend = TestBackend::new(80, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| render(&app, frame)).unwrap();

        terminal
            .backend_mut()
            .assert_cursor_position((focused.inner_rect.x + 4, focused.inner_rect.y));
    }
    #[test]
    fn configured_mobile_width_threshold_controls_layout_switch() {
        let mut app = crate::app::state::AppState::test_new();
        app.workspaces = vec![Workspace::test_new("one")];
        app.active = Some(0);
        app.selected = 0;
        app.mode = Mode::Terminal;

        compute_view(&mut app, Rect::new(0, 0, 80, 20));
        assert_eq!(app.view.layout, ViewLayout::Desktop);

        app.mobile_width_threshold = 90;
        compute_view(&mut app, Rect::new(0, 0, 80, 20));
        assert_eq!(app.view.layout, ViewLayout::Mobile);
        assert_eq!(app.view.mobile_header_rect, Rect::new(0, 4, 80, 2));
        assert_eq!(app.view.terminal_area, Rect::new(0, 6, 80, 14));
    }

    #[test]
    fn desktop_top_row_draws_menu_at_far_right() {
        let mut app = crate::app::state::AppState::test_new();
        app.workspaces = vec![Workspace::test_new("one")];
        app.active = Some(0);
        app.selected = 0;
        app.mode = Mode::Terminal;
        app.mouse_capture = true;

        compute_view(&mut app, Rect::new(0, 0, 100, 20));
        let launcher = app.global_launcher_rect();
        assert_eq!(launcher.x + launcher.width, 100);
        assert_eq!(launcher.y, 0);

        let mut terminal = Terminal::new(TestBackend::new(100, 20)).expect("test terminal");
        terminal
            .draw(|frame| render(&app, frame))
            .expect("draw desktop UI");

        let rendered = (launcher.x..launcher.x + launcher.width)
            .map(|x| {
                terminal
                    .backend()
                    .buffer()
                    .cell((x, launcher.y))
                    .expect("launcher cell")
                    .symbol()
            })
            .collect::<String>();
        assert_eq!(rendered.trim(), "menu");
    }

    #[test]
    fn pane_scrollbar_rect_uses_reserved_rightmost_column() {
        let info = PaneInfo {
            id: crate::layout::PaneId::from_raw(1),
            rect: Rect::new(0, 0, 12, 8),
            inner_rect: Rect::new(1, 1, 9, 6),
            scrollbar_rect: Some(Rect::new(10, 1, 1, 6)),
            is_focused: true,
            exposed: crate::layout::ExposedSides::all(),
        };

        assert_eq!(pane_scrollbar_rect(&info), Some(Rect::new(10, 1, 1, 6)));
    }

    #[tokio::test]
    async fn compute_view_reserves_terminal_column_when_pane_scrollbar_is_visible() {
        let mut app = crate::app::state::AppState::test_new();
        let mut ws = Workspace::test_new("test");
        let pane_id = ws.tabs[0].root_pane;
        ws.insert_test_runtime(
            pane_id,
            crate::terminal::TerminalRuntime::test_with_scrollback_bytes(
                12,
                4,
                4096,
                b"000000000000\r\n111111111111\r\n222222222222\r\n333333333333\r\n444444444444\r\n",
            ),
        );

        app.workspaces = vec![ws];
        app.active = Some(0);
        app.selected = 0;

        compute_view(&mut app, Rect::new(0, 0, 40, 12));

        let info = app.view.pane_infos.first().expect("pane info");
        assert_eq!(
            info.inner_rect.width + 3,
            app.view.terminal_area.width,
            "terminal width leaves room for the rounded frame and scrollbar gutter"
        );
        assert_eq!(
            info.scrollbar_rect,
            Some(Rect::new(
                info.inner_rect.x + info.inner_rect.width,
                info.inner_rect.y,
                1,
                info.inner_rect.height,
            ))
        );
    }

    #[test]
    fn scrollbar_stays_hidden_without_scrollback() {
        let metrics = crate::pane::ScrollMetrics {
            offset_from_bottom: 0,
            max_offset_from_bottom: 0,
            viewport_rows: 5,
        };

        assert!(!self::scrollbar::should_show_scrollbar(metrics));
    }

    #[test]
    fn scrollbar_shows_with_scrollback() {
        let metrics = crate::pane::ScrollMetrics {
            offset_from_bottom: 0,
            max_offset_from_bottom: 20,
            viewport_rows: 5,
        };

        assert!(self::scrollbar::should_show_scrollbar(metrics));
    }

    #[test]
    fn scrollbar_thumb_reaches_bottom_when_scrolled_to_bottom() {
        let metrics = crate::pane::ScrollMetrics {
            offset_from_bottom: 0,
            max_offset_from_bottom: 20,
            viewport_rows: 5,
        };
        let track = Rect::new(9, 4, 1, 5);

        let thumb = scrollbar_thumb(metrics, track).expect("thumb");
        assert_eq!(thumb.top + thumb.len, track.y + track.height);
    }

    #[test]
    fn scrollbar_offset_mapping_hits_top_middle_and_bottom() {
        let metrics = crate::pane::ScrollMetrics {
            offset_from_bottom: 0,
            max_offset_from_bottom: 20,
            viewport_rows: 5,
        };
        let track = Rect::new(9, 4, 1, 5);

        assert_eq!(scrollbar_offset_from_row(metrics, track, 4), 20);
        assert_eq!(scrollbar_offset_from_row(metrics, track, 6), 10);
        assert_eq!(scrollbar_offset_from_row(metrics, track, 8), 0);
    }

    #[test]
    fn dragging_from_current_thumb_row_preserves_offset() {
        let metrics = crate::pane::ScrollMetrics {
            offset_from_bottom: 7,
            max_offset_from_bottom: 20,
            viewport_rows: 5,
        };
        let track = Rect::new(9, 4, 1, 8);
        let thumb = scrollbar_thumb(metrics, track).expect("thumb");
        let row = thumb.top + thumb.len / 2;
        let grab = scrollbar_thumb_grab_offset(metrics, track, row).expect("grab");

        assert_eq!(scrollbar_offset_from_drag_row(metrics, track, row, grab), 7);
    }
    #[test]
    fn prefix_mode_renders_prefix_indicator() {
        let mut app = crate::app::state::AppState::test_new();
        app.mode = Mode::Prefix;
        app.view.terminal_area = ratatui::layout::Rect::new(0, 0, 60, 4);
        let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(60, 4))
            .expect("test terminal");

        terminal
            .draw(|frame| render_prefix_overlay(&app, frame, app.view.terminal_area))
            .expect("draw prefix overlay");

        let rendered = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(rendered.contains("PREFIX"));
    }

    #[test]
    fn keybind_help_shows_unset_for_optional_actions() {
        let app = crate::app::state::AppState::test_new();
        let groups = keybind_help_groups(&app);

        let workspace_tab = groups
            .iter()
            .find(|(name, _)| *name == "spaces / agents")
            .expect("spaces group")
            .1
            .clone();
        let panes = groups
            .iter()
            .find(|(name, _)| *name == "panes")
            .expect("panes group")
            .1
            .clone();

        assert!(workspace_tab
            .iter()
            .any(|(key, label)| key == "unset" && label.as_ref() == "previous workspace"));
        assert!(workspace_tab
            .iter()
            .any(|(key, label)| key == "unset" && label.as_ref() == "next workspace"));
        assert!(workspace_tab
            .iter()
            .any(|(key, label)| key == "unset" && label.as_ref() == "previous agent"));
        assert!(workspace_tab
            .iter()
            .any(|(key, label)| key == "unset" && label.as_ref() == "next agent"));
        assert!(workspace_tab
            .iter()
            .any(|(key, label)| key == "unset" && label.as_ref() == "focus agent 1-9"));
        assert!(workspace_tab
            .iter()
            .any(|(key, label)| key == "unset" && label.as_ref() == "switch workspace 1-9"));
        assert!(panes
            .iter()
            .any(|(key, label)| key == "ctrl+left" && label.as_ref() == "focus pane left"));
        assert!(panes
            .iter()
            .any(|(key, label)| key == "ctrl+down" && label.as_ref() == "focus pane down"));
        assert!(panes
            .iter()
            .any(|(key, label)| key == "ctrl+up" && label.as_ref() == "focus pane up"));
        assert!(panes
            .iter()
            .any(|(key, label)| key == "ctrl+right" && label.as_ref() == "focus pane right"));
    }

    #[test]
    fn keybind_help_shows_custom_command_descriptions() {
        let mut app = crate::app::state::AppState::test_new();
        app.keybinds.custom_commands = vec![
            crate::config::CustomCommandKeybind {
                bindings: crate::config::ActionKeybinds::prefix("alt+g"),
                label: "prefix+alt+g".to_string(),
                command: "lazygit".to_string(),
                action: crate::config::CustomCommandAction::Pane,
                description: Some("open lazygit".to_string()),
            },
            crate::config::CustomCommandKeybind {
                bindings: crate::config::ActionKeybinds::prefix("alt+h"),
                label: "prefix+alt+h".to_string(),
                command: "echo hello".to_string(),
                action: crate::config::CustomCommandAction::Shell,
                description: None,
            },
        ];

        let groups = keybind_help_groups(&app);
        let custom = groups
            .iter()
            .find(|(name, _)| *name == "custom")
            .expect("custom group")
            .1
            .clone();
        assert!(custom
            .iter()
            .any(|(key, label)| key == "prefix+alt+g" && label.as_ref() == "open lazygit"));
        assert!(custom
            .iter()
            .any(|(key, label)| key == "prefix+alt+h" && label.as_ref() == "custom command"));

        let rendered_help = keybind_help_lines(&app)
            .into_iter()
            .flat_map(|(_, line)| line.spans)
            .map(|span| span.content.into_owned())
            .collect::<Vec<_>>()
            .join("");
        assert!(rendered_help.contains("open lazygit"));
        assert!(rendered_help.contains("custom command"));
    }

    #[test]
    fn the_done_marker_toggles_between_a_dot_and_a_check() {
        let mut app = crate::app::state::AppState::test_new();
        let workspace = Workspace::test_new("space");
        let pane_id = workspace.tabs[0].root_pane;
        app.workspaces = vec![workspace];
        app.active = Some(0);
        app.selected = 0;
        app.mode = Mode::Terminal;
        app.ensure_test_terminals();
        let terminal_id = app.workspaces[0]
            .pane_state(pane_id)
            .expect("agent pane")
            .attached_terminal_id
            .clone();
        let terminal = app.terminals.get_mut(&terminal_id).expect("agent terminal");
        terminal.set_agent_name("codex".into());
        terminal.state = crate::detect::AgentState::Idle;
        let pane = app.workspaces[0].tabs[0]
            .panes
            .get_mut(&pane_id)
            .expect("agent pane");
        pane.seen = false;
        pane.completed = true;

        let marker = |app: &crate::app::state::AppState| {
            let row = app.view.agent_table.rows[0].clone();
            let backend = TestBackend::new(106, 20);
            let mut terminal = Terminal::new(backend).unwrap();
            terminal.draw(|frame| render(app, frame)).unwrap();
            terminal.backend().buffer()[(row.rect.x, row.rect.y)]
                .symbol()
                .to_string()
        };

        compute_view(&mut app, Rect::new(0, 0, 106, 20));
        assert_eq!(marker(&app), "\u{25cf}");

        assert!(app.toggle_agent_completion_acknowledgement(pane_id));
        compute_view(&mut app, Rect::new(0, 0, 106, 20));
        assert_eq!(marker(&app), "\u{2713}");

        assert!(app.toggle_agent_completion_acknowledgement(pane_id));
        compute_view(&mut app, Rect::new(0, 0, 106, 20));
        assert_eq!(marker(&app), "\u{25cf}");
    }
}
