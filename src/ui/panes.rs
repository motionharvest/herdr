use ratatui::{
    layout::{Alignment, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph},
    Frame,
};

use super::scrollbar::{render_pane_scrollbar, should_show_scrollbar};
use super::widgets::panel_contrast_fg;
use crate::app::state::{Palette, PaneChromeAction, PaneChromeControl, PaneTitleHitArea};
use crate::app::{AppState, Mode};
use crate::layout::{
    adjacent_right_edge_y_range, placement_is_adjacent, y_segments_outside, ExposedSides, PaneInfo,
    SplitSide,
};
use crate::terminal::{TerminalRuntime, TerminalRuntimeRegistry};
use crate::workspace::Workspace;
use unicode_width::UnicodeWidthStr;

const PANE_BORDER_SET: ratatui::symbols::border::Set = ratatui::symbols::border::ROUNDED;
const PANE_CLOSE_CONTROL_SUFFIX: &str = " ✕ ";
const PANE_INNER_PADDING: u16 = 0;

struct PaneTitleChromeLayout {
    /// Width of the pane name/path details section (before the horizontal rule glyphs).
    details_width: u16,
    rule_width: u16,
}

fn pane_title_chrome_layout(
    title_width: u16,
    title: &PaneChromeTitle,
    maximized: bool,
    hide: bool,
) -> PaneTitleChromeLayout {
    let (_, controls_width) = pane_controls_text(title_width, maximized, hide);
    let title_prefix_width = "╭─ ".chars().count();
    let text_available = title_width
        .saturating_sub(title_prefix_width as u16)
        .saturating_sub(controls_width)
        .saturating_sub(3) as usize;
    let title_text = truncate_to_width(&title.formatted_title(), text_available);
    let details_width = (title_prefix_width + title_text.chars().count() + 1) as u16;
    let rule_width = title_width
        .saturating_sub(details_width)
        .saturating_sub(controls_width)
        .saturating_sub(1);
    PaneTitleChromeLayout {
        details_width,
        rule_width,
    }
}

fn pane_controls_text(title_width: u16, maximized: bool, hide: bool) -> (&'static str, u16) {
    let text = if title_width >= 24 {
        match (maximized, hide) {
            (true, true) => " ◱ BACK  HIDE ",
            (true, false) => " ◱ BACK  ✕ ",
            (false, true) => " ⛶ FOCUS HIDE ",
            (false, false) => " ⛶ FOCUS ✕ ",
        }
    } else if title_width >= 12 {
        if hide {
            " ⛶ HIDE "
        } else {
            " ⛶ ✕ "
        }
    } else {
        ""
    };
    (text, text.width() as u16)
}

fn pane_close_suffix(controls_text: &str) -> &str {
    if controls_text.ends_with("HIDE ") {
        " HIDE "
    } else {
        PANE_CLOSE_CONTROL_SUFFIX
    }
}

fn pane_chrome_controls_x(area: Rect, controls_width: u16) -> u16 {
    area.x + area.width.saturating_sub(controls_width + 1)
}

fn pane_close_control_rect(area: Rect, controls_x: u16, controls_text: &str) -> Rect {
    let suffix_width = pane_close_suffix(controls_text).width() as u16;
    let controls_display_width = controls_text.width() as u16;
    let start = controls_x + controls_display_width.saturating_sub(suffix_width);
    let end = area.x + area.width.saturating_sub(1);
    Rect::new(start, area.y, end.saturating_sub(start).max(1), 1)
}

fn pane_focus_control_rect(area: Rect, controls_x: u16, controls_text: &str) -> Rect {
    let close = pane_close_control_rect(area, controls_x, controls_text);
    let focus_width = close.x.saturating_sub(controls_x);
    Rect::new(controls_x, area.y, focus_width, 1)
}

fn pane_content_rect(area: Rect, framed: bool) -> Rect {
    if !framed {
        return area;
    }

    Block::default()
        .borders(Borders::ALL)
        .border_set(PANE_BORDER_SET)
        .inner(area)
}

fn apply_pane_inner_padding(rect: Rect) -> Rect {
    let horizontal_inset = PANE_INNER_PADDING * 2;
    let width = rect.width.saturating_sub(horizontal_inset);
    if width == 0 {
        return rect;
    }

    Rect::new(
        rect.x.saturating_add(PANE_INNER_PADDING),
        rect.y,
        width,
        rect.height,
    )
}

fn render_pane_inner_padding(frame: &mut Frame, content_rect: Rect, padded_rect: Rect) {
    if content_rect == padded_rect {
        return;
    }

    let style = Style::default().bg(Color::Reset);
    let buf = frame.buffer_mut();
    for y in content_rect.y..content_rect.y + content_rect.height {
        for x in content_rect.x..content_rect.x + content_rect.width {
            if x >= padded_rect.x
                && x < padded_rect.x + padded_rect.width
                && y >= padded_rect.y
                && y < padded_rect.y + padded_rect.height
            {
                continue;
            }
            let cell = &mut buf[(x, y)];
            cell.set_symbol(" ").set_style(style);
        }
    }
}

fn pane_draws_left_vertical_edge(exposed: ExposedSides, focused: bool) -> bool {
    exposed.left || focused
}

fn pane_frame_borders(exposed: ExposedSides, focused: bool, hide_full_right_edge: bool) -> Borders {
    let mut borders = Borders::TOP | Borders::BOTTOM;
    if pane_draws_left_vertical_edge(exposed, focused) {
        borders |= Borders::LEFT;
    }

    // Match code-ui: internal right edges live on the left pane's block border.
    // Outer right edges still render so the workspace keeps a closed rounded frame.
    let show_internal_right_edge = !exposed.right;
    if exposed.right || (show_internal_right_edge && !hide_full_right_edge) {
        borders |= Borders::RIGHT;
    }

    borders
}

/// Shared left edge without focus: keep rounded top/bottom corners and draw a
/// dashed dim vertical rule between them where the adjacent pane did not already
/// paint a visible border.
fn render_pane_open_left_edge(
    frame: &mut Frame,
    area: Rect,
    edge_style: Style,
    dim_edge_style: Style,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    let bottom_y = area.y + area.height.saturating_sub(1);
    if area.height > 2 {
        let blank_style = Style::default().bg(Color::Reset);
        for (i, y) in (area.y.saturating_add(1)..bottom_y).enumerate() {
            let cell = &mut frame.buffer_mut()[(area.x, y)];
            if cell.symbol() == "│" {
                continue;
            }
            if i.is_multiple_of(2) {
                cell.set_symbol("│").set_style(dim_edge_style);
            } else {
                cell.set_symbol(" ").set_style(blank_style);
            }
        }
    }

    frame.render_widget(
        Paragraph::new(PANE_BORDER_SET.bottom_left)
            .alignment(Alignment::Left)
            .style(edge_style),
        Rect::new(area.x, bottom_y, 1, 1),
    );
}

fn render_vertical_edge_column(
    frame: &mut Frame,
    x: u16,
    y_start: u16,
    y_end: u16,
    edge_style: Style,
) {
    if y_end <= y_start {
        return;
    }

    let height = y_end.saturating_sub(y_start);
    let column = "│\n".repeat(height.saturating_sub(1) as usize) + "│";
    frame.render_widget(
        Paragraph::new(column)
            .alignment(Alignment::Left)
            .style(edge_style),
        Rect::new(x, y_start, 1, height),
    );
}

fn render_dashed_vertical_edge_column(
    frame: &mut Frame,
    x: u16,
    y_start: u16,
    y_end: u16,
    edge_style: Style,
) {
    if y_end <= y_start {
        return;
    }

    let blank_style = Style::default().bg(Color::Reset);
    for (i, y) in (y_start..y_end).enumerate() {
        let cell = &mut frame.buffer_mut()[(x, y)];
        if i.is_multiple_of(2) {
            cell.set_symbol("│").set_style(edge_style);
        } else {
            cell.set_symbol(" ").set_style(blank_style);
        }
    }
}

fn render_pane_internal_right_edge(
    frame: &mut Frame,
    panel_area: Rect,
    title_y: u16,
    edge_style: Style,
    dim_edge_style: Style,
    show_bottom_edge: bool,
    hidden_y: (u16, u16),
) {
    if panel_area.width == 0 || panel_area.height == 0 {
        return;
    }

    let x = panel_area.right().saturating_sub(1);
    let bottom_y = panel_area.bottom().saturating_sub(1);
    let vertical_top = title_y.saturating_add(1);
    let vertical_bottom = bottom_y.saturating_sub(1);
    let vertical_end = vertical_bottom.saturating_add(1);

    if vertical_top <= vertical_bottom {
        for (segment_start, segment_end) in y_segments_outside(vertical_top, vertical_end, hidden_y)
        {
            render_vertical_edge_column(frame, x, segment_start, segment_end, edge_style);
        }

        let hidden_start = hidden_y.0.max(vertical_top);
        let hidden_end = hidden_y.1.min(vertical_end);
        if hidden_start < hidden_end {
            render_dashed_vertical_edge_column(frame, x, hidden_start, hidden_end, dim_edge_style);
        }
    }

    frame.render_widget(
        Paragraph::new(PANE_BORDER_SET.top_right)
            .alignment(Alignment::Left)
            .style(edge_style),
        Rect::new(x, title_y, 1, 1),
    );

    if panel_area.height >= 2 && (show_bottom_edge || panel_area.width >= 2) {
        frame.render_widget(
            Paragraph::new(PANE_BORDER_SET.bottom_right)
                .alignment(Alignment::Left)
                .style(edge_style),
            Rect::new(x, bottom_y, 1, 1),
        );
    } else {
        frame.render_widget(
            Paragraph::new(PANE_BORDER_SET.vertical_right)
                .alignment(Alignment::Left)
                .style(edge_style),
            Rect::new(x, bottom_y, 1, 1),
        );
    }
}

fn compute_hidden_right_edge_ranges(
    pane_infos: &[PaneInfo],
) -> std::collections::HashMap<crate::layout::PaneId, (u16, u16)> {
    let focused = pane_infos.iter().find(|info| info.is_focused);
    let Some(focused) = focused else {
        return std::collections::HashMap::new();
    };

    let focused_has_panel_above = pane_infos.iter().any(|info| {
        info.id != focused.id && placement_is_adjacent(focused.rect, info.rect, SplitSide::Top)
    });
    let focused_has_panel_below = pane_infos.iter().any(|info| {
        info.id != focused.id && placement_is_adjacent(focused.rect, info.rect, SplitSide::Bottom)
    });

    let mut hidden = std::collections::HashMap::new();
    for info in pane_infos {
        if info.id == focused.id {
            continue;
        }
        if placement_is_adjacent(info.rect, focused.rect, SplitSide::Right) {
            if let Some(mut range) = adjacent_right_edge_y_range(info.rect, focused.rect) {
                if focused_has_panel_above && info.rect.y < focused.rect.y {
                    // Start hiding below the focused pane title row so the vertical rule
                    // reaches through the panel-above bottom cap.
                    range.0 = range.0.saturating_add(1);
                }
                if focused_has_panel_below && info.rect.bottom() > focused.rect.bottom() {
                    // Stop hiding above the panel-below title row so the vertical rule
                    // reaches through the focused pane bottom cap.
                    range.1 = range.1.saturating_sub(1);
                }
                if range.0 < range.1 {
                    hidden.insert(info.id, range);
                }
            }
        }
    }
    hidden
}

