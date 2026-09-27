//! Drawing the space dialog and the purpose fly-out.
//!
//! The dialog asks what a new space is for and where it starts. The purpose is
//! a text area of a few rows. The directory is one filled row drawn the way the
//! launcher draws its directory control, and its list opens under that row one
//! folder per line, the same way. Editing a space's purpose later draws the
//! same dialog without the directory row, because a space's directory is where
//! its panes already run.
//!
//! The fly-out is the purpose read back: a small panel beside the sidebar,
//! level with the card the pointer rests on.

use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Clear, Paragraph},
    Frame,
};

use super::composer::{
    directory_style, draw_caption, elide, item_style, paint_fill, CLOSED, FOLDER_CAPTION, OPENED,
    USED_MARK,
};
use super::widgets::{
    action_button_row_rects, centered_popup_rect, panel_contrast_fg, render_action_button,
    render_modal_header, render_panel_shell, ActionButtonSpec,
};
use crate::app::state::{SpaceDialogField, SpaceDialogState};
use crate::app::{AppState, Mode};
use crate::composer::Focus;

const DIALOG_WIDTH: u16 = 64;
/// How many rows the purpose area shows. A longer purpose scrolls with the
/// cursor.
const PURPOSE_ROWS: u16 = 4;
/// The most folders the open list shows at once.
const LIST_ROWS: u16 = 8;
const PURPOSE_CAPTION: &str = "Purpose";
const PURPOSE_PLACEHOLDER: &str = "what is this space for?";
const FLYOUT_WIDTH: u16 = 44;

/// Where everything in the dialog sits. The drawing and the mouse both read
/// this, so a click lands on what was drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SpaceDialogLayout {
    pub inner: Rect,
    pub header: Rect,
    /// The purpose text area, including its one-column margins.
    pub purpose: Rect,
    /// The directory row. Absent when editing an existing space.
    pub directory: Option<Rect>,
    /// The rows of the open folder list, hung under the directory row.
    pub list: Option<Rect>,
    pub message: Rect,
    pub create: Rect,
    pub cancel: Rect,
}

fn submit_label(dialog: &SpaceDialogState) -> &'static str {
    if dialog.editing.is_some() {
        "save"
    } else {
        "create"
    }
}

pub(crate) fn space_dialog_layout(
    dialog: &SpaceDialogState,
    area: Rect,
) -> Option<SpaceDialogLayout> {
    // Header, gap, caption, purpose rows, gap, then either the directory row
    // and a message row, or just the message row, then the buttons.
    let body = 1 + 1 + 1 + PURPOSE_ROWS + 1 + if dialog.folders.is_some() { 2 } else { 1 } + 1;
    let popup = centered_popup_rect(area, DIALOG_WIDTH, body + 2)?;
    let inner = Rect::new(
        popup.x + 1,
        popup.y + 1,
        popup.width.saturating_sub(2),
        popup.height.saturating_sub(2),
    );
    if inner.height < body || inner.width < 20 {
        return None;
    }
    let row = |offset: u16| Rect::new(inner.x, inner.y + offset, inner.width, 1);
    let header = row(0);
    let purpose = Rect::new(inner.x + 1, inner.y + 3, inner.width - 2, PURPOSE_ROWS);
    let mut next = 3 + PURPOSE_ROWS + 1;
    let directory = dialog.folders.as_ref().map(|_| {
        let rect = Rect::new(inner.x + 1, inner.y + next, inner.width - 2, 1);
        next += 1;
        rect
    });
    let message = row(next);
    let buttons = action_button_row_rects(
        inner,
        &[
            ActionButtonSpec {
                hint: Some("↵"),
                label: submit_label(dialog),
            },
            ActionButtonSpec {
                hint: Some("esc"),
                label: "cancel",
            },
        ],
        2,
        inner.height.saturating_sub(1),
    );
    let list = match (directory, dialog.folders.as_ref()) {
        (Some(directory), Some(folders)) if folders.open == Some(Focus::Folder) => {
            let wanted = (folders.folder_visible_rows() as u16).min(LIST_ROWS);
            let below = area.bottom().saturating_sub(directory.bottom());
            let rows = wanted.min(below);
            (rows > 0).then(|| Rect::new(directory.x, directory.bottom(), directory.width, rows))
        }
        _ => None,
    };
    Some(SpaceDialogLayout {
        inner,
        header,
        purpose,
        directory,
        list,
        message,
        create: buttons[0],
        cancel: buttons[1],
    })
}

