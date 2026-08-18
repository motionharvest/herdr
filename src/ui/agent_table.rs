//! The agent table: the band between the composer and the panes.
//!
//! One row per agent, every agent herdr is running, whichever space it belongs
//! to. The row is what the sidebar's card used to be, written across instead of
//! down: what the agent is called, what it says it is doing, the folder it is
//! in, which harness is behind it, how long it has been running or idle, and
//! the branch it is on. How it is getting on is the margin beside the row
//! rather than a column of its own.
//!
//! The table takes the rows it needs and no more, up to a share of the frame,
//! because the panes below it are the part actually being watched. Agents past
//! what one column of rows can hold start a second group of columns to its
//! right rather than making the table taller, so the table grows sideways into
//! room it already has before it starts scrolling.
//!
//! Where everything sits is worked out once, in [`split_agent_table`], and the
//! drawing and the hit testing both read that one answer — so a click cannot
//! land somewhere the drawing did not put anything.

use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::Line,
    widgets::Paragraph,
    Frame,
};

use super::panes::{focus_accent, mute_when_host_unfocused};
use crate::app::state::Palette;
use crate::app::AppState;
use crate::detect::AgentState;
use crate::layout::PaneId;
use crate::terminal::TerminalRuntimeRegistry;

/// The columns, left to right. Summary has no length of its own: it takes
/// whatever the others leave, even though it is not last.
///
/// State is not a column. The margin beside a row already says whether the
/// agent is working, waiting on an answer, or finished, in one cell and in
/// color, and a word repeating it would only take room from the summary.
const HEADINGS: [&str; 7] = [
    "Agent Name",
    "Summary",
    "Directory",
    "Agent",
    "Run",
    "Idle",
    "Git Status",
];
const COLUMNS: usize = HEADINGS.len();
const COL_NAME: usize = 0;
const COL_SUMMARY: usize = 1;
const COL_DIRECTORY: usize = 2;
const COL_AGENT: usize = 3;
const COL_GIT: usize = 6;
/// The air after a column's widest cell, before the next column starts.
const COLUMN_GAP: usize = 3;
/// The left margin, clear of the first column, where the animation goes.
const GUTTER: u16 = 2;
/// The table may not take more than this share of the room below the composer,
/// however many agents there are.
const TABLE_SHARE: u16 = 3;
/// How many agents one group of columns holds before the next one starts to its
/// right. A group that runs out of rows before it runs out of agents starts the
/// next one early, so this is the most a group ever holds, not the least.
const COLUMN_AGENTS: usize = 10;
/// The narrowest a group may be squeezed before the table stops adding them and
/// goes back to scrolling.
const MIN_GROUP: u16 = 24;
/// A blank row between the table and the panes, so the two read as separate
/// bands rather than one list that happens to have borders halfway down it.
const TABLE_FOOTER_ROWS: u16 = 1;
/// What stands in the margin beside an agent that finished unwatched. A round
/// dot rather than an emoji: an emoji is two columns wide in some terminals and
/// one in others, and a margin is one column wide in all of them.
const FINISHED: &str = "●";
/// What the dot becomes once it is clicked: the agent still finished, and the
/// row still says so, but it is no longer asking to be looked at.
const ACKNOWLEDGED: &str = "✓";
/// What stands beside an agent that is waiting on an answer, which is the one
/// state that wants something from you rather than reporting on itself.
const BLOCKED: &str = "◆";
const LAND_FAILED: &str = "✕";
/// Half blocks closing either end of the selected row's band. Each fills the
/// half of its cell that faces the text, so the band gains half a column of
/// padding on each side instead of a full one.
const BAND_CAP_LEFT: &str = "▐";
const BAND_CAP_RIGHT: &str = "▌";
/// The rule standing between one group of columns and the next, drawn the whole
/// height of the table so the two groups read as two tables side by side rather
/// than as one row that happens to be twice as long. Doubled because a single
/// line is what a box border is made of, and this is not a border.
const GROUP_DIVIDER: &str = "║";
/// The column the divider stands in. Every group but the rightmost gives one up,
/// so no group's text ever runs into the rule.
const DIVIDER_WIDTH: u16 = 1;
/// The insertion marker replaces the row's one-cell status gutter while a row
/// is carried. Its direction says which side of the pointed row owns the slot.
const DROP_BEFORE: &str = "▲";
const DROP_AFTER: &str = "▼";
/// What the table says in place of rows when nothing is running yet.
const NO_AGENTS: &str = "nothing running yet — send a task to start an agent";
/// The hues a folder may be written in, in degrees. Reds and greens are left
/// out on purpose: those two carry state in this table — blocked and finished —
/// and a folder that borrowed either would answer a question it was not asked.
/// The rest of the circle is sampled far enough apart that two folders on
/// screen together read as two colors rather than one shade twice.
const FOLDER_HUES: [f32; 9] = [45.0, 170.0, 192.0, 214.0, 236.0, 258.0, 280.0, 302.0, 324.0];
/// How much color a folder is written with. Held constant so the hue is the
/// only thing that varies between folders; the theme still sets how light the
/// text is, which is what decides its contrast against the background.
const FOLDER_SATURATION: f32 = 0.45;

/// One agent, as the table knows it.
pub(crate) struct AgentPanelEntry {
    /// Whether a pane somewhere shows this agent. A set-down agent keeps its
    /// row with `docked` false; `ws_idx` and `tab_idx` mean nothing for it.
    pub docked: bool,
    pub ws_idx: usize,
    pub tab_idx: usize,
    pub pane_id: PaneId,
    pub terminal_id: crate::terminal::TerminalId,
    /// Human name for the pane (manual label, else the assigned pane name).
    pub name: String,
    pub agent_label: Option<String>,
    pub model_info: Option<crate::agent_model::AgentModelInfo>,
    /// Where the agent is working: cwd plus git branch and dirty marker.
    pub location: Option<AgentLocation>,
    pub state: AgentState,
    pub seen: bool,
    /// Whether the agent has finished a run since it last worked, which is what
    /// separates a row that has something to report from a quiet one.
    pub completed: bool,
    pub custom_status: Option<String>,
    pub state_labels: std::collections::HashMap<String, String>,
    pub run_duration: Option<std::time::Duration>,
    pub idle_duration: Option<std::time::Duration>,
    pub landing: bool,
    pub land_failed: bool,
}

