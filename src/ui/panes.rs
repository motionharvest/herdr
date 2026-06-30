use ratatui::{
    layout::{Alignment, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph},
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
const FULL_PANE_CONTROLS_WIDTH: u16 = 12;
const COMPACT_PANE_CONTROLS_WIDTH: u16 = 5;
const PANE_CLOSE_CONTROL_SUFFIX: &str = " ✕ ";
const PANE_INNER_PADDING: u16 = 0;
const PANE_TITLE_ICON: &str = "\u{f04fd}";

struct PaneTitleChromeLayout {
    /// Width of the pane name/path details section (before the horizontal rule glyphs).
    details_width: u16,
    rule_width: u16,
}

fn pane_title_chrome_layout(
    title_width: u16,
    title: &PaneChromeTitle,
    maximized: bool,
) -> PaneTitleChromeLayout {
    let (_, controls_width) = pane_controls_text(title_width, maximized);
    let title_prefix_width = "╭─ ".chars().count() + PANE_TITLE_ICON.chars().count() + 1;
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

fn pane_controls_text(title_width: u16, maximized: bool) -> (&'static str, u16) {
    if title_width >= 24 {
        (
            if maximized {
                " ◱ BACK  ✕ "
            } else {
                " ⛶ FOCUS ✕ "
            },
            FULL_PANE_CONTROLS_WIDTH,
        )
    } else if title_width >= 12 {
        (" ⛶ ✕ ", COMPACT_PANE_CONTROLS_WIDTH)
    } else {
        ("", 0)
    }
}

fn pane_chrome_controls_x(area: Rect, controls_width: u16) -> u16 {
    area.x + area.width.saturating_sub(controls_width + 1)
}

fn pane_close_control_rect(area: Rect, controls_x: u16, controls_text: &str) -> Rect {
    let suffix_width = PANE_CLOSE_CONTROL_SUFFIX.width() as u16;
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
        for (segment_start, segment_end) in
            y_segments_outside(vertical_top, vertical_end, hidden_y)
        {
            render_vertical_edge_column(frame, x, segment_start, segment_end, edge_style);
        }

        let hidden_start = hidden_y.0.max(vertical_top);
        let hidden_end = hidden_y.1.min(vertical_end);
        if hidden_start < hidden_end {
            render_dashed_vertical_edge_column(
                frame,
                x,
                hidden_start,
                hidden_end,
                dim_edge_style,
            );
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

fn pane_title_hit_area(area: Rect, title: &PaneChromeTitle, maximized: bool) -> Option<Rect> {
    if area.width < 4 || area.height == 0 {
        return None;
    }
    let layout = pane_title_chrome_layout(area.width, title, maximized);
    Some(Rect::new(
        area.x,
        area.y,
        layout.details_width.min(area.width).max(1),
        1,
    ))
}

fn pane_chrome_title_for_pane(
    app: &AppState,
    ws: &Workspace,
    pane_id: crate::layout::PaneId,
) -> PaneChromeTitle {
    let terminal = ws
        .pane_state(pane_id)
        .and_then(|pane| app.terminals.get(&pane.attached_terminal_id));
    let cwd = ws.active_tab().and_then(|tab| {
        tab.terminal_id(pane_id)
            .and_then(|terminal_id| app.terminals.get(&terminal_id))
            .map(|terminal| terminal.cwd.clone())
    });
    let git_status = ws.git_status_for_pane(pane_id);
    let repo_path = git_status
        .space
        .as_ref()
        .map(|space| display_path_with_home(&space.repo_root));
    PaneChromeTitle {
        pane_type: pane_type_label(terminal),
        folder_name: pane_name_label(terminal, cwd.as_deref()),
        repo_path,
        branch: git_status.branch.filter(|branch| !branch.is_empty()),
        worktree_state: git_status.worktree_state,
    }
}

pub(super) fn pane_swap_preview_target(app: &AppState) -> Option<crate::layout::PaneId> {
    let crate::app::state::DragState {
        target:
            crate::app::state::DragTarget::PaneSwap {
                moved: true,
                hovered_pane_id: Some(target),
                ..
            },
    } = app.drag.as_ref()?
    else {
        return None;
    };
    Some(*target)
}

fn render_pane_swap_drop_overlay(app: &AppState, frame: &mut Frame, area: Rect, highlighted: bool) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    let edge_color = if highlighted {
        app.palette.focused_pane_border()
    } else {
        app.palette.overlay0
    };
    let edge_style = Style::default().fg(edge_color).bg(Color::Reset);
    frame.render_widget(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Plain)
            .border_style(edge_style)
            .style(Style::default().bg(Color::Reset)),
        area,
    );

    if area.height == 0 {
        return;
    }
    let message_y = area.y + area.height.saturating_sub(1) / 2;
    frame.render_widget(
        Paragraph::new("Drop to swap")
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
    pane_type: String,
    folder_name: Option<String>,
    repo_path: Option<String>,
    branch: Option<String>,
    worktree_state: crate::workspace::GitWorktreeState,
}

impl PaneChromeTitle {
    fn formatted_title(&self) -> String {
        let folder = self.folder_name.as_deref().unwrap_or("Workspace");
        let agent = &self.pane_type;
        // Format: "Pane Name {Pi}"
        let mut result = format!("{} {{{}}}", folder, agent);

        if let (Some(repo_path), Some(branch)) = (&self.repo_path, &self.branch) {
            let git_icon = "\u{f418}"; //  git repo icon
            let status = match self.worktree_state {
                crate::workspace::GitWorktreeState::Clean => "✓",
                crate::workspace::GitWorktreeState::Staged => "+",
                crate::workspace::GitWorktreeState::Unstaged => "!",
                crate::workspace::GitWorktreeState::Mixed => "±",
            };
            // Format: "  ~/lab/code-ui (feat/git-status ✓)"
            result.push_str(&format!(
                " {} {} ({} {})",
                git_icon, repo_path, branch, status
            ));
        }

        result
    }
}

fn push_title_name_spans(
    spans: &mut Vec<Span<'static>>,
    text: &str,
    pane_name_style: Style,
    agent_label_style: Style,
    brace_style: Style,
) {
    if let Some((name, rest)) = text.split_once(" {") {
        // `rest` is the agent label plus a closing brace, optionally followed by
        // more title text (e.g. a trailing space before the git section). Locate
        // the closing brace rather than requiring it to be the final character so
        // the agent label keeps its distinct style in those cases too.
        if let Some(close_idx) = rest.find('}') {
            let (agent, after_brace) = rest.split_at(close_idx);
            let after = &after_brace["}".len()..];
            spans.push(Span::styled(name.to_string(), pane_name_style));
            spans.push(Span::styled(" {".to_string(), brace_style));
            spans.push(Span::styled(agent.to_string(), agent_label_style));
            spans.push(Span::styled("}".to_string(), brace_style));
            if !after.is_empty() {
                spans.push(Span::styled(after.to_string(), pane_name_style));
            }
            return;
        }
    }
    spans.push(Span::styled(text.to_string(), pane_name_style));
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
    exposed: ExposedSides,
    hidden_right_edge_y: Option<(u16, u16)>,
) -> Vec<PaneChromeControl> {
    if area.width == 0 || area.height == 0 {
        return Vec::new();
    }

    let edge_color = if focused || highlighted {
        app.palette.focused_pane_border()
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
    let layout = pane_title_chrome_layout(title_width, &title, maximized);
    let (controls_text, controls_width) = pane_controls_text(title_width, maximized);

    let rule_glyph = if focused || highlighted { '═' } else { '─' };
    let git_icon = "\u{f418}";
    let icon_style = if focused || highlighted {
        Style::default().fg(Color::White).bg(Color::Reset)
    } else {
        edge_style
    };
    let pane_name_style = Style::default()
        .fg(app.palette.focused_pane_border())
        .bg(Color::Reset);
    let agent_label_style = Style::default().fg(app.palette.overlay0).bg(Color::Reset);
    let repo_path_style = Style::default()
        .fg(Color::Rgb(0x36, 0xF9, 0xF6))
        .bg(Color::Reset);
    let git_style = Style::default()
        .fg(match title.worktree_state {
            crate::workspace::GitWorktreeState::Clean => Color::Green,
            crate::workspace::GitWorktreeState::Staged => Color::Blue,
            crate::workspace::GitWorktreeState::Unstaged => Color::Red,
            crate::workspace::GitWorktreeState::Mixed => Color::Rgb(0xBE, 0x9A, 0x4A),
        })
        .bg(Color::Reset);
    let text_available = title_width
        .saturating_sub("╭─ ".chars().count() as u16 + PANE_TITLE_ICON.chars().count() as u16 + 1)
        .saturating_sub(controls_width)
        .saturating_sub(3) as usize;
    let title_text = truncate_to_width(&title.formatted_title(), text_available);
    let rule_text = if layout.rule_width > 0 {
        rule_glyph.to_string().repeat(layout.rule_width as usize)
    } else {
        String::new()
    };

    let mut spans = vec![
        Span::styled("╭─ ".to_string(), edge_style),
        Span::styled(PANE_TITLE_ICON.to_string(), icon_style),
        Span::styled(" ".to_string(), edge_style),
    ];
    if let Some((before_git_icon, after_git_icon)) = title_text.split_once(git_icon) {
        push_title_name_spans(
            &mut spans,
            before_git_icon,
            pane_name_style,
            agent_label_style,
            pane_name_style,
        );
        spans.push(Span::styled(git_icon.to_string(), icon_style));
        if let Some(paren_start) = after_git_icon.rfind(" (") {
            let (repo_path, branch_status) = after_git_icon.split_at(paren_start);
            spans.push(Span::styled(repo_path.to_string(), repo_path_style));
            spans.push(Span::styled(branch_status.to_string(), git_style));
        } else {
            spans.push(Span::styled(after_git_icon.to_string(), repo_path_style));
        }
    } else {
        push_title_name_spans(
            &mut spans,
            &title_text,
            pane_name_style,
            agent_label_style,
            pane_name_style,
        );
    }
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
    cwd: Option<&std::path::Path>,
) -> Option<String> {
    terminal
        .and_then(|terminal| terminal.manual_label.clone())
        .or_else(|| cwd.map(display_path_with_home))
}

fn pane_type_label(terminal: Option<&crate::terminal::TerminalState>) -> String {
    terminal
        .and_then(|terminal| {
            terminal
                .effective_display_agent()
                .or_else(|| terminal.effective_agent_label().map(format_agent_label))
        })
        .unwrap_or_else(|| "Terminal".to_string())
}

fn format_agent_label(label: &str) -> String {
    match label.to_ascii_lowercase().as_str() {
        "pi" => "Pi".to_string(),
        "codex" => "Codex".to_string(),
        "opencode" => "OpenCode".to_string(),
        "cursor" => "Cursor-CLI".to_string(),
        "claude" => "Claude Code".to_string(),
        _ => label
            .split(['-', '_', ' '])
            .filter(|part| !part.is_empty())
            .map(|part| {
                let mut chars = part.chars();
                match chars.next() {
                    Some(first) => first.to_uppercase().chain(chars).collect::<String>(),
                    None => String::new(),
                }
            })
            .collect::<Vec<_>>()
            .join(" "),
    }
}

fn display_path_with_home(path: &std::path::Path) -> String {
    let home = std::path::Path::new("/home/aaron");
    if let Ok(stripped) = path.strip_prefix(home) {
        if stripped.as_os_str().is_empty() {
            return "~".to_string();
        }
        return format!("~/{}", stripped.display());
    }
    path.display().to_string()
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

    let multi_pane = ws.layout.pane_count() > 1;
    let framed = ws.layout.pane_count() >= 1;
    let terminal_active = app.mode == Mode::Terminal;
    let hidden_right_edges = if multi_pane {
        compute_hidden_right_edge_ranges(&app.view.pane_infos)
    } else {
        std::collections::HashMap::new()
    };

    let swap_preview = pane_swap_preview_target(app);

    for info in &app.view.pane_infos {
        if let Some(rt) = app.runtime_for_pane_in_workspace(terminal_runtimes, ws_idx, info.id) {
            let is_swap_preview = swap_preview == Some(info.id);
            if framed {
                let terminal = ws
                    .pane_state(info.id)
                    .and_then(|pane| app.terminals.get(&pane.attached_terminal_id));
                let cwd = ws
                    .active_tab()
                    .and_then(|tab| tab.cwd_for_pane(info.id, &app.terminals, terminal_runtimes));

                let git_status = ws.git_status_for_pane(info.id);
                let repo_path = git_status
                    .space
                    .as_ref()
                    .map(|space| display_path_with_home(&space.repo_root));
                let title = PaneChromeTitle {
                    pane_type: pane_type_label(terminal),
                    folder_name: pane_name_label(terminal, cwd.as_deref()),
                    repo_path,
                    branch: git_status.branch.filter(|branch| !branch.is_empty()),
                    worktree_state: git_status.worktree_state,
                };
                render_code_ui_pane_chrome(
                    app,
                    frame,
                    info.rect,
                    title,
                    info.id,
                    info.is_focused,
                    is_swap_preview,
                    ws.zoomed,
                    info.exposed,
                    hidden_right_edges.get(&info.id).copied(),
                );
                let content_rect = pane_content_rect(info.rect, framed);
                let padded_rect = pane_inner_rect(info.rect, framed);
                render_pane_inner_padding(frame, content_rect, padded_rect);
            }

            if is_swap_preview {
                render_pane_swap_drop_overlay(app, frame, info.inner_rect, true);
            } else {
                let show_cursor = info.is_focused && terminal_active && !pane_is_scrolled_back(rt);
                rt.render(frame, info.inner_rect, show_cursor);
                render_pane_scrollbar(app, frame, info, rt);
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

    let zoomed = ws.active_tab().map(|tab| tab.zoomed).unwrap_or(false);

    app.view
        .pane_infos
        .iter()
        .flat_map(|info| {
            let (controls_text, controls_width) = pane_controls_text(info.rect.width, zoomed);
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
            pane_title_hit_area(info.rect, &title, zoomed).map(|rect| PaneTitleHitArea {
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

fn mix_rgb(base: Rgb, target: Rgb, amount: f32) -> Rgb {
    fn channel(base: u8, target: u8, amount: f32) -> u8 {
        (f32::from(base) + (f32::from(target) - f32::from(base)) * amount).round() as u8
    }
    (
        channel(base.0, target.0, amount),
        channel(base.1, target.1, amount),
        channel(base.2, target.2, amount),
    )
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

fn color_to_rgb(color: Color) -> Option<Rgb> {
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
    use crate::workspace::Workspace;
    use ratatui::layout::Direction;

    fn rect_contains(rect: Rect, col: u16, row: u16) -> bool {
        col >= rect.x && col < rect.x + rect.width && row >= rect.y && row < rect.y + rect.height
    }

    #[test]
    fn pane_close_control_rect_covers_close_suffix_and_padding() {
        let area = Rect::new(10, 2, 24, 5);
        let (controls_text, controls_width) = pane_controls_text(area.width, false);
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
        let title = PaneChromeTitle {
            pane_type: "Cursor-CLI".to_string(),
            folder_name: Some("Agent Work".to_string()),
            repo_path: Some("~/lab/herdr".to_string()),
            branch: Some("messin".to_string()),
            worktree_state: crate::workspace::GitWorktreeState::Unstaged,
        };
        let layout = pane_title_chrome_layout(area.width, &title, false);
        let hit = pane_title_hit_area(area, &title, false).expect("title hit area");

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
                    PaneChromeTitle {
                        pane_type: "Pi".to_string(),
                        folder_name: Some("panel".to_string()),
                        repo_path: None,
                        branch: None,
                        worktree_state: crate::workspace::GitWorktreeState::Clean,
                    },
                    PaneId::from_raw(1),
                    true,
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
        let (controls_text, controls_width) = pane_controls_text(area.width, false);
        let controls_x = pane_chrome_controls_x(area, controls_width);
        let close = pane_close_control_rect(area, controls_x, controls_text);

        assert!(
            rect_contains(close, cross_col, area.y),
            "close rect {close:?} should cover rendered cross at column {cross_col}"
        );
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
                    PaneChromeTitle {
                        pane_type: "Pi".to_string(),
                        folder_name: Some("panel".to_string()),
                        repo_path: None,
                        branch: None,
                        worktree_state: crate::workspace::GitWorktreeState::Clean,
                    },
                    PaneId::from_raw(1),
                    true,
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
                    PaneChromeTitle {
                        pane_type: "Pi".to_string(),
                        folder_name: Some("panel".to_string()),
                        repo_path: None,
                        branch: None,
                        worktree_state: crate::workspace::GitWorktreeState::Clean,
                    },
                    PaneId::from_raw(2),
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
        assert_eq!(buffer[(left_x, right.y + 1)].fg, app.palette.dim_pane_border());
        assert_ne!(
            buffer[(left_x, right.y + 2)].symbol(),
            "│",
            "dashed shared left border should leave every other row blank"
        );
        assert_eq!(buffer[(left_x, right.y + 3)].symbol(), "│");
        assert_eq!(buffer[(left_x, right.y + 3)].fg, app.palette.dim_pane_border());
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
                    PaneChromeTitle {
                        pane_type: "Pi".to_string(),
                        folder_name: Some("panel".to_string()),
                        repo_path: None,
                        branch: None,
                        worktree_state: crate::workspace::GitWorktreeState::Clean,
                    },
                    PaneId::from_raw(2),
                    true,
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
    fn pane_chrome_git_status_symbol_uses_state_color() {
        let cases = [
            (crate::workspace::GitWorktreeState::Clean, "✓", Color::Green),
            (crate::workspace::GitWorktreeState::Staged, "+", Color::Blue),
            (
                crate::workspace::GitWorktreeState::Unstaged,
                "!",
                Color::Red,
            ),
            (
                crate::workspace::GitWorktreeState::Mixed,
                "±",
                Color::Rgb(0xBE, 0x9A, 0x4A),
            ),
        ];

        for (worktree_state, symbol, expected_color) in cases {
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
                        PaneChromeTitle {
                            pane_type: "Pi".to_string(),
                            folder_name: Some("Pane".to_string()),
                            repo_path: Some("~/lab/herdr".to_string()),
                            branch: Some("feat/git-status".to_string()),
                            worktree_state,
                        },
                        PaneId::from_raw(1),
                        true,
                        false,
                        false,
                        ExposedSides::all(),
                        None,
                    );
                })
                .unwrap();

            let buffer = terminal.backend().buffer();
            let repo_path_col = (0..area.width)
                .find(|x| buffer[(*x, 0)].symbol() == "~")
                .expect("repo path should render");
            assert_eq!(buffer[(repo_path_col, 0)].fg, Color::Rgb(0x36, 0xF9, 0xF6));

            let paren_col = (0..area.width)
                .find(|x| buffer[(*x, 0)].symbol() == "(")
                .expect("branch status parenthesis should render");
            assert_eq!(buffer[(paren_col, 0)].fg, expected_color);
            let marker_col = (0..area.width)
                .find(|x| buffer[(*x, 0)].symbol() == symbol)
                .expect("git status symbol should render");
            assert_eq!(buffer[(marker_col, 0)].fg, expected_color);
        }
    }

    #[test]
    fn pane_chrome_title_styles_name_agent_and_braces_separately() {
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
                    PaneChromeTitle {
                        pane_type: "Pi".to_string(),
                        folder_name: Some("~/lab/herdr".to_string()),
                        repo_path: None,
                        branch: None,
                        worktree_state: crate::workspace::GitWorktreeState::Clean,
                    },
                    PaneId::from_raw(1),
                    true,
                    false,
                    false,
                    ExposedSides::all(),
                    None,
                );
            })
            .unwrap();

        let buffer = terminal.backend().buffer();
        let name_col = (0..area.width)
            .find(|x| buffer[(*x, 0)].symbol() == "~")
            .expect("pane name should render");
        assert_eq!(buffer[(name_col, 0)].fg, app.palette.focused_pane_border());

        let open_brace_col = (0..area.width)
            .find(|x| buffer[(*x, 0)].symbol() == "{")
            .expect("agent brace should render");
        assert_eq!(
            buffer[(open_brace_col, 0)].fg,
            app.palette.focused_pane_border()
        );

        let agent_col = (0..area.width)
            .find(|x| buffer[(*x, 0)].symbol() == "P")
            .expect("agent label should render");
        assert_eq!(buffer[(agent_col, 0)].fg, app.palette.overlay0);
    }

    #[test]
    fn pane_chrome_title_styles_agent_label_when_git_section_present() {
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
                    PaneChromeTitle {
                        pane_type: "Pi".to_string(),
                        folder_name: Some("~/lab/herdr".to_string()),
                        repo_path: Some("~/lab/herdr".to_string()),
                        branch: Some("main".to_string()),
                        worktree_state: crate::workspace::GitWorktreeState::Clean,
                    },
                    PaneId::from_raw(1),
                    true,
                    false,
                    false,
                    ExposedSides::all(),
                    None,
                );
            })
            .unwrap();

        let buffer = terminal.backend().buffer();
        // The agent label keeps the muted overlay color even though the git
        // section follows the closing brace (regression: the trailing space
        // before the git icon used to collapse the whole title to one color).
        let agent_col = (0..area.width)
            .find(|x| buffer[(*x, 0)].symbol() == "P")
            .expect("agent label should render");
        assert_eq!(buffer[(agent_col, 0)].fg, app.palette.overlay0);

        let name_col = (0..area.width)
            .find(|x| buffer[(*x, 0)].symbol() == "~")
            .expect("pane name should render");
        assert_eq!(buffer[(name_col, 0)].fg, app.palette.focused_pane_border());
        assert_ne!(buffer[(agent_col, 0)].fg, buffer[(name_col, 0)].fg);
    }

    #[test]
    fn pane_name_label_formats_home_and_absolute_paths() {
        assert_eq!(
            pane_name_label(None, Some(std::path::Path::new("/home/aaron/lab/herdr"))).as_deref(),
            Some("~/lab/herdr")
        );
        assert_eq!(
            pane_name_label(None, Some(std::path::Path::new("/opt/project"))).as_deref(),
            Some("/opt/project")
        );
    }

    #[test]
    fn pane_name_and_agent_type_are_separate() {
        let terminal_id = crate::terminal::TerminalId::alloc();
        let mut terminal = crate::terminal::TerminalState::new(
            terminal_id,
            std::path::PathBuf::from("/tmp/herdr"),
        );
        terminal.set_detected_state(
            Some(crate::detect::Agent::Pi),
            crate::detect::AgentState::Idle,
        );
        terminal.set_manual_label("review notes".into());

        assert_eq!(
            pane_name_label(Some(&terminal), Some(std::path::Path::new("/tmp/herdr"))).as_deref(),
            Some("review notes")
        );
        assert_eq!(pane_type_label(Some(&terminal)), "Pi");

        terminal.clear_manual_label();
        assert_eq!(
            pane_name_label(Some(&terminal), Some(std::path::Path::new("/tmp/herdr"))).as_deref(),
            Some("/tmp/herdr")
        );
        assert_eq!(pane_type_label(Some(&terminal)), "Pi");
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