/// The width the purpose wraps at, which is the width of its text area.
pub(crate) fn space_dialog_purpose_width(dialog: &SpaceDialogState, area: Rect) -> usize {
    space_dialog_layout(dialog, area)
        .map(|layout| layout.purpose.width as usize)
        .unwrap_or(0)
}

/// The first row of the folder list on show: the list scrolls so the row
/// pointed at stays inside it.
pub(crate) fn space_dialog_list_first(dialog: &SpaceDialogState, rows: u16) -> usize {
    let Some(folders) = dialog.folders.as_ref() else {
        return 0;
    };
    let count = folders.folder_rows().len();
    let rows = rows as usize;
    folders
        .highlight
        .saturating_sub(rows.saturating_sub(1))
        .min(count.saturating_sub(rows))
}

/// The first purpose row on show: the area scrolls so the cursor stays in it.
pub(crate) fn space_dialog_purpose_first(dialog: &SpaceDialogState) -> usize {
    let (row, _) = dialog.purpose.cursor_row();
    row.saturating_sub(PURPOSE_ROWS as usize - 1)
}

pub(super) fn render_space_dialog(app: &AppState, frame: &mut Frame, area: Rect) {
    let Some(dialog) = app.space_dialog.as_ref() else {
        return;
    };
    let Some(layout) = space_dialog_layout(dialog, area) else {
        return;
    };
    let p = &app.palette;
    super::dim_background(frame, area);
    let popup = Rect::new(
        layout.inner.x - 1,
        layout.inner.y - 1,
        layout.inner.width + 2,
        layout.inner.height + 2,
    );
    if render_panel_shell(frame, popup, p.accent, p.panel_bg).is_none() {
        return;
    }

    let title = if dialog.editing.is_some() {
        " space purpose"
    } else {
        " new space"
    };
    render_modal_header(frame, layout.header, title, p);

    let purpose_focused = dialog.field == SpaceDialogField::Purpose;
    let caption_colour = if purpose_focused {
        p.accent
    } else {
        p.overlay0
    };
    frame.render_widget(
        Paragraph::new(Line::styled(
            PURPOSE_CAPTION,
            Style::default()
                .fg(caption_colour)
                .add_modifier(Modifier::ITALIC),
        )),
        Rect::new(
            layout.purpose.x,
            layout.purpose.y - 1,
            layout.purpose.width,
            1,
        ),
    );
    draw_purpose(app, frame, dialog, layout.purpose);

    if let (Some(rect), Some(folders)) = (layout.directory, dialog.folders.as_ref()) {
        let focused = dialog.field == SpaceDialogField::Directory;
        let open = folders.open == Some(Focus::Folder);
        let fill = p.surface0;
        paint_fill(frame.buffer_mut(), rect, fill);
        draw_caption(app, frame, rect, FOLDER_CAPTION, focused, fill);
        let marker_style = if focused {
            Style::default().fg(p.accent)
        } else {
            Style::default().fg(p.overlay0)
        };
        frame.render_widget(
            Paragraph::new(Line::styled(
                if open { OPENED } else { CLOSED },
                marker_style,
            )),
            Rect::new(rect.right().saturating_sub(2), rect.y, 1, 1),
        );
        let value = value_rect(rect);
        if open {
            let (visible, _) = typed_path(dialog, value.width);
            let ghost = folders.ghost_suffix().unwrap_or_default();
            let mut spans = vec![Span::styled(visible, Style::default().fg(p.text))];
            if !ghost.is_empty() {
                spans.push(Span::styled(ghost, Style::default().fg(p.overlay0)));
            }
            frame.render_widget(Paragraph::new(Line::from(spans)), value);
        } else {
            let (text, style) = match folders.folder {
                Some(index) => (
                    elide(
                        folders.folder_label().unwrap_or_default(),
                        value.width as usize,
                    ),
                    directory_style(app, index),
                ),
                None => (
                    "choose a directory…".to_string(),
                    Style::default().fg(p.overlay0),
                ),
            };
            frame.render_widget(Paragraph::new(Line::styled(text, style)), value);
        }
    }

    if let Some(error) = &dialog.error {
        frame.render_widget(
            Paragraph::new(format!(" {error}")).style(Style::default().fg(p.red)),
            layout.message,
        );
    }

    render_action_button(
        frame,
        layout.create,
        Some("↵"),
        submit_label(dialog),
        Style::default()
            .fg(panel_contrast_fg(p))
            .bg(p.accent)
            .add_modifier(Modifier::BOLD),
    );
    render_action_button(
        frame,
        layout.cancel,
        Some("esc"),
        "cancel",
        Style::default()
            .fg(p.text)
            .bg(p.surface0)
            .add_modifier(Modifier::BOLD),
    );

    // The list goes on last: it hangs over the rows under the directory.
    if let (Some(list), Some(directory)) = (layout.list, layout.directory) {
        draw_list(app, frame, dialog, list, directory);
    }

    place_cursor(frame, dialog, &layout);
}

