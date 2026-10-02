//! Drawing the Commander, and the star it sends.
//!
//! The box sits at the bottom of the frame, half the frame wide and centred,
//! one row up from the bottom edge. It holds the typed line, wrapped at its
//! width and growing a row for each wrap up to a third of the frame, and under
//! that one line saying what `Enter` would do. The star is drawn last, over
//! everything else in the frame, because it only recolours what is already
//! there.

use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use super::widgets::{panel_contrast_fg, render_panel_shell};
use crate::app::{AppState, Mode};
use crate::composer::field::Row;

const TITLE: &str = " Commander ";
/// Below this the box is not much use, so it takes the whole frame width.
const MIN_WIDTH: u16 = 40;
/// Columns between the border and the text on each side.
const PAD: u16 = 1;
/// The border above the text, and the reading line and border below it.
const CHROME_ROWS: u16 = 3;
const PLACEHOLDER: &str = "switch to herdr · tell Ada to run the tests · split right";
const HINT: &str = "enter to run · shift+enter for a new line · esc to close";

/// Width of the text inside a box `width` wide.
fn text_width(width: u16) -> u16 {
    width.saturating_sub(2 + 2 * PAD)
}

/// Lay the box out in `area`, the whole frame. The field is told its width
/// here, since how many rows it wraps to decides how tall the box is.
pub(crate) fn compute_commander(app: &mut AppState, area: Rect) {
    app.commander.frame = area;
    if app.mode != Mode::Commander {
        return;
    }
    let width = (area.width / 2).max(MIN_WIDTH).min(area.width);
    app.commander
        .field
        .set_width(text_width(width).max(1) as usize);
    let max_rows = (area.height / 3).max(1);
    let rows = (app.commander.field.rows().len() as u16).clamp(1, max_rows);
    let height = (rows + CHROME_ROWS).min(area.height);
    let y = area
        .bottom()
        .saturating_sub(height)
        .saturating_sub(1)
        .max(area.y);
    let x = area.x + (area.width - width) / 2;
    app.commander.area = Rect::new(x, y, width, height);
}

pub(super) fn render_commander(app: &AppState, frame: &mut Frame) {
    let rect = app.commander.area;
    let p = &app.palette;
    let Some(inner) = render_panel_shell(frame, rect, p.accent, p.panel_bg) else {
        return;
    };
    if inner.height < 2 || inner.width <= 2 * PAD {
        return;
    }

    let title_width = (TITLE.len() as u16).min(rect.width.saturating_sub(4));
    frame.render_widget(
        Paragraph::new(Span::styled(
            TITLE,
            Style::default().fg(p.accent).add_modifier(Modifier::BOLD),
        )),
        Rect::new(rect.x + 2, rect.y, title_width, 1),
    );

    let text = Rect::new(
        inner.x + PAD,
        inner.y,
        inner.width - 2 * PAD,
        inner.height - 1,
    );
    let field = &app.commander.field;
    let rows = field.rows();
    let (cursor_row, cursor_col) = field.cursor_row();
    // When the text has outgrown the box, the rows shown follow the cursor.
    let visible = text.height as usize;
    let first = (cursor_row + 1).saturating_sub(visible);

    if field.is_empty() {
        frame.render_widget(
            Paragraph::new(Span::styled(PLACEHOLDER, Style::default().fg(p.overlay0))),
            Rect::new(text.x, text.y, text.width, 1),
        );
    } else {
        for (offset, row) in rows.iter().skip(first).take(visible).enumerate() {
            frame.render_widget(
                Paragraph::new(Line::from(row_spans(app, row))),
                Rect::new(text.x, text.y + offset as u16, text.width, 1),
            );
        }
    }

    let reading = match &app.commander.reading {
        Some(Ok(sentence)) => Line::from(vec![
            Span::styled("→ ", Style::default().fg(p.accent)),
            Span::styled(sentence.clone(), Style::default().fg(p.text)),
        ]),
        Some(Err(reason)) => Line::from(Span::styled(reason.clone(), Style::default().fg(p.red))),
        None => Line::from(Span::styled(HINT, Style::default().fg(p.overlay0))),
    };
    frame.render_widget(
        Paragraph::new(reading),
        Rect::new(text.x, inner.bottom() - 1, text.width, 1),
    );

    let row = (cursor_row - first).min(visible.saturating_sub(1)) as u16;
    frame.set_cursor_position((
        text.x + (cursor_col as u16).min(text.width.saturating_sub(1)),
        text.y + row,
    ));
}

/// One row of the field, with any selected stretch drawn in the accent.
fn row_spans(app: &AppState, row: &Row) -> Vec<Span<'static>> {
    let p = &app.palette;
    let plain = Style::default().fg(p.text);
    let Some((start, end)) = app.commander.field.selection_on_row(row) else {
        return vec![Span::styled(row.text.clone(), plain)];
    };
    let chars: Vec<char> = row.text.chars().collect();
    let (start, end) = (start.min(chars.len()), end.min(chars.len()));
    vec![
        Span::styled(chars[..start].iter().collect::<String>(), plain),
        Span::styled(
            chars[start..end].iter().collect::<String>(),
            Style::default().fg(panel_contrast_fg(p)).bg(p.accent),
        ),
        Span::styled(chars[end..].iter().collect::<String>(), plain),
    ]
}

/// The star and its light, over everything else in the frame.
pub(super) fn render_commander_trail(app: &AppState, frame: &mut Frame) {
    if let Some(trail) = &app.commander.trail {
        let area = frame.area();
        trail.render(area, frame.buffer_mut());
    }
}
