use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Clear, Paragraph},
    Frame,
};

use super::scrollbar::should_show_scrollbar;
use crate::app::state::AgentPanelScope;
use crate::app::{AppState, Mode};
use crate::detect::AgentState;
use crate::terminal::TerminalRuntimeRegistry;

const WORKSPACE_SECTION_HEADER_ROWS: u16 = 1;
const WORKSPACE_SECTION_FOOTER_ROWS: u16 = 3;
const WORKSPACE_SECTION_DROP_SLOT_ROWS: u16 = 1;
const AGENT_PANEL_HEADER_ROWS: u16 = 3;
/// Minimum rows reserved for the agent panel section (divider + header + body).
const MIN_AGENT_SECTION_ROWS: u16 = AGENT_PANEL_HEADER_ROWS.saturating_add(1);

pub(crate) struct AgentPanelEntry {
    pub ws_idx: usize,
    pub tab_idx: usize,
    pub pane_id: crate::layout::PaneId,
    pub primary_label: String,
    pub primary_tab_label: Option<String>,
    pub agent_label: Option<String>,
    pub state: AgentState,
    pub seen: bool,
    pub custom_status: Option<String>,
    pub state_labels: std::collections::HashMap<String, String>,
}

fn workspace_list_content_height(app: &AppState) -> u16 {
    let entry_rows = workspace_list_entries(app)
        .iter()
        .fold(0u16, |rows, entry| match entry {
            WorkspaceListEntry::Workspace { ws_idx, .. } => app
                .workspaces
                .get(*ws_idx)
                .map_or(rows, |ws| rows.saturating_add(workspace_row_height(ws))),
        });

    WORKSPACE_SECTION_HEADER_ROWS
        .saturating_add(WORKSPACE_SECTION_FOOTER_ROWS)
        .saturating_add(WORKSPACE_SECTION_DROP_SLOT_ROWS)
        .saturating_add(entry_rows)
}

fn sidebar_section_heights(
    total_h: u16,
    split_ratio: f32,
    workspace_required_h: u16,
) -> (u16, u16) {
    if total_h == 0 {
        return (0, 0);
    }

    if total_h < 6 {
        let ws_h = total_h.div_ceil(2);
        return (ws_h, total_h.saturating_sub(ws_h));
    }

    let ratio = split_ratio.clamp(0.65, 0.9);
    let ws_h_max = ((total_h as f32) * ratio).round() as u16;
    let ws_h_max = ws_h_max.clamp(3, total_h.saturating_sub(3));
    let ws_h_cap = total_h
        .saturating_sub(MIN_AGENT_SECTION_ROWS)
        .max(2);
    let ws_h_max = ws_h_max.min(ws_h_cap);
    let ws_h = workspace_required_h.clamp(2, ws_h_max);
    let detail_h = total_h.saturating_sub(ws_h);
    (ws_h, detail_h)
}

pub(crate) fn expanded_sidebar_sections(app: &AppState, area: Rect) -> (Rect, Rect) {
    let content = Rect::new(area.x, area.y, area.width.saturating_sub(1), area.height);
    if content.width == 0 || content.height == 0 {
        return (Rect::default(), Rect::default());
    }

    let (ws_h, detail_h) = sidebar_section_heights(
        content.height,
        app.sidebar_section_split,
        workspace_list_content_height(app),
    );
    let ws_area = Rect::new(content.x, content.y, content.width, ws_h);
    let detail_area = Rect::new(content.x, content.y + ws_h, content.width, detail_h);
    (ws_area, detail_area)
}

pub(crate) fn sidebar_section_divider_rect(app: &AppState, area: Rect) -> Rect {
    let content = Rect::new(area.x, area.y, area.width.saturating_sub(1), area.height);
    if content.width == 0 || content.height < 6 {
        return Rect::default();
    }

    let (ws_h, _) = sidebar_section_heights(
        content.height,
        app.sidebar_section_split,
        workspace_list_content_height(app),
    );
    Rect::new(content.x, content.y + ws_h, content.width, 1)
}

fn agent_panel_current_workspace_idx(app: &AppState) -> Option<usize> {
    if matches!(
        app.mode,
        Mode::Navigate
            | Mode::RenameWorkspace
            | Mode::RenamePane
            | Mode::Resize
            | Mode::ConfirmClose
            | Mode::ContextMenu
            | Mode::Settings
            | Mode::GlobalMenu
            | Mode::KeybindHelp
            | Mode::ProductAnnouncement
    ) {
        Some(app.selected)
    } else {
        app.active
    }
}

fn agent_panel_toggle_label(scope: AgentPanelScope) -> &'static str {
    match scope {
        AgentPanelScope::CurrentWorkspace => "current",
        AgentPanelScope::AllWorkspaces => "all",
    }
}

pub(crate) fn agent_panel_header_rect(area: Rect) -> Rect {
    if area.width == 0 || area.height < 2 {
        return Rect::default();
    }

    Rect::new(area.x, area.y + 1, area.width, 1)
}

pub(crate) fn agent_panel_toggle_rect(area: Rect, _scope: AgentPanelScope) -> Rect {
    agent_panel_header_rect(area)
}

pub(crate) fn agent_panel_entries(app: &AppState) -> Vec<AgentPanelEntry> {
    agent_panel_entries_with_runtimes(app, None)
}

pub(crate) fn agent_panel_entries_from(
    app: &AppState,
    terminal_runtimes: &TerminalRuntimeRegistry,
) -> Vec<AgentPanelEntry> {
    agent_panel_entries_with_runtimes(app, Some(terminal_runtimes))
}

