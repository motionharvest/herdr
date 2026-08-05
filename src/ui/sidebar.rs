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
/// Content rows per agent list entry: tab name, pane name, status.
pub(crate) const AGENT_PANEL_ENTRY_CONTENT_ROWS: u16 = 3;
/// Status glyph drawn down the left edge of an agent entry.
const AGENT_STATUS_BAR_GLYPH: &str = "▎";
/// Rows a space card occupies: top border, its name, bottom border.
const WORKSPACE_CARD_ROWS: u16 = 3;

pub(crate) struct AgentPanelEntry {
    pub ws_idx: usize,
    pub tab_idx: usize,
    pub pane_id: crate::layout::PaneId,
    /// Human name for the pane (manual label, else the assigned pane name).
    pub name: String,
    /// Name of the tab the pane lives in, as shown in the tab bar.
    pub tab_name: String,
    pub agent_label: Option<String>,
    pub model_info: Option<crate::agent_model::AgentModelInfo>,
    /// Where the agent is working: cwd plus git branch/dirty marker. The
    /// sidebar leads with the tab name instead; this is for the mobile list.
    pub location: Option<String>,
    pub state: AgentState,
    pub seen: bool,
    pub custom_status: Option<String>,
    pub state_labels: std::collections::HashMap<String, String>,
}