fn pane_title_hit_area(
    area: Rect,
    title: &PaneChromeTitle,
    maximized: bool,
    hide: bool,
) -> Option<Rect> {
    if area.width < 4 || area.height == 0 {
        return None;
    }
    let layout = pane_title_chrome_layout(area.width, title, maximized, hide);
    Some(Rect::new(
        area.x,
        area.y,
        layout.details_width.min(area.width).max(1),
        1,
    ))
}

fn pane_hides_instead_of_closing(
    app: &AppState,
    _ws: &Workspace,
    pane_id: crate::layout::PaneId,
) -> bool {
    app.terminal_state_for_pane(pane_id)
        .is_some_and(crate::terminal::TerminalState::is_agent_terminal)
}

fn pane_chrome_title_for_pane(
    app: &AppState,
    ws: &Workspace,
    pane_id: crate::layout::PaneId,
) -> PaneChromeTitle {
    let terminal = app.terminal_state_for_pane(pane_id);
    let header = app.pane_header();
    let assigned_name = terminal.and_then(|terminal| {
        crate::pane_names::assigned_names(&app.terminals).remove(&terminal.id)
    });
    let name = header.agent_name.then(|| {
        pane_name_label(terminal, assigned_name, ws.public_pane_number(pane_id))
            .unwrap_or_else(|| "Workspace".to_string())
    });
    let git_status = ws.git_status_for_pane(pane_id);
    let path = pane_header_path(app, pane_id, terminal, &git_status);
    let folder = path.as_deref().and_then(|path| {
        header_folder_label(path, header.working_directory, header.parent_directory)
    });
    let in_repo = git_status.space.is_some()
        || git_status
            .branch
            .as_deref()
            .is_some_and(|branch| !branch.is_empty());
    let branch = header
        .git_branch
        .then(|| git_branch_label(&git_status))
        .flatten();
    let status = (header.git_status && in_repo)
        .then(|| worktree_state_marker(git_status.worktree_state).to_string());
    PaneChromeTitle {
        name,
        folder,
        git: git_suffix(branch.as_deref(), status.as_deref()),
    }
}

fn pane_header_path(
    app: &AppState,
    pane_id: crate::layout::PaneId,
    terminal: Option<&crate::terminal::TerminalState>,
    git_status: &crate::workspace::WorkspaceGitStatusSnapshot,
) -> Option<String> {
    if let Some(location) = app.view.agent_locations.get(&pane_id) {
        return Some(location.path.clone());
    }
    let cwd = terminal.map(|terminal| terminal.cwd.as_path())?;
    if cwd.as_os_str().is_empty() {
        return None;
    }
    Some(display_location_path(cwd, git_status))
}

/// The folder text a pane header writes from a display path.
///
/// Working directory is the last folder. Parent directory is the folder above
/// it. Both together are `parent/current`, the same pair the agent table uses.
fn header_folder_label(path: &str, show_working: bool, show_parent: bool) -> Option<String> {
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() {
        return None;
    }
    let mut parts = trimmed.rsplit('/');
    let current = parts.next().filter(|part| !part.is_empty())?;
    let parent = parts.next();
    match (show_parent, show_working) {
        (false, false) => None,
        (false, true) => Some(current.to_string()),
        (true, false) => match parent {
            Some("") | None => None,
            Some(parent) => Some(parent.to_string()),
        },
        (true, true) => Some(match parent {
            Some("") => format!("/{current}"),
            Some(parent) => format!("{parent}/{current}"),
            None => current.to_string(),
        }),
    }
}

fn git_suffix(branch: Option<&str>, status: Option<&str>) -> Option<String> {
    match (branch, status) {
        (None, None) => None,
        (Some(branch), Some(status)) => Some(format!("({branch} {status})")),
        (Some(branch), None) => Some(format!("({branch})")),
        (None, Some(status)) => Some(format!("({status})")),
    }
}

/// The pane a drop is aimed at and where on it, whether the thing being carried
pub(super) fn pane_swap_preview_target(
    app: &AppState,
) -> Option<(crate::layout::PaneId, crate::layout::DropZone)> {
    match &app.drag.as_ref()?.target {
        crate::app::state::DragTarget::PaneSwap {
            moved: true,
            hovered_pane_id: Some(target),
            drop_zone,
            ..
        }
        | crate::app::state::DragTarget::AgentDock {
            hovered_pane_id: Some(target),
            drop_zone,
            ..
        } => Some((*target, *drop_zone)),
        _ => None,
    }
}

/// What a drop would do, said in the room it would take. The same movement
/// means a swap or a cut depending only on where the pointer is, so the room
/// itself has to say which — a message alone would leave the two looking the
/// same until it was too late to aim again.
fn render_pane_swap_drop_overlay(
    app: &AppState,
    frame: &mut Frame,
    area: Rect,
    zone: crate::layout::DropZone,
) {
    let area = crate::layout::drop_zone_rect(area, zone);
    if area.width == 0 || area.height == 0 {
        return;
    }

    let edge_style = Style::default()
        .fg(app.palette.focused_pane_border())
        .bg(Color::Reset);
    frame.render_widget(Clear, area);
    frame.render_widget(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Plain)
            .border_style(edge_style)
            .style(Style::default().bg(Color::Reset)),
        area,
    );

    let message = match zone {
        crate::layout::DropZone::Over => "Drop to swap",
        crate::layout::DropZone::Edge(_) => "Drop to split",
    };
    let message_y = area.y + area.height.saturating_sub(1) / 2;
    frame.render_widget(
        Paragraph::new(message)
            .alignment(Alignment::Center)
            .style(edge_style),
        Rect {
            x: area.x,
            y: message_y,
            width: area.width,
            height: 1,
        },
    );
}

fn pane_chrome_controls(
    area: Rect,
    pane_id: crate::layout::PaneId,
    controls_text: &str,
    controls_width: u16,
) -> Vec<PaneChromeControl> {
    if controls_width == 0 || controls_text.is_empty() {
        return Vec::new();
    }

    let controls_x = pane_chrome_controls_x(area, controls_width);
    vec![
        PaneChromeControl {
            pane_id,
            action: PaneChromeAction::Close,
            rect: pane_close_control_rect(area, controls_x, controls_text),
        },
        PaneChromeControl {
            pane_id,
            action: PaneChromeAction::Focus,
            rect: pane_focus_control_rect(area, controls_x, controls_text),
        },
    ]
}

pub(crate) fn pane_is_scrolled_back(rt: &TerminalRuntime) -> bool {
    rt.scroll_metrics()
        .is_some_and(|metrics| metrics.offset_from_bottom > 0)
}

fn truncate_label(text: &str, max_width: usize) -> String {
    let len = text.chars().count();
    if len <= max_width {
        return text.to_string();
    }
    if max_width == 0 {
        return String::new();
    }
    if max_width == 1 {
        return "…".to_string();
    }
    let prefix: String = text.chars().take(max_width.saturating_sub(1)).collect();
    format!("{prefix}…")
}

fn truncate_to_width(text: &str, width: usize) -> String {
    truncate_label(text, width)
}

struct PaneChromeTitle {
    name: Option<String>,
    folder: Option<String>,
    git: Option<String>,
}

impl PaneChromeTitle {
    #[cfg(test)]
    fn name_only(name: impl Into<String>) -> Self {
        Self {
            name: Some(name.into()),
            folder: None,
            git: None,
        }
    }

    fn formatted_title(&self) -> String {
        let mut parts = Vec::new();
        if let Some(name) = &self.name {
            parts.push(name.as_str());
        }
        if let Some(folder) = &self.folder {
            parts.push(folder.as_str());
        }
        if let Some(git) = &self.git {
            parts.push(git.as_str());
        }
        parts.join(" ")
    }
}

fn pane_title_spans(
    title: &PaneChromeTitle,
    truncated: &str,
    name_style: Style,
    rest_style: Style,
) -> Vec<Span<'static>> {
    if truncated.is_empty() {
        return Vec::new();
    }
    if let Some(name) = &title.name {
        if let Some(rest) = truncated.strip_prefix(name) {
            let mut spans = vec![Span::styled(name.clone(), name_style)];
            if !rest.is_empty() {
                spans.push(Span::styled(rest.to_string(), rest_style));
            }
            return spans;
        }
    }
    vec![Span::styled(
        truncated.to_string(),
        if title.name.is_some() {
            name_style
        } else {
            rest_style
        },
    )]
}

fn render_code_ui_pane_chrome(
    app: &AppState,
    frame: &mut Frame,
    area: Rect,
    title: PaneChromeTitle,
    pane_id: crate::layout::PaneId,
    focused: bool,
    highlighted: bool,
    maximized: bool,
    hide: bool,
    exposed: ExposedSides,
    hidden_right_edge_y: Option<(u16, u16)>,
) -> Vec<PaneChromeControl> {
    if area.width == 0 || area.height == 0 {
        return Vec::new();
    }

    let edge_color = if focused || highlighted {
        focus_accent(app)
    } else {
        app.palette.overlay0
    };
    let edge_style = Style::default().fg(edge_color).bg(Color::Reset);
    let dim_edge_style = Style::default()
        .fg(app.palette.dim_pane_border())
        .bg(Color::Reset);
    let show_right_edge = !exposed.right;
    let hide_full_right_edge = hidden_right_edge_y.is_some() && show_right_edge && area.width > 0;
    let chrome_active = focused || highlighted;
    let borders = pane_frame_borders(exposed, chrome_active, hide_full_right_edge);

    let block = Block::default()
        .borders(borders)
        .border_type(BorderType::Plain)
        .border_style(edge_style)
        .style(Style::default().bg(Color::Reset))
        .border_set(PANE_BORDER_SET);
    frame.render_widget(block, area);

    if !pane_draws_left_vertical_edge(exposed, chrome_active) {
        render_pane_open_left_edge(frame, area, edge_style, dim_edge_style);
    }

    if let Some(hidden_y) = hidden_right_edge_y {
        if show_right_edge {
            render_pane_internal_right_edge(
                frame,
                area,
                area.y,
                edge_style,
                dim_edge_style,
                exposed.bottom,
                hidden_y,
            );
        }
    }

    if area.width < 4 {
        return Vec::new();
    }

    let title_width = area.width;
    let layout = pane_title_chrome_layout(title_width, &title, maximized, hide);
    let (controls_text, controls_width) = pane_controls_text(title_width, maximized, hide);

    let rule_glyph = '─';
    let unfocused_style = Style::default().fg(app.palette.overlay0).bg(Color::Reset);
    let pane_name_style = if chrome_active {
        Style::default().fg(focus_accent(app)).bg(Color::Reset)
    } else {
        unfocused_style
    };
    let text_available = title_width
        .saturating_sub("╭─ ".chars().count() as u16)
        .saturating_sub(controls_width)
        .saturating_sub(3) as usize;
    let title_text = truncate_to_width(&title.formatted_title(), text_available);
    let rest_style = if chrome_active {
        Style::default().fg(app.palette.overlay1).bg(Color::Reset)
    } else {
        unfocused_style
    };
    let rule_text = if layout.rule_width > 0 {
        rule_glyph.to_string().repeat(layout.rule_width as usize)
    } else {
        String::new()
    };

    let mut spans = vec![Span::styled("╭─ ".to_string(), edge_style)];
    spans.extend(pane_title_spans(
        &title,
        &title_text,
        pane_name_style,
        rest_style,
    ));
    spans.push(Span::styled(" ".to_string(), edge_style));
    if !rule_text.is_empty() {
        spans.push(Span::styled(rule_text, edge_style));
    }
    if !controls_text.is_empty() {
        spans.push(Span::styled(controls_text, edge_style));
    }

    frame.render_widget(
        Paragraph::new(Line::from(spans)).alignment(Alignment::Left),
        Rect::new(area.x, area.y, area.width.saturating_sub(1), 1),
    );

    pane_chrome_controls(area, pane_id, controls_text, controls_width)
}