fn agent_panel_entries_with_runtimes(
    app: &AppState,
    terminal_runtimes: Option<&TerminalRuntimeRegistry>,
) -> Vec<AgentPanelEntry> {
    let empty_runtimes;
    let terminal_runtimes = match terminal_runtimes {
        Some(terminal_runtimes) => terminal_runtimes,
        None => {
            empty_runtimes = TerminalRuntimeRegistry::new();
            &empty_runtimes
        }
    };

    match app.agent_panel_scope {
        AgentPanelScope::CurrentWorkspace => {
            let Some(ws_idx) = agent_panel_current_workspace_idx(app) else {
                return Vec::new();
            };
            let Some(ws) = app.workspaces.get(ws_idx) else {
                return Vec::new();
            };
            ws.pane_details(&app.terminals)
                .into_iter()
                .map(|detail| AgentPanelEntry {
                    ws_idx,
                    tab_idx: detail.tab_idx,
                    pane_id: detail.pane_id,
                    primary_label: detail.label,
                    primary_tab_label: None,
                    agent_label: None,
                    state: detail.state,
                    seen: detail.seen,
                    custom_status: detail.custom_status,
                    state_labels: detail.state_labels,
                })
                .collect()
        }
        AgentPanelScope::AllWorkspaces => app
            .workspaces
            .iter()
            .enumerate()
            .flat_map(|(ws_idx, ws)| {
                let multi_tab = ws.tabs.len() > 1;
                let workspace_label = ws.display_name_from(&app.terminals, terminal_runtimes);
                ws.pane_details(&app.terminals)
                    .into_iter()
                    .map(move |detail| AgentPanelEntry {
                        ws_idx,
                        tab_idx: detail.tab_idx,
                        pane_id: detail.pane_id,
                        primary_label: workspace_label.clone(),
                        primary_tab_label: multi_tab.then_some(detail.tab_label),
                        agent_label: Some(detail.agent_label),
                        state: detail.state,
                        seen: detail.seen,
                        custom_status: detail.custom_status,
                        state_labels: detail.state_labels,
                    })
            })
            .collect(),
    }
}

pub(super) fn agent_panel_status_key(state: AgentState, seen: bool) -> &'static str {
    match (state, seen) {
        (AgentState::Idle, false) => "done",
        (AgentState::Idle, true) => "idle",
        (AgentState::Working, _) => "working",
        (AgentState::Blocked, _) => "blocked",
        (AgentState::Unknown, _) => "unknown",
    }
}

