use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
    Frame,
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use super::widgets::panel_contrast_fg;
use crate::{
    app::state::{CopyFeedback, Palette, ToastKind, ToastNotification},
    detect::AgentState,
};

const CONFIG_DIAGNOSTIC_DISMISS_SUFFIX: &str = " ✕ ";

pub(crate) fn copy_feedback_rect(area: Rect, feedback: &CopyFeedback, offset_rows: u16) -> Rect {
    if area.width == 0 || area.height == 0 {
        return Rect::default();
    }

    let content_width = feedback.message.len() as u16 + 4;
    let width = content_width.min(area.width);
    let height = 3u16.min(area.height);
    let x = area.x + area.width.saturating_sub(width) / 2;
    let y = area.y + area.height.saturating_sub(height + offset_rows);
    Rect::new(x, y, width, height)
}

pub(crate) fn toast_notification_rect(
    area: Rect,
    toast: &ToastNotification,
    offset_for_warning: bool,
) -> Rect {
    let content_width = (toast.title.len().max(toast.context.len()) as u16) + 4;
    let width = content_width.saturating_add(2).min(area.width);
    let content_height = if toast.context.is_empty() { 1 } else { 2 };
    let height = (content_height + 2).min(area.height);
    let x = area.x + area.width.saturating_sub(width);
    let y = area.y
        + area
            .height
            .saturating_sub(height + if offset_for_warning { 1 } else { 0 });
    Rect::new(x, y, width, height)
}

pub(super) fn render_toast_notification(
    frame: &mut Frame,
    area: Rect,
    toast: &ToastNotification,
    offset_for_warning: bool,
    p: &Palette,
) {
    let dot_color = match toast.kind {
        ToastKind::NeedsAttention => p.red,
        ToastKind::Finished => p.blue,
        ToastKind::UpdateInstalled => p.accent,
    };
    let toast_area = toast_notification_rect(area, toast, offset_for_warning);

    frame.render_widget(Clear, toast_area);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(p.overlay0))
        .style(Style::default().bg(p.panel_bg));
    let inner = block.inner(toast_area);
    frame.render_widget(block, toast_area);

    if inner.height < 1 {
        return;
    }

    let [title_row, context_row] =
        Layout::vertical([Constraint::Length(1), Constraint::Length(1)]).areas(inner);

    let title = Line::from(vec![
        Span::styled("●", Style::default().fg(dot_color)),
        Span::raw(" "),
        Span::styled(
            &toast.title,
            Style::default().fg(p.text).add_modifier(Modifier::BOLD),
        ),
    ]);
    let context = Line::from(vec![
        Span::styled("  ", Style::default().fg(p.overlay0)),
        Span::styled(&toast.context, Style::default().fg(p.overlay0)),
    ]);

    frame.render_widget(Paragraph::new(title), title_row);
    if !toast.context.is_empty() && inner.height >= 2 {
        frame.render_widget(Paragraph::new(context), context_row);
    }
}

pub(super) fn render_copy_feedback(
    frame: &mut Frame,
    area: Rect,
    feedback: &CopyFeedback,
    offset_rows: u16,
    p: &Palette,
) {
    let feedback_area = copy_feedback_rect(area, feedback, offset_rows);
    if feedback_area.is_empty() {
        return;
    }

    frame.render_widget(Clear, feedback_area);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(p.green))
        .style(Style::default().bg(p.panel_bg));
    let inner = block.inner(feedback_area);
    frame.render_widget(block, feedback_area);

    if inner.height == 0 {
        return;
    }

    let text = Line::from(vec![
        Span::styled("●", Style::default().fg(p.green).bg(p.panel_bg)),
        Span::raw(" "),
        Span::styled(
            &feedback.message,
            Style::default()
                .fg(p.text)
                .bg(p.panel_bg)
                .add_modifier(Modifier::BOLD),
        ),
    ]);
    frame.render_widget(Paragraph::new(text), inner);
}

pub(crate) fn config_diagnostic_dismiss_rect(area: Rect, message: &str) -> Option<Rect> {
    if area.width == 0 || area.height == 0 || first_nonempty_line(message).is_none() {
        return None;
    }
    let hit_width = (CONFIG_DIAGNOSTIC_DISMISS_SUFFIX.width() as u16)
        .min(area.width)
        .max(1);
    Some(Rect::new(
        area.x + area.width.saturating_sub(hit_width),
        area.y,
        hit_width,
        1,
    ))
}

pub(super) fn render_config_diagnostic(frame: &mut Frame, area: Rect, message: &str, p: &Palette) {
    let style = Style::default()
        .fg(panel_contrast_fg(p))
        .bg(p.yellow)
        .add_modifier(Modifier::BOLD);

    for (row, line) in message
        .lines()
        .filter(|line| !line.trim().is_empty())
        .take(area.height as usize)
        .enumerate()
    {
        let text = if row == 0 {
            config_diagnostic_banner_text(line, area.width)
        } else {
            truncate_to_display_width(&format!(" config warning: {line} "), area.width as usize)
        };
        let width = (text.width() as u16).min(area.width);
        let notif_area = Rect::new(
            area.x + area.width.saturating_sub(width),
            area.y + row as u16,
            width,
            1,
        );

        frame.render_widget(Clear, notif_area);
        frame.render_widget(Paragraph::new(Span::styled(text, style)), notif_area);
    }
}