fn stable_terminal_inner_rect(pane_inner: Rect) -> Rect {
    if pane_inner.width <= 4 {
        return pane_inner;
    }

    Rect::new(
        pane_inner.x,
        pane_inner.y,
        pane_inner.width.saturating_sub(1),
        pane_inner.height,
    )
}

fn pane_name_label(
    terminal: Option<&crate::terminal::TerminalState>,
    assigned_name: Option<String>,
    pane_number: Option<usize>,
) -> Option<String> {
    terminal
        .and_then(|terminal| terminal.manual_label.clone())
        .or_else(|| terminal.and_then(|terminal| terminal.agent_name.clone()))
        .or(assigned_name)
        .or_else(|| pane_number.map(|number| format!("Pane {number}")))
}

pub(super) use crate::workspace::display_path_with_home;

/// The checkout a worktree nested inside its own repository is written
/// against: the repository's own checkout.
///
/// A worktree made under `<repo>/.claude/worktrees/<name>` repeats the
/// repository's whole path and then adds a name the branch beside it already
/// carries, so the location says the same thing twice and pushes everything
/// else off the row. Written against the repository, the path is the
/// repository's and the name is the branch's. A worktree that lives outside
/// its repository keeps its own path, which is the only place its folder is
/// named.
pub(super) fn primary_checkout_root(
    space: &crate::workspace::GitSpaceMetadata,
) -> Option<&std::path::Path> {
    if !space.is_linked_worktree {
        return None;
    }
    let common_dir = std::path::Path::new(&space.key);
    if common_dir.file_name() != Some(std::ffi::OsStr::new(".git")) {
        return None;
    }
    let primary_root = common_dir.parent()?;
    std::path::Path::new(&space.checkout_key)
        .starts_with(primary_root)
        .then_some(primary_root)
}

/// Where a pane reads as working: its own path, unless it sits in a worktree
/// nested inside its repository, in which case the same place written against
/// the repository's checkout.
pub(super) fn display_location_path(
    path: &std::path::Path,
    git_status: &crate::workspace::WorkspaceGitStatusSnapshot,
) -> String {
    let rewritten = git_status.space.as_ref().and_then(|space| {
        let primary_root = primary_checkout_root(space)?;
        let below_root = path.strip_prefix(&space.repo_root).ok()?;
        if below_root.as_os_str().is_empty() {
            return Some(primary_root.to_path_buf());
        }
        Some(primary_root.join(below_root))
    });
    display_path_with_home(rewritten.as_deref().unwrap_or(path))
}

/// The branch the checkout is on. A nested worktree with no branch still says
/// `worktree`, because the rewritten path no longer names the checkout.
pub(super) fn git_branch_label(
    git_status: &crate::workspace::WorkspaceGitStatusSnapshot,
) -> Option<String> {
    let branch = git_status
        .branch
        .as_deref()
        .filter(|branch| !branch.is_empty())
        .map(str::to_string);
    if branch.is_some() {
        return branch;
    }
    git_status
        .space
        .as_ref()
        .and_then(primary_checkout_root)
        .is_some()
        .then(|| "worktree".to_string())
}

/// Single-glyph dirty marker for a worktree state, shared by pane chrome and
/// the agent panel.
pub(super) fn worktree_state_marker(state: crate::workspace::GitWorktreeState) -> &'static str {
    match state {
        crate::workspace::GitWorktreeState::Clean => "✓",
        crate::workspace::GitWorktreeState::Staged => "+",
        crate::workspace::GitWorktreeState::Unstaged => "!",
        crate::workspace::GitWorktreeState::Mixed => "±",
    }
}

fn pane_inner_rect(area: Rect, framed: bool) -> Rect {
    apply_pane_inner_padding(pane_content_rect(area, framed))
}

fn runtime_for_tab_pane<'a>(
    terminal_runtimes: &'a TerminalRuntimeRegistry,
    tab: &'a crate::workspace::Tab,
    pane_id: crate::layout::PaneId,
) -> Option<(&'a crate::terminal::TerminalId, &'a TerminalRuntime)> {
    let terminal_id = tab.terminal_id(pane_id)?;
    #[cfg(test)]
    if let Some(runtime) = tab.runtimes.get(&pane_id) {
        return Some((terminal_id, runtime));
    }
    terminal_runtimes
        .get(terminal_id)
        .map(|runtime| (terminal_id, runtime))
}

fn stable_scrollbar_gutter(rt: &TerminalRuntime, pane_inner: Rect) -> (Rect, Option<Rect>) {
    let inner_rect = stable_terminal_inner_rect(pane_inner);
    if inner_rect == pane_inner {
        return (inner_rect, None);
    }
    let gutter = Rect::new(
        pane_inner.x + pane_inner.width.saturating_sub(1),
        pane_inner.y,
        1,
        pane_inner.height,
    );
    let scrollbar_rect = rt
        .scroll_metrics()
        .filter(|metrics| should_show_scrollbar(*metrics))
        .map(|_| gutter);

    (inner_rect, scrollbar_rect)
}

/// Resize every visible runtime in a tab to the geometry it would receive if the tab were selected.
pub(super) fn resize_tab_panes(
    app: &AppState,
    terminal_runtimes: &TerminalRuntimeRegistry,
    tab: &crate::workspace::Tab,
    area: Rect,
    cell_size: crate::kitty_graphics::HostCellSize,
) {
    let framed = tab.layout.pane_count() >= 1;

    if tab.zoomed {
        let focused_id = tab.layout.focused();
        if let Some((terminal_id, rt)) = runtime_for_tab_pane(terminal_runtimes, tab, focused_id) {
            let pane_inner = pane_inner_rect(area, framed);
            let inner_rect = stable_terminal_inner_rect(pane_inner);
            if !app.direct_attach_resize_locks.contains(terminal_id) {
                rt.resize(
                    inner_rect.height,
                    inner_rect.width,
                    cell_size.width_px,
                    cell_size.height_px,
                );
            }
        }
        return;
    }

    for info in tab.layout.panes(area) {
        let pane_inner = pane_inner_rect(info.rect, framed);

        if let Some((terminal_id, rt)) = runtime_for_tab_pane(terminal_runtimes, tab, info.id) {
            let inner_rect = stable_terminal_inner_rect(pane_inner);
            if !app.direct_attach_resize_locks.contains(terminal_id) {
                rt.resize(
                    inner_rect.height,
                    inner_rect.width,
                    cell_size.width_px,
                    cell_size.height_px,
                );
            }
        }
    }
}

/// Compute pane layout info and optionally resize pane runtimes to match.
pub(super) fn compute_pane_infos(
    app: &AppState,
    terminal_runtimes: &TerminalRuntimeRegistry,
    area: Rect,
    resize_panes: bool,
    cell_size: crate::kitty_graphics::HostCellSize,
) -> Vec<PaneInfo> {
    let Some(ws_idx) = app.active else {
        return Vec::new();
    };
    let Some(ws) = app.workspaces.get(ws_idx) else {
        return Vec::new();
    };

    let framed = ws.layout.pane_count() >= 1;
    if let Some(pane_id) = app.agent_peek {
        let pane_inner = pane_inner_rect(area, true);
        let mut inner_rect = pane_inner;
        let mut scrollbar_rect = None;
        if let Some(rt) = app.runtime_for_agent_pane(terminal_runtimes, pane_id) {
            (inner_rect, scrollbar_rect) = stable_scrollbar_gutter(rt, pane_inner);
            if resize_panes
                && app
                    .terminal_id_for_any_pane(pane_id)
                    .is_some_and(|terminal_id| {
                        !app.direct_attach_resize_locks.contains(&terminal_id)
                    })
            {
                rt.resize(
                    inner_rect.height,
                    inner_rect.width,
                    cell_size.width_px,
                    cell_size.height_px,
                );
            }
        }
        return vec![PaneInfo {
            id: pane_id,
            rect: area,
            inner_rect,
            scrollbar_rect,
            is_focused: true,
            exposed: ExposedSides::all(),
        }];
    }
    if ws.zoomed {
        let focused_id = ws.layout.focused();
        let pane_inner = pane_inner_rect(area, framed);
        let mut inner_rect = pane_inner;
        let mut scrollbar_rect = None;
        if let Some(rt) = app.runtime_for_pane_in_workspace(terminal_runtimes, ws_idx, focused_id) {
            (inner_rect, scrollbar_rect) = stable_scrollbar_gutter(rt, pane_inner);
            if resize_panes
                && ws.terminal_id(focused_id).is_some_and(|terminal_id| {
                    !app.direct_attach_resize_locks.contains(terminal_id)
                })
            {
                rt.resize(
                    inner_rect.height,
                    inner_rect.width,
                    cell_size.width_px,
                    cell_size.height_px,
                );
            }
        }
        return vec![PaneInfo {
            id: focused_id,
            rect: area,
            inner_rect,
            scrollbar_rect,
            is_focused: true,
            exposed: ExposedSides::all(),
        }];
    }

    let mut pane_infos = ws.layout.panes(area);

    for info in &mut pane_infos {
        let pane_inner = pane_inner_rect(info.rect, framed);

        let mut inner_rect = pane_inner;
        let mut scrollbar_rect = None;
        if let Some(rt) = app.runtime_for_pane_in_workspace(terminal_runtimes, ws_idx, info.id) {
            (inner_rect, scrollbar_rect) = stable_scrollbar_gutter(rt, pane_inner);
            if resize_panes
                && ws.terminal_id(info.id).is_some_and(|terminal_id| {
                    !app.direct_attach_resize_locks.contains(terminal_id)
                })
            {
                rt.resize(
                    inner_rect.height,
                    inner_rect.width,
                    cell_size.width_px,
                    cell_size.height_px,
                );
            }
        }

        info.inner_rect = inner_rect;
        info.scrollbar_rect = scrollbar_rect;
    }

    pane_infos
}