/// Where an agent is working, kept in its two parts so the table can put the
/// folder in one column and the branch in another.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AgentLocation {
    /// The pane's cwd, with `$HOME` written as `~`.
    pub path: String,
    /// `feat/space-done !` — branch and worktree marker, absent outside a repo.
    pub git: Option<String>,
}

impl AgentLocation {
    /// The last folder of the path: `herdr` from `~/lab/herdr`.
    pub(crate) fn folder(&self) -> &str {
        last_folder(&self.path)
    }

    /// The whole thing at full length: `~/lab/herdr (feat/space-done !)`.
    pub(crate) fn label(&self) -> String {
        match &self.git {
            Some(git) => format!("{} ({git})", self.path),
            None => self.path.clone(),
        }
    }
}

/// The last folder in a path. A trailing slash is ignored, so `~/lab/herdr/`
/// and `~/lab/herdr` name the same folder. A path with no slash is itself.
fn last_folder(path: &str) -> &str {
    path.trim_end_matches('/')
        .rsplit('/')
        .next()
        .filter(|part| !part.is_empty())
        .unwrap_or(path)
}

/// One group of columns: its own headings, its own columns, and its own slice of
/// the table's width. Every group is written the same way, so which one an agent
/// is in says nothing about the agent — only that the group to its left was
/// full.
#[derive(Debug, Clone, Default)]
pub struct AgentTableGroup {
    pub area: Rect,
    /// Where each of this group's columns starts and how wide it is.
    pub columns: Vec<Rect>,
}

/// Where one agent is drawn, and which agent it is. The rect is the whole of the
/// row inside its group, which is what a click lands on.
#[derive(Debug, Clone)]
pub struct AgentTableRow {
    /// False for a set-down agent, whose row is a handle for docking rather
    /// than a shortcut to a pane.
    pub docked: bool,
    pub ws_idx: usize,
    pub pane_id: PaneId,
    /// Absolute position in the full, possibly scrolled agent list.
    pub entry_idx: usize,
    pub rect: Rect,
    pub group: usize,
}

/// Where the table sits and what is in it. Empty when there is no room for it.
#[derive(Debug, Clone, Default)]
pub struct AgentTableLayout {
    /// The whole band, heading row included, without the blank row below it.
    pub area: Rect,
    pub groups: Vec<AgentTableGroup>,
    pub rows: Vec<AgentTableRow>,
    /// How far down the agent list the first drawn row sits.
    pub scroll: usize,
}

impl AgentTableLayout {
    /// The row under a point, if a row is.
    pub fn row_at(&self, x: u16, y: u16) -> Option<&AgentTableRow> {
        self.rows.iter().find(|row| {
            x >= row.rect.x && x < row.rect.x.saturating_add(row.rect.width) && y == row.rect.y
        })
    }

    /// The last agent the table can show at once, which is what a wheel notch
    /// scrolls past and what the scroll is clamped against.
    pub fn visible(&self) -> usize {
        self.rows.len()
    }
}

/// Carve the table off the top of what the composer left, and work out where its
/// rows and columns go.
///
/// Returns the rows the table owns and what is left for the panes. The table is
/// as tall as its agents need, up to its share of the frame; a table with no
/// agents still keeps its heading row, so the band does not appear and disappear
/// under the pointer as the last agent stops.
pub(crate) fn split_agent_table(app: &mut AppState, area: Rect) -> (AgentTableLayout, Rect) {
    let entries = agent_panel_entries(app);
    let least_below = TABLE_FOOTER_ROWS + 3;
    if area.width <= GUTTER || area.height < 2 + least_below {
        app.agent_table_scroll = 0;
        return (AgentTableLayout::default(), area);
    }

    let inset = Rect {
        x: area.x + 1,
        y: area.y,
        width: area.width.saturating_sub(2),
        height: area.height,
    };
    // A group is as tall as the table is allowed to be, and the table is as tall
    // as its agents need up to that. Agents past what one group holds do not make
    // it taller; they make another group.
    let ceiling = ((area.height.saturating_sub(least_below)) / TABLE_SHARE).max(2);
    let per_group = (ceiling.saturating_sub(1) as usize).clamp(1, COLUMN_AGENTS);
    let table_height = 1 + entries
        .len()
        .min(per_group)
        .max(usize::from(entries.is_empty())) as u16;
    let table = Rect {
        height: table_height,
        ..inset
    };

    let most = (table.width / MIN_GROUP).max(1) as usize;
    let count = entries.len().div_ceil(per_group.max(1)).clamp(1, most);
    let group_width = table.width / count as u16;
    let visible = per_group * count;

    let selected =
        focused_agent_row(app).and_then(|(ws_idx, pane_id)| position_of(&entries, ws_idx, pane_id));
    if let Some(selected) = selected {
        if selected < app.agent_table_scroll {
            app.agent_table_scroll = selected;
        } else if visible > 0 && selected >= app.agent_table_scroll + visible {
            app.agent_table_scroll = selected + 1 - visible;
        }
    }
    app.agent_table_scroll = app
        .agent_table_scroll
        .min(entries.len().saturating_sub(visible));
    let shown = visible.min(entries.len().saturating_sub(app.agent_table_scroll));

    let groups: Vec<AgentTableGroup> = (0..count)
        .map(|index| {
            // Every group but the last hands its rightmost column to the rule
            // that separates it from the group beside it, so the columns, the
            // rows, and the selected row's band all stop short of that rule.
            let divider = u16::from(index + 1 < count) * DIVIDER_WIDTH;
            let area = Rect {
                x: table.x + index as u16 * group_width,
                y: table.y,
                width: group_width.saturating_sub(divider),
                height: table.height,
            };
            let first = app.agent_table_scroll + index * per_group;
            let held = &entries[first.min(entries.len())
                ..(first + per_group)
                    .min(app.agent_table_scroll + shown)
                    .min(entries.len())];
            let mut columns = Vec::with_capacity(COLUMNS);
            let mut x = area.x + GUTTER;
            for width in column_widths(app, held, area.width) {
                columns.push(Rect {
                    x,
                    y: area.y,
                    width: width as u16,
                    height: area.height,
                });
                x += width as u16;
            }
            AgentTableGroup { area, columns }
        })
        .collect();

    // Agents fill a group top to bottom before the next one starts, so reading
    // the table in order is reading down a group and then across.
    let rows: Vec<AgentTableRow> = (0..shown)
        .map(|offset| {
            let entry = &entries[app.agent_table_scroll + offset];
            let group = offset / per_group;
            let area = groups[group].area;
            AgentTableRow {
                docked: entry.docked,
                ws_idx: entry.ws_idx,
                pane_id: entry.pane_id,
                entry_idx: app.agent_table_scroll + offset,
                rect: Rect {
                    x: area.x,
                    y: table.y + 1 + (offset % per_group) as u16,
                    width: area.width,
                    height: 1,
                },
                group,
            }
        })
        .collect();

    let below = Rect {
        x: area.x,
        y: table.y + table.height + TABLE_FOOTER_ROWS,
        width: area.width,
        height: area.height.saturating_sub(table.height + TABLE_FOOTER_ROWS),
    };
    (
        AgentTableLayout {
            area: table,
            groups,
            rows,
            scroll: app.agent_table_scroll,
        },
        below,
    )
}