/// Whether the spaces section is folded down to the header row plus the
/// active space card.
/// Navigate mode temporarily reveals the list so workspace selection stays visible.
pub(crate) fn spaces_section_collapsed(app: &AppState) -> bool {
    app.spaces_collapsed && !matches!(app.mode, Mode::Navigate)
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

pub(crate) fn agent_panel_entries(app: &AppState) -> Vec<AgentPanelEntry> {
    agent_panel_entries_with_runtimes(app, None, scoped_workspace_indices(app))
}

pub(crate) fn agent_panel_entries_from(
    app: &AppState,
    terminal_runtimes: &TerminalRuntimeRegistry,
) -> Vec<AgentPanelEntry> {
    agent_panel_entries_with_runtimes(app, Some(terminal_runtimes), scoped_workspace_indices(app))
}

/// Sidebar entries cover every space: the list itself decides which spaces are
/// expanded, so the scope setting must not hide their agents.
fn all_workspace_agent_entries(
    app: &AppState,
    terminal_runtimes: &TerminalRuntimeRegistry,
) -> Vec<AgentPanelEntry> {
    agent_panel_entries_with_runtimes(
        app,
        Some(terminal_runtimes),
        (0..app.workspaces.len()).collect(),
    )
}

fn scoped_workspace_indices(app: &AppState) -> Vec<usize> {
    match app.agent_panel_scope {
        AgentPanelScope::CurrentWorkspace => {
            agent_panel_current_workspace_idx(app).into_iter().collect()
        }
        AgentPanelScope::AllWorkspaces => (0..app.workspaces.len()).collect(),
    }
}

fn agent_panel_entries_with_runtimes(
    app: &AppState,
    terminal_runtimes: Option<&TerminalRuntimeRegistry>,
    ws_indices: Vec<usize>,
) -> Vec<AgentPanelEntry> {
    let empty_runtimes;
    let terminal_runtimes = match terminal_runtimes {
        Some(terminal_runtimes) => terminal_runtimes,
        None => {
            empty_runtimes = TerminalRuntimeRegistry::new();
            &empty_runtimes
        }
    };

    let names = crate::pane_names::assigned_names(&app.terminals);

    let mut entries = Vec::new();
    for ws_idx in ws_indices {
        let Some(ws) = app.workspaces.get(ws_idx) else {
            continue;
        };
        for detail in ws.pane_details(&app.terminals) {
            let name = app
                .terminals
                .get(&detail.terminal_id)
                .and_then(|terminal| terminal.manual_label.clone())
                .or_else(|| names.get(&detail.terminal_id).cloned())
                .unwrap_or_else(|| detail.agent_label.clone());
            let location =
                agent_location_label(app, ws, detail.tab_idx, detail.pane_id, terminal_runtimes);
            entries.push(AgentPanelEntry {
                ws_idx,
                tab_idx: detail.tab_idx,
                pane_id: detail.pane_id,
                name,
                tab_name: detail.tab_label,
                agent_label: Some(detail.agent_label),
                model_info: detail.model_info,
                location,
                state: detail.state,
                seen: detail.seen,
                custom_status: detail.custom_status,
                state_labels: detail.state_labels,
            });
        }
    }
    entries
}

/// `~/lab/herdr (feat/space-done !)` — the pane's cwd with its git branch and
/// dirty marker when the pane is in a repository.
fn agent_location_label(
    app: &AppState,
    ws: &crate::workspace::Workspace,
    tab_idx: usize,
    pane_id: crate::layout::PaneId,
    terminal_runtimes: &TerminalRuntimeRegistry,
) -> Option<String> {
    let tab = ws.tabs.get(tab_idx)?;
    let cwd = tab.cwd_for_pane(pane_id, &app.terminals, terminal_runtimes)?;
    let mut label = super::panes::display_path_with_home(&cwd);
    let git_status = ws.git_status_for_pane(pane_id);
    if let Some(branch) = git_status.branch.filter(|branch| !branch.is_empty()) {
        label.push_str(&format!(
            " ({branch} {})",
            super::panes::worktree_state_marker(git_status.worktree_state)
        ));
    }
    Some(label)
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
    WORKSPACE_CARD_ROWS
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
    Workspace {
        ws_idx: usize,
        indented: bool,
    },
    /// Agent pane listed under its space's card.
    Agent {
        ws_idx: usize,
        tab_idx: usize,
        pane_id: crate::layout::PaneId,
    },
}

fn entry_row_height(
    app: &AppState,
    entry: &WorkspaceListEntry,
    prev: Option<&WorkspaceListEntry>,
    next: Option<&WorkspaceListEntry>,
) -> Option<u16> {
    match entry {
        WorkspaceListEntry::Workspace { ws_idx, .. } => {
            app.workspaces.get(*ws_idx).map(workspace_row_height)
        }
        // A space's last agent reserves two rows below it: the blank row that
        // pads the bottom of the space's outline, and the outline's own floor.
        WorkspaceListEntry::Agent { .. } => Some(
            agent_leading_gap(prev)
                + AGENT_PANEL_ENTRY_CONTENT_ROWS
                + if matches!(next, Some(WorkspaceListEntry::Agent { .. })) {
                    0
                } else {
                    2
                },
        ),
    }
}

/// Blank rows above an agent's content. Agents are separated from each other
/// by one, but the first agent under a space needs none: the space card's own
/// floor row already sits above it.
fn agent_leading_gap(prev: Option<&WorkspaceListEntry>) -> u16 {
    u16::from(matches!(prev, Some(WorkspaceListEntry::Agent { .. })))
}

/// Whether a space's agents are listed under its card. Spaces start expanded
/// and clicking the card folds them away.
pub(crate) fn workspace_agents_expanded(app: &AppState, ws_idx: usize) -> bool {
    app.workspaces
        .get(ws_idx)
        .is_some_and(|ws| !app.collapsed_agent_space_ids.contains(&ws.id))
}

fn push_workspace_with_agents(
    app: &AppState,
    ws_idx: usize,
    indented: bool,
    entries: &mut Vec<WorkspaceListEntry>,
) {
    entries.push(WorkspaceListEntry::Workspace { ws_idx, indented });
    if !workspace_agents_expanded(app, ws_idx) {
        return;
    }
    let Some(ws) = app.workspaces.get(ws_idx) else {
        return;
    };
    for detail in ws.pane_details(&app.terminals) {
        entries.push(WorkspaceListEntry::Agent {
            ws_idx,
            tab_idx: detail.tab_idx,
            pane_id: detail.pane_id,
        });
    }
}

pub(crate) fn normalized_workspace_scroll(app: &AppState, area: Rect, requested: usize) -> usize {
    let ws_area = workspace_list_rect(app, area);
    let body = workspace_list_body_rect(app, ws_area, false);
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
    // A collapsed spaces section keeps only the active space (and its agents)
    // visible under the header.
    if spaces_section_collapsed(app) {
        let mut entries = Vec::new();
        if let Some(ws_idx) = app.active.filter(|idx| *idx < app.workspaces.len()) {
            push_workspace_with_agents(app, ws_idx, false, &mut entries);
        }
        return entries;
    }

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
            push_workspace_with_agents(app, ws_idx, false, &mut entries);
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
            push_workspace_with_agents(app, ws_idx, false, &mut entries);
            continue;
        };
        let collapsed = app.collapsed_space_keys.contains(&space.key);
        push_workspace_with_agents(app, parent_idx, false, &mut entries);

        if collapsed {
            if let Some(active_idx) = visible_group_idx
                .filter(|idx| *idx != parent_idx)
                .filter(|_| active_group.as_deref() == Some(space.key.as_str()))
            {
                push_workspace_with_agents(app, active_idx, true, &mut entries);
            }
        } else {
            for member_idx in members {
                if *member_idx == parent_idx {
                    continue;
                }
                push_workspace_with_agents(app, *member_idx, true, &mut entries);
            }
        }
    }
    entries
}

/// The merged spaces+agents list fills the whole sidebar next to the divider
/// column.
pub(crate) fn workspace_list_rect(_app: &AppState, area: Rect) -> Rect {
    let content = Rect::new(area.x, area.y, area.width.saturating_sub(1), area.height);
    if content.width == 0 || content.height == 0 {
        return Rect::default();
    }
    content
}