pub(super) fn render_panes(
    app: &AppState,
    terminal_runtimes: &TerminalRuntimeRegistry,
    frame: &mut Frame,
    area: Rect,
) {
    let Some(ws_idx) = app.active else {
        render_empty(app, frame, area);
        return;
    };
    let Some(ws) = app.workspaces.get(ws_idx) else {
        render_empty(app, frame, area);
        return;
    };

    let peeking = app.agent_peek.is_some();
    let multi_pane = !peeking && ws.layout.pane_count() > 1;
    let framed = peeking || ws.layout.pane_count() >= 1;
    let terminal_active = app.mode == Mode::Terminal;
    let hidden_right_edges = if multi_pane {
        compute_hidden_right_edge_ranges(&app.view.pane_infos)
    } else {
        std::collections::HashMap::new()
    };

    let swap_preview = pane_swap_preview_target(app);

    for info in &app.view.pane_infos {
        if let Some(rt) = app.runtime_for_agent_pane(terminal_runtimes, info.id) {
            let preview_zone = swap_preview
                .filter(|(target, _)| *target == info.id)
                .map(|(_, zone)| zone);
            let is_swap_preview = preview_zone.is_some();
            // A cut takes half the pane, so the other half has to go on being
            // read: the preview is drawn over the pane rather than instead of
            // it. A swap takes the whole pane, and there is nothing left to see.
            let covers_pane = preview_zone == Some(crate::layout::DropZone::Over);
            if framed {
                let title = pane_chrome_title_for_pane(app, ws, info.id);
                render_code_ui_pane_chrome(
                    app,
                    frame,
                    info.rect,
                    title,
                    info.id,
                    info.is_focused,
                    is_swap_preview,
                    peeking || ws.zoomed,
                    pane_hides_instead_of_closing(app, ws, info.id),
                    info.exposed,
                    hidden_right_edges.get(&info.id).copied(),
                );
                let content_rect = pane_content_rect(info.rect, framed);
                let padded_rect = pane_inner_rect(info.rect, framed);
                render_pane_inner_padding(frame, content_rect, padded_rect);
            }

            if !covers_pane {
                let show_cursor = info.is_focused
                    && terminal_active
                    && !pane_is_scrolled_back(rt)
                    && !cursor_hidden_by_host_focus(app)
                    && !is_swap_preview;
                rt.render(frame, info.inner_rect, show_cursor);
                render_pane_scrollbar(app, frame, info, rt);
            }
            if let Some(zone) = preview_zone {
                render_pane_swap_drop_overlay(app, frame, info.inner_rect, zone);
            }

            let pane_dimmed = ws.pane_state(info.id).is_some_and(|pane| pane.dimmed);
            if pane_dimmed && !is_swap_preview {
                let muted = app.palette.overlay0;
                let inner = info.inner_rect;
                let buf = frame.buffer_mut();
                for y in inner.y..inner.y + inner.height {
                    for x in inner.x..inner.x + inner.width {
                        buf[(x, y)].set_fg(muted);
                    }
                }
            }

            let should_dim = !info.is_focused && multi_pane && !terminal_active && !is_swap_preview;
            if should_dim {
                let inner = info.inner_rect;
                let buf = frame.buffer_mut();
                for y in inner.y..inner.y + inner.height {
                    for x in inner.x..inner.x + inner.width {
                        let cell = &mut buf[(x, y)];
                        cell.set_style(cell.style().add_modifier(Modifier::DIM));
                    }
                }
            }

            render_selection_highlight(
                &app.selection,
                frame,
                info.id,
                info.inner_rect,
                rt.scroll_metrics(),
                &app.palette,
                app.host_terminal_theme,
            );
            render_copy_mode_cursor(app, frame, info);
        }
    }
}

pub(super) fn compute_pane_chrome_controls(app: &AppState) -> Vec<PaneChromeControl> {
    let Some(ws_idx) = app.active else {
        return Vec::new();
    };
    let Some(ws) = app.workspaces.get(ws_idx) else {
        return Vec::new();
    };
    if ws.layout.pane_count() == 0 {
        return Vec::new();
    }

    let zoomed = app.agent_peek.is_some() || ws.active_tab().map(|tab| tab.zoomed).unwrap_or(false);

    app.view
        .pane_infos
        .iter()
        .flat_map(|info| {
            let hide = pane_hides_instead_of_closing(app, ws, info.id);
            let (controls_text, controls_width) = pane_controls_text(info.rect.width, zoomed, hide);
            if controls_width == 0 || info.rect.height == 0 {
                return Vec::new();
            }
            pane_chrome_controls(info.rect, info.id, controls_text, controls_width)
        })
        .collect()
}

pub(super) fn compute_pane_title_hit_areas(app: &AppState) -> Vec<PaneTitleHitArea> {
    let Some(ws_idx) = app.active else {
        return Vec::new();
    };
    let Some(ws) = app.workspaces.get(ws_idx) else {
        return Vec::new();
    };
    if let Some(pane_id) = app.agent_peek {
        return app
            .view
            .pane_infos
            .iter()
            .find(|info| info.id == pane_id)
            .and_then(|info| {
                let title = pane_chrome_title_for_pane(app, ws, info.id);
                let hide = pane_hides_instead_of_closing(app, ws, info.id);
                pane_title_hit_area(info.rect, &title, true, hide).map(|rect| PaneTitleHitArea {
                    pane_id: info.id,
                    rect,
                })
            })
            .into_iter()
            .collect();
    }
    if ws.layout.pane_count() <= 1 {
        return Vec::new();
    }

    let zoomed = ws.active_tab().map(|tab| tab.zoomed).unwrap_or(false);
    if zoomed {
        return Vec::new();
    }

    app.view
        .pane_infos
        .iter()
        .filter_map(|info| {
            let title = pane_chrome_title_for_pane(app, ws, info.id);
            let hide = pane_hides_instead_of_closing(app, ws, info.id);
            pane_title_hit_area(info.rect, &title, zoomed, hide).map(|rect| PaneTitleHitArea {
                pane_id: info.id,
                rect,
            })
        })
        .collect()
}

fn render_copy_mode_cursor(app: &AppState, frame: &mut Frame, info: &PaneInfo) {
    if app.mode != Mode::Copy {
        return;
    }
    let Some(copy_mode) = app.copy_mode else {
        return;
    };
    if copy_mode.pane_id != info.id
        || copy_mode.cursor_row >= info.inner_rect.height
        || copy_mode.cursor_col >= info.inner_rect.width
    {
        return;
    }

    let x = info.inner_rect.x + copy_mode.cursor_col;
    let y = info.inner_rect.y + copy_mode.cursor_row;
    let cell = &mut frame.buffer_mut()[(x, y)];
    cell.set_style(
        Style::default()
            .fg(panel_contrast_fg(&app.palette))
            .bg(app.palette.accent)
            .add_modifier(Modifier::BOLD),
    );
}

fn render_selection_highlight(
    selection: &Option<crate::selection::Selection>,
    frame: &mut Frame,
    pane_id: crate::layout::PaneId,
    inner: Rect,
    scroll_metrics: Option<crate::pane::ScrollMetrics>,
    p: &Palette,
    host_theme: crate::terminal_theme::TerminalTheme,
) {
    if let Some(sel) = selection {
        if sel.is_visible() && sel.pane_id == pane_id {
            let buf = frame.buffer_mut();
            let style = automatic_selection_style(p, host_theme);
            for y in 0..inner.height {
                for x in 0..inner.width {
                    if sel.contains(y, x, scroll_metrics) {
                        let cell = &mut buf[(inner.x + x, inner.y + y)];
                        cell.set_style(style);
                    }
                }
            }
        }
    }
}

type Rgb = (u8, u8, u8);

fn automatic_selection_style(
    p: &Palette,
    host_theme: crate::terminal_theme::TerminalTheme,
) -> Style {
    let bg = automatic_selection_bg(p, host_theme);
    Style::reset().fg(selection_fg_for_bg(bg, p)).bg(bg)
}

fn automatic_selection_bg(p: &Palette, host_theme: crate::terminal_theme::TerminalTheme) -> Color {
    let Some(background) = host_theme.background.map(terminal_theme_to_rgb) else {
        return selection_palette_background(p);
    };

    let target = if relative_luminance(background) < 0.5 {
        (255, 255, 255)
    } else {
        (0, 0, 0)
    };
    let selected = mix_rgb(background, target, 0.28);
    Color::Rgb(selected.0, selected.1, selected.2)
}

fn selection_palette_background(p: &Palette) -> Color {
    if p.panel_bg == Color::Reset {
        p.surface_dim
    } else {
        p.panel_bg
    }
}

fn terminal_theme_to_rgb(color: crate::terminal_theme::RgbColor) -> Rgb {
    (color.r, color.g, color.b)
}

fn selection_fg_for_bg(bg: Color, p: &Palette) -> Color {
    color_to_rgb(bg)
        .map(|bg| {
            if relative_luminance(bg) < 0.5 {
                Color::White
            } else {
                Color::Black
            }
        })
        .unwrap_or_else(|| panel_contrast_fg(p))
}

/// How much chroma a focus color gives up while the host terminal window is
/// unfocused. Draining color is what actually makes a mark stop claiming
/// attention, and because it holds the color's luminance it costs no contrast
/// against anything — so every palette can afford all of it.
const HOST_UNFOCUSED_DESATURATION: f32 = 0.7;

/// The furthest a focus color travels toward the panel background while the
/// host terminal window is unfocused. Past half way the color is more panel
/// than accent and stops reading as a mark at all.
const HOST_UNFOCUSED_MAX_MIX: f32 = 0.5;

/// How far short of [`HOST_UNFOCUSED_MAX_MIX`] the mute will settle for, in
/// steps, when the full amount would break the contrast floor.
const HOST_UNFOCUSED_MIX_STEPS: u8 = 10;