/// Which agent has the keyboard: the focused pane of the active space.
fn focused_agent_row(app: &AppState) -> Option<(usize, PaneId)> {
    let ws_idx = app.active?;
    let ws = app.workspaces.get(ws_idx)?;
    Some((ws_idx, ws.focused_pane_id()?))
}

fn position_of(entries: &[AgentPanelEntry], ws_idx: usize, pane_id: PaneId) -> Option<usize> {
    entries
        .iter()
        .position(|entry| entry.ws_idx == ws_idx && entry.pane_id == pane_id)
}

/// Every column is as wide as the widest cell in that group, heading included,
/// except the summary, which takes whatever the others leave. A group is
/// measured against its own agents only, so one group's long path does not
/// stretch the same column in the group beside it.
fn column_widths(app: &AppState, held: &[AgentPanelEntry], group_width: u16) -> Vec<usize> {
    let mut wanted: Vec<usize> = HEADINGS
        .iter()
        .map(|heading| heading.chars().count())
        .collect();
    for entry in held {
        for (column, text) in cell_texts(app, entry).iter().enumerate() {
            if column == COL_SUMMARY {
                continue;
            }
            wanted[column] = wanted[column].max(text.chars().count());
        }
    }
    // Each measured column takes what it wants or what is left, whichever is
    // less. A group squeezed thin therefore loses its rightmost columns rather
    // than spilling over the group beside it. Summary is skipped here and
    // takes whatever remains, because a long title must not push Directory or
    // Git Status off the row.
    let mut room = (group_width as usize).saturating_sub(GUTTER as usize);
    let mut widths = vec![0usize; COLUMNS];
    for (column, width) in widths.iter_mut().enumerate() {
        if column == COL_SUMMARY {
            continue;
        }
        *width = (wanted[column] + COLUMN_GAP).min(room);
        room -= *width;
    }
    widths[COL_SUMMARY] = room;
    widths
}

/// What one agent's row says, column by column, at full length.
fn cell_texts(app: &AppState, entry: &AgentPanelEntry) -> [String; COLUMNS] {
    [
        entry.name.clone(),
        if entry.landing {
            "landing".to_string()
        } else if entry.land_failed {
            "land failed".to_string()
        } else {
            agent_status_detail_text(app, &entry.terminal_id).unwrap_or_default()
        },
        entry
            .location
            .as_ref()
            .map(|location| location.folder().to_string())
            .unwrap_or_default(),
        entry
            .agent_label
            .as_deref()
            .map(harness_display_name)
            .unwrap_or_default(),
        entry.run_duration.map(compact_duration).unwrap_or_default(),
        entry
            .idle_duration
            .map(compact_duration)
            .unwrap_or_default(),
        entry
            .location
            .as_ref()
            .and_then(|location| location.git.clone())
            .unwrap_or_default(),
    ]
}

fn compact_duration(duration: std::time::Duration) -> String {
    let seconds = duration.as_secs();
    if seconds < 60 {
        format!("{seconds}s")
    } else if seconds < 60 * 60 {
        format!("{}m", seconds / 60)
    } else if seconds < 24 * 60 * 60 {
        format!("{}h{}m", seconds / 3600, seconds % 3600 / 60)
    } else {
        format!("{}d{}h", seconds / 86_400, seconds % 86_400 / 3600)
    }
}

/// The state, in the one word the rest of herdr uses for it.
pub(super) fn agent_panel_status_key(state: AgentState, seen: bool) -> &'static str {
    match (state, seen) {
        (AgentState::Idle, false) => "done",
        (AgentState::Idle, true) => "idle",
        (AgentState::Working, _) => "working",
        (AgentState::Blocked, _) => "blocked",
        (AgentState::Unknown, _) => "unknown",
    }
}