pub(crate) fn workspace_list_body_rect(app: &AppState, area: Rect, has_scrollbar: bool) -> Rect {
    if area.width == 0 || area.height <= WORKSPACE_SECTION_HEADER_ROWS {
        return Rect::default();
    }

    let footer_rows = if spaces_section_collapsed(app) {
        0
    } else {
        WORKSPACE_SECTION_FOOTER_ROWS
    };
    let body_y = area.y.saturating_add(WORKSPACE_SECTION_HEADER_ROWS);
    let footer_y = area.y + area.height.saturating_sub(footer_rows);
    let body_height = footer_y.saturating_sub(body_y);
    let body_width = area.width.saturating_sub(u16::from(has_scrollbar));
    Rect::new(area.x, body_y, body_width, body_height)
}

fn workspace_list_visible_count(app: &AppState, area: Rect, scroll: usize) -> usize {
    let body = workspace_list_body_rect(app, area, false);
    if body.width == 0 || body.height == 0 {
        return 0;
    }

    let mut used_rows = 0u16;
    let mut visible = 0usize;
    let entries = workspace_list_entries(app);
    for (idx, entry) in entries.iter().enumerate().skip(scroll) {
        let Some(row_height) = entry_row_height(
            app,
            entry,
            idx.checked_sub(1).and_then(|prev| entries.get(prev)),
            entries.get(idx + 1),
        ) else {
            continue;
        };
        if used_rows.saturating_add(row_height) > body.height {
            break;
        }
        used_rows = used_rows.saturating_add(row_height);
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
    let body = workspace_list_body_rect(app, area, true);
    (should_show_scrollbar(metrics) && body.width > 0 && body.height > 0).then_some(Rect::new(
        area.x + area.width.saturating_sub(1),
        body.y,
        1,
        body.height,
    ))
}

#[derive(Default)]
pub(crate) struct WorkspaceListLayout {
    pub cards: Vec<crate::app::state::WorkspaceCardArea>,
    pub agent_rows: Vec<crate::app::state::AgentRowArea>,
    /// `+ new` button, placed right below the last entry in the list.
    pub new_button: Rect,
}

fn workspace_list_layout(app: &AppState, area: Rect) -> WorkspaceListLayout {
    let ws_area = workspace_list_rect(app, area);
    if ws_area == Rect::default() {
        return WorkspaceListLayout::default();
    }

    let metrics = workspace_list_scroll_metrics(app, ws_area);
    let body = workspace_list_body_rect(app, ws_area, should_show_scrollbar(metrics));
    if body.width == 0 || body.height == 0 {
        return WorkspaceListLayout::default();
    }

    // A stale scroll offset from the expanded list must not hide the lone
    // active card while the section is collapsed.
    let scroll = if spaces_section_collapsed(app) {
        0
    } else {
        app.workspace_scroll
    };
    let mut row_y = body.y;
    let body_bottom = body.y + body.height;
    let mut cards = Vec::new();
    let mut agent_rows = Vec::new();

    let entries = workspace_list_entries(app);
    for (idx, entry) in entries.iter().enumerate().skip(scroll) {
        let prev = idx.checked_sub(1).and_then(|prev| entries.get(prev));
        let Some(row_height) = entry_row_height(app, entry, prev, entries.get(idx + 1)) else {
            continue;
        };
        if row_y.saturating_add(row_height) > body_bottom {
            break;
        }
        match entry {
            WorkspaceListEntry::Workspace { ws_idx, indented } => {
                cards.push(crate::app::state::WorkspaceCardArea {
                    ws_idx: *ws_idx,
                    rect: Rect::new(body.x, row_y, body.width, row_height),
                    indented: *indented,
                });
            }
            WorkspaceListEntry::Agent {
                ws_idx,
                tab_idx,
                pane_id,
            } => {
                agent_rows.push(crate::app::state::AgentRowArea {
                    ws_idx: *ws_idx,
                    tab_idx: *tab_idx,
                    pane_id: *pane_id,
                    rect: Rect::new(
                        body.x,
                        row_y + agent_leading_gap(prev),
                        body.width,
                        AGENT_PANEL_ENTRY_CONTENT_ROWS,
                    ),
                });
            }
        }
        row_y = row_y.saturating_add(row_height);
    }

    WorkspaceListLayout {
        cards,
        agent_rows,
        new_button: new_workspace_button_rect_below(app, ws_area, row_y),
    }
}

/// The `+ new` button hugs the bottom of the list rather than the panel: it
/// follows the last entry down and stops once it reaches the sidebar's floor.
fn new_workspace_button_rect_below(app: &AppState, ws_area: Rect, list_bottom: u16) -> Rect {
    if spaces_section_collapsed(app) || ws_area.height < WORKSPACE_SECTION_FOOTER_ROWS {
        return Rect::default();
    }
    // A gap row keeps the button off the last entry and leaves room for the
    // reorder drop indicator.
    let floor = ws_area.y + ws_area.height.saturating_sub(WORKSPACE_SECTION_FOOTER_ROWS);
    Rect::new(
        ws_area.x,
        list_bottom.saturating_add(1).min(floor),
        ws_area.width,
        WORKSPACE_SECTION_FOOTER_ROWS,
    )
}

pub(crate) fn compute_workspace_list_areas(
    app: &AppState,
    area: Rect,
) -> (
    Vec<crate::app::state::WorkspaceCardArea>,
    Vec<crate::app::state::AgentRowArea>,
) {
    let layout = workspace_list_layout(app, area);
    (layout.cards, layout.agent_rows)
}

/// Hit area and draw target for the sidebar's `+ new` button.
pub(crate) fn new_workspace_button_rect(app: &AppState, area: Rect) -> Rect {
    workspace_list_layout(app, area).new_button
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

/// Screen row for an agent reorder drop marker. `agent_rows` must already be
/// narrowed to the dragged space, and `ordered` is that space's current agent
/// order. Returns `None` when the slot is scrolled out of view.
pub(crate) fn agent_drop_indicator_row(
    agent_rows: &[crate::app::state::AgentRowArea],
    ordered: &[crate::layout::PaneId],
    insert_idx: usize,
) -> Option<u16> {
    let last = agent_rows.last()?;
    // Every agent entry reserves a gap row above its content, which is where
    // the marker goes; dropping at the end uses the row just past the block.
    match ordered.get(insert_idx) {
        Some(pane_id) => agent_rows
            .iter()
            .find(|area| area.pane_id == *pane_id)
            .and_then(|area| area.rect.y.checked_sub(1)),
        None => Some(last.rect.y.saturating_add(last.rect.height)),
    }
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

/// Clickable "spaces" header label — everything on the header row between the
/// sidebar toggle glyph and the right divider column.
pub(crate) fn spaces_section_header_rect(area: Rect) -> Rect {
    if area.width <= 2 || area.height == 0 {
        return Rect::default();
    }
    Rect::new(area.x + 1, area.y, area.width.saturating_sub(2), 1)
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

    let spaces_chevron = if spaces_section_collapsed(app) {
        "▸"
    } else {
        "▾"
    };
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
            Span::styled(
                format!(" spaces {spaces_chevron}"),
                Style::default().fg(app.palette.overlay0),
            ),
        ]),
    );

    render_workspace_rows(app, terminal_runtimes, frame, area);
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
    let layout = workspace_list_layout(app, area);
    let (cards, agent_rows) = if app.view.workspace_card_areas.is_empty() {
        (layout.cards, layout.agent_rows)
    } else {
        (
            app.view.workspace_card_areas.clone(),
            app.view.agent_row_areas.clone(),
        )
    };

    let outlined = outlined_space_group(app, &cards, &agent_rows);

    render_agent_rows(app, terminal_runtimes, frame, &agent_rows);
    render_agent_drop_indicator(app, frame, &agent_rows);

    for card in cards {
        let Some(ws) = app.workspaces.get(card.ws_idx) else {
            continue;
        };
        let is_active = Some(card.ws_idx) == app.active;
        let is_selected = app.mode == Mode::Navigate && card.ws_idx == app.selected;
        let selected = is_selected || is_active;
        let (state, seen) = ws.aggregate_state(&app.terminals);
        let name = workspace_card_label(app, terminal_runtimes, ws, card.indented, state, seen);
        // A card inside the group outline stops at its name: the outline
        // supplies the enclosing box, so its own floor would only cut the
        // group in half.
        let open_bottom =
            outlined.is_some_and(|group| group.y == card.rect.y && group.height > card.rect.height);
        render_workspace_card(frame, card.rect, &name, selected, open_bottom, app);
    }

    render_space_group_outline(app, frame, area, outlined);

    if layout.new_button != Rect::default() {
        render_new_workspace_button(frame, layout.new_button, app);
    }
}