fn workspace_row_height(_ws: &crate::workspace::Workspace) -> u16 {
    4
}
pub(crate) fn workspace_parent_group_state(
    app: &AppState,
    ws_idx: usize,
) -> Option<(String, bool)> {
    let space = app.workspaces.get(ws_idx)?.worktree_space()?;
    if space.is_linked_worktree {
        return None;
    }
    let member_count = app
        .workspaces
        .iter()
        .filter(|ws| {
            ws.worktree_space()
                .is_some_and(|member| member.key == space.key)
        })
        .count();
    (member_count >= 2).then(|| {
        (
            space.key.clone(),
            app.collapsed_space_keys.contains(&space.key),
        )
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum WorkspaceListEntry {
    Workspace { ws_idx: usize, indented: bool },
}

pub(crate) fn normalized_workspace_scroll(app: &AppState, area: Rect, requested: usize) -> usize {
    let ws_area = workspace_list_rect(app, area);
    let body = workspace_list_body_rect(ws_area, false);
    if body.height == 0 {
        return requested;
    }

    let entry_count = workspace_list_entries(app).len();
    if entry_count == 0 {
        0
    } else {
        requested.min(entry_count.saturating_sub(1))
    }
}

pub(crate) fn workspace_list_entries(app: &AppState) -> Vec<WorkspaceListEntry> {
    let mut members_by_key = std::collections::HashMap::<String, Vec<usize>>::new();
    for (ws_idx, ws) in app.workspaces.iter().enumerate() {
        if let Some(space) = ws.worktree_space() {
            members_by_key
                .entry(space.key.clone())
                .or_default()
                .push(ws_idx);
        }
    }
    let grouped_keys = members_by_key
        .iter()
        .filter(|(_, members)| {
            members.len() >= 2
                && members.iter().any(|idx| {
                    app.workspaces
                        .get(*idx)
                        .and_then(|ws| ws.worktree_space())
                        .is_some_and(|space| !space.is_linked_worktree)
                })
        })
        .map(|(key, _)| key.clone())
        .collect::<std::collections::HashSet<_>>();

    let visible_group_idx = if matches!(app.mode, Mode::Navigate) {
        Some(app.selected)
    } else {
        app.active
    };
    let active_group = visible_group_idx.and_then(|idx| {
        app.workspaces
            .get(idx)
            .and_then(|ws| ws.worktree_space())
            .map(|space| space.key.clone())
    });

    let mut emitted_groups = std::collections::HashSet::<String>::new();
    let mut entries = Vec::new();
    for (ws_idx, ws) in app.workspaces.iter().enumerate() {
        let Some(space) = ws
            .worktree_space()
            .filter(|space| grouped_keys.contains(&space.key))
        else {
            entries.push(WorkspaceListEntry::Workspace {
                ws_idx,
                indented: false,
            });
            continue;
        };

        if !emitted_groups.insert(space.key.clone()) {
            continue;
        }

        let Some(members) = members_by_key.get(&space.key) else {
            continue;
        };
        let Some(parent_idx) = members.iter().copied().find(|idx| {
            app.workspaces
                .get(*idx)
                .and_then(|member| member.worktree_space())
                .is_some_and(|member_space| !member_space.is_linked_worktree)
        }) else {
            entries.push(WorkspaceListEntry::Workspace {
                ws_idx,
                indented: false,
            });
            continue;
        };
        let collapsed = app.collapsed_space_keys.contains(&space.key);
        entries.push(WorkspaceListEntry::Workspace {
            ws_idx: parent_idx,
            indented: false,
        });

        if collapsed {
            if let Some(active_idx) = visible_group_idx
                .filter(|idx| *idx != parent_idx)
                .filter(|_| active_group.as_deref() == Some(space.key.as_str()))
            {
                entries.push(WorkspaceListEntry::Workspace {
                    ws_idx: active_idx,
                    indented: true,
                });
            }
        } else {
            for member_idx in members {
                if *member_idx == parent_idx {
                    continue;
                }
                entries.push(WorkspaceListEntry::Workspace {
                    ws_idx: *member_idx,
                    indented: true,
                });
            }
        }
    }
    entries
}

pub(crate) fn workspace_list_rect(app: &AppState, area: Rect) -> Rect {
    let (ws_area, _) = expanded_sidebar_sections(app, area);
    ws_area
}

pub(crate) fn workspace_list_body_rect(area: Rect, has_scrollbar: bool) -> Rect {
    if area.width == 0 || area.height <= WORKSPACE_SECTION_HEADER_ROWS {
        return Rect::default();
    }

    let body_y = area.y.saturating_add(WORKSPACE_SECTION_HEADER_ROWS);
    let footer_y = area.y + area.height.saturating_sub(WORKSPACE_SECTION_FOOTER_ROWS);
    let body_height = footer_y.saturating_sub(body_y);
    let body_width = area.width.saturating_sub(u16::from(has_scrollbar));
    Rect::new(area.x, body_y, body_width, body_height)
}

fn workspace_list_visible_count(app: &AppState, area: Rect, scroll: usize) -> usize {
    let body = workspace_list_body_rect(area, false);
    if body.width == 0 || body.height == 0 {
        return 0;
    }

    let mut used_rows = 0u16;
    let mut visible = 0usize;
    let entries = workspace_list_entries(app);
    for entry in entries.iter().skip(scroll) {
        let (row_height, gap) = match entry {
            WorkspaceListEntry::Workspace { ws_idx, .. } => {
                let Some(ws) = app.workspaces.get(*ws_idx) else {
                    continue;
                };
                (workspace_row_height(ws), 0)
            }
        };
        if used_rows.saturating_add(row_height) > body.height {
            break;
        }
        used_rows = used_rows.saturating_add(row_height).saturating_add(gap);
        visible += 1;
    }
    visible
}

pub(crate) fn workspace_list_scroll_metrics(
    app: &AppState,
    area: Rect,
) -> crate::pane::ScrollMetrics {
    let entries = workspace_list_entries(app);
    let total_rows = entries.len();
    let scroll = app.workspace_scroll.min(total_rows.saturating_sub(1));
    let viewport_rows = workspace_list_visible_count(app, area, scroll);
    let max_offset_from_bottom = total_rows.saturating_sub(viewport_rows);
    let offset_from_bottom = total_rows
        .saturating_sub(scroll)
        .saturating_sub(viewport_rows);

    crate::pane::ScrollMetrics {
        offset_from_bottom,
        max_offset_from_bottom,
        viewport_rows,
    }
}

pub(crate) fn workspace_list_scrollbar_rect(app: &AppState, area: Rect) -> Option<Rect> {
    let metrics = workspace_list_scroll_metrics(app, area);
    let body = workspace_list_body_rect(area, true);
    (should_show_scrollbar(metrics) && body.width > 0 && body.height > 0).then_some(Rect::new(
        area.x + area.width.saturating_sub(1),
        body.y,
        1,
        body.height,
    ))
}

pub(crate) fn agent_panel_body_rect(area: Rect, has_scrollbar: bool) -> Rect {
    if area.width == 0 || area.height <= AGENT_PANEL_HEADER_ROWS {
        return Rect::default();
    }

    let body_y = area.y.saturating_add(AGENT_PANEL_HEADER_ROWS);
    let body_height = (area.y + area.height).saturating_sub(body_y);
    let body_width = area.width.saturating_sub(u16::from(has_scrollbar));
    Rect::new(area.x, body_y, body_width, body_height)
}

fn agent_panel_visible_count(area: Rect) -> usize {
    let body = agent_panel_body_rect(area, false);
    if body.width == 0 || body.height < 2 {
        return 0;
    }

    let mut used_rows = 0u16;
    let mut visible = 0usize;
    while used_rows.saturating_add(2) <= body.height {
        used_rows = used_rows.saturating_add(2);
        visible += 1;
        if used_rows < body.height {
            used_rows = used_rows.saturating_add(1);
        }
    }
    visible
}

pub(crate) fn agent_panel_scroll_metrics(app: &AppState, area: Rect) -> crate::pane::ScrollMetrics {
    let viewport_rows = agent_panel_visible_count(area);
    let total_rows = agent_panel_entries(app).len();
    let max_offset_from_bottom = total_rows.saturating_sub(viewport_rows);
    let offset_from_bottom = total_rows
        .saturating_sub(app.agent_panel_scroll)
        .saturating_sub(viewport_rows);

    crate::pane::ScrollMetrics {
        offset_from_bottom,
        max_offset_from_bottom,
        viewport_rows,
    }
}

pub(crate) fn agent_panel_scrollbar_rect(app: &AppState, area: Rect) -> Option<Rect> {
    let metrics = agent_panel_scroll_metrics(app, area);
    let body = agent_panel_body_rect(area, true);
    (should_show_scrollbar(metrics) && body.width > 0 && body.height > 0).then_some(Rect::new(
        area.x + area.width.saturating_sub(1),
        body.y,
        1,
        body.height,
    ))
}

pub(crate) fn compute_workspace_list_areas(
    app: &AppState,
    area: Rect,
) -> (Vec<crate::app::state::WorkspaceCardArea>, Vec<()>) {
    let ws_area = workspace_list_rect(app, area);
    if ws_area == Rect::default() {
        return (Vec::new(), Vec::new());
    }

    let metrics = workspace_list_scroll_metrics(app, ws_area);
    let body = workspace_list_body_rect(ws_area, should_show_scrollbar(metrics));
    if body.width == 0 || body.height == 0 {
        return (Vec::new(), Vec::new());
    }

    let scroll = app.workspace_scroll;
    let mut row_y = body.y;
    let body_bottom = body.y + body.height;
    let mut cards = Vec::new();
    let headers = Vec::new();

    let entries = workspace_list_entries(app);
    for entry in entries.iter().skip(scroll) {
        match entry {
            WorkspaceListEntry::Workspace { ws_idx, indented } => {
                let Some(ws) = app.workspaces.get(*ws_idx) else {
                    continue;
                };
                let row_height = workspace_row_height(ws);
                let gap = 0;
                if row_y.saturating_add(row_height) > body_bottom {
                    break;
                }
                cards.push(crate::app::state::WorkspaceCardArea {
                    ws_idx: *ws_idx,
                    rect: Rect::new(body.x, row_y, body.width, row_height),
                    indented: *indented,
                });
                row_y = row_y.saturating_add(row_height + gap);
            }
        }
    }

    (cards, headers)
}

pub(crate) fn compute_workspace_card_areas(
    app: &AppState,
    area: Rect,
) -> Vec<crate::app::state::WorkspaceCardArea> {
    compute_workspace_list_areas(app, area).0
}

/// Auto-scale sidebar width based on workspace identity + agent summary.
pub(crate) fn collapsed_sidebar_sections(area: Rect) -> (Rect, Option<u16>, Rect) {
    let content = Rect::new(area.x, area.y, area.width.saturating_sub(1), area.height);
    if content.width == 0 || content.height == 0 {
        return (Rect::default(), None, Rect::default());
    }

    if content.height < 7 {
        return (content, None, Rect::default());
    }

    let total_h = content.height as usize;
    let ws_h = total_h.div_ceil(2);
    let detail_h = total_h.saturating_sub(ws_h + 1);
    if ws_h == 0 || detail_h == 0 {
        return (content, None, Rect::default());
    }

    let divider_y = content.y + ws_h as u16;
    let ws_area = Rect::new(content.x, content.y, content.width, ws_h as u16);
    let detail_area = Rect::new(content.x, divider_y + 1, content.width, detail_h as u16);
    (ws_area, Some(divider_y), detail_area)
}

pub(crate) fn workspace_drop_indicator_row(
    cards: &[crate::app::state::WorkspaceCardArea],
    area: Rect,
    insert_idx: usize,
) -> Option<u16> {
    if area.height == 0 {
        return None;
    }
    let list_bottom = area.y + area.height.saturating_sub(1);

    let first = cards.first()?;
    if insert_idx == first.ws_idx {
        return first.rect.y.checked_sub(1).filter(|y| *y < list_bottom);
    }

    if let Some(row) = cards
        .last()
        .filter(|card| insert_idx == card.ws_idx.saturating_add(1))
        .map(|card| card.rect.y.saturating_add(card.rect.height))
        .filter(|y| *y < list_bottom)
    {
        return Some(row);
    }

    if let Some(card) = cards.iter().find(|card| card.ws_idx == insert_idx) {
        return card.rect.y.checked_sub(1).filter(|y| *y < list_bottom);
    }

    None
}

pub(crate) fn collapsed_sidebar_toggle_rect(area: Rect) -> Rect {
    if area.width == 0 || area.height == 0 {
        return Rect::default();
    }
    Rect::new(area.x, area.y, 1, 1)
}

pub(crate) fn expanded_sidebar_toggle_rect(area: Rect) -> Rect {
    if area.width == 0 || area.height == 0 {
        return Rect::default();
    }
    Rect::new(area.x, area.y, 1, 1)
}

pub(crate) fn render_sidebar(
    app: &AppState,
    terminal_runtimes: &TerminalRuntimeRegistry,
    frame: &mut Frame,
    area: Rect,
) {
    if area == Rect::default() || area.width == 0 || area.height == 0 {
        return;
    }

    frame.render_widget(Clear, area);
    fill_sidebar_background(app, frame, area);

    if app.sidebar_collapsed {
        render_sidebar_line(
            frame,
            collapsed_sidebar_toggle_rect(area),
            Line::from(Span::styled(
                "›",
                Style::default()
                    .fg(app.palette.accent)
                    .add_modifier(Modifier::BOLD),
            )),
        );
        return;
    }

    render_sidebar_line(
        frame,
        Rect::new(area.x, area.y, area.width.saturating_sub(1), 1),
        Line::from(vec![
            Span::styled(
                "‹",
                Style::default()
                    .fg(app.palette.accent)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" spaces", Style::default().fg(app.palette.overlay0)),
        ]),
    );

    render_workspace_rows(app, terminal_runtimes, frame, area);
    render_agent_panel(app, terminal_runtimes, frame, area);
}

fn fill_sidebar_background(app: &AppState, frame: &mut Frame, area: Rect) {
    let divider_style = Style::default().fg(app.palette.surface_dim);
    let divider_x = area.x + area.width.saturating_sub(1);
    let buf = frame.buffer_mut();
    for y in area.y..area.y + area.height {
        for x in area.x..area.x + area.width {
            let cell = &mut buf[(x, y)];
            cell.set_symbol(" ");
            cell.set_style(Style::default());
        }
        if area.width > 1 {
            let cell = &mut buf[(divider_x, y)];
            cell.set_symbol("│");
            cell.set_style(divider_style);
        }
    }
}

fn render_workspace_rows(
    app: &AppState,
    terminal_runtimes: &TerminalRuntimeRegistry,
    frame: &mut Frame,
    area: Rect,
) {
    let cards = if app.view.workspace_card_areas.is_empty() {
        compute_workspace_card_areas(app, area)
    } else {
        app.view.workspace_card_areas.clone()
    };

    for card in cards {
        let Some(ws) = app.workspaces.get(card.ws_idx) else {
            continue;
        };
        let is_active = Some(card.ws_idx) == app.active;
        let is_selected = app.mode == Mode::Navigate && card.ws_idx == app.selected;
        let selected = is_selected || is_active;
        let (state, seen) = ws.aggregate_state(&app.terminals);
        let (name, branch) =
            workspace_card_labels(app, terminal_runtimes, ws, card.indented, state, seen);
        render_workspace_card(frame, card.rect, &name, &branch, selected, app);
    }

    let ws_area = workspace_list_rect(app, area);
    if ws_area != Rect::default() && ws_area.height >= WORKSPACE_SECTION_FOOTER_ROWS {
        let footer = Rect::new(
            ws_area.x,
            ws_area.y + ws_area.height.saturating_sub(WORKSPACE_SECTION_FOOTER_ROWS),
            ws_area.width,
            WORKSPACE_SECTION_FOOTER_ROWS,
        );
        render_new_workspace_button(frame, footer, app);
    }
}

fn render_new_workspace_button(frame: &mut Frame, rect: Rect, app: &AppState) {
    if rect.width < 2 || rect.height < 3 {
        return;
    }

    let border_style = Style::default().fg(app.palette.overlay0);
    let label_style = Style::default().fg(app.palette.overlay0);

    let buf = frame.buffer_mut();
    let right = rect.x + rect.width.saturating_sub(1);
    let bottom = rect.y + rect.height.saturating_sub(1);

    buf[(rect.x, rect.y)]
        .set_symbol("╭")
        .set_style(border_style);
    buf[(right, rect.y)].set_symbol("╮").set_style(border_style);
    buf[(rect.x, bottom)]
        .set_symbol("╰")
        .set_style(border_style);
    buf[(right, bottom)].set_symbol("╯").set_style(border_style);
    for x in rect.x + 1..right {
        buf[(x, rect.y)].set_symbol("─").set_style(border_style);
        buf[(x, bottom)].set_symbol("─").set_style(border_style);
    }
    for y in rect.y + 1..bottom {
        buf[(rect.x, y)].set_symbol("│").set_style(border_style);
        buf[(right, y)].set_symbol("│").set_style(border_style);
    }

    let label = "+ new";
    let inner_width = rect.width.saturating_sub(2) as usize;
    let label = truncate_chars(label, inner_width);
    let label_len = label.chars().count() as u16;
    let mid_y = rect.y + rect.height / 2;
    let start_x = rect.x + 1 + inner_width.saturating_sub(label_len as usize) as u16 / 2;
    for (idx, ch) in label.chars().enumerate() {
        let x = start_x + idx as u16;
        if x >= right {
            break;
        }
        buf[(x, mid_y)]
            .set_symbol(&ch.to_string())
            .set_style(label_style);
    }
}

fn workspace_card_labels(
    app: &AppState,
    terminal_runtimes: &TerminalRuntimeRegistry,
    ws: &crate::workspace::Workspace,
    indented: bool,
    state: AgentState,
    seen: bool,
) -> (String, String) {
    let (dot, _) = sidebar_state_dot(state, seen, app);
    let indent = if indented { "  " } else { "" };
    let name = ws.display_name_from(&app.terminals, terminal_runtimes);
    let branch = ws.branch().unwrap_or_else(|| "shell".to_string());
    (
        format!("{indent}{dot} {name}"),
        format!("{indent}  {branch}"),
    )
}

fn render_workspace_card(
    frame: &mut Frame,
    rect: Rect,
    name: &str,
    branch: &str,
    selected: bool,
    app: &AppState,
) {
    if rect.width < 2 || rect.height == 0 {
        return;
    }

    let border_color = if selected {
        app.palette.accent
    } else {
        app.palette.surface_dim
    };
    let name_color = if selected {
        app.palette.accent
    } else {
        app.palette.text
    };
    let border_style = Style::default().fg(border_color);
    let name_style = Style::default().fg(name_color).add_modifier(Modifier::BOLD);
    let branch_style = Style::default().fg(app.palette.overlay0);
    let inner_width = rect.width.saturating_sub(2) as usize;
    let name = pad_to_width(&truncate_chars(name, inner_width), inner_width);
    let branch = pad_to_width(&truncate_chars(branch, inner_width), inner_width);

    let buf = frame.buffer_mut();
    let right = rect.x + rect.width.saturating_sub(1);

    if rect.height < 4 {
        render_workspace_card_compact(buf, rect, right, &name, border_style, name_style);
        return;
    }

    let bottom = rect.y + rect.height.saturating_sub(1);

    buf[(rect.x, rect.y)]
        .set_symbol("╭")
        .set_style(border_style);
    for x in rect.x + 1..right {
        buf[(x, rect.y)].set_symbol("─").set_style(border_style);
    }
    buf[(right, rect.y)].set_symbol("╮").set_style(border_style);

    render_workspace_card_text_row(
        buf,
        rect.x,
        right,
        rect.y + 1,
        &name,
        border_style,
        name_style,
    );
    render_workspace_card_text_row(
        buf,
        rect.x,
        right,
        rect.y + 2,
        &branch,
        border_style,
        branch_style,
    );

    buf[(rect.x, bottom)]
        .set_symbol("╰")
        .set_style(border_style);
    for x in rect.x + 1..right {
        buf[(x, bottom)].set_symbol("─").set_style(border_style);
    }
    buf[(right, bottom)].set_symbol("╯").set_style(border_style);
}

fn render_workspace_card_compact(
    buf: &mut ratatui::buffer::Buffer,
    rect: Rect,
    right: u16,
    name: &str,
    border_style: Style,
    name_style: Style,
) {
    buf[(rect.x, rect.y)]
        .set_symbol("╭")
        .set_style(border_style);
    for (idx, ch) in name.chars().enumerate() {
        let x = rect.x + 1 + idx as u16;
        if x >= right {
            break;
        }
        buf[(x, rect.y)]
            .set_symbol(&ch.to_string())
            .set_style(name_style);
    }
    buf[(right, rect.y)].set_symbol("╮").set_style(border_style);
}

fn render_workspace_card_text_row(
    buf: &mut ratatui::buffer::Buffer,
    left: u16,
    right: u16,
    y: u16,
    text: &str,
    border_style: Style,
    text_style: Style,
) {
    buf[(left, y)].set_symbol("│").set_style(border_style);
    for (idx, ch) in text.chars().enumerate() {
        let x = left + 1 + idx as u16;
        if x >= right {
            break;
        }
        buf[(x, y)]
            .set_symbol(&ch.to_string())
            .set_style(text_style);
    }
    buf[(right, y)].set_symbol("│").set_style(border_style);
}

fn pad_to_width(text: &str, width: usize) -> String {
    format!("{text:<width$}")
}

fn render_agent_panel(
    app: &AppState,
    terminal_runtimes: &TerminalRuntimeRegistry,
    frame: &mut Frame,
    area: Rect,
) {
    let (_, detail_area) = expanded_sidebar_sections(app, area);
    if detail_area.width == 0 || detail_area.height == 0 {
        return;
    }

    let divider = sidebar_section_divider_rect(app, area);
    if divider.width > 0 && divider.height > 0 {
        let buf = frame.buffer_mut();
        for x in divider.x..divider.x + divider.width {
            buf[(x, divider.y)]
                .set_symbol("─")
                .set_style(Style::default().fg(app.palette.surface_dim));
        }
    }

    render_sidebar_line(
        frame,
        Rect::new(detail_area.x, detail_area.y + 1, detail_area.width, 1),
        Line::from(vec![
            Span::styled(" agents", Style::default().fg(app.palette.overlay0)),
            Span::styled(
                format!(" {}", agent_panel_toggle_label(app.agent_panel_scope)),
                Style::default().fg(app.palette.accent),
            ),
        ]),
    );

    let entries = agent_panel_entries_from(app, terminal_runtimes);
    let body = agent_panel_body_rect(detail_area, false);
    if body.width == 0 || body.height == 0 {
        return;
    }

    let mut row = body.y;
    for entry in entries.into_iter().skip(app.agent_panel_scroll) {
        if row >= body.y + body.height {
            break;
        }
        let label = agent_entry_label(&entry);
        let status = agent_panel_status_key(entry.state, entry.seen);
        let color = match entry.state {
            AgentState::Working => app.palette.yellow,
            AgentState::Blocked => app.palette.red,
            AgentState::Idle if !entry.seen => app.palette.green,
            AgentState::Idle => app.palette.overlay0,
            AgentState::Unknown => app.palette.overlay0,
        };
        let text_width = body.width.saturating_sub(4) as usize;
        render_sidebar_line(
            frame,
            Rect::new(body.x, row, body.width, 1),
            Line::from(vec![
                Span::styled(" ● ", Style::default().fg(color)),
                Span::styled(
                    truncate_chars(&label, text_width),
                    Style::default().fg(app.palette.text),
                ),
            ]),
        );
        row = row.saturating_add(1);
        if row >= body.y + body.height {
            break;
        }
        render_sidebar_line(
            frame,
            Rect::new(body.x, row, body.width, 1),
            Line::from(Span::styled(
                format!("   {}", truncate_chars(status, text_width)),
                Style::default().fg(app.palette.overlay0),
            )),
        );
        row = row.saturating_add(2);
    }
}

fn agent_entry_label(entry: &AgentPanelEntry) -> String {
    let mut label = entry.primary_label.clone();
    if let Some(tab) = &entry.primary_tab_label {
        label.push_str(" · ");
        label.push_str(tab);
    }
    if let Some(agent) = &entry.agent_label {
        label.push_str(" · ");
        label.push_str(agent);
    }
    label
}

fn render_sidebar_line(frame: &mut Frame, rect: Rect, line: Line<'_>) {
    if rect.width == 0 || rect.height == 0 {
        return;
    }
    frame.render_widget(Paragraph::new(line), rect);
}

fn sidebar_state_dot(state: AgentState, seen: bool, app: &AppState) -> (&'static str, Style) {
    let color = match state {
        AgentState::Working => app.palette.yellow,
        AgentState::Blocked => app.palette.red,
        AgentState::Idle if !seen => app.palette.green,
        AgentState::Idle => app.palette.overlay0,
        AgentState::Unknown => app.palette.overlay0,
    };
    ("●", Style::default().fg(color))
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    text.chars()
        .take(max_chars.saturating_sub(1))
        .collect::<String>()
        + "…"
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{detect::Agent, workspace::Workspace};

    #[test]
    fn expanded_sidebar_toggle_sits_in_upper_left_corner() {
        let area = Rect::new(2, 3, 26, 20);
        let toggle = expanded_sidebar_toggle_rect(area);

        assert_eq!(toggle, Rect::new(area.x, area.y, 1, 1));
    }

    #[test]
    fn collapsed_sidebar_toggle_sits_in_upper_left_corner() {
        let area = Rect::new(2, 3, 4, 20);
        let toggle = collapsed_sidebar_toggle_rect(area);

        assert_eq!(toggle, Rect::new(area.x, area.y, 1, 1));
    }

    #[test]
    fn all_workspaces_agent_panel_entries_use_workspace_and_optional_tab_labels() {
        let mut app = crate::app::state::AppState::test_new();
        let first = Workspace::test_new("one");
        let first_pane = first.tabs[0].root_pane;
        let mut second = Workspace::test_new("two");
        let second_tab = second.test_add_tab(Some("logs"));
        let second_pane = second.tabs[second_tab].root_pane;

        app.workspaces = vec![first, second];
        app.ensure_test_terminals();
        let first_terminal_id = app.workspaces[0].tabs[0].panes[&first_pane]
            .attached_terminal_id
            .clone();
        app.terminals
            .get_mut(&first_terminal_id)
            .unwrap()
            .detected_agent = Some(Agent::Pi);
        let second_terminal_id = app.workspaces[1].tabs[second_tab].panes[&second_pane]
            .attached_terminal_id
            .clone();
        app.terminals
            .get_mut(&second_terminal_id)
            .unwrap()
            .detected_agent = Some(Agent::Claude);
        app.active = Some(0);
        app.selected = 0;
        app.agent_panel_scope = AgentPanelScope::AllWorkspaces;

        let entries = agent_panel_entries(&app);
        assert_eq!(entries[0].primary_label, "one");
        assert!(entries[0].primary_tab_label.is_none());
        assert_eq!(entries[0].agent_label.as_deref(), Some("pi"));
        assert_eq!(entries[1].primary_label, "two");
        assert_eq!(entries[1].primary_tab_label.as_deref(), Some("logs"));
        assert_eq!(entries[1].agent_label.as_deref(), Some("claude"));
    }

    #[tokio::test]
    async fn all_workspaces_agent_panel_entries_use_live_root_runtime_cwd_for_workspace_label() {
        let unique = format!(
            "herdr-agent-panel-runtime-cwd-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let root = std::env::temp_dir().join(unique);
        let stale_cwd = root.join("issue-264-nix-support");
        let live_cwd = root.join("herdr");
        std::fs::create_dir_all(stale_cwd.join(".git")).unwrap();
        std::fs::create_dir_all(live_cwd.join(".git")).unwrap();

        let mut app = crate::app::state::AppState::test_new();
        let mut workspace = Workspace::test_new("stale-name");
        workspace.custom_name = None;
        workspace.identity_cwd = stale_cwd.clone();
        let pane = workspace.tabs[0].root_pane;

        app.workspaces = vec![workspace];
        app.ensure_test_terminals();
        let terminal_id = app.workspaces[0].tabs[0].panes[&pane]
            .attached_terminal_id
            .clone();
        let terminal = app.terminals.get_mut(&terminal_id).unwrap();
        terminal.cwd = stale_cwd;
        terminal.detected_agent = Some(Agent::Pi);
        app.active = Some(0);
        app.selected = 0;
        app.agent_panel_scope = AgentPanelScope::AllWorkspaces;

        let (events, _) = tokio::sync::mpsc::channel(4);
        let runtime = crate::terminal::TerminalRuntime::spawn(
            pane,
            24,
            80,
            live_cwd.clone(),
            0,
            crate::terminal_theme::TerminalTheme::default(),
            crate::pane::PaneShellConfig::new("/bin/sh", crate::config::ShellModeConfig::NonLogin),
            events,
            std::sync::Arc::new(tokio::sync::Notify::new()),
            std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        )
        .unwrap();

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while runtime.cwd() != Some(live_cwd.clone()) && std::time::Instant::now() < deadline {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }

        let mut runtime_registry = TerminalRuntimeRegistry::new();
        runtime_registry.insert(terminal_id, runtime);
        let entries = agent_panel_entries_from(&app, &runtime_registry);
        let primary_label = entries[0].primary_label.clone();

        for (_, runtime) in runtime_registry.drain() {
            runtime.shutdown();
        }
        let _ = std::fs::remove_dir_all(root);

        assert_eq!(primary_label, "herdr");
    }

    #[test]
    fn all_workspaces_agent_panel_entries_prefer_agent_names_for_agent_identity() {
        let mut app = crate::app::state::AppState::test_new();
        let workspace = Workspace::test_new("bridge");
        let first_pane = workspace.tabs[0].root_pane;

        app.workspaces = vec![workspace];
        app.ensure_test_terminals();
        let first_terminal_id = app.workspaces[0].tabs[0].panes[&first_pane]
            .attached_terminal_id
            .clone();
        app.terminals
            .get_mut(&first_terminal_id)
            .unwrap()
            .detected_agent = Some(Agent::Pi);
        app.terminals
            .get_mut(&first_terminal_id)
            .unwrap()
            .set_agent_name("planner".into());
        app.active = Some(0);
        app.selected = 0;
        app.agent_panel_scope = AgentPanelScope::AllWorkspaces;

        let entries = agent_panel_entries(&app);
        assert_eq!(entries[0].primary_label, "bridge");
        assert_eq!(entries[0].agent_label.as_deref(), Some("planner"));
    }

    #[test]
    fn expanded_sidebar_sections_handle_tiny_heights() {
        let app = AppState::test_new();
        let (ws_area, detail_area) = expanded_sidebar_sections(&app, Rect::new(0, 0, 20, 5));

        assert_eq!(ws_area, Rect::new(0, 0, 19, 3));
        assert_eq!(detail_area, Rect::new(0, 3, 19, 2));
    }

    #[test]
    fn sidebar_section_divider_is_hidden_for_tiny_heights() {
        let app = AppState::test_new();
        let divider = sidebar_section_divider_rect(&app, Rect::new(0, 0, 20, 5));

        assert_eq!(divider, Rect::default());
    }

    fn workspace_with_worktree_space(
        name: &str,
        key: Option<&str>,
        checkout_key: &str,
    ) -> crate::workspace::Workspace {
        let mut ws = crate::workspace::Workspace::test_new(name);
        if let Some(key) = key {
            ws.worktree_space = Some(crate::workspace::WorktreeSpaceMembership {
                key: key.into(),
                label: "herdr".into(),
                repo_root: std::path::PathBuf::from("/repo/herdr"),
                checkout_path: std::path::PathBuf::from(checkout_key),
                is_linked_worktree: name != "main",
            });
        }
        ws
    }

    fn workspace_with_git_space(name: &str, key: &str) -> crate::workspace::Workspace {
        let mut ws = crate::workspace::Workspace::test_new(name);
        ws.cached_git_space = Some(crate::workspace::GitSpaceMetadata {
            key: key.into(),
            checkout_key: format!("/repo/{name}"),
            label: "herdr".into(),
            repo_root: std::path::PathBuf::from(format!("/repo/{name}")),
            is_linked_worktree: false,
        });
        ws
    }

    #[test]
    fn parent_workspace_row_stays_clickable_when_grouped() {
        let mut app = AppState::test_new();
        app.workspaces = vec![
            workspace_with_worktree_space("main", Some("repo-key"), "/repo/herdr"),
            workspace_with_worktree_space("issue", Some("repo-key"), "/repo/herdr-issue"),
        ];

        let (cards, headers) = compute_workspace_list_areas(&app, Rect::new(0, 0, 30, 20));

        assert!(headers.is_empty());
        assert_eq!(cards[0].ws_idx, 0);
        assert!(!cards[0].indented);
        assert_eq!(cards[1].ws_idx, 1);
        assert!(cards[1].indented);
        assert_eq!(cards[1].rect.y, cards[0].rect.y + cards[0].rect.height);
    }

    #[test]
    fn linked_only_worktree_members_do_not_form_parentless_group() {
        let mut app = AppState::test_new();
        app.workspaces = vec![
            workspace_with_worktree_space("issue", Some("repo-key"), "/repo/herdr-issue"),
            workspace_with_worktree_space("review", Some("repo-key"), "/repo/herdr-review"),
        ];

        let entries = workspace_list_entries(&app);

        assert_eq!(
            entries,
            vec![
                WorkspaceListEntry::Workspace {
                    ws_idx: 0,
                    indented: false
                },
                WorkspaceListEntry::Workspace {
                    ws_idx: 1,
                    indented: false
                },
            ]
        );
    }

    #[test]
    fn compact_space_group_scroll_offset_can_start_inside_group() {
        let mut app = AppState::test_new();
        app.workspaces = vec![
            workspace_with_worktree_space("main", Some("repo-key"), "/repo/herdr"),
            workspace_with_worktree_space("one", Some("repo-key"), "/repo/herdr-one"),
            workspace_with_worktree_space("two", Some("repo-key"), "/repo/herdr-two"),
        ];
        let area = Rect::new(0, 0, 30, 20);
        app.workspace_scroll = normalized_workspace_scroll(&app, area, 2);

        let (cards, headers) = compute_workspace_list_areas(&app, area);

        assert!(headers.is_empty());
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].ws_idx, 2);
    }

    #[test]
    fn workspace_scroll_metrics_count_display_entries_not_raw_workspaces() {
        let mut app = AppState::test_new();
        app.workspaces = vec![
            workspace_with_worktree_space("main", Some("repo-key"), "/repo/herdr"),
            workspace_with_worktree_space("issue", Some("repo-key"), "/repo/herdr-issue"),
            Workspace::test_new("notes"),
        ];
        app.collapsed_space_keys.insert("repo-key".into());
        app.active = None;
        app.mode = Mode::Terminal;

        let ws_area = Rect::new(0, 0, 30, 8);
        let metrics = workspace_list_scroll_metrics(&app, ws_area);

        assert_eq!(metrics.viewport_rows, 1);
        assert_eq!(metrics.max_offset_from_bottom, 1);
        assert_eq!(metrics.offset_from_bottom, 1);
    }

    #[test]
    fn workspace_scroll_offset_applies_to_group_children() {
        let mut app = AppState::test_new();
        app.workspaces = vec![
            workspace_with_worktree_space("main", Some("repo-key"), "/repo/herdr"),
            workspace_with_worktree_space("issue", Some("repo-key"), "/repo/herdr-issue"),
            Workspace::test_new("notes"),
        ];
        app.collapsed_space_keys.insert("repo-key".into());
        app.active = None;
        app.mode = Mode::Terminal;
        app.workspace_scroll = 1;

        let (cards, headers) = compute_workspace_list_areas(&app, Rect::new(0, 0, 30, 12));

        assert!(headers.is_empty());
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].ws_idx, 2);
    }

    #[test]
    fn workspace_list_entries_group_multiple_workspaces_in_same_git_space() {
        let mut app = AppState::test_new();
        app.workspaces = vec![
            workspace_with_worktree_space("main", Some("repo-key"), "/repo/herdr"),
            workspace_with_worktree_space("issue", Some("repo-key"), "/repo/herdr-issue"),
        ];

        assert_eq!(
            workspace_list_entries(&app),
            vec![
                WorkspaceListEntry::Workspace {
                    ws_idx: 0,
                    indented: false,
                },
                WorkspaceListEntry::Workspace {
                    ws_idx: 1,
                    indented: true,
                },
            ]
        );
    }

    #[test]
    fn workspace_list_entries_group_non_contiguous_explicit_members() {
        let mut app = AppState::test_new();
        app.workspaces = vec![
            workspace_with_worktree_space("main", Some("repo-key"), "/repo/herdr"),
            workspace_with_git_space("normal", "other-key"),
            workspace_with_worktree_space("issue", Some("repo-key"), "/repo/herdr-issue"),
        ];

        assert_eq!(
            workspace_list_entries(&app),
            vec![
                WorkspaceListEntry::Workspace {
                    ws_idx: 0,
                    indented: false,
                },
                WorkspaceListEntry::Workspace {
                    ws_idx: 2,
                    indented: true,
                },
                WorkspaceListEntry::Workspace {
                    ws_idx: 1,
                    indented: false,
                },
            ]
        );
    }

    #[test]
    fn workspace_list_entries_do_not_group_normal_git_workspaces() {
        let mut app = AppState::test_new();
        app.workspaces = vec![
            workspace_with_git_space("one", "repo-key"),
            workspace_with_git_space("two", "repo-key"),
        ];

        assert_eq!(
            workspace_list_entries(&app),
            vec![
                WorkspaceListEntry::Workspace {
                    ws_idx: 0,
                    indented: false,
                },
                WorkspaceListEntry::Workspace {
                    ws_idx: 1,
                    indented: false,
                },
            ]
        );
    }

    #[test]
    fn workspace_list_entries_do_not_auto_attach_normal_git_workspace_to_group() {
        let mut app = AppState::test_new();
        app.workspaces = vec![
            workspace_with_worktree_space("main", Some("repo-key"), "/repo/herdr"),
            workspace_with_git_space("scratch", "repo-key"),
            workspace_with_worktree_space("issue", Some("repo-key"), "/repo/herdr-issue"),
        ];

        assert_eq!(
            workspace_list_entries(&app),
            vec![
                WorkspaceListEntry::Workspace {
                    ws_idx: 0,
                    indented: false,
                },
                WorkspaceListEntry::Workspace {
                    ws_idx: 2,
                    indented: true,
                },
                WorkspaceListEntry::Workspace {
                    ws_idx: 1,
                    indented: false,
                },
            ]
        );
    }

    #[test]
    fn workspace_list_entries_leave_single_git_and_non_git_workspaces_flat() {
        let mut app = AppState::test_new();
        app.workspaces = vec![
            workspace_with_git_space("one", "repo-key"),
            workspace_with_worktree_space("notes", None, "/notes"),
        ];

        assert_eq!(
            workspace_list_entries(&app),
            vec![
                WorkspaceListEntry::Workspace {
                    ws_idx: 0,
                    indented: false,
                },
                WorkspaceListEntry::Workspace {
                    ws_idx: 1,
                    indented: false,
                },
            ]
        );
    }

    #[test]
    fn collapsed_group_hides_inactive_children_but_keeps_active_visible() {
        let mut app = AppState::test_new();
        app.workspaces = vec![
            workspace_with_worktree_space("main", Some("repo-key"), "/repo/herdr"),
            workspace_with_worktree_space("issue", Some("repo-key"), "/repo/herdr-issue"),
        ];
        app.active = Some(1);
        app.mode = Mode::Terminal;
        app.collapsed_space_keys.insert("repo-key".into());

        assert_eq!(
            workspace_list_entries(&app),
            vec![
                WorkspaceListEntry::Workspace {
                    ws_idx: 0,
                    indented: false,
                },
                WorkspaceListEntry::Workspace {
                    ws_idx: 1,
                    indented: true,
                },
            ]
        );

        app.active = None;
        app.mode = Mode::Terminal;
        assert_eq!(
            workspace_list_entries(&app),
            vec![WorkspaceListEntry::Workspace {
                ws_idx: 0,
                indented: false,
            }]
        );
    }

    #[test]
    fn collapsed_group_keeps_selected_child_visible_in_navigate_mode() {
        let mut app = AppState::test_new();
        app.workspaces = vec![
            workspace_with_worktree_space("main", Some("repo-key"), "/repo/herdr"),
            workspace_with_worktree_space("issue", Some("repo-key"), "/repo/herdr-issue"),
        ];
        app.mode = Mode::Navigate;
        app.selected = 1;
        app.active = Some(1);
        app.collapsed_space_keys.insert("repo-key".into());

        assert_eq!(
            workspace_list_entries(&app),
            vec![
                WorkspaceListEntry::Workspace {
                    ws_idx: 0,
                    indented: false,
                },
                WorkspaceListEntry::Workspace {
                    ws_idx: 1,
                    indented: true,
                },
            ]
        );
    }
}