/// Every agent in every space, in the order the spaces hold them.
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

    let names = crate::pane_names::assigned_names(&app.terminals);

    let mut entries = Vec::new();
    for (ws_idx, ws) in app.workspaces.iter().enumerate() {
        for detail in ws.pane_details(&app.terminals) {
            let name = app
                .terminals
                .get(&detail.terminal_id)
                .and_then(|terminal| terminal.manual_label.clone())
                .or_else(|| names.get(&detail.terminal_id).cloned())
                .unwrap_or_else(|| detail.agent_label.clone());
            let Some(terminal_id) = ws
                .pane_state(detail.pane_id)
                .map(|pane| pane.attached_terminal_id.clone())
            else {
                continue;
            };
            let location = app
                .view
                .agent_locations
                .get(&detail.pane_id)
                .cloned()
                .or_else(|| {
                    agent_location(app, ws, detail.tab_idx, detail.pane_id, terminal_runtimes)
                });
            entries.push(AgentPanelEntry {
                docked: true,
                ws_idx,
                tab_idx: detail.tab_idx,
                pane_id: detail.pane_id,
                terminal_id: terminal_id.clone(),
                name,
                agent_label: Some(detail.agent_label),
                model_info: detail.model_info,
                location,
                state: detail.state,
                seen: detail.seen,
                completed: detail.completed,
                custom_status: detail.custom_status,
                state_labels: detail.state_labels,
                run_duration: app
                    .terminals
                    .get(&terminal_id)
                    .and_then(|terminal| terminal.agent_run_duration(std::time::SystemTime::now())),
                idle_duration: app.terminals.get(&terminal_id).and_then(|terminal| {
                    terminal.agent_idle_duration(std::time::SystemTime::now())
                }),
                landing: app.landing_worktrees.contains(&ws.id),
                land_failed: app.landing_failures.contains_key(&ws.id),
            });
        }
    }
    for detached in &app.detached_agents {
        let Some(terminal) = app.terminals.get(&detached.pane.attached_terminal_id) else {
            continue;
        };
        let fallback_label = terminal
            .agent_name
            .as_deref()
            .or_else(|| terminal.effective_agent_label())
            .map(str::to_string);
        let agent_label = terminal
            .effective_display_agent()
            .or(fallback_label)
            .unwrap_or_else(|| "Terminal".to_string());
        let name = terminal
            .manual_label
            .clone()
            .or_else(|| names.get(&terminal.id).cloned())
            .unwrap_or_else(|| agent_label.clone());
        let presentation = terminal.effective_presentation();
        entries.push(AgentPanelEntry {
            docked: false,
            ws_idx: 0,
            tab_idx: 0,
            pane_id: detached.pane_id,
            terminal_id: terminal.id.clone(),
            name,
            agent_label: Some(agent_label),
            model_info: terminal.model_info.clone(),
            location: app
                .view
                .agent_locations
                .get(&detached.pane_id)
                .cloned()
                .or_else(|| detached_agent_location(app, detached, terminal_runtimes)),
            state: terminal.state,
            seen: detached.pane.seen,
            completed: detached.pane.completed,
            custom_status: presentation.custom_status,
            state_labels: presentation.state_labels,
            run_duration: terminal.agent_run_duration(std::time::SystemTime::now()),
            idle_duration: terminal.agent_idle_duration(std::time::SystemTime::now()),
            landing: false,
            land_failed: false,
        });
    }
    let rank: std::collections::HashMap<_, _> = app
        .agent_order
        .iter()
        .enumerate()
        .map(|(idx, terminal_id)| (terminal_id, idx))
        .collect();
    entries.sort_by_key(|entry| {
        rank.get(&entry.terminal_id)
            .copied()
            .map_or((1, usize::MAX), |idx| (0, idx))
    });
    entries
}

/// Reconcile the durable table order with the agents visible this frame.
/// Existing ids never move; agents not seen before append in their first
/// observed order, and agents that ended stop occupying saved slots.
pub(crate) fn sync_agent_order(app: &mut AppState) {
    let listed: Vec<_> = agent_panel_entries(app)
        .into_iter()
        .map(|entry| entry.terminal_id)
        .collect();
    app.agent_order
        .retain(|terminal_id| listed.contains(terminal_id));
    for terminal_id in listed {
        if !app.agent_order.contains(&terminal_id) {
            app.agent_order.push(terminal_id);
        }
    }
}

/// The pane's cwd, with its git branch and dirty marker when the pane is in a
/// repository.
fn agent_location(
    app: &AppState,
    ws: &crate::workspace::Workspace,
    tab_idx: usize,
    pane_id: PaneId,
    terminal_runtimes: &TerminalRuntimeRegistry,
) -> Option<AgentLocation> {
    let tab = ws.tabs.get(tab_idx)?;
    let cwd = tab.cwd_for_pane(pane_id, &app.terminals, terminal_runtimes)?;
    let git_status = ws.git_status_for_pane(pane_id);
    let git = super::panes::git_branch_label(&git_status).map(|branch| {
        format!(
            "{branch} {}",
            super::panes::worktree_state_marker(git_status.worktree_state)
        )
    });
    Some(AgentLocation {
        path: super::panes::display_location_path(&cwd, &git_status),
        git,
    })
}

/// Where a set-down agent is working. It belongs to no space, so its branch
/// comes from the answer kept against its own pane rather than from a space's
/// cached one.
fn detached_agent_location(
    app: &AppState,
    detached: &crate::app::state::DetachedAgent,
    terminal_runtimes: &TerminalRuntimeRegistry,
) -> Option<AgentLocation> {
    let cwd =
        crate::app::state::detached_agent_cwd(&detached.pane, &app.terminals, terminal_runtimes)?;
    let git = app
        .detached_git_statuses
        .get(&detached.pane_id)
        .and_then(|status| {
            let branch = super::panes::git_branch_label(status)?;
            Some(format!(
                "{branch} {}",
                super::panes::worktree_state_marker(status.worktree_state)
            ))
        });
    let path = app
        .detached_git_statuses
        .get(&detached.pane_id)
        .map_or_else(
            || super::panes::display_path_with_home(&cwd),
            |status| super::panes::display_location_path(&cwd, status),
        );
    Some(AgentLocation { path, git })
}

/// Where every listed pane is working, computed once per frame from the live
/// runtimes and kept on [`crate::app::state::ViewState`].
///
/// The table measures and places itself from `AppState` alone — mouse handling
/// re-runs that layout between frames, with no runtimes in reach — but a pane's
/// live cwd only exists in the runtimes. Caching it here lets the layout and the
/// paint read the same folder for an agent.
pub(crate) fn compute_agent_locations(
    app: &AppState,
    terminal_runtimes: &TerminalRuntimeRegistry,
) -> std::collections::HashMap<PaneId, AgentLocation> {
    let mut locations = std::collections::HashMap::new();
    for ws in &app.workspaces {
        for detail in ws.pane_details(&app.terminals) {
            if let Some(location) =
                agent_location(app, ws, detail.tab_idx, detail.pane_id, terminal_runtimes)
            {
                locations.insert(detail.pane_id, location);
            }
        }
    }
    for detached in &app.detached_agents {
        if let Some(location) = detached_agent_location(app, detached, terminal_runtimes) {
            locations.insert(detached.pane_id, location);
        }
    }
    locations
}

/// What the agent reports it is doing — the session title its harness set, then
/// any custom status it announced — the same text `herdr agent status` prints
/// after the state.
///
/// Read from the terminal itself rather than through the pane holding it, so a
/// set-down agent, which no workspace lists, still says what it is doing.
fn agent_status_detail_text(
    app: &AppState,
    terminal_id: &crate::terminal::TerminalId,
) -> Option<String> {
    let presentation = app.terminals.get(terminal_id)?.effective_presentation();
    let parts: Vec<&str> = [
        presentation.title.as_deref(),
        presentation.custom_status.as_deref(),
    ]
    .into_iter()
    .flatten()
    .map(str::trim)
    .filter(|part| !part.is_empty())
    .collect();
    (!parts.is_empty()).then(|| parts.join(" · "))
}