/// The contrast a muted focus color keeps against the panel behind it. This
/// color is worn by bold names and border glyphs, and 3:1 is the accepted floor
/// for both. A palette whose accent starts near the floor — Tokyo Night Day's
/// sits at about 3.1:1 — therefore recedes less, or not at all, and leans on
/// losing its color instead.
const HOST_UNFOCUSED_MIN_CONTRAST: f32 = 3.0;

/// Whether the focused pane should drop its cursor because the host terminal
/// window is unfocused.
///
/// Emulators vary in how (or whether) they hollow the cursor on focus loss, so
/// the block can keep reading as live on another screen. Removing it entirely
/// is unambiguous: no caret, no keystrokes landing here.
pub(crate) fn cursor_hidden_by_host_focus(app: &AppState) -> bool {
    app.hide_cursor_when_unfocused && app.host_window_unfocused()
}

/// Mutes a color that means "focused" while the host terminal window itself is
/// unfocused.
///
/// Every mark that claims your input — a pane border, a sidebar band, the name
/// on the row you are typing into — is making a claim that is false for the
/// whole window when the window is not the one you are in. Rather than each
/// surface inventing its own unfocused tone, they all pass their focus color
/// through here. Palettes whose colors are not RGB (the terminal 16-color
/// theme) fall back to the muted overlay color, which is where unfocused chrome
/// already lives.
pub(super) fn mute_when_host_unfocused(app: &AppState, color: Color) -> Color {
    if !app.host_window_unfocused() {
        return color;
    }
    let (Some(rgb), Some(panel_bg)) = (color_to_rgb(color), color_to_rgb(app.palette.panel_bg))
    else {
        return app.palette.overlay0;
    };
    // Drain the color first. This is free — it holds luminance, so it cannot
    // push anything below the floor — and it is the part of the effect that
    // every palette gets in full.
    let drained = desaturate(rgb, HOST_UNFOCUSED_DESATURATION);
    // Then recede toward the panel as far as this palette can afford, backing
    // off a step at a time until the result still reads against it.
    for step in (1..=HOST_UNFOCUSED_MIX_STEPS).rev() {
        let amount = HOST_UNFOCUSED_MAX_MIX * f32::from(step) / f32::from(HOST_UNFOCUSED_MIX_STEPS);
        let muted = mix_rgb(drained, panel_bg, amount);
        if contrast_ratio(muted, panel_bg) >= HOST_UNFOCUSED_MIN_CONTRAST {
            return Color::Rgb(muted.0, muted.1, muted.2);
        }
    }
    // A palette with no headroom at all keeps the accent's brightness and gives
    // up only its color.
    Color::Rgb(drained.0, drained.1, drained.2)
}

/// Pulls a color toward the neutral gray of the same relative luminance, so it
/// loses chroma without gaining or losing contrast against anything.
fn desaturate(color: Rgb, amount: f32) -> Rgb {
    mix_rgb(
        color,
        gray_with_luminance(relative_luminance(color)),
        amount,
    )
}

/// The neutral gray at a given relative luminance — [`relative_luminance`]
/// run backwards through the sRGB transfer.
fn gray_with_luminance(luminance: f32) -> Rgb {
    let value = if luminance <= 0.003_130_8 {
        luminance * 12.92
    } else {
        1.055 * luminance.powf(1.0 / 2.4) - 0.055
    };
    let channel = (value.clamp(0.0, 1.0) * 255.0).round() as u8;
    (channel, channel, channel)
}

/// The pane-focus color that says "your keystrokes land here", muted while the
/// host window is unfocused.
pub(super) fn focus_accent(app: &AppState) -> Color {
    mute_when_host_unfocused(app, app.palette.focused_pane_border())
}

pub(super) fn mix_rgb(base: Rgb, target: Rgb, amount: f32) -> Rgb {
    fn channel(base: u8, target: u8, amount: f32) -> u8 {
        (f32::from(base) + (f32::from(target) - f32::from(base)) * amount).round() as u8
    }
    (
        channel(base.0, target.0, amount),
        channel(base.1, target.1, amount),
        channel(base.2, target.2, amount),
    )
}

/// WCAG contrast ratio, 1.0 (identical) through 21.0 (black on white).
pub(super) fn contrast_ratio(a: Rgb, b: Rgb) -> f32 {
    let (a, b) = (relative_luminance(a), relative_luminance(b));
    let (lighter, darker) = if a > b { (a, b) } else { (b, a) };
    (lighter + 0.05) / (darker + 0.05)
}

fn relative_luminance(color: Rgb) -> f32 {
    fn channel(value: u8) -> f32 {
        let value = f32::from(value) / 255.0;
        if value <= 0.03928 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    }
    0.2126 * channel(color.0) + 0.7152 * channel(color.1) + 0.0722 * channel(color.2)
}

pub(super) fn color_to_rgb(color: Color) -> Option<Rgb> {
    match color {
        Color::Reset => None,
        Color::Black => Some((0, 0, 0)),
        Color::Red => Some((128, 0, 0)),
        Color::Green => Some((0, 128, 0)),
        Color::Yellow => Some((128, 128, 0)),
        Color::Blue => Some((0, 0, 128)),
        Color::Magenta => Some((128, 0, 128)),
        Color::Cyan => Some((0, 128, 128)),
        Color::Gray => Some((192, 192, 192)),
        Color::DarkGray => Some((128, 128, 128)),
        Color::LightRed => Some((255, 0, 0)),
        Color::LightGreen => Some((0, 255, 0)),
        Color::LightYellow => Some((255, 255, 0)),
        Color::LightBlue => Some((0, 0, 255)),
        Color::LightMagenta => Some((255, 0, 255)),
        Color::LightCyan => Some((0, 255, 255)),
        Color::White => Some((255, 255, 255)),
        Color::Rgb(r, g, b) => Some((r, g, b)),
        Color::Indexed(_) => None,
    }
}