/// The space you are working in, as the box the outline should draw: its card
/// plus every agent row listed under it. `None` when that space has scrolled
/// out of the list.
fn outlined_space_group(
    app: &AppState,
    cards: &[crate::app::state::WorkspaceCardArea],
    agent_rows: &[crate::app::state::AgentRowArea],
) -> Option<Rect> {
    let ws_idx = if app.mode == Mode::Navigate {
        Some(app.selected)
    } else {
        app.active
    }?;
    let card = cards.iter().find(|card| card.ws_idx == ws_idx)?.rect;
    if card.width < 2 || card.height == 0 {
        return None;
    }
    // The last agent reserves a blank row and then the floor, so the box pads
    // its bottom the way the card's own floor row pads its top.
    let bottom = agent_rows
        .iter()
        .filter(|row| row.ws_idx == ws_idx)
        .map(|row| row.rect.y + row.rect.height + 1)
        .max()
        .unwrap_or(card.y + card.height - 1);
    Some(Rect::new(
        card.x,
        card.y,
        card.width,
        bottom.saturating_sub(card.y).saturating_add(1),
    ))
}

/// Draws the selected space and its agents as one box. This runs after the
/// cards so the accent edges win over the card's own dim border, which is what
/// turns a card and a loose list into a group you can read at a glance.
fn render_space_group_outline(app: &AppState, frame: &mut Frame, area: Rect, group: Option<Rect>) {
    let Some(rect) = group else {
        return;
    };
    if rect.width < 2 || rect.height < 2 {
        return;
    }

    let list = workspace_list_rect(app, area);
    let visible = |y: u16| list.height > 0 && y >= list.y && y < list.y + list.height;
    let style = Style::default().fg(app.palette.focused_pane_border());
    let right = rect.x + rect.width - 1;
    let top = rect.y;
    let bottom = rect.y + rect.height - 1;
    let buf = frame.buffer_mut();

    for y in top + 1..bottom {
        if !visible(y) {
            continue;
        }
        buf[(rect.x, y)].set_symbol("│").set_style(style);
        buf[(right, y)].set_symbol("│").set_style(style);
    }

    for (y, left_corner, right_corner) in [(top, "╭", "╮"), (bottom, "╰", "╯")] {
        if !visible(y) {
            continue;
        }
        buf[(rect.x, y)].set_symbol(left_corner).set_style(style);
        buf[(right, y)].set_symbol(right_corner).set_style(style);
        for x in rect.x + 1..right {
            buf[(x, y)].set_symbol("─").set_style(style);
        }
    }
}