pub(crate) fn harness_display_name(label: &str) -> String {
    match crate::detect::parse_agent_label(label) {
        Some(agent) => crate::detect::agent_display_name(agent).to_string(),
        None => label.to_string(),
    }
}

/// A cell in a column that wide: padded out, or cut short with an ellipsis.
fn pad(text: &str, width: usize) -> String {
    let length = text.chars().count();
    if length <= width {
        return format!("{text:<width$}");
    }
    let kept: String = text.chars().take(width.saturating_sub(2)).collect();
    format!("{kept}… ")
}

pub(crate) fn render_agent_table(
    app: &AppState,
    frame: &mut Frame,
    layout: &AgentTableLayout,
    entries: &[AgentPanelEntry],
) {
    if layout.area.width == 0 || layout.area.height == 0 {
        return;
    }

    render_group_dividers(app, frame, layout);

    let heading = Style::default().fg(app.palette.overlay0);
    for group in &layout.groups {
        for (column, label) in HEADINGS.iter().enumerate() {
            let Some(rect) = group.columns.get(column) else {
                continue;
            };
            render_line(
                frame,
                cell(*rect, layout.area.y),
                Line::styled(pad(label, rect.width as usize), heading),
            );
        }
    }

    if entries.is_empty() {
        render_line(
            frame,
            Rect {
                x: layout.area.x + GUTTER,
                y: layout.area.y + 1,
                width: layout.area.width.saturating_sub(GUTTER),
                height: 1,
            },
            Line::styled(NO_AGENTS, Style::default().fg(app.palette.overlay0)),
        );
        return;
    }

    let focused = focused_agent_row(app);
    let folders = folder_colors(&app.palette, entries);
    for (offset, row) in layout.rows.iter().enumerate() {
        let Some(entry) = entries.get(layout.scroll + offset) else {
            continue;
        };
        let Some(group) = layout.groups.get(row.group) else {
            continue;
        };
        let selected = entry.docked && focused == Some((entry.ws_idx, entry.pane_id));
        if selected {
            fill_row_band(app, frame, row.rect);
        }
        render_margin(app, frame, entry, row.rect);
        let texts = cell_texts(app, entry);
        for (column, text) in texts.iter().enumerate() {
            let Some(rect) = group.columns.get(column) else {
                continue;
            };
            let area = cell(*rect, row.rect.y);
            render_line(
                frame,
                area,
                cell_line(
                    app,
                    entry,
                    column,
                    text,
                    area.width as usize,
                    selected,
                    &folders,
                ),
            );
        }
    }
    render_agent_drop_indicator(app, frame, layout);
}

/// The rule down the right of every group but the last, from the heading row to
/// the bottom of the table. It stands in the column each group gave up in
/// [`split_agent_table`], so it never lands on a cell that holds text.
fn render_group_dividers(app: &AppState, frame: &mut Frame, layout: &AgentTableLayout) {
    let style = Style::default().fg(app.palette.surface1);
    let buffer = frame.buffer_mut();
    for group in layout
        .groups
        .iter()
        .take(layout.groups.len().saturating_sub(1))
    {
        let x = group.area.x + group.area.width;
        for y in group.area.y..group.area.y.saturating_add(group.area.height) {
            if let Some(cell) = buffer.cell_mut((x, y)) {
                cell.set_symbol(GROUP_DIVIDER).set_style(style);
            }
        }
    }
}

/// Marks the exact before/after slot under a dragged agent. The table is dense
/// (one terminal row per agent), so the marker lives in its dedicated gutter
/// and does not erase any agent text.
fn render_agent_drop_indicator(app: &AppState, frame: &mut Frame, layout: &AgentTableLayout) {
    let Some(crate::app::state::DragTarget::AgentReorder {
        insert_idx: Some(insert_idx),
        ..
    }) = app.drag.as_ref().map(|drag| &drag.target)
    else {
        return;
    };
    let Some((row, glyph)) = (if *insert_idx == 0 {
        layout.rows.first().map(|row| (row, DROP_BEFORE))
    } else {
        layout
            .rows
            .iter()
            .find(|row| row.entry_idx + 1 == *insert_idx)
            .map(|row| (row, DROP_AFTER))
            .or_else(|| {
                layout
                    .rows
                    .iter()
                    .find(|row| row.entry_idx == *insert_idx)
                    .map(|row| (row, DROP_BEFORE))
            })
    }) else {
        return;
    };
    if let Some(cell) = frame.buffer_mut().cell_mut((row.rect.x, row.rect.y)) {
        cell.set_symbol(glyph).set_style(
            Style::default()
                .fg(app.palette.accent)
                .add_modifier(Modifier::BOLD),
        );
    }
}

/// The global menu stays at the far right of the frame's top row, aligned with
/// the composer captions.
pub(super) fn render_global_launcher(app: &AppState, frame: &mut Frame) {
    if !app.mouse_capture {
        return;
    }

    let rect = app.global_launcher_rect();
    if rect == Rect::default() {
        return;
    }

    let needs_attention = app.global_menu_attention_badge_visible();
    let label = if needs_attention { "● menu" } else { "menu" };
    let style = if needs_attention {
        Style::default()
            .fg(app.palette.accent)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(app.palette.overlay0)
    };
    let line = format!("{label:>width$}", width = rect.width as usize);
    frame.render_widget(Paragraph::new(line).style(style), rect);
}

/// The margin says what wants you: something turning is working, a dot is
/// something that finished while you were not looking, and a diamond is an agent
/// stopped on a question. Clicking a dot is the one thing that clears it, and it
/// leaves a check behind, so an agent that finished never becomes
/// indistinguishable from one that never ran.
fn render_margin(app: &AppState, frame: &mut Frame, entry: &AgentPanelEntry, row: Rect) {
    let marker = if entry.landing {
        Some((
            super::spinner_frame(app.spinner_tick),
            mute_when_host_unfocused(app, app.palette.yellow),
        ))
    } else if entry.land_failed {
        Some((LAND_FAILED, mute_when_host_unfocused(app, app.palette.red)))
    } else {
        match (entry.state, entry.seen) {
            (AgentState::Working, _) => Some((
                super::spinner_frame(app.spinner_tick),
                mute_when_host_unfocused(app, app.palette.yellow),
            )),
            (AgentState::Blocked, _) => {
                Some((BLOCKED, mute_when_host_unfocused(app, app.palette.red)))
            }
            (AgentState::Idle, false) => {
                Some((FINISHED, mute_when_host_unfocused(app, app.palette.green)))
            }
            (AgentState::Idle, true) if entry.completed => Some((
                ACKNOWLEDGED,
                mute_when_host_unfocused(app, app.palette.green),
            )),
            _ => None,
        }
    };
    let Some((glyph, color)) = marker else {
        return;
    };
    render_line(
        frame,
        Rect {
            x: row.x,
            y: row.y,
            width: 1,
            height: 1,
        },
        Line::styled(glyph, Style::default().fg(color)),
    );
}