fn first_nonempty_line(message: &str) -> Option<&str> {
    message.lines().find(|line| !line.trim().is_empty())
}

fn config_diagnostic_banner_text(line: &str, max_width: u16) -> String {
    let suffix = CONFIG_DIAGNOSTIC_DISMISS_SUFFIX;
    let suffix_width = suffix.width();
    let max = max_width as usize;
    if max == 0 {
        return String::new();
    }
    if max <= suffix_width {
        return truncate_to_display_width(suffix, max);
    }
    format!(
        "{}{suffix}",
        truncate_to_display_width(&format!(" config warning: {line}"), max - suffix_width)
    )
}

fn truncate_to_display_width(text: &str, max_width: usize) -> String {
    if text.width() <= max_width {
        return text.to_string();
    }
    let mut out = String::new();
    let mut used = 0;
    for ch in text.chars() {
        let width = ch.width().unwrap_or(0);
        if used + width > max_width {
            break;
        }
        out.push(ch);
        used += width;
    }
    out
}

pub(super) fn state_dot(state: AgentState, seen: bool, p: &Palette) -> (&'static str, Style) {
    match (state, seen) {
        (AgentState::Blocked, _) => ("●", Style::default().fg(p.red)),
        (AgentState::Working, _) => ("●", Style::default().fg(p.yellow)),
        (AgentState::Idle, false) => ("●", Style::default().fg(p.teal)),
        (AgentState::Idle, true) => ("○", Style::default().fg(p.green)),
        (AgentState::Unknown, _) => ("·", Style::default().fg(p.overlay0)),
    }
}

pub(super) fn agent_icon(
    state: AgentState,
    seen: bool,
    tick: u32,
    p: &Palette,
) -> (&'static str, Style) {
    match (state, seen) {
        (AgentState::Blocked, _) => ("◉", Style::default().fg(p.red)),
        (AgentState::Working, _) => (super::spinner_frame(tick), Style::default().fg(p.yellow)),
        (AgentState::Idle, false) => ("●", Style::default().fg(p.teal)),
        (AgentState::Idle, true) => ("✓", Style::default().fg(p.green)),
        (AgentState::Unknown, _) => ("○", Style::default().fg(p.overlay0)),
    }
}

pub(super) fn state_label(state: AgentState, seen: bool) -> &'static str {
    match (state, seen) {
        (AgentState::Blocked, _) => "blocked",
        (AgentState::Working, _) => "working",
        (AgentState::Idle, false) => "done",
        (AgentState::Idle, true) => "idle",
        (AgentState::Unknown, _) => "idle",
    }
}

pub(super) fn state_label_color(state: AgentState, seen: bool, p: &Palette) -> Color {
    match (state, seen) {
        (AgentState::Blocked, _) => p.red,
        (AgentState::Working, _) => p.yellow,
        (AgentState::Idle, false) => p.teal,
        (AgentState::Idle, true) => p.green,
        (AgentState::Unknown, _) => p.overlay0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::state::Palette;
    use ratatui::{backend::TestBackend, Terminal};

    #[test]
    fn config_warning_renders_dismiss_cross_at_the_far_right() {
        let palette = Palette::catppuccin();
        let area = Rect::new(0, 0, 80, 5);
        let message = "This workspace is not a Herdr-managed worktree checkout.";
        let backend = TestBackend::new(80, 5);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|frame| {
                render_config_diagnostic(frame, area, message, &palette);
            })
            .unwrap();

        let buffer = terminal.backend().buffer();
        let cross_col = (0..area.width)
            .rev()
            .find(|x| buffer[(*x, 0)].symbol() == "✕")
            .expect("dismiss cross should render");
        let last_content_col = (0..area.width)
            .rev()
            .find(|x| !buffer[(*x, 0)].symbol().trim().is_empty())
            .expect("warning should render");

        assert_eq!(
            cross_col, last_content_col,
            "dismiss cross should be the last glyph on the warning"
        );
        assert!(
            cross_col + 2 >= area.x + area.width,
            "dismiss cross at column {cross_col} should sit at the far right of the {area:?} warning"
        );
        let dismiss = config_diagnostic_dismiss_rect(area, message).expect("dismiss rect");
        assert!(
            cross_col >= dismiss.x && cross_col < dismiss.x + dismiss.width,
            "dismiss rect {dismiss:?} should cover rendered cross at column {cross_col}"
        );
    }

    #[test]
    fn config_warning_keeps_dismiss_cross_when_the_message_is_wider_than_the_area() {
        let palette = Palette::catppuccin();
        let area = Rect::new(0, 0, 24, 1);
        let message = "This workspace is not a Herdr-managed worktree checkout.";
        let backend = TestBackend::new(24, 1);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|frame| {
                render_config_diagnostic(frame, area, message, &palette);
            })
            .unwrap();

        let buffer = terminal.backend().buffer();
        let cross_col = (0..area.width)
            .rev()
            .find(|x| buffer[(*x, 0)].symbol() == "✕")
            .expect("dismiss cross should remain visible");
        assert_eq!(cross_col + 2, area.width);
    }
}