fn draw_purpose(app: &AppState, frame: &mut Frame, dialog: &SpaceDialogState, rect: Rect) {
    let p = &app.palette;
    paint_fill(frame.buffer_mut(), rect, p.surface0);
    if dialog.purpose.is_empty() {
        frame.render_widget(
            Paragraph::new(Line::styled(
                PURPOSE_PLACEHOLDER,
                Style::default().fg(p.overlay0).bg(p.surface0),
            )),
            Rect::new(rect.x, rect.y, rect.width, 1),
        );
        return;
    }
    let first = space_dialog_purpose_first(dialog);
    let text = Style::default().fg(p.text).bg(p.surface0);
    let selected = Style::default().fg(panel_contrast_fg(p)).bg(p.accent);
    for (offset, row) in dialog
        .purpose
        .rows()
        .iter()
        .skip(first)
        .take(rect.height as usize)
        .enumerate()
    {
        let chars: Vec<char> = row.text.chars().collect();
        let spans = match dialog.purpose.selection_on_row(row) {
            Some((start, end)) => vec![
                Span::styled(chars[..start].iter().collect::<String>(), text),
                Span::styled(chars[start..end].iter().collect::<String>(), selected),
                Span::styled(chars[end..].iter().collect::<String>(), text),
            ],
            None => vec![Span::styled(row.text.clone(), text)],
        };
        frame.render_widget(
            Paragraph::new(Line::from(spans)),
            Rect::new(rect.x, rect.y + offset as u16, rect.width, 1),
        );
    }
}

fn draw_list(
    app: &AppState,
    frame: &mut Frame,
    dialog: &SpaceDialogState,
    list: Rect,
    directory: Rect,
) {
    let Some(folders) = dialog.folders.as_ref() else {
        return;
    };
    let p = &app.palette;
    frame.render_widget(Clear, list);
    paint_fill(frame.buffer_mut(), list, p.surface0);
    let first = space_dialog_list_first(dialog, list.height);
    let value = value_rect(directory);
    for offset in 0..list.height {
        let index = first + offset as usize;
        let Some(folder) = folders.folder_rows().get(index) else {
            break;
        };
        let y = list.y + offset;
        frame.render_widget(
            Paragraph::new(Line::styled(
                elide(&folder.label, value.width as usize),
                item_style(
                    app,
                    folders.pointing() && index == folders.highlight,
                    folders.hover == Some(index),
                ),
            )),
            Rect::new(value.x, y, value.width, 1),
        );
        if folders.is_used(&folder.path) {
            frame.render_widget(
                Paragraph::new(Line::styled(USED_MARK, Style::default().fg(p.accent))),
                Rect::new(value.x.saturating_sub(2), y, 1, 1),
            );
        }
    }
}

/// Where a directory row's value is written: clear of the caption in front
/// and the marker behind.
fn value_rect(row: Rect) -> Rect {
    let indent = FOLDER_CAPTION.chars().count() as u16 + 3;
    Rect::new(
        row.x + indent,
        row.y,
        row.width.saturating_sub(indent + 3),
        1,
    )
}

/// The stretch of the path being typed that fits, and the cursor's column in
/// it. It follows the cursor once the path is wider than the row.
fn typed_path(dialog: &SpaceDialogState, room: u16) -> (String, usize) {
    let Some(folders) = dialog.folders.as_ref() else {
        return (String::new(), 0);
    };
    let text: Vec<char> = folders.path().text().chars().collect();
    let (_, cursor) = folders.path().cursor_row();
    let room = room as usize;
    if room == 0 {
        return (String::new(), 0);
    }
    let start = (cursor + 1).saturating_sub(room);
    let end = (start + room).min(text.len());
    (text[start..end].iter().collect(), cursor - start)
}