fn render_empty(app: &AppState, frame: &mut Frame, area: Rect) {
    let p = &app.palette;
    let lines = vec![
        Line::from(""),
        Line::from(""),
        Line::from(Span::styled(
            "  No workspaces yet",
            Style::default().fg(p.overlay0),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "  A workspace is one project context.",
            Style::default().fg(p.overlay1),
        )),
        Line::from(Span::styled(
            "  Its root pane (top-left) sets the default repo or folder name.",
            Style::default().fg(p.overlay1),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled("  Press ", Style::default().fg(p.overlay0)),
            Span::styled(
                app.keybinds
                    .new_workspace
                    .label()
                    .unwrap_or_else(|| "unset".to_string()),
                Style::default().fg(p.accent).add_modifier(Modifier::BOLD),
            ),
            Span::styled(" to create one", Style::default().fg(p.overlay0)),
        ]),
    ];
    frame.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(p.surface_dim)),
        ),
        area,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::Mode;
    use crate::layout::PaneId;
    use crate::selection::Selection;
    use crate::terminal::TerminalRuntime;
    use crate::workspace::{Workspace, WorkspaceGitStatusSnapshot};
    use ratatui::layout::Direction;

    fn rect_contains(rect: Rect, col: u16, row: u16) -> bool {
        col >= rect.x && col < rect.x + rect.width && row >= rect.y && row < rect.y + rect.height
    }

    fn checkout(
        key: &str,
        checkout_key: &str,
        is_linked_worktree: bool,
    ) -> WorkspaceGitStatusSnapshot {
        WorkspaceGitStatusSnapshot {
            branch: Some("eich".to_string()),
            ahead_behind: None,
            space: Some(crate::workspace::GitSpaceMetadata {
                key: key.to_string(),
                checkout_key: checkout_key.to_string(),
                label: "herdr".to_string(),
                repo_root: checkout_key.into(),
                is_linked_worktree,
            }),
            worktree_state: crate::workspace::GitWorktreeState::Clean,
            landed: false,
        }
    }

    #[test]
    fn a_worktree_inside_its_repository_reads_as_the_repository() {
        let status = checkout(
            "/repo/herdr/.git",
            "/repo/herdr/.claude/worktrees/eich",
            true,
        );

        assert_eq!(
            display_location_path(
                std::path::Path::new("/repo/herdr/.claude/worktrees/eich"),
                &status
            ),
            "/repo/herdr"
        );
        assert_eq!(git_branch_label(&status).as_deref(), Some("eich"));
    }

    #[test]
    fn a_herdr_worktree_inside_its_repository_reads_as_the_repository() {
        let status = checkout(
            "/repo/herdr/.git",
            "/repo/herdr/.herdr/worktrees/silver-river",
            true,
        );

        assert_eq!(
            display_location_path(
                std::path::Path::new("/repo/herdr/.herdr/worktrees/silver-river"),
                &status
            ),
            "/repo/herdr"
        );
        assert_eq!(git_branch_label(&status).as_deref(), Some("eich"));
    }

    #[test]
    fn a_worktree_branch_is_not_prefixed_with_worktree() {
        let mut status = checkout(
            "/repo/herdr/.git",
            "/repo/herdr/.herdr/worktrees/worktree-quiet-river-1085",
            true,
        );
        status.branch = Some("worktree/quiet-river-1085".to_string());

        assert_eq!(
            git_branch_label(&status).as_deref(),
            Some("worktree/quiet-river-1085")
        );
    }

    #[test]
    fn a_folder_below_a_nested_worktree_keeps_what_is_below_it() {
        let status = checkout(
            "/repo/herdr/.git",
            "/repo/herdr/.claude/worktrees/eich",
            true,
        );

        assert_eq!(
            display_location_path(
                std::path::Path::new("/repo/herdr/.claude/worktrees/eich/website"),
                &status
            ),
            "/repo/herdr/website"
        );
    }

    #[test]
    fn a_worktree_outside_its_repository_keeps_its_own_path() {
        let status = checkout("/repo/herdr/.git", "/repo/herdr-eich", true);

        assert_eq!(
            display_location_path(std::path::Path::new("/repo/herdr-eich"), &status),
            "/repo/herdr-eich"
        );
        assert_eq!(git_branch_label(&status).as_deref(), Some("eich"));
    }

    #[test]
    fn a_repository_that_is_not_a_worktree_keeps_its_own_path() {
        let status = checkout("/repo/herdr/.git", "/repo/herdr", false);

        assert_eq!(
            display_location_path(std::path::Path::new("/repo/herdr"), &status),
            "/repo/herdr"
        );
        assert_eq!(git_branch_label(&status).as_deref(), Some("eich"));
    }

    #[test]
    fn a_nested_worktree_off_a_branch_still_reads_as_a_worktree() {
        let mut status = checkout(
            "/repo/herdr/.git",
            "/repo/herdr/.claude/worktrees/eich",
            true,
        );
        status.branch = None;

        assert_eq!(git_branch_label(&status).as_deref(), Some("worktree"));
    }

    #[test]
    fn header_folder_label_selects_parent_and_working_segments() {
        assert_eq!(
            header_folder_label("~/lab/herdr", true, false).as_deref(),
            Some("herdr")
        );
        assert_eq!(
            header_folder_label("~/lab/herdr", false, true).as_deref(),
            Some("lab")
        );
        assert_eq!(
            header_folder_label("~/lab/herdr", true, true).as_deref(),
            Some("lab/herdr")
        );
        assert_eq!(header_folder_label("~/lab/herdr", false, false), None);
        assert_eq!(
            header_folder_label("herdr", true, false).as_deref(),
            Some("herdr")
        );
        assert_eq!(header_folder_label("herdr", false, true), None);
        assert_eq!(
            header_folder_label("/herdr", true, true).as_deref(),
            Some("/herdr")
        );
    }

    #[test]
    fn pane_chrome_title_joins_enabled_fields() {
        let title = PaneChromeTitle {
            name: Some("Olivia".into()),
            folder: Some("lab/herdr".into()),
            git: git_suffix(Some("main"), Some("✓")),
        };
        assert_eq!(title.formatted_title(), "Olivia lab/herdr (main ✓)");
        assert_eq!(
            PaneChromeTitle {
                name: None,
                folder: Some("herdr".into()),
                git: git_suffix(Some("main"), None),
            }
            .formatted_title(),
            "herdr (main)"
        );
        assert_eq!(
            PaneChromeTitle::name_only("Olivia").formatted_title(),
            "Olivia"
        );
        assert!(PaneChromeTitle {
            name: None,
            folder: None,
            git: None,
        }
        .formatted_title()
        .is_empty());
    }

    #[test]
    fn pane_chrome_title_for_pane_follows_header_toggles() {
        let mut app = AppState::test_new();
        let workspace = Workspace::test_new("test");
        let pane_id = workspace.tabs[0].root_pane;
        let terminal_id = workspace.tabs[0]
            .terminal_id(pane_id)
            .expect("test workspace has a root terminal")
            .clone();
        let mut terminal = crate::terminal::TerminalState::new(
            terminal_id.clone(),
            std::path::PathBuf::from("/home/aaron/lab/herdr"),
        );
        terminal.set_manual_label("Olivia".into());
        app.terminals.insert(terminal_id, terminal);
        app.workspaces = vec![workspace];
        app.workspaces[0].pane_git_statuses.insert(
            pane_id,
            WorkspaceGitStatusSnapshot {
                branch: Some("main".into()),
                ahead_behind: None,
                space: None,
                worktree_state: crate::workspace::GitWorktreeState::Unstaged,
                landed: false,
            },
        );

        assert_eq!(
            pane_chrome_title_for_pane(&app, &app.workspaces[0], pane_id).formatted_title(),
            "Olivia"
        );

        app.pane_header.working_directory = true;
        app.pane_header.parent_directory = true;
        let folder = header_folder_label(
            &display_path_with_home(std::path::Path::new("/home/aaron/lab/herdr")),
            true,
            true,
        )
        .expect("parent/current folder");
        assert_eq!(
            pane_chrome_title_for_pane(&app, &app.workspaces[0], pane_id).formatted_title(),
            format!("Olivia {folder}")
        );

        app.pane_header.git_branch = true;
        app.pane_header.git_status = true;
        assert_eq!(
            pane_chrome_title_for_pane(&app, &app.workspaces[0], pane_id).formatted_title(),
            format!("Olivia {folder} (main !)")
        );

        app.pane_header.agent_name = false;
        assert_eq!(
            pane_chrome_title_for_pane(&app, &app.workspaces[0], pane_id).formatted_title(),
            format!("{folder} (main !)")
        );
    }

    #[test]
    fn an_agent_pane_says_hide_instead_of_close() {
        let (full, _) = pane_controls_text(40, false, true);
        assert!(full.contains("HIDE"), "{full:?}");
        assert!(!full.contains('✕'), "{full:?}");

        let (compact, _) = pane_controls_text(12, false, true);
        assert!(compact.contains("HIDE"), "{compact:?}");
        assert!(!compact.contains('✕'), "{compact:?}");

        let (shell, _) = pane_controls_text(40, false, false);
        assert!(shell.contains('✕'), "{shell:?}");
        assert!(!shell.contains("HIDE"), "{shell:?}");
    }

    #[test]
    fn pane_close_control_rect_covers_hide_word_on_an_agent_pane() {
        let area = Rect::new(10, 2, 40, 5);
        let (controls_text, controls_width) = pane_controls_text(area.width, false, true);
        let controls_x = pane_chrome_controls_x(area, controls_width);
        let close = pane_close_control_rect(area, controls_x, controls_text);
        let suffix = pane_close_suffix(controls_text);
        let suffix_start = controls_x + controls_text.width() as u16 - suffix.width() as u16;

        assert_eq!(suffix, " HIDE ");
        assert!(rect_contains(close, suffix_start, area.y));
        assert!(rect_contains(close, suffix_start + 1, area.y));
        assert!(rect_contains(close, area.x + area.width - 2, area.y));
    }

    #[test]
    fn pane_close_control_rect_covers_close_suffix_and_padding() {
        let area = Rect::new(10, 2, 24, 5);
        let (controls_text, controls_width) = pane_controls_text(area.width, false, false);
        let controls_x = pane_chrome_controls_x(area, controls_width);
        let close = pane_close_control_rect(area, controls_x, controls_text);
        let suffix_start =
            controls_x + controls_text.width() as u16 - PANE_CLOSE_CONTROL_SUFFIX.width() as u16;
        let cross_col = suffix_start + 1;
        let trailing_col = suffix_start + 2;
        let padding_col = area.x + area.width - 2;

        assert!(rect_contains(close, suffix_start, area.y));
        assert!(rect_contains(close, cross_col, area.y));
        assert!(rect_contains(close, trailing_col, area.y));
        assert!(rect_contains(close, padding_col, area.y));
    }

    #[test]
    fn pane_title_hit_area_excludes_rule_glyphs() {
        let area = Rect::new(0, 0, 80, 10);
        let title = PaneChromeTitle::name_only("Agent Work");
        let layout = pane_title_chrome_layout(area.width, &title, false, false);
        let hit = pane_title_hit_area(area, &title, false, false).expect("title hit area");

        assert!(layout.rule_width > 0, "expected decorative rule glyphs");
        assert_eq!(hit.width, layout.details_width);
        assert!(hit.width < area.width);
        let rule_col = hit.x + hit.width;
        assert!(
            !rect_contains(hit, rule_col, area.y),
            "rule glyph column should not be part of swap hit area"
        );
    }

    #[test]
    fn rendered_hide_control_covers_hide_word() {
        let app = AppState::test_new();
        let area = Rect::new(0, 0, 40, 5);
        let backend = ratatui::backend::TestBackend::new(40, 5);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();

        terminal
            .draw(|frame| {
                render_code_ui_pane_chrome(
                    &app,
                    frame,
                    area,
                    PaneChromeTitle::name_only("panel"),
                    PaneId::from_raw(1),
                    true,
                    false,
                    false,
                    true,
                    ExposedSides::all(),
                    None,
                );
            })
            .unwrap();

        let buffer = terminal.backend().buffer();
        let top_row: String = (0..area.width).map(|x| buffer[(x, 0)].symbol()).collect();
        assert!(top_row.contains("HIDE"), "{top_row:?}");
        assert!(!top_row.contains('✕'), "{top_row:?}");

        let hide_col = (0..area.width)
            .find(|x| buffer[(*x, 0)].symbol() == "H")
            .expect("HIDE should render in pane chrome");
        let (controls_text, controls_width) = pane_controls_text(area.width, false, true);
        let controls_x = pane_chrome_controls_x(area, controls_width);
        let close = pane_close_control_rect(area, controls_x, controls_text);
        assert!(
            rect_contains(close, hide_col, area.y),
            "hide rect {close:?} should cover rendered HIDE at column {hide_col}"
        );
    }

    #[test]
    fn rendered_close_control_rect_covers_cross() {
        let app = AppState::test_new();
        let area = Rect::new(0, 0, 24, 5);
        let backend = ratatui::backend::TestBackend::new(24, 5);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();

        terminal
            .draw(|frame| {
                render_code_ui_pane_chrome(
                    &app,
                    frame,
                    area,
                    PaneChromeTitle::name_only("panel"),
                    PaneId::from_raw(1),
                    true,
                    false,
                    false,
                    false,
                    ExposedSides::all(),
                    None,
                );
            })
            .unwrap();

        let buffer = terminal.backend().buffer();
        let cross_col = (0..area.width)
            .find(|x| buffer[(*x, 0)].symbol() == "✕")
            .expect("cross glyph should render in pane chrome");
        let (controls_text, controls_width) = pane_controls_text(area.width, false, false);
        let controls_x = pane_chrome_controls_x(area, controls_width);
        let close = pane_close_control_rect(area, controls_x, controls_text);

        assert!(
            rect_contains(close, cross_col, area.y),
            "close rect {close:?} should cover rendered cross at column {cross_col}"
        );
    }

    #[test]
    fn focus_accent_mutes_while_host_window_is_unfocused() {
        let mut app = AppState::test_new();
        let accent = app.palette.focused_pane_border();

        // Terminals that never report focus keep the live accent.
        assert_eq!(focus_accent(&app), accent);
        app.outer_terminal_focus = Some(true);
        assert_eq!(focus_accent(&app), accent);

        app.outer_terminal_focus = Some(false);
        let muted = focus_accent(&app);
        assert_ne!(
            muted, accent,
            "unfocused host window should mute the accent"
        );
        assert_ne!(
            muted, app.palette.overlay0,
            "the focused pane must stay distinguishable from unfocused panes"
        );
    }

    #[test]
    fn muted_focus_accent_holds_its_contrast_floor_in_every_built_in_theme() {
        const THEMES: &[&str] = &[
            "catppuccin",
            "catppuccin-latte",
            "tokyo-night",
            "tokyo-night-day",
            "dracula",
            "synthwave",
            "nord",
            "gruvbox",
            "gruvbox-light",
            "one-dark",
            "one-light",
            "solarized",
            "solarized-light",
            "kanagawa",
            "kanagawa-lotus",
            "rose-pine",
            "rose-pine-dawn",
            "vesper",
        ];

        for name in THEMES {
            let mut app = AppState::test_new();
            app.palette = crate::app::state::Palette::from_name(name)
                .unwrap_or_else(|| panic!("{name} should be a known theme"));
            app.outer_terminal_focus = Some(false);

            let muted = focus_accent(&app);
            assert_ne!(
                muted,
                app.palette.focused_pane_border(),
                "{name}: the accent should visibly mute"
            );

            let (Some(muted), Some(panel_bg)) =
                (color_to_rgb(muted), color_to_rgb(app.palette.panel_bg))
            else {
                continue;
            };
            let contrast = contrast_ratio(muted, panel_bg);
            assert!(
                contrast >= HOST_UNFOCUSED_MIN_CONTRAST,
                "{name}: muted accent sits at {contrast:.2}:1 against the panel"
            );
        }
    }

    #[test]
    fn focus_accent_falls_back_to_overlay_without_rgb_colors() {
        let mut app = AppState::test_new();
        app.palette = crate::app::state::Palette::terminal();
        app.outer_terminal_focus = Some(false);

        assert_eq!(focus_accent(&app), app.palette.overlay0);
    }

    #[test]
    fn cursor_hides_only_when_enabled_and_host_window_unfocused() {
        let mut app = AppState::test_new();
        assert!(app.hide_cursor_when_unfocused);

        assert!(!cursor_hidden_by_host_focus(&app), "focus unknown");
        app.outer_terminal_focus = Some(true);
        assert!(!cursor_hidden_by_host_focus(&app));

        app.outer_terminal_focus = Some(false);
        assert!(cursor_hidden_by_host_focus(&app));

        app.hide_cursor_when_unfocused = false;
        assert!(!cursor_hidden_by_host_focus(&app));
    }

    #[test]
    fn focused_pane_border_renders_muted_while_host_window_unfocused() {
        fn left_edge_color(app: &AppState) -> Option<Color> {
            let area = Rect::new(0, 0, 24, 5);
            let backend = ratatui::backend::TestBackend::new(24, 5);
            let mut terminal = ratatui::Terminal::new(backend).unwrap();
            terminal
                .draw(|frame| {
                    render_code_ui_pane_chrome(
                        app,
                        frame,
                        area,
                        PaneChromeTitle::name_only("panel"),
                        PaneId::from_raw(1),
                        true,
                        false,
                        false,
                        false,
                        ExposedSides::all(),
                        None,
                    );
                })
                .unwrap();
            terminal.backend().buffer()[(0, 2)].fg.into()
        }

        let mut app = AppState::test_new();
        app.outer_terminal_focus = Some(true);
        let focused_edge = left_edge_color(&app);
        assert_eq!(focused_edge, Some(app.palette.focused_pane_border()));

        app.outer_terminal_focus = Some(false);
        assert_eq!(left_edge_color(&app), Some(focus_accent(&app)));
        assert_ne!(left_edge_color(&app), focused_edge);
    }

    #[test]
    fn code_ui_pane_chrome_overwrites_existing_top_right_corner() {
        let app = AppState::test_new();
        let backend = ratatui::backend::TestBackend::new(20, 5);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();

        terminal
            .draw(|frame| {
                render_code_ui_pane_chrome(
                    &app,
                    frame,
                    Rect::new(0, 0, 20, 5),
                    PaneChromeTitle::name_only("panel"),
                    PaneId::from_raw(1),
                    true,
                    false,
                    false,
                    false,
                    ExposedSides::all(),
                    None,
                );
            })
            .unwrap();

        let buffer = terminal.backend().buffer();
        let top_row: String = (0..20).map(|x| buffer[(x, 0)].symbol()).collect();
        assert!(top_row.ends_with('╮'), "{top_row:?}");
        assert!(!top_row.ends_with("╮╮"));
    }

    #[tokio::test]
    async fn tall_left_pane_draws_right_edge_through_panel_above_focused_split() {
        let mut app = AppState::test_new();
        let mut ws = Workspace::test_new("test");
        let c = ws.tabs[0].root_pane;
        let a = ws.test_split(Direction::Horizontal);
        ws.tabs[0].layout.focus_pane(a);
        let b = ws.test_split(Direction::Vertical);
        ws.insert_test_runtime(c, TerminalRuntime::test_with_screen_bytes(10, 6, b"left"));
        ws.insert_test_runtime(a, TerminalRuntime::test_with_screen_bytes(10, 3, b"top"));
        ws.insert_test_runtime(b, TerminalRuntime::test_with_screen_bytes(10, 3, b"bot"));

        app.workspaces = vec![ws];
        app.active = Some(0);
        app.mode = Mode::Terminal;

        let area = Rect::new(0, 0, 40, 12);
        crate::ui::compute_view(&mut app, area);

        let c_info = app
            .view
            .pane_infos
            .iter()
            .find(|info| info.id == c)
            .expect("left pane");
        let b_info = app
            .view
            .pane_infos
            .iter()
            .find(|info| info.id == b)
            .expect("focused bottom-right pane");
        assert!(c_info.rect.y < b_info.rect.y);

        let edge_x = c_info.rect.right().saturating_sub(1);
        let focused_title_row = b_info.rect.y;

        let backend = ratatui::backend::TestBackend::new(area.width, area.height);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| crate::ui::render(&app, frame))
            .unwrap();
        let buffer = terminal.backend().buffer();

        assert_eq!(
            buffer[(edge_x, focused_title_row)].symbol(),
            "│",
            "left pane should keep the vertical rule through the panel-above bottom cap row"
        );
    }

    #[tokio::test]
    async fn tall_left_pane_draws_right_edge_through_panel_below_focused_split() {
        let mut app = AppState::test_new();
        let mut ws = Workspace::test_new("test");
        let l = ws.tabs[0].root_pane;
        let b = ws.test_split(Direction::Horizontal);
        ws.tabs[0].layout.focus_pane(b);
        let d = ws.test_split(Direction::Vertical);
        ws.tabs[0].layout.focus_pane(b);
        ws.insert_test_runtime(l, TerminalRuntime::test_with_screen_bytes(10, 6, b"left"));
        ws.insert_test_runtime(b, TerminalRuntime::test_with_screen_bytes(10, 3, b"top"));
        ws.insert_test_runtime(d, TerminalRuntime::test_with_screen_bytes(10, 3, b"bot"));

        app.workspaces = vec![ws];
        app.active = Some(0);
        app.mode = Mode::Terminal;

        let area = Rect::new(0, 0, 40, 12);
        crate::ui::compute_view(&mut app, area);

        let l_info = app
            .view
            .pane_infos
            .iter()
            .find(|info| info.id == l)
            .expect("left pane");
        let b_info = app
            .view
            .pane_infos
            .iter()
            .find(|info| info.id == b)
            .expect("focused top-right pane");
        assert!(l_info.rect.bottom() > b_info.rect.bottom());

        let edge_x = l_info.rect.right().saturating_sub(1);
        let focused_bottom_cap_row = b_info.rect.bottom().saturating_sub(1);

        let backend = ratatui::backend::TestBackend::new(area.width, area.height);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| crate::ui::render(&app, frame))
            .unwrap();
        let buffer = terminal.backend().buffer();

        assert_eq!(
            buffer[(edge_x, focused_bottom_cap_row)].symbol(),
            "│",
            "left pane should keep the vertical rule through the focused pane bottom cap row"
        );
    }

    #[test]
    fn horizontal_split_unfocused_right_pane_draws_dashed_dim_shared_left_border() {
        let app = AppState::test_new();
        let backend = ratatui::backend::TestBackend::new(40, 8);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let right = Rect::new(20, 0, 20, 8);
        let mut exposed = ExposedSides::all();
        exposed.left = false;

        terminal
            .draw(|frame| {
                render_code_ui_pane_chrome(
                    &app,
                    frame,
                    right,
                    PaneChromeTitle::name_only("panel"),
                    PaneId::from_raw(2),
                    false,
                    false,
                    false,
                    false,
                    exposed,
                    None,
                );
            })
            .unwrap();

        let buffer = terminal.backend().buffer();
        let left_x = right.x;
        let bottom_y = right.y + right.height.saturating_sub(1);
        assert_eq!(buffer[(left_x, right.y)].symbol(), "╭");
        assert_eq!(buffer[(left_x, bottom_y)].symbol(), "╰");
        assert_eq!(
            buffer[(left_x, right.y + 1)].symbol(),
            "│",
            "dashed shared left border should start with a vertical rule"
        );
        assert_eq!(
            buffer[(left_x, right.y + 1)].fg,
            app.palette.dim_pane_border()
        );
        assert_ne!(
            buffer[(left_x, right.y + 2)].symbol(),
            "│",
            "dashed shared left border should leave every other row blank"
        );
        assert_eq!(buffer[(left_x, right.y + 3)].symbol(), "│");
        assert_eq!(
            buffer[(left_x, right.y + 3)].fg,
            app.palette.dim_pane_border()
        );
    }

    #[test]
    fn focused_right_pane_draws_left_border_on_shared_edge() {
        let app = AppState::test_new();
        let backend = ratatui::backend::TestBackend::new(40, 8);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let right = Rect::new(20, 0, 20, 8);
        let mut exposed = ExposedSides::all();
        exposed.left = false;

        terminal
            .draw(|frame| {
                render_code_ui_pane_chrome(
                    &app,
                    frame,
                    right,
                    PaneChromeTitle::name_only("panel"),
                    PaneId::from_raw(2),
                    true,
                    false,
                    false,
                    false,
                    exposed,
                    None,
                );
            })
            .unwrap();

        let buffer = terminal.backend().buffer();
        assert_eq!(
            buffer[(right.x, 1)].symbol(),
            "│",
            "focused pane should draw the shared left border"
        );
    }

    #[test]
    fn horizontal_split_keeps_symmetric_terminal_insets() {
        let mut app = AppState::test_new();
        let mut workspace = Workspace::test_new("test");
        let root = workspace.tabs[0].root_pane;
        let _ = workspace.test_split(ratatui::layout::Direction::Horizontal);
        workspace.tabs[0].layout.focus_pane(root);
        app.workspaces = vec![workspace];
        app.active = Some(0);

        let area = Rect::new(0, 0, 40, 10);
        let terminal_runtimes = TerminalRuntimeRegistry::new();
        let infos = compute_pane_infos(
            &app,
            &terminal_runtimes,
            area,
            false,
            crate::kitty_graphics::HostCellSize::default(),
        );
        for info in &infos {
            assert_eq!(info.inner_rect.x, info.rect.x + 1);
            assert_eq!(
                info.inner_rect.width,
                info.rect.width.saturating_sub(2),
                "pane {id:?} should keep border insets",
                id = info.id
            );
        }
    }

    #[tokio::test]
    async fn pane_scrollbar_gutter_is_reserved_before_scrollback_exists() {
        let mut app = AppState::test_new();
        let mut workspace = Workspace::test_new("test");
        let root_pane = workspace.tabs[0].root_pane;
        workspace.tabs[0].runtimes.insert(
            root_pane,
            TerminalRuntime::test_with_scrollback_bytes(40, 8, 1024, b"ready\n"),
        );
        app.workspaces = vec![workspace];
        app.active = Some(0);

        let area = Rect::new(10, 3, 40, 8);
        let terminal_runtimes = TerminalRuntimeRegistry::new();
        let infos = compute_pane_infos(
            &app,
            &terminal_runtimes,
            area,
            false,
            crate::kitty_graphics::HostCellSize::default(),
        );
        let info = &infos[0];

        assert_eq!(info.rect, area);
        assert_eq!(info.scrollbar_rect, None);
        assert_eq!(info.inner_rect, Rect::new(11, 4, 37, 6));
    }

    #[tokio::test]
    async fn zoomed_pane_scrollbar_gutter_is_reserved_before_scrollback_exists() {
        let mut app = AppState::test_new();
        let mut workspace = Workspace::test_new("test");
        workspace.zoomed = true;
        let root_pane = workspace.tabs[0].root_pane;
        workspace.tabs[0].runtimes.insert(
            root_pane,
            TerminalRuntime::test_with_scrollback_bytes(40, 8, 1024, b"ready\n"),
        );
        app.workspaces = vec![workspace];
        app.active = Some(0);

        let area = Rect::new(10, 3, 40, 8);
        let terminal_runtimes = TerminalRuntimeRegistry::new();
        let infos = compute_pane_infos(
            &app,
            &terminal_runtimes,
            area,
            false,
            crate::kitty_graphics::HostCellSize::default(),
        );
        let info = &infos[0];

        assert_eq!(info.rect, area);
        assert_eq!(info.scrollbar_rect, None);
        assert_eq!(info.inner_rect, Rect::new(11, 4, 37, 6));
    }

    #[tokio::test]
    async fn zoomed_multi_pane_keeps_border_space() {
        let mut app = AppState::test_new();
        let mut workspace = Workspace::test_new("test");
        let focused_pane = workspace.test_split(ratatui::layout::Direction::Horizontal);
        workspace.zoomed = true;
        workspace.tabs[0].runtimes.insert(
            focused_pane,
            TerminalRuntime::test_with_scrollback_bytes(40, 8, 1024, b"ready\n"),
        );
        app.workspaces = vec![workspace];
        app.active = Some(0);

        let area = Rect::new(10, 3, 40, 8);
        let terminal_runtimes = TerminalRuntimeRegistry::new();
        let infos = compute_pane_infos(
            &app,
            &terminal_runtimes,
            area,
            false,
            crate::kitty_graphics::HostCellSize::default(),
        );
        let info = &infos[0];

        assert_eq!(info.id, focused_pane);
        assert_eq!(info.rect, area);
        assert_eq!(info.scrollbar_rect, None);
        assert_eq!(info.inner_rect, Rect::new(11, 4, 37, 6));
    }

    #[test]
    fn pane_chrome_renders_only_the_pane_name_with_a_single_line_rule() {
        let app = AppState::test_new();
        let area = Rect::new(0, 0, 80, 5);
        let backend = ratatui::backend::TestBackend::new(80, 5);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();

        terminal
            .draw(|frame| {
                render_code_ui_pane_chrome(
                    &app,
                    frame,
                    area,
                    PaneChromeTitle::name_only("Review notes"),
                    PaneId::from_raw(1),
                    true,
                    false,
                    false,
                    false,
                    ExposedSides::all(),
                    None,
                );
            })
            .unwrap();

        let buffer = terminal.backend().buffer();
        let top_row: String = (0..area.width)
            .map(|x| buffer[(x, area.y)].symbol())
            .collect();
        assert!(top_row.starts_with("╭─ Review notes "), "{top_row:?}");
        assert!(!top_row.contains('═'), "{top_row:?}");
    }

    #[test]
    fn pane_chrome_title_unfocused_uses_muted_color_throughout() {
        let app = AppState::test_new();
        let area = Rect::new(0, 0, 120, 5);
        let backend = ratatui::backend::TestBackend::new(120, 5);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();

        terminal
            .draw(|frame| {
                render_code_ui_pane_chrome(
                    &app,
                    frame,
                    area,
                    PaneChromeTitle::name_only("Review notes"),
                    PaneId::from_raw(1),
                    false, // unfocused
                    false, // not highlighted
                    false,
                    false,
                    ExposedSides::all(),
                    None,
                );
            })
            .unwrap();

        let buffer = terminal.backend().buffer();
        for symbol in ["R", "n"] {
            let col = (0..area.width)
                .find(|x| buffer[(*x, 0)].symbol() == symbol)
                .unwrap_or_else(|| panic!("title glyph {symbol:?} should render"));
            assert_eq!(
                buffer[(col, 0)].fg,
                app.palette.overlay0,
                "glyph {symbol:?} should be muted when unfocused"
            );
        }
    }

    #[test]
    fn pane_name_label_prefers_assigned_name_then_pane_number() {
        assert_eq!(
            pane_name_label(None, Some("Olivia".into()), Some(1)).as_deref(),
            Some("Olivia")
        );
        assert_eq!(
            pane_name_label(None, None, Some(12)).as_deref(),
            Some("Pane 12")
        );
        assert_eq!(pane_name_label(None, None, None), None);
    }

    #[test]
    fn pane_name_prefers_manual_label_over_assigned_name() {
        let terminal_id = crate::terminal::TerminalId::alloc();
        let mut terminal = crate::terminal::TerminalState::new(
            terminal_id,
            std::path::PathBuf::from("/tmp/herdr"),
        );
        terminal.set_manual_label("review notes".into());

        assert_eq!(
            pane_name_label(Some(&terminal), Some("Olivia".into()), Some(2)).as_deref(),
            Some("review notes")
        );
        terminal.clear_manual_label();
        assert_eq!(
            pane_name_label(Some(&terminal), Some("Olivia".into()), Some(2)).as_deref(),
            Some("Olivia")
        );
        terminal.set_agent_name("reviewer".into());
        assert_eq!(
            pane_name_label(Some(&terminal), Some("Olivia".into()), Some(2)).as_deref(),
            Some("reviewer")
        );
    }

    #[test]
    fn pane_border_set_uses_rounded_corners() {
        assert_eq!(PANE_BORDER_SET.top_left, "╭");
        assert_eq!(PANE_BORDER_SET.top_right, "╮");
        assert_eq!(PANE_BORDER_SET.bottom_left, "╰");
        assert_eq!(PANE_BORDER_SET.bottom_right, "╯");
    }

    #[tokio::test]
    async fn tiny_pane_does_not_reserve_scrollbar_gutter() {
        let mut app = AppState::test_new();
        let mut workspace = Workspace::test_new("test");
        let root_pane = workspace.tabs[0].root_pane;
        workspace.tabs[0].runtimes.insert(
            root_pane,
            TerminalRuntime::test_with_scrollback_bytes(4, 8, 1024, b"ready\n"),
        );
        app.workspaces = vec![workspace];
        app.active = Some(0);

        let area = Rect::new(10, 3, 4, 8);
        let terminal_runtimes = TerminalRuntimeRegistry::new();
        let infos = compute_pane_infos(
            &app,
            &terminal_runtimes,
            area,
            false,
            crate::kitty_graphics::HostCellSize::default(),
        );
        let info = &infos[0];

        assert_eq!(info.rect, area);
        assert_eq!(info.scrollbar_rect, None);
        assert_eq!(info.inner_rect, Rect::new(11, 4, 2, 6));
    }

    #[tokio::test]
    async fn pane_scrollbar_reserves_last_column_from_terminal_area() {
        let mut app = AppState::test_new();
        let mut workspace = Workspace::test_new("test");
        let root_pane = workspace.tabs[0].root_pane;
        workspace.tabs[0].runtimes.insert(
            root_pane,
            TerminalRuntime::test_with_scrollback_bytes(
                40,
                8,
                1024,
                b"one\ntwo\nthree\nfour\nfive\nsix\nseven\neight\nnine\nten\n",
            ),
        );
        app.workspaces = vec![workspace];
        app.active = Some(0);

        let area = Rect::new(10, 3, 40, 8);
        let terminal_runtimes = TerminalRuntimeRegistry::new();
        let infos = compute_pane_infos(
            &app,
            &terminal_runtimes,
            area,
            false,
            crate::kitty_graphics::HostCellSize::default(),
        );
        let info = &infos[0];

        assert_eq!(info.rect, area);
        assert_eq!(info.scrollbar_rect, Some(Rect::new(48, 4, 1, 6)));
        assert_eq!(info.inner_rect, Rect::new(11, 4, 37, 6));
    }

    #[test]
    fn selection_highlight_uses_one_uniform_style() {
        let palette = Palette::catppuccin();
        let host_theme = crate::terminal_theme::TerminalTheme {
            foreground: None,
            background: Some(crate::terminal_theme::RgbColor {
                r: 12,
                g: 14,
                b: 16,
            }),
        };
        let expected_style = automatic_selection_style(&palette, host_theme);
        let selection = Some(Selection::range(PaneId::from_raw(1), 0, 0, 2, None));
        let backend = ratatui::backend::TestBackend::new(4, 1);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();

        terminal
            .draw(|frame| {
                let buf = frame.buffer_mut();
                buf[(0, 0)].set_style(
                    Style::default()
                        .fg(Color::Rgb(10, 220, 120))
                        .bg(Color::Black),
                );
                buf[(1, 0)].set_style(
                    Style::default()
                        .fg(Color::Rgb(220, 180, 40))
                        .bg(Color::DarkGray)
                        .add_modifier(Modifier::BOLD),
                );
                buf[(2, 0)].set_style(Style::default().fg(Color::Blue).bg(Color::Reset));
                render_selection_highlight(
                    &selection,
                    frame,
                    PaneId::from_raw(1),
                    Rect::new(0, 0, 4, 1),
                    None,
                    &palette,
                    host_theme,
                );
            })
            .unwrap();

        let buffer = terminal.backend().buffer();
        let first = buffer[(0, 0)].style();
        let second = buffer[(1, 0)].style();
        let third = buffer[(2, 0)].style();

        assert_eq!(first.fg, expected_style.fg);
        assert_eq!(second.fg, expected_style.fg);
        assert_eq!(third.fg, expected_style.fg);
        assert_eq!(first.bg, expected_style.bg);
        assert_eq!(second.bg, expected_style.bg);
        assert_eq!(third.bg, expected_style.bg);
        assert_eq!(first.add_modifier, expected_style.add_modifier);
        assert_eq!(second.add_modifier, expected_style.add_modifier);
        assert_eq!(third.add_modifier, expected_style.add_modifier);
        assert!(!second.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn automatic_selection_background_uses_host_background() {
        let bg = automatic_selection_bg(
            &Palette::terminal(),
            crate::terminal_theme::TerminalTheme {
                foreground: Some(crate::terminal_theme::RgbColor {
                    r: 230,
                    g: 230,
                    b: 230,
                }),
                background: Some(crate::terminal_theme::RgbColor {
                    r: 12,
                    g: 14,
                    b: 16,
                }),
            },
        );

        let Color::Rgb(r, g, b) = bg else {
            panic!("selection background should resolve to rgb");
        };
        assert!(relative_luminance((r, g, b)) > relative_luminance((12, 14, 16)));
    }
}