/// One cell's text, in the tone that column reads in.
fn cell_line<'a>(
    app: &AppState,
    entry: &AgentPanelEntry,
    column: usize,
    text: &'a str,
    width: usize,
    selected: bool,
    folders: &FolderColors,
) -> Line<'a> {
    let dim = Style::default().fg(app.palette.overlay0);
    match column {
        COL_NAME => {
            // A set-down agent's name is written a shade quieter: the row is
            // real, but nothing on screen is showing it.
            let style = if selected {
                Style::default()
                    .fg(focus_accent(app))
                    .add_modifier(Modifier::BOLD)
            } else if entry.docked {
                Style::default().fg(app.palette.text)
            } else {
                Style::default().fg(app.palette.subtext0)
            };
            Line::styled(pad(text, width), style)
        }
        COL_DIRECTORY => folder_line(app, entry, width, folders),
        COL_AGENT => Line::styled(
            pad(text, width),
            Style::default().fg(if selected {
                app.palette.text
            } else {
                app.palette.subtext0
            }),
        ),
        COL_GIT => Line::styled(
            pad(text, width),
            Style::default().fg(mute_when_host_unfocused(app, app.palette.mauve)),
        ),
        _ => Line::styled(pad(text, width), dim),
    }
}

/// The last folder, in the color that folder is written in everywhere in the
/// table. The branch lives in its own column, so this cell never shares the
/// cell with git status.
fn folder_line<'a>(
    app: &AppState,
    entry: &AgentPanelEntry,
    width: usize,
    folders: &FolderColors,
) -> Line<'a> {
    let Some(location) = entry.location.as_ref() else {
        return Line::styled(pad("", width), Style::default().fg(app.palette.overlay0));
    };
    let color = folders
        .get(location.path.as_str())
        .copied()
        .unwrap_or(app.palette.subtext0);
    Line::styled(
        pad(location.folder(), width),
        Style::default().fg(mute_when_host_unfocused(app, color)),
    )
}

/// What color each folder in the table is written in.
pub(crate) type FolderColors = std::collections::HashMap<String, Color>;

/// One color per directory, so a row's folder says which other rows are working
/// in the same place.
///
/// A directory prefers the slot its path hashes to, which keeps a folder the
/// same color from frame to frame and between runs. Two directories that want
/// the same slot cannot both have it — one folder standing for two places is
/// the one failure this column cannot afford — so the later one takes the next
/// free slot instead. Preference is settled in hash order rather than table
/// order, so reordering or scrolling the table cannot repaint anything; only
/// the set of directories on screen can. Past one slot per directory the colors
/// necessarily repeat, and a repeat is what is left.
///
/// How light the colors are comes from the theme, sampled from the tone
/// secondary text is written in, so folders sit at the brightness the theme
/// already uses for this kind of text on light and dark backgrounds alike.
fn folder_colors(palette: &Palette, entries: &[AgentPanelEntry]) -> FolderColors {
    let mut wanted: Vec<(u64, &str)> = Vec::new();
    for entry in entries {
        let Some(location) = entry.location.as_ref() else {
            continue;
        };
        if wanted.iter().any(|(_, path)| *path == location.path) {
            continue;
        }
        wanted.push((path_hash(&location.path), location.path.as_str()));
    }
    wanted.sort_unstable();

    let slots = FOLDER_HUES.len();
    let mut taken = vec![false; slots];
    let mut colors = FolderColors::new();
    for (hash, path) in wanted {
        let preferred = (hash % slots as u64) as usize;
        let slot = (0..slots)
            .map(|step| (preferred + step) % slots)
            .find(|slot| !taken[*slot])
            .unwrap_or(preferred);
        taken[slot] = true;
        colors.insert(path.to_string(), folder_slot_color(palette, slot));
    }
    colors
}

/// One slot's color in this theme. A theme that paints in the terminal's own
/// sixteen colors has no light level to sample, so there the slot picks one of
/// the palette's named colors instead of a hue of its own.
fn folder_slot_color(palette: &Palette, slot: usize) -> Color {
    let Color::Rgb(r, g, b) = palette.subtext0 else {
        let named = [palette.blue, palette.teal, palette.mauve, palette.peach];
        return named[slot % named.len()];
    };
    let high = r.max(g).max(b) as f32 / 255.0;
    let low = r.min(g).min(b) as f32 / 255.0;
    hsl_to_rgb(FOLDER_HUES[slot], FOLDER_SATURATION, (high + low) / 2.0)
}