fn place_cursor(frame: &mut Frame, dialog: &SpaceDialogState, layout: &SpaceDialogLayout) {
    match dialog.field {
        SpaceDialogField::Purpose => {
            let (row, col) = dialog.purpose.cursor_row();
            let row = row - space_dialog_purpose_first(dialog);
            frame.set_cursor_position((
                layout.purpose.x + (col as u16).min(layout.purpose.width.saturating_sub(1)),
                layout.purpose.y + row as u16,
            ));
        }
        SpaceDialogField::Directory => {
            let Some(directory) = layout.directory else {
                return;
            };
            if dialog
                .folders
                .as_ref()
                .is_some_and(|folders| folders.open == Some(Focus::Folder))
            {
                let value = value_rect(directory);
                let (_, col) = typed_path(dialog, value.width);
                frame.set_cursor_position((value.x + col as u16, value.y));
            }
        }
    }
}

/// Where the purpose fly-out goes for the hovered space: beside the sidebar,
/// level with the space's card, and pulled up when it would run off the
/// bottom. `None` when there is nothing to show.
fn space_purpose_flyout(app: &AppState, area: Rect) -> Option<(Rect, Vec<String>)> {
    if !matches!(app.mode, Mode::Terminal | Mode::Navigate | Mode::Composer)
        || app.sidebar_collapsed
    {
        return None;
    }
    let ws_idx = app.hovered_space?;
    let purpose = app.workspaces.get(ws_idx)?.purpose.as_deref()?;
    let card = app
        .view
        .workspace_card_areas
        .iter()
        .find(|card| card.ws_idx == ws_idx)?;
    let x = app.view.sidebar_rect.right();
    let room = area.right().saturating_sub(x).saturating_sub(1);
    let width = FLYOUT_WIDTH.min(room);
    if width < 12 {
        return None;
    }
    let text_width = width.saturating_sub(4) as usize;
    let lines = wrap_words(purpose, text_width);
    let height = (lines.len() as u16 + 2).min(area.height);
    let y = card.rect.y.min(area.bottom().saturating_sub(height));
    Some((Rect::new(x, y, width, height), lines))
}

pub(super) fn render_space_purpose_flyout(app: &AppState, frame: &mut Frame, area: Rect) {
    let Some((rect, lines)) = space_purpose_flyout(app, area) else {
        return;
    };
    let p = &app.palette;
    let Some(inner) = render_panel_shell(frame, rect, p.accent, p.panel_bg) else {
        return;
    };
    for (offset, line) in lines.iter().take(inner.height as usize).enumerate() {
        frame.render_widget(
            Paragraph::new(Line::styled(line.clone(), Style::default().fg(p.text))),
            Rect::new(inner.x + 1, inner.y + offset as u16, inner.width - 1, 1),
        );
    }
}

/// Break text into lines no wider than `width`, at spaces where it can and
/// mid-word where a word alone is wider. Line breaks in the text are kept.
pub(crate) fn wrap_words(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut lines = Vec::new();
    for paragraph in text.split('\n') {
        let mut line = String::new();
        for word in paragraph.split_whitespace() {
            let mut word: Vec<char> = word.chars().collect();
            let line_len = line.chars().count();
            if line_len > 0 && line_len + 1 + word.len() <= width {
                line.push(' ');
                line.extend(word.iter());
                continue;
            }
            if line_len > 0 {
                lines.push(std::mem::take(&mut line));
            }
            while word.len() > width {
                lines.push(word.drain(..width).collect());
            }
            line.extend(word.iter());
        }
        lines.push(line);
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrap_words_breaks_at_spaces() {
        assert_eq!(
            wrap_words("ship the chat ui redesign", 12),
            vec!["ship the", "chat ui", "redesign"]
        );
    }

    #[test]
    fn wrap_words_splits_a_word_wider_than_the_line() {
        assert_eq!(wrap_words("abcdefgh", 3), vec!["abc", "def", "gh"]);
    }

    #[test]
    fn wrap_words_keeps_line_breaks() {
        assert_eq!(wrap_words("one\ntwo", 20), vec!["one", "two"]);
    }
}