/// The pane the user is typing into, as a sidebar agent-row key.
fn focused_agent_row(app: &AppState) -> Option<(usize, usize, crate::layout::PaneId)> {
    let ws_idx = app.active?;
    let ws = app.workspaces.get(ws_idx)?;
    Some((ws_idx, ws.active_tab, ws.focused_pane_id()?))
}

/// Marks where a dragged agent row would land. Space cards have no equivalent
/// marker, but agent rows are three lines tall and identical in shape, so the
/// drop slot is otherwise impossible to read.
fn render_agent_drop_indicator(
    app: &AppState,
    frame: &mut Frame,
    agent_rows: &[crate::app::state::AgentRowArea],
) {
    let Some(crate::app::state::DragTarget::AgentReorder {
        ws_idx,
        insert_idx: Some(insert_idx),
        ..
    }) = app.drag.as_ref().map(|drag| &drag.target)
    else {
        return;
    };
    let Some(ws) = app.workspaces.get(*ws_idx) else {
        return;
    };
    let rows = agent_rows
        .iter()
        .filter(|area| area.ws_idx == *ws_idx)
        .cloned()
        .collect::<Vec<_>>();
    let Some(row) = agent_drop_indicator_row(&rows, &ws.ordered_pane_ids(), *insert_idx) else {
        return;
    };
    let Some(rect) = rows.first().map(|area| area.rect) else {
        return;
    };
    let list = workspace_list_rect(app, app.view.sidebar_rect);
    if list.height == 0 || row < list.y || row >= list.y + list.height {
        return;
    }

    let style = Style::default().fg(app.palette.accent);
    let buf = frame.buffer_mut();
    for x in rect.x..rect.x + rect.width {
        buf[(x, row)].set_symbol("─").set_style(style);
    }
}

fn pane_swap_create_space_hover(app: &AppState) -> bool {
    matches!(
        app.drag.as_ref().map(|drag| &drag.target),
        Some(crate::app::state::DragTarget::PaneSwap {
            create_space: true,
            moved: true,
            ..
        })
    )
}