/// FNV-1a. Written out rather than taken from the standard library because the
/// color a folder gets has to be the same one it got last time, and a hasher
/// the library is free to change is free to change every folder's color.
fn path_hash(path: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in path.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

fn hsl_to_rgb(hue: f32, saturation: f32, lightness: f32) -> Color {
    let chroma = (1.0 - (2.0 * lightness - 1.0).abs()) * saturation;
    let sector = hue / 60.0;
    let second = chroma * (1.0 - (sector % 2.0 - 1.0).abs());
    let (r, g, b) = match sector as u32 {
        0 => (chroma, second, 0.0),
        1 => (second, chroma, 0.0),
        2 => (0.0, chroma, second),
        3 => (0.0, second, chroma),
        4 => (second, 0.0, chroma),
        _ => (chroma, 0.0, second),
    };
    let base = lightness - chroma / 2.0;
    let channel = |value: f32| (((value + base) * 255.0).round().clamp(0.0, 255.0)) as u8;
    Color::Rgb(channel(r), channel(g), channel(b))
}

/// The band behind the row that has the keyboard. It is capped with half blocks
/// one column outside the fill, so the band has half a column of air at each end
/// rather than sitting flush against the text.
fn fill_row_band(app: &AppState, frame: &mut Frame, row: Rect) {
    if row.width <= 2 {
        return;
    }
    let bg = app.palette.surface_dim;
    let fill = Rect {
        x: row.x + 1,
        y: row.y,
        width: row.width.saturating_sub(2),
        height: 1,
    };
    let buffer = frame.buffer_mut();
    for x in fill.x..fill.x.saturating_add(fill.width) {
        if let Some(cell) = buffer.cell_mut((x, row.y)) {
            cell.set_bg(bg);
        }
    }
    if let Some(cell) = buffer.cell_mut((row.x, row.y)) {
        cell.set_symbol(BAND_CAP_LEFT).set_fg(bg);
    }
    let right = row.x + row.width - 1;
    if let Some(cell) = buffer.cell_mut((right, row.y)) {
        cell.set_symbol(BAND_CAP_RIGHT).set_fg(bg);
    }
}

/// One column's slice of one row.
fn cell(column: Rect, row: u16) -> Rect {
    Rect {
        y: row,
        height: 1,
        ..column
    }
}

fn render_line(frame: &mut Frame, rect: Rect, line: Line<'_>) {
    if rect.width == 0 || rect.height == 0 {
        return;
    }
    frame.render_widget(Paragraph::new(line), rect);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(path: &str) -> AgentPanelEntry {
        AgentPanelEntry {
            docked: true,
            ws_idx: 0,
            tab_idx: 0,
            pane_id: PaneId::alloc(),
            terminal_id: crate::terminal::TerminalId::alloc(),
            name: path.to_string(),
            agent_label: None,
            model_info: None,
            location: Some(AgentLocation {
                path: path.to_string(),
                git: None,
            }),
            state: AgentState::Idle,
            seen: true,
            completed: false,
            custom_status: None,
            state_labels: std::collections::HashMap::new(),
            run_duration: None,
            idle_duration: None,
            landing: false,
            land_failed: false,
        }
    }

    #[test]
    fn compact_duration_uses_readable_table_units() {
        assert_eq!(compact_duration(std::time::Duration::from_secs(42)), "42s");
        assert_eq!(compact_duration(std::time::Duration::from_secs(125)), "2m");
        assert_eq!(
            compact_duration(std::time::Duration::from_secs(7_380)),
            "2h3m"
        );
        assert_eq!(
            compact_duration(std::time::Duration::from_secs(93_600)),
            "1d2h"
        );
    }

    fn entries(paths: &[&str]) -> Vec<AgentPanelEntry> {
        paths.iter().map(|path| entry(path)).collect()
    }

    /// One space holding two agents, the second of which is then set down, so
    /// the table holds one docked row and one set-down row.
    fn state_with_a_set_down_agent() -> (AppState, PaneId) {
        let mut state = AppState::test_new();
        let mut ws = crate::workspace::Workspace::test_new("test");
        let pane_id = ws.test_split(ratatui::layout::Direction::Horizontal);
        state.workspaces.push(ws);
        state.active = Some(0);
        state.ensure_test_terminals();
        let terminal_id = state.workspaces[0].tabs[0].panes[&pane_id]
            .attached_terminal_id
            .clone();
        if let Some(terminal) = state.terminals.get_mut(&terminal_id) {
            terminal.set_detected_state(Some(crate::detect::Agent::Pi), AgentState::Idle);
            terminal.session_title = Some("Reposition the credits".into());
        }
        state.workspaces[0].tabs[0].layout.focus_pane(pane_id);
        state.close_pane();
        (state, pane_id)
    }

    /// One space holding `count` docked agents, which is what makes the table
    /// spill into a second group of columns.
    fn state_with_agents(count: usize) -> AppState {
        let mut state = AppState::test_new();
        let mut ws = crate::workspace::Workspace::test_new("test");
        for _ in 1..count {
            ws.test_split(ratatui::layout::Direction::Horizontal);
        }
        state.workspaces.push(ws);
        state.active = Some(0);
        state.ensure_test_terminals();
        let ids: Vec<_> = state.terminals.keys().cloned().collect();
        for id in ids {
            if let Some(terminal) = state.terminals.get_mut(&id) {
                terminal.set_detected_state(Some(crate::detect::Agent::Pi), AgentState::Idle);
            }
        }
        state
    }

    #[test]
    fn a_double_rule_stands_between_one_group_of_columns_and_the_next() {
        let mut state = state_with_agents(12);
        let area = Rect::new(0, 0, 200, 40);
        let (layout, _) = split_agent_table(&mut state, area);
        assert!(layout.groups.len() > 1, "the table should hold two groups");

        let entries = agent_panel_entries(&state);
        let backend = ratatui::backend::TestBackend::new(area.width, area.height);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| render_agent_table(&state, frame, &layout, &entries))
            .unwrap();
        let buffer = terminal.backend().buffer().clone();

        let first = layout.groups[0].area;
        let rule = first.x + first.width;
        for y in layout.area.y..layout.area.y + layout.area.height {
            assert_eq!(
                buffer.cell((rule, y)).map(|cell| cell.symbol()),
                Some(GROUP_DIVIDER),
                "row {y} should carry the rule between the groups"
            );
        }
        assert_eq!(rule, layout.groups[1].area.x - 1);
    }

    #[test]
    fn the_rightmost_group_keeps_its_last_column_for_text() {
        let mut state = state_with_agents(12);
        let (layout, _) = split_agent_table(&mut state, Rect::new(0, 0, 200, 40));
        let groups = &layout.groups;
        let pitch = groups[1].area.x - groups[0].area.x;
        assert_eq!(groups[0].area.width, pitch - DIVIDER_WIDTH);
        assert_eq!(groups[groups.len() - 1].area.width, pitch);
    }

    fn row_of(state: &AppState, pane_id: PaneId) -> AgentPanelEntry {
        agent_panel_entries(state)
            .into_iter()
            .find(|entry| entry.pane_id == pane_id)
            .expect("a row for the agent")
    }

    #[test]
    fn a_set_down_agent_row_says_what_the_agent_is_doing() {
        let (state, pane_id) = state_with_a_set_down_agent();
        let row = row_of(&state, pane_id);
        assert!(!row.docked);
        assert_eq!(
            cell_texts(&state, &row)[COL_SUMMARY],
            "Reposition the credits"
        );
    }

    #[test]
    fn directory_is_the_last_folder_and_git_status_is_its_own_column() {
        let state = AppState::test_new();
        let mut row = entry("~/lab/herdr");
        row.name = "Olivia".into();
        row.agent_label = Some("codex".into());
        row.location = Some(AgentLocation {
            path: "~/lab/herdr".into(),
            git: Some("feat/space-done !".into()),
        });
        let texts = cell_texts(&state, &row);
        assert_eq!(texts[COL_NAME], "Olivia");
        assert_eq!(texts[COL_DIRECTORY], "herdr");
        assert_eq!(texts[COL_AGENT], "Codex");
        assert_eq!(texts[COL_GIT], "feat/space-done !");
        assert!(!texts[COL_DIRECTORY].contains('/'));
        assert!(!texts[COL_DIRECTORY].contains('('));
    }

    #[test]
    fn last_folder_drops_the_path_and_keeps_the_name() {
        assert_eq!(last_folder("~/lab/herdr"), "herdr");
        assert_eq!(last_folder("~/lab/herdr/"), "herdr");
        assert_eq!(last_folder("herdr"), "herdr");
        assert_eq!(last_folder("~"), "~");
        assert_eq!(last_folder("/"), "/");
    }

    #[test]
    fn a_long_summary_does_not_steal_the_short_columns() {
        let mut state = state_with_agents(1);
        let pane_id = state.workspaces[0].tabs[0].root_pane;
        let path = "~/very/long/path/that/would/stretch/the/column/if/kept";
        let git = "feat/a-long-branch-name !";
        state.view.agent_locations.insert(
            pane_id,
            AgentLocation {
                path: path.into(),
                git: Some(git.into()),
            },
        );
        let id = state.terminals.keys().next().cloned().expect("a terminal");
        if let Some(terminal) = state.terminals.get_mut(&id) {
            terminal.session_title = Some(
                "A long summary that would steal the row if it were measured like the others"
                    .into(),
            );
        }
        let (layout, _) = split_agent_table(&mut state, Rect::new(0, 0, 120, 20));
        let columns = &layout.groups[0].columns;
        let texts = cell_texts(&state, &row_of(&state, pane_id));
        assert_eq!(texts[COL_DIRECTORY], "kept");
        assert_eq!(texts[COL_GIT], git);
        assert!(
            columns[COL_DIRECTORY].width < path.chars().count() as u16,
            "directory {} kept the path's width {}",
            columns[COL_DIRECTORY].width,
            path.chars().count()
        );
        assert!(
            columns[COL_GIT].width as usize >= HEADINGS[COL_GIT].chars().count(),
            "git status {} lost its heading",
            columns[COL_GIT].width
        );
        assert!(
            (columns[COL_SUMMARY].width as usize) < texts[COL_SUMMARY].chars().count(),
            "summary {} was measured to its text {}",
            columns[COL_SUMMARY].width,
            texts[COL_SUMMARY].chars().count()
        );
    }

    #[test]
    fn a_set_down_agent_row_carries_its_branch() {
        let (mut state, pane_id) = state_with_a_set_down_agent();
        assert_eq!(row_of(&state, pane_id).location.and_then(|at| at.git), None);

        state.detached_git_statuses.insert(
            pane_id,
            crate::workspace::WorkspaceGitStatusSnapshot {
                branch: Some("main".into()),
                ahead_behind: None,
                space: None,
                worktree_state: crate::workspace::GitWorktreeState::Unstaged,
            },
        );

        let git = row_of(&state, pane_id)
            .location
            .and_then(|at| at.git)
            .expect("the branch of the folder the agent is in");
        assert!(git.starts_with("main "), "{git}");
    }

    fn rgb(color: Color) -> (u8, u8, u8) {
        match color {
            Color::Rgb(r, g, b) => (r, g, b),
            other => panic!("expected an rgb color, got {other:?}"),
        }
    }

    #[test]
    fn agents_in_one_folder_share_its_color() {
        let colors = folder_colors(
            &Palette::catppuccin(),
            &entries(&["~/lab/herdr", "~/lab/herdr", "~/lab/herdr", "~/lab/what"]),
        );
        assert_eq!(colors.len(), 2);
        assert_ne!(colors["~/lab/herdr"], colors["~/lab/what"]);
    }

    #[test]
    fn folders_that_differ_never_share_a_color() {
        let paths: Vec<String> = (0..FOLDER_HUES.len())
            .map(|index| format!("~/lab/project-{index}"))
            .collect();
        let borrowed: Vec<&str> = paths.iter().map(String::as_str).collect();
        let colors = folder_colors(&Palette::catppuccin(), &entries(&borrowed));
        let mut used: Vec<Color> = colors.values().copied().collect();
        used.sort_by_key(|color| format!("{color:?}"));
        used.dedup();
        assert_eq!(used.len(), paths.len(), "two folders were given one color");
    }

    #[test]
    fn a_folder_keeps_its_color_when_the_table_is_reordered() {
        let palette = Palette::catppuccin();
        let forwards = folder_colors(&palette, &entries(&["~/a", "~/b", "~/c"]));
        let backwards = folder_colors(&palette, &entries(&["~/c", "~/b", "~/a"]));
        assert_eq!(forwards, backwards);
    }

    #[test]
    fn no_folder_is_written_in_red_or_green() {
        for hue in FOLDER_HUES {
            assert!(
                (25.0..=85.0).contains(&hue) || (160.0..=335.0).contains(&hue),
                "hue {hue} falls in the red or green arc"
            );
        }
    }

    #[test]
    fn folder_colors_take_their_light_level_from_the_theme() {
        let lightness = |palette: &Palette| {
            let (r, g, b) = rgb(folder_slot_color(palette, 0));
            (r.max(g).max(b) as f32 + r.min(g).min(b) as f32) / 2.0
        };
        let dark = lightness(&Palette::catppuccin());
        let light = lightness(&Palette::catppuccin_latte());
        assert!(
            dark > light,
            "a dark theme writes folders lighter than a light one: {dark} vs {light}"
        );
    }

    #[test]
    fn a_sixteen_color_theme_keeps_to_its_named_colors() {
        let palette = Palette::terminal();
        for slot in 0..FOLDER_HUES.len() {
            let color = folder_slot_color(&palette, slot);
            assert!(
                color == palette.blue
                    || color == palette.teal
                    || color == palette.mauve
                    || color == palette.peach,
                "slot {slot} fell outside the terminal palette: {color:?}"
            );
        }
    }
}