fn render_new_workspace_button(frame: &mut Frame, rect: Rect, app: &AppState) {
    if rect.width < 2 || rect.height < 3 {
        return;
    }

    let create_space_hover = pane_swap_create_space_hover(app);
    let accent = if create_space_hover {
        app.palette.accent
    } else {
        app.palette.overlay0
    };
    let border_style = Style::default().fg(accent);
    let label_style = Style::default().fg(accent);

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

    let label = if create_space_hover {
        "Drop to create"
    } else {
        "+ new"
    };
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

/// A space card carries its name and nothing else: the branch it used to show
/// is already on the pane, where the work happens.
fn workspace_card_label(
    app: &AppState,
    terminal_runtimes: &TerminalRuntimeRegistry,
    ws: &crate::workspace::Workspace,
    indented: bool,
    state: AgentState,
    seen: bool,
) -> String {
    let (dot, _) = sidebar_state_dot(state, seen, app);
    let indent = if indented { "  " } else { "" };
    let name = ws.display_name_from(&app.terminals, terminal_runtimes);
    format!("{indent}{dot} {name}")
}

fn render_workspace_card(
    frame: &mut Frame,
    rect: Rect,
    name: &str,
    selected: bool,
    open_bottom: bool,
    app: &AppState,
) {
    if rect.width < 2 || rect.height == 0 {
        return;
    }

    // Cards keep a dim border whatever their state: the accent outline belongs
    // to the focused agent row, so a selected space reads through its name.
    let border_color = app.palette.surface_dim;
    let name_color = if selected {
        app.palette.accent
    } else {
        app.palette.text
    };
    let border_style = Style::default().fg(border_color);
    let name_style = Style::default().fg(name_color).add_modifier(Modifier::BOLD);
    let inner_width = rect.width.saturating_sub(2) as usize;
    let name = pad_to_width(&truncate_chars(name, inner_width), inner_width);

    let buf = frame.buffer_mut();
    let right = rect.x + rect.width.saturating_sub(1);

    if rect.height < WORKSPACE_CARD_ROWS {
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

    if open_bottom {
        return;
    }

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

fn render_agent_rows(
    app: &AppState,
    terminal_runtimes: &TerminalRuntimeRegistry,
    frame: &mut Frame,
    agent_rows: &[crate::app::state::AgentRowArea],
) {
    if agent_rows.is_empty() {
        return;
    }

    let entries = all_workspace_agent_entries(app, terminal_runtimes);
    let focused_agent = focused_agent_row(app);

    for row in agent_rows {
        let Some(entry) = entries.iter().find(|entry| {
            entry.ws_idx == row.ws_idx
                && entry.tab_idx == row.tab_idx
                && entry.pane_id == row.pane_id
        }) else {
            continue;
        };
        let rect = row.rect;
        if rect.width < 4 || rect.height < AGENT_PANEL_ENTRY_CONTENT_ROWS {
            continue;
        }
        let is_focused_pane = focused_agent.is_some_and(|(ws_idx, tab_idx, pane_id)| {
            entry.ws_idx == ws_idx && entry.tab_idx == tab_idx && entry.pane_id == pane_id
        });
        let title = agent_entry_title(entry);
        let status_line = agent_status_line(entry);
        let color = match entry.state {
            AgentState::Working => app.palette.yellow,
            AgentState::Blocked => app.palette.red,
            AgentState::Idle if !entry.seen => app.palette.green,
            AgentState::Idle => app.palette.overlay0,
            AgentState::Unknown => app.palette.overlay0,
        };
        // Status is shown as a vertical bar running down the entry's left edge
        // instead of a bullet. It is inset by two so it clears the space
        // outline's edge with a column to spare.
        let bar = Span::styled(
            format!("  {AGENT_STATUS_BAR_GLYPH} "),
            Style::default().fg(color),
        );
        let text_width = rect.width.saturating_sub(5) as usize;
        // The pane you are typing into reads in the same color its own chrome
        // uses when focused, so the sidebar and the pane agree.
        let name_style = if is_focused_pane {
            Style::default()
                .fg(app.palette.focused_pane_border())
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(app.palette.text)
        };
        // The tab is what the row is really about, so it leads; the pane's own
        // name sits under it.
        render_sidebar_line(
            frame,
            Rect::new(rect.x, rect.y, rect.width, 1),
            Line::from(vec![
                bar.clone(),
                Span::styled(truncate_chars(&entry.tab_name, text_width), name_style),
            ]),
        );
        let mut title_spans = vec![bar.clone()];
        super::panes::push_title_name_spans(
            &mut title_spans,
            &truncate_chars(&title, text_width),
            Style::default().fg(app.palette.overlay1),
            Style::default().fg(app.palette.overlay1),
            Style::default().fg(app.palette.overlay1),
        );
        render_sidebar_line(
            frame,
            Rect::new(rect.x, rect.y + 1, rect.width, 1),
            Line::from(title_spans),
        );
        render_sidebar_line(
            frame,
            Rect::new(rect.x, rect.y + 2, rect.width, 1),
            Line::from(vec![
                bar,
                Span::styled(
                    truncate_chars(&status_line, text_width),
                    Style::default().fg(app.palette.overlay0),
                ),
            ]),
        );
    }
}

/// Second entry row: the pane's human name plus the harness, e.g.
/// `Olivia {Claude}`.
pub(crate) fn agent_entry_title(entry: &AgentPanelEntry) -> String {
    match entry.agent_label.as_deref().map(harness_display_name) {
        Some(harness) => format!("{} {{{harness}}}", entry.name),
        None => entry.name.clone(),
    }
}

/// Third entry row: model + effort with the pane state, e.g.
/// `Fable 5 high · idle`, or just the state when no model is known.
pub(crate) fn agent_status_line(entry: &AgentPanelEntry) -> String {
    let status = agent_panel_status_key(entry.state, entry.seen);
    match &entry.model_info {
        Some(info) => format!("{} · {status}", info.display_label()),
        None => status.to_string(),
    }
}

fn harness_display_name(label: &str) -> String {
    match crate::detect::parse_agent_label(label) {
        Some(agent) => crate::detect::agent_display_name(agent).to_string(),
        None => label.to_string(),
    }
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
    use ratatui::{backend::TestBackend, Terminal};

    /// One space holding a single agent pane, rendered into `area`.
    fn render_sidebar_list(area: Rect) -> (crate::app::state::AppState, Terminal<TestBackend>) {
        let mut app = crate::app::state::AppState::test_new();
        let workspace = Workspace::test_new("herdr");
        let pane = workspace.tabs[0].root_pane;
        app.workspaces = vec![workspace];
        app.ensure_test_terminals();
        let terminal_id = app.workspaces[0].tabs[0].panes[&pane]
            .attached_terminal_id
            .clone();
        app.terminals.get_mut(&terminal_id).unwrap().detected_agent = Some(Agent::Claude);
        app.active = Some(0);
        app.selected = 0;

        let runtimes = TerminalRuntimeRegistry::new();
        let backend = TestBackend::new(area.x + area.width, area.y + area.height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| render_workspace_rows(&app, &runtimes, frame, area))
            .unwrap();
        (app, terminal)
    }

    #[test]
    fn selected_space_outline_encloses_its_card_and_every_agent() {
        let area = Rect::new(0, 0, 28, 24);
        let (app, terminal) = render_sidebar_list(area);
        let card = compute_workspace_card_areas(&app, area)[0].rect;
        let rows = compute_workspace_list_areas(&app, area).1;
        let last = rows
            .last()
            .expect("the space's agent should have a row")
            .rect;
        let right = card.x + card.width - 1;
        // A blank row pads the bottom of the box, matching the card's floor row
        // at the top, so the outline closes one row below that.
        let bottom = last.y + last.height + 1;
        let accent = app.palette.focused_pane_border();
        let buf = terminal.backend().buffer();

        // One box: it opens on the card and closes below the last agent.
        assert_eq!(buf[(card.x, card.y)].symbol(), "╭");
        assert_eq!(buf[(right, card.y)].symbol(), "╮");
        assert_eq!(buf[(card.x, bottom)].symbol(), "╰");
        assert_eq!(buf[(right, bottom)].symbol(), "╯");
        assert_eq!(buf[(card.x, card.y)].style().fg, Some(accent));
        assert_eq!(buf[(card.x, bottom)].style().fg, Some(accent));

        // Every row in between is a side, including the card's own floor and
        // the agent rows the outline now owns.
        for y in card.y + 1..bottom {
            assert_eq!(buf[(card.x, y)].symbol(), "│", "left edge at row {y}");
            assert_eq!(buf[(right, y)].symbol(), "│", "right edge at row {y}");
            assert_eq!(buf[(card.x, y)].style().fg, Some(accent));
        }

        // The card's old floor is gone, so the group does not read as a card
        // with a list stuck under it.
        let card_floor = card.y + card.height - 1;
        assert_eq!(buf[(card.x + 1, card_floor)].symbol(), " ");
    }

    #[test]
    fn unselected_space_card_keeps_its_own_dim_border() {
        let area = Rect::new(0, 0, 28, 24);
        let (mut app, _) = render_sidebar_list(area);
        app.active = None;
        app.selected = 1;

        let runtimes = TerminalRuntimeRegistry::new();
        let backend = TestBackend::new(area.x + area.width, area.y + area.height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| render_workspace_rows(&app, &runtimes, frame, area))
            .unwrap();

        let card = compute_workspace_card_areas(&app, area)[0].rect;
        let buf = terminal.backend().buffer();
        let floor = card.y + card.height - 1;

        assert_eq!(buf[(card.x, card.y)].symbol(), "╭");
        assert_eq!(
            buf[(card.x, card.y)].style().fg,
            Some(app.palette.surface_dim)
        );
        assert_eq!(buf[(card.x, floor)].symbol(), "╰");
    }

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
    fn all_workspaces_agent_panel_entries_use_pane_names_and_agent_labels() {
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

        let names = crate::pane_names::assigned_names(&app.terminals);
        let entries = agent_panel_entries(&app);
        assert_eq!(Some(&entries[0].name), names.get(&first_terminal_id));
        assert_eq!(entries[0].agent_label.as_deref(), Some("pi"));
        assert_eq!(Some(&entries[1].name), names.get(&second_terminal_id));
        assert_eq!(entries[1].agent_label.as_deref(), Some("claude"));
    }

    #[test]
    fn agent_panel_entry_name_prefers_manual_label() {
        let mut app = crate::app::state::AppState::test_new();
        let workspace = Workspace::test_new("bridge");
        let pane = workspace.tabs[0].root_pane;

        app.workspaces = vec![workspace];
        app.ensure_test_terminals();
        let terminal_id = app.workspaces[0].tabs[0].panes[&pane]
            .attached_terminal_id
            .clone();
        let terminal = app.terminals.get_mut(&terminal_id).unwrap();
        terminal.detected_agent = Some(Agent::Claude);
        terminal.set_manual_label("reviewer".into());
        app.active = Some(0);
        app.selected = 0;
        app.agent_panel_scope = AgentPanelScope::AllWorkspaces;

        let entries = agent_panel_entries(&app);
        assert_eq!(entries[0].name, "reviewer");
    }

    #[tokio::test]
    async fn all_workspaces_agent_panel_entries_use_live_runtime_cwd_for_location() {
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
        let location = entries[0].location.clone();

        for (_, runtime) in runtime_registry.drain() {
            runtime.shutdown();
        }
        let _ = std::fs::remove_dir_all(root);

        let location = location.expect("live runtime cwd should produce a location");
        assert!(
            location.starts_with(&live_cwd.display().to_string()),
            "location {location:?} should start with live cwd {live_cwd:?}"
        );
    }

    #[test]
    fn current_workspace_agent_panel_entries_use_pane_names_and_agent_labels() {
        let mut app = crate::app::state::AppState::test_new();
        let workspace = Workspace::test_new("bridge");
        let pane = workspace.tabs[0].root_pane;

        app.workspaces = vec![workspace];
        app.ensure_test_terminals();
        let terminal_id = app.workspaces[0].tabs[0].panes[&pane]
            .attached_terminal_id
            .clone();
        app.terminals.get_mut(&terminal_id).unwrap().detected_agent = Some(Agent::Claude);
        app.active = Some(0);
        app.selected = 0;
        app.agent_panel_scope = AgentPanelScope::CurrentWorkspace;

        let names = crate::pane_names::assigned_names(&app.terminals);
        let entries = agent_panel_entries(&app);
        assert_eq!(Some(&entries[0].name), names.get(&terminal_id));
        assert_eq!(entries[0].agent_label.as_deref(), Some("claude"));
    }

    fn test_entry() -> AgentPanelEntry {
        AgentPanelEntry {
            ws_idx: 0,
            tab_idx: 0,
            pane_id: crate::layout::PaneId::from_raw(1),
            name: "Olivia".into(),
            tab_name: "api".into(),
            agent_label: Some("claude".into()),
            model_info: Some(crate::agent_model::AgentModelInfo {
                model: "claude-fable-5".into(),
                effort: Some("high".into()),
            }),
            location: Some("~/lab/herdr (feat/space-done !)".into()),
            state: AgentState::Idle,
            seen: true,
            custom_status: None,
            state_labels: std::collections::HashMap::new(),
        }
    }

    #[test]
    fn agent_entry_title_shows_name_with_braced_harness() {
        let mut entry = test_entry();
        assert_eq!(agent_entry_title(&entry), "Olivia {Claude}");

        entry.agent_label = Some("planner".into());
        assert_eq!(agent_entry_title(&entry), "Olivia {planner}");

        entry.agent_label = None;
        assert_eq!(agent_entry_title(&entry), "Olivia");
    }

    #[test]
    fn agent_status_line_combines_model_effort_and_state() {
        let mut entry = test_entry();
        assert_eq!(agent_status_line(&entry), "Fable 5 high · idle");

        entry.model_info = None;
        assert_eq!(agent_status_line(&entry), "idle");

        entry.seen = false;
        assert_eq!(agent_status_line(&entry), "done");
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
        assert_eq!(entries[0].agent_label.as_deref(), Some("planner"));
    }

    #[test]
    fn collapsed_spaces_section_keeps_only_active_space_at_top() {
        let mut app = AppState::test_new();
        app.workspaces = vec![Workspace::test_new("one"), Workspace::test_new("two")];
        app.mode = Mode::Terminal;
        app.active = Some(1);
        app.spaces_collapsed = true;
        app.workspace_scroll = 1;
        let area = Rect::new(0, 0, 30, 40);

        let cards = compute_workspace_card_areas(&app, area);
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].ws_idx, 1);
        assert_eq!(cards[0].rect, Rect::new(0, 1, 29, WORKSPACE_CARD_ROWS));
    }

    #[test]
    fn collapsed_spaces_section_without_active_space_leaves_header_row_only() {
        let mut app = AppState::test_new();
        app.workspaces = vec![Workspace::test_new("one"), Workspace::test_new("two")];
        app.mode = Mode::Terminal;
        app.active = None;
        app.spaces_collapsed = true;
        let area = Rect::new(0, 0, 30, 40);

        assert!(workspace_list_entries(&app).is_empty());
        assert!(compute_workspace_card_areas(&app, area).is_empty());
    }

    #[test]
    fn navigate_mode_reveals_collapsed_spaces_section() {
        let mut app = AppState::test_new();
        app.workspaces = vec![Workspace::test_new("one"), Workspace::test_new("two")];
        app.mode = Mode::Navigate;
        app.selected = 0;
        app.spaces_collapsed = true;
        let area = Rect::new(0, 0, 30, 40);

        assert_eq!(compute_workspace_card_areas(&app, area).len(), 2);
    }

    #[test]
    fn spaces_section_header_sits_between_toggle_and_divider() {
        let area = Rect::new(2, 3, 26, 20);
        let header = spaces_section_header_rect(area);

        assert_eq!(header, Rect::new(3, 3, 24, 1));
    }

    #[test]
    fn workspace_list_rect_spans_sidebar_next_to_divider_column() {
        let app = AppState::test_new();

        assert_eq!(
            workspace_list_rect(&app, Rect::new(0, 0, 20, 5)),
            Rect::new(0, 0, 19, 5)
        );
        assert_eq!(
            workspace_list_rect(&app, Rect::new(0, 0, 1, 5)),
            Rect::default()
        );
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
