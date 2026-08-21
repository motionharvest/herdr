//! Drawing the composer band.
//!
//! Four controls, read left to right: where to work, whether in a worktree,
//! who works, what to do. Directory, Agent, and Task are rounded boxes with
//! their captions above them; Worktree is a checkbox in the same style,
//! between Directory and Agent. A dropdown opens inside its own box: the
//! value row, a rule, then one row per item. The band's chrome is a fixed
//! four rows; an open dropdown or a wrapped task grows its box downward over
//! the panes rather than pushing them down, because a band that changed
//! height would resize every pane under it, and resizing a pane resizes the
//! agent's terminal.

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Clear, Paragraph},
    Frame,
};

use crate::app::{AppState, Mode};
use crate::composer::Focus;

use super::widgets::panel_contrast_fg;

const CLOSED: &str = "▾";
const OPENED: &str = "▴";
const FOLDER_CAPTION: &str = "Directory";
const AGENT_CAPTION: &str = "Agent";
const TASK_CAPTION: &str = "Task";
const WORKTREE_CAPTION: &str = "Worktree";
const WORKTREE_MARK: &str = "✓";
/// The checkbox itself: `╭───╮` / `│ ✓ │` / `╰───╯`.
const WORKTREE_BOX_WIDTH: u16 = 5;
/// What the folder control says when there is nowhere to work yet.
const PLACEHOLDER: &str = "add a directory…";
/// What the agent control says when nothing this band can start is installed.
const NO_AGENT: &str = "none installed";
/// The band's chrome: a caption row and a closed box under it.
const BAND_HEIGHT: u16 = 4;
/// Below this much room the band would crowd out the panes, so it is dropped.
const MIN_ROWS_BELOW: u16 = 3;
/// Border, then two spaces, and a field's text begins.
const PROMPT_INSET: u16 = 3;
/// The gap between two controls.
const GAP: u16 = 2;
/// Room enough to read a line of a task in. Below this the task keeps its
/// minimum and the folder control gives up its width instead.
const MIN_TASK_TEXT: u16 = 20;
/// Room enough to type a path into, whatever the folders listed need.
const MIN_FOLDER_TEXT: u16 = 24;

/// Where everything in the band sits. Worked out once, so that a click cannot
/// land somewhere the drawing did not put anything.
#[derive(Debug, Clone, Default)]
pub(crate) struct ComposerLayout {
    /// The whole band, open boxes included, which is what a click is tested
    /// against. Taller than [`BAND_HEIGHT`] whenever a box hangs over the panes.
    pub area: Rect,
    pub folder: Rect,
    /// The Worktree checkbox between Directory and Agent: caption and box.
    pub worktree: Rect,
    pub agent: Rect,
    pub task: Rect,
    /// The columns between the left of the task box and its text — the border,
    /// its inset, and the prefix. The drawing indents by it and a click
    /// subtracts it, and they only land on the same character if one place
    /// decides it.
    pub task_lead: u16,
    /// The row all four boxes put their value on.
    pub value_row: u16,
    /// The row the captions stand on, just above the boxes.
    pub caption_row: u16,
    /// The item rows of the open dropdown, inside its box's borders.
    pub dropdown: Rect,
    /// The screen row of each of those items, in list order.
    pub dropdown_rows: Vec<u16>,
}

/// Carve the band off the top of the frame, and work out where its controls go.
///
/// Returns the rows the band owns and what is left for the sidebar, tabs, and
/// panes. What is left is always the frame less the band's four chrome rows: a
/// task that wraps or a dropdown that opens grows its box downward over the
/// panes rather than pushing them down, so typing never resizes an agent's
/// terminal. Settling the task field's width is part of this, because the
/// wrapping, the height of the box, and where the cursor is drawn all have to
/// agree, and they only agree if one place decides it.
pub(crate) fn split_composer(app: &mut AppState, area: Rect) -> (ComposerLayout, Rect) {
    if area.width == 0 || area.height < BAND_HEIGHT + MIN_ROWS_BELOW {
        app.composer.task.set_width(0);
        return (ComposerLayout::default(), area);
    }

    let lead = lead_width(app.composer.task_prefix());
    let least_task = PROMPT_INSET + lead + 2 + MIN_TASK_TEXT;
    let worktree_width = worktree_control_width();
    let agent_width =
        agent_control_width(app).min(area.width.saturating_sub(2 + 3 * GAP + worktree_width));
    let budget = area
        .width
        .saturating_sub(2 + 3 * GAP + agent_width + worktree_width);
    let folder_width = folder_control_width(app, budget.saturating_sub(least_task));
    let task_x = 1 + folder_width + GAP + worktree_width + GAP + agent_width + GAP;
    let task_width = budget - folder_width;

    app.composer
        .task
        .set_width(task_width.saturating_sub(PROMPT_INSET + lead + 2) as usize);

    let caption_row = area.y;
    let top = area.y + 1;
    let value_row = top + 1;

    let task_rows = (app.composer.task.rows().len() as u16).clamp(
        1,
        area.height
            .saturating_sub(BAND_HEIGHT + MIN_ROWS_BELOW)
            .max(1),
    );

    let most_items = area.height.saturating_sub(5) as usize;
    let listing = |open: bool, count: usize| -> u16 {
        if open && count > 0 {
            4 + count.min(most_items) as u16
        } else {
            3
        }
    };
    let folder = Rect::new(
        area.x + 1,
        top,
        folder_width,
        listing(
            app.composer.open == Some(Focus::Folder),
            app.composer.folder_rows().len(),
        ),
    );
    let worktree = Rect::new(
        folder.x + folder.width + GAP,
        caption_row,
        worktree_width,
        BAND_HEIGHT,
    );
    let agent = Rect::new(
        worktree.x + worktree.width + GAP,
        top,
        agent_width,
        listing(
            app.composer.open == Some(Focus::Agent),
            app.composer.harnesses().len(),
        ),
    );
    let task = Rect::new(area.x + task_x, top, task_width, 2 + task_rows);

    let (dropdown, dropdown_rows) = match app.composer.open {
        Some(Focus::Folder) if folder.height > 3 => item_rows(folder, value_row),
        Some(Focus::Agent) if agent.height > 3 => item_rows(agent, value_row),
        _ => (Rect::default(), Vec::new()),
    };

    let tallest = folder.height.max(agent.height).max(task.height);
    let layout = ComposerLayout {
        area: Rect::new(area.x, area.y, area.width, 1 + tallest),
        folder,
        worktree,
        agent,
        task,
        task_lead: PROMPT_INSET + lead,
        value_row,
        caption_row,
        dropdown,
        dropdown_rows,
    };

    let rest = Rect::new(
        area.x,
        area.y + BAND_HEIGHT,
        area.width,
        area.height - BAND_HEIGHT,
    );
    (layout, rest)
}

/// The item rows of an open box: everything under its rule, inside its borders.
fn item_rows(open_box: Rect, value_row: u16) -> (Rect, Vec<u16>) {
    let count = open_box.height - 4;
    let top = value_row + 2;
    let area = Rect::new(open_box.x + 1, top, open_box.width.saturating_sub(2), count);
    (area, (0..count).map(|row| top + row).collect())
}

/// The agent box is as wide as its widest row wants, so a name is never cut —
/// the names are all short.
fn agent_control_width(app: &AppState) -> u16 {
    let widest = app
        .composer
        .harnesses()
        .iter()
        .map(|harness| harness.name.chars().count())
        .max()
        .unwrap_or(NO_AGENT.chars().count());
    widest as u16 + 6
}

/// The folder box is as wide as its widest row wants to be, so a path is
/// shortened only when the frame cannot hold it. Its ceiling is whatever
/// leaves the task its own minimum, so on a narrow frame the folder is the one
/// that gives up its width. A path being typed is what the box has to hold, so
/// it grows with what is typed rather than cutting it off — the +1 is the
/// cursor's own column, which has to be inside the box to be seen.
fn folder_control_width(app: &AppState, cap: u16) -> u16 {
    let mut wanted = app
        .composer
        .folder_row_width()
        .max(PLACEHOLDER.chars().count())
        .max(MIN_FOLDER_TEXT as usize) as u16;
    if app.composer.open == Some(Focus::Folder) {
        let typed = app.composer.path().text().chars().count() as u16 + 1;
        wanted = wanted.max(typed);
    }
    (wanted + 5).min(cap)
}

/// The Worktree control is as wide as its caption; the checkbox under it is
/// narrower and left-aligned, so the word is what spaces it from Agent.
fn worktree_control_width() -> u16 {
    WORKTREE_CAPTION.chars().count() as u16
}

/// What stands in front of the typed text after the box's inset: the prefix,
/// where there is one, and the space after it.
fn lead_width(prefix: &str) -> u16 {
    if prefix.is_empty() {
        0
    } else {
        prefix.chars().count() as u16 + 1
    }
}

pub(super) fn render_composer(app: &AppState, frame: &mut Frame, layout: &ComposerLayout) {
    if layout.area.width == 0 || layout.area.height == 0 {
        return;
    }
    draw_controls(app, frame, layout);
}

/// The parts of the band that hang over the panes: an open dropdown, or the
/// rows of a task past its first. The boxes are drawn again after the panes
/// have gone down, because whatever is drawn last is what is seen.
pub(super) fn render_composer_dropdown(app: &AppState, frame: &mut Frame, layout: &ComposerLayout) {
    if layout.area.width == 0 || layout.area.height <= BAND_HEIGHT {
        return;
    }
    draw_controls(app, frame, layout);
}

fn draw_controls(app: &AppState, frame: &mut Frame, layout: &ComposerLayout) {
    let in_band = app.mode == Mode::Composer;
    draw_folder(app, frame, layout, in_band);
    draw_worktree(app, frame, layout, in_band);
    draw_agent(app, frame, layout, in_band);
    draw_task(app, frame, layout, in_band);
    place_cursor(app, frame, layout, in_band);
}

fn draw_worktree(app: &AppState, frame: &mut Frame, layout: &ComposerLayout, in_band: bool) {
    let rect = layout.worktree;
    if rect.width == 0 {
        return;
    }
    let box_rect = Rect {
        x: rect.x,
        y: layout.value_row.saturating_sub(1),
        width: WORKTREE_BOX_WIDTH.min(rect.width),
        height: 3,
    };
    frame.render_widget(
        caption(app, WORKTREE_CAPTION, false),
        Rect {
            x: rect.x,
            y: layout.caption_row,
            width: rect.width,
            height: 1,
        },
    );
    frame.render_widget(Clear, box_rect);
    frame.render_widget(rounded(app, false, in_band), box_rect);
    if app.composer.worktree {
        frame.render_widget(
            Paragraph::new(Line::styled(WORKTREE_MARK, value_style(app, in_band))),
            Rect {
                x: box_rect.x + 2,
                y: layout.value_row,
                width: 1,
                height: 1,
            },
        );
    }
}

fn draw_folder(app: &AppState, frame: &mut Frame, layout: &ComposerLayout, in_band: bool) {
    let rect = layout.folder;
    if rect.width < 5 {
        return;
    }
    let focused = in_band && app.composer.focus == Focus::Folder;
    let open = app.composer.open == Some(Focus::Folder);
    frame.render_widget(Clear, rect);
    frame.render_widget(rounded(app, focused, in_band), rect);
    frame.render_widget(
        marker(app, open, focused, in_band),
        marker_area(rect, layout.value_row),
    );
    frame.render_widget(
        caption(app, FOLDER_CAPTION, focused),
        inner(rect, layout.caption_row, 2),
    );

    if open {
        let (visible, _) = typing(app, rect.width);
        frame.render_widget(
            Paragraph::new(Line::styled(visible, value_style(app, in_band))),
            inner(rect, layout.value_row, PROMPT_INSET),
        );
        if layout.dropdown_rows.is_empty() {
            return;
        }
        rule(
            frame.buffer_mut(),
            rect,
            layout.value_row + 1,
            border_style(app, focused, in_band),
        );
        draw_items(app, frame, layout, rect, Focus::Folder);
        return;
    }

    let (text, style) = match app.composer.folder {
        Some(index) => (
            elide(
                app.composer.folder_label().unwrap_or_default(),
                rect.width.saturating_sub(5) as usize,
            ),
            directory_style(app, index),
        ),
        None => (
            PLACEHOLDER.to_string(),
            Style::default().fg(app.palette.overlay0),
        ),
    };
    frame.render_widget(
        Paragraph::new(Line::styled(text, style)),
        inner(rect, layout.value_row, 2),
    );
}

fn draw_agent(app: &AppState, frame: &mut Frame, layout: &ComposerLayout, in_band: bool) {
    let rect = layout.agent;
    if rect.width < 5 {
        return;
    }
    let focused = in_band && app.composer.focus == Focus::Agent;
    let open = app.composer.open == Some(Focus::Agent);
    frame.render_widget(Clear, rect);
    frame.render_widget(rounded(app, focused, in_band), rect);
    frame.render_widget(
        marker(app, open, focused, in_band),
        marker_area(rect, layout.value_row),
    );
    frame.render_widget(
        caption(app, AGENT_CAPTION, focused),
        inner(rect, layout.caption_row, 2),
    );

    let showing = if open {
        app.composer
            .harnesses()
            .get(app.composer.highlight)
            .copied()
    } else {
        app.composer.harness()
    };
    let (text, style) = match showing {
        Some(harness) => (harness.name.to_string(), value_style(app, in_band)),
        None => (
            NO_AGENT.to_string(),
            Style::default().fg(app.palette.overlay0),
        ),
    };
    frame.render_widget(
        Paragraph::new(Line::styled(text, style)),
        inner(rect, layout.value_row, 2),
    );

    if !open || layout.dropdown_rows.is_empty() {
        return;
    }
    rule(
        frame.buffer_mut(),
        rect,
        layout.value_row + 1,
        border_style(app, focused, in_band),
    );
    draw_items(app, frame, layout, rect, Focus::Agent);
}

fn draw_task(app: &AppState, frame: &mut Frame, layout: &ComposerLayout, in_band: bool) {
    let rect = layout.task;
    if rect.width < 5 {
        return;
    }
    let focused = in_band && app.composer.focus == Focus::Task;
    frame.render_widget(Clear, rect);
    frame.render_widget(rounded(app, focused, in_band), rect);
    frame.render_widget(
        caption(app, TASK_CAPTION, focused),
        inner(rect, layout.caption_row, 2),
    );

    let p = &app.palette;
    let prefix = app.composer.task_prefix();
    let lead = lead_width(prefix);
    let text_style = value_style(app, in_band);
    for (index, row) in app
        .composer
        .task
        .rows()
        .iter()
        .take(rect.height.saturating_sub(2) as usize)
        .enumerate()
    {
        let y = layout.value_row + index as u16;
        if index == 0 {
            let mut line = prompt_line(app, prefix, &row.text, in_band);
            if !in_band {
                line = line.style(Style::default().fg(p.overlay1));
            }
            frame.render_widget(line, inner(rect, y, PROMPT_INSET));
        } else {
            frame.render_widget(
                Paragraph::new(Line::styled(row.text.clone(), text_style)),
                inner(rect, y, PROMPT_INSET + lead),
            );
        }
    }
}

/// The rows of the open dropdown, windowed to keep the pointed row inside the
/// room its box was given. The window is the same one a click is mapped
/// through, so the row clicked is the row taken.
fn draw_items(
    app: &AppState,
    frame: &mut Frame,
    layout: &ComposerLayout,
    open_box: Rect,
    which: Focus,
) {
    let rows = layout.dropdown_rows.len();
    let count = app.composer.item_count();
    let first = app
        .composer
        .highlight
        .saturating_sub(rows.saturating_sub(1))
        .min(count.saturating_sub(rows));
    for (offset, &y) in layout.dropdown_rows.iter().enumerate() {
        let index = first + offset;
        let Some(text) = item_text(app, which, index) else {
            break;
        };
        frame.render_widget(
            Paragraph::new(Line::styled(
                elide(&text, open_box.width.saturating_sub(4) as usize),
                item_style(
                    app,
                    app.composer.pointing() && index == app.composer.highlight,
                    app.composer.hover == Some(index),
                ),
            )),
            inner(open_box, y, 2),
        );
    }
}

fn item_text(app: &AppState, which: Focus, index: usize) -> Option<String> {
    match which {
        Focus::Folder => app
            .composer
            .folder_rows()
            .get(index)
            .map(|folder| folder.label.clone()),
        Focus::Agent => app
            .composer
            .harnesses()
            .get(index)
            .map(|harness| harness.name.to_string()),
        Focus::Task => None,
    }
}

/// The stretch of a path being typed that the box can show, and where the
/// cursor sits within it. The box grows to fit what is typed, so this only
/// bites once it has run out of frame to grow into — and then it follows the
/// cursor, because a field you cannot see the end of is worse than one that
/// scrolls. The drawing and the cursor both read this, so they cannot disagree.
fn typing(app: &AppState, box_width: u16) -> (String, usize) {
    let text: Vec<char> = app.composer.path().text().chars().collect();
    let (_, cursor) = app.composer.path().cursor_row();
    let room = box_width.saturating_sub(PROMPT_INSET + 2) as usize;
    if room == 0 {
        return (String::new(), 0);
    }
    let start = (cursor + 1).saturating_sub(room);
    let end = (start + room).min(text.len());
    (text[start..end].iter().collect(), cursor - start)
}

fn place_cursor(app: &AppState, frame: &mut Frame, layout: &ComposerLayout, in_band: bool) {
    if !in_band {
        return;
    }
    match app.composer.focus {
        Focus::Task if app.composer.open.is_none() => {
            let (row, col) = app.composer.task.cursor_row();
            let row = row.min(layout.task.height.saturating_sub(3) as usize) as u16;
            frame.set_cursor_position((
                layout.task.x + layout.task_lead + col as u16,
                layout.value_row + row,
            ));
        }
        Focus::Folder if app.composer.open == Some(Focus::Folder) => {
            let (_, col) = typing(app, layout.folder.width);
            frame.set_cursor_position((
                layout.folder.x + PROMPT_INSET + col as u16,
                layout.value_row,
            ));
        }
        _ => {}
    }
}

// ---- shared pieces --------------------------------------------------------

/// A box's border: lit when its control has the keyboard, plain while the band
/// does, and receded when the band does not.
fn border_style(app: &AppState, focused: bool, in_band: bool) -> Style {
    let p = &app.palette;
    if focused {
        Style::default().fg(p.accent)
    } else if in_band {
        Style::default().fg(p.overlay0)
    } else {
        Style::default().fg(p.surface1)
    }
}

fn value_style(app: &AppState, in_band: bool) -> Style {
    let p = &app.palette;
    if in_band {
        Style::default().fg(p.text)
    } else {
        Style::default().fg(p.overlay1)
    }
}

/// One colour per directory, so a folder reads as the same place wherever it
/// appears.
fn directory_style(app: &AppState, index: usize) -> Style {
    let p = &app.palette;
    let colours = [p.blue, p.mauve, p.teal, p.peach];
    Style::default().fg(colours[index % colours.len()])
}

fn item_style(app: &AppState, pointed: bool, hovered: bool) -> Style {
    let p = &app.palette;
    if hovered {
        Style::default()
            .fg(panel_contrast_fg(p))
            .bg(p.accent)
            .add_modifier(Modifier::BOLD)
    } else if pointed {
        Style::default().fg(p.accent).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(p.overlay1)
    }
}

fn rounded(app: &AppState, focused: bool, in_band: bool) -> Block<'static> {
    Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(border_style(app, focused, in_band))
}

/// The word above a box. Dim and italic, so it reads as a note about the box
/// rather than as anything in it — and lit when that box has the keyboard,
/// because a border alone is too quiet to answer "where does what I type go?"
/// at a glance.
fn caption(app: &AppState, text: &'static str, focused: bool) -> Paragraph<'static> {
    let p = &app.palette;
    let colour = if focused { p.accent } else { p.overlay0 };
    Paragraph::new(Line::styled(
        text,
        Style::default().fg(colour).add_modifier(Modifier::ITALIC),
    ))
}

/// The `▾` that says a box has more behind it.
fn marker(app: &AppState, open: bool, focused: bool, in_band: bool) -> Paragraph<'static> {
    Paragraph::new(Line::styled(
        if open { OPENED } else { CLOSED },
        border_style(app, focused, in_band),
    ))
}

fn marker_area(box_area: Rect, row: u16) -> Rect {
    Rect {
        x: box_area.x + box_area.width.saturating_sub(2),
        y: row,
        width: 1,
        height: 1,
    }
}

/// The rule between a dropdown's value row and its list.
fn rule(buffer: &mut Buffer, box_area: Rect, row: u16, style: Style) {
    let right = box_area.x + box_area.width.saturating_sub(1);
    for x in box_area.x..=right {
        let symbol = if x == box_area.x {
            "├"
        } else if x == right {
            "┤"
        } else {
            "─"
        };
        if let Some(cell) = buffer.cell_mut((x, row)) {
            cell.set_symbol(symbol).set_style(style);
        }
    }
}

fn prompt_line(app: &AppState, prefix: &str, text: &str, in_band: bool) -> Paragraph<'static> {
    let p = &app.palette;
    let mut spans = Vec::new();
    if !prefix.is_empty() {
        spans.push(Span::styled(
            format!("{prefix} "),
            Style::default().fg(p.accent),
        ));
    }
    spans.push(Span::styled(text.to_string(), value_style(app, in_band)));
    Paragraph::new(Line::from(spans))
}

/// The writable strip on one row of a box, starting `indent` in from its left
/// border and stopping clear of the right one.
fn inner(box_area: Rect, row: u16, indent: u16) -> Rect {
    Rect {
        x: box_area.x + indent,
        y: row,
        width: box_area.width.saturating_sub(indent + 2),
        height: 1,
    }
}

/// Text cut to fit, with the front dropped rather than the back: the end of a
/// path is the part that says which folder it is.
///
/// A path drops whole leading folders, marked `…/`, the way the sidebar drops
/// them — half a folder name reads as a folder that does not exist. When even
/// the last folder will not fit, the cut falls wherever it has to.
fn elide(text: &str, room: usize) -> String {
    let count = text.chars().count();
    if room == 0 {
        return String::new();
    }
    if count <= room {
        return text.to_string();
    }
    let chars: Vec<char> = text.chars().collect();
    let cut = count - room + 1;
    if let Some(boundary) = (cut..count).find(|at| chars[*at] == '/') {
        let kept: String = chars[boundary..].iter().collect();
        return format!("…{kept}");
    }
    let kept: String = chars[cut..].iter().collect();
    format!("…{kept}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{backend::TestBackend, Terminal};

    fn row_text(buffer: &ratatui::buffer::Buffer, row: u16, width: u16) -> String {
        (0..width)
            .map(|col| buffer[(col, row)].symbol().to_string())
            .collect()
    }

    fn band_state() -> AppState {
        let mut app = AppState::test_new();
        app.mode = Mode::Composer;
        app.composer
            .add_folder(std::path::PathBuf::from("/home/nobody/lab/herdr"));
        app
    }

    fn draw(app: &mut AppState, width: u16, height: u16) -> ratatui::buffer::Buffer {
        let (layout, _) = split_composer(app, Rect::new(0, 0, width, height));
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| {
                render_composer(app, frame, &layout);
                render_composer_dropdown(app, frame, &layout);
            })
            .unwrap();
        terminal.backend().buffer().clone()
    }

    #[test]
    fn a_short_frame_keeps_all_its_rows_for_panes() {
        let mut app = band_state();
        let (layout, rest) = split_composer(&mut app, Rect::new(0, 0, 80, 5));
        assert_eq!(layout.area, Rect::default());
        assert_eq!(rest, Rect::new(0, 0, 80, 5));
    }

    #[test]
    fn a_one_line_task_takes_four_rows_off_the_top() {
        let mut app = band_state();
        let (layout, rest) = split_composer(&mut app, Rect::new(0, 0, 100, 24));
        assert_eq!(layout.area, Rect::new(0, 0, 100, 4));
        assert_eq!(rest, Rect::new(0, 4, 100, 20));
    }

    #[test]
    fn a_task_that_wraps_grows_its_box_over_the_panes_rather_than_pushing_them() {
        let mut app = band_state();
        app.composer.task.set_text(&"word ".repeat(40));
        let (layout, rest) = split_composer(&mut app, Rect::new(0, 0, 100, 24));
        assert!(layout.area.height > BAND_HEIGHT, "the box grew");
        assert_eq!(
            rest,
            Rect::new(0, 4, 100, 20),
            "and the panes kept their rows"
        );
    }

    #[test]
    fn the_band_stops_growing_before_it_takes_the_frame() {
        let mut app = band_state();
        app.composer.task.set_text(&"word ".repeat(500));
        let (layout, _) = split_composer(&mut app, Rect::new(0, 0, 100, 24));
        assert!(layout.area.height <= 24 - MIN_ROWS_BELOW);
    }

    #[test]
    fn the_rows_of_a_long_task_are_drawn_over_the_panes() {
        let mut app = band_state();
        app.composer.task.set_text("first\nsecond\nthird");
        let (layout, _) = split_composer(&mut app, Rect::new(0, 0, 100, 24));
        let buffer = draw(&mut app, 100, 24);
        let second = row_text(&buffer, layout.value_row + 1, 100);
        assert!(second.contains("second"), "row two: {second}");
        let third = row_text(&buffer, layout.value_row + 2, 100);
        assert!(third.contains("third"), "row three: {third}");
        let bottom = row_text(&buffer, layout.value_row + 3, 100);
        assert!(
            bottom.contains("╰"),
            "and the box closed under them: {bottom}"
        );
    }

    #[test]
    fn each_control_is_a_captioned_box() {
        let mut app = band_state();
        app.composer.task.set_text("fix the drag preview");
        let (layout, _) = split_composer(&mut app, Rect::new(0, 0, 100, 24));
        assert_eq!(
            layout.worktree.x,
            layout.folder.x + layout.folder.width + GAP
        );
        assert_eq!(
            layout.agent.x,
            layout.worktree.x + layout.worktree.width + GAP
        );
        let buffer = draw(&mut app, 100, 24);

        let captions = row_text(&buffer, 0, 100);
        let directory_at = captions.find("Directory").expect("Directory caption");
        let worktree_at = captions.find("Worktree").expect("Worktree caption");
        let agent_at = captions.find("Agent").expect("Agent caption");
        let task_at = captions.find("Task").expect("Task caption");
        assert!(
            directory_at < worktree_at && worktree_at < agent_at && agent_at < task_at,
            "Worktree sits between Directory and Agent: {captions}"
        );
        let borders = row_text(&buffer, 1, 100);
        assert!(borders.contains("╭"), "the boxes open: {borders}");
        let box_top: String = borders
            .chars()
            .skip(layout.worktree.x as usize)
            .take(WORKTREE_BOX_WIDTH as usize)
            .collect();
        assert_eq!(box_top, "╭───╮", "the checkbox: {borders}");
        let values = row_text(&buffer, 2, 100);
        assert!(values.contains("herdr"), "the folder is on show: {values}");
        let check: String = values
            .chars()
            .skip(layout.worktree.x as usize)
            .take(WORKTREE_BOX_WIDTH as usize)
            .collect();
        assert_eq!(check, "│ ✓ │", "the box starts checked: {values}");
        assert!(values.contains("Auto"), "the agent is on show: {values}");
        assert!(
            values.contains("fix the drag preview"),
            "the task: {values}"
        );
        assert!(values.contains(CLOSED), "the lists say they open: {values}");
        let bottoms = row_text(&buffer, 3, 100);
        assert!(bottoms.contains("╰"), "and the boxes close: {bottoms}");
        let box_bottom: String = bottoms
            .chars()
            .skip(layout.worktree.x as usize)
            .take(WORKTREE_BOX_WIDTH as usize)
            .collect();
        assert_eq!(box_bottom, "╰───╯", "the checkbox closes: {bottoms}");
    }

    #[test]
    fn an_empty_folder_control_offers_to_add_a_directory() {
        let mut app = AppState::test_new();
        app.mode = Mode::Composer;
        let values = row_text(&draw(&mut app, 100, 24), 2, 100);
        assert!(values.contains("add a directory…"), "{values}");
    }

    #[test]
    fn an_open_folder_list_turns_the_box_into_a_path_field() {
        let mut app = band_state();
        app.composer.open_dropdown(Focus::Folder);
        app.composer.edit_path(|path| path.insert_str("/tmp"));
        let row = row_text(&draw(&mut app, 100, 24), 2, 100);
        assert!(row.contains("/tmp"), "the path is in the field: {row}");
        assert!(!row.contains('❯'), "no prompt glyph: {row}");
    }

    #[test]
    fn an_open_agent_list_shows_the_row_being_pointed_at() {
        let mut app = band_state();
        app.composer.open_dropdown(Focus::Agent);
        if app.composer.harnesses().len() < 2 {
            return;
        }
        app.composer.point(1);
        let pointed = app.composer.harnesses()[1].name;
        let row = row_text(&draw(&mut app, 100, 24), 2, 100);
        assert!(row.contains(pointed), "trying it on: {row}");
    }

    #[test]
    fn auto_writes_the_command_it_stands_for_in_front_of_the_task() {
        let mut app = band_state();
        app.composer.task.set_text("fix the drag preview");
        let row = row_text(&draw(&mut app, 100, 24), 2, 100);
        assert!(
            row.contains("/who fix the drag preview"),
            "the field reads as what will be sent: {row}"
        );
        assert!(!row.contains('❯'), "no prompt glyph: {row}");
    }

    #[test]
    fn naming_a_harness_writes_nothing_in_front_of_the_task() {
        let mut app = band_state();
        app.composer.agent = "Claude Code".to_string();
        app.composer.task.set_text("fix the drag preview");
        let row = row_text(&draw(&mut app, 100, 24), 2, 100);
        assert!(!row.contains("/who"), "no prefix: {row}");
        assert!(!row.contains('❯'), "no prompt glyph: {row}");
        assert!(row.contains("fix the drag preview"), "{row}");
    }

    #[test]
    fn an_open_dropdown_grows_its_own_box_under_a_rule() {
        let mut app = band_state();
        app.composer.add_folder(std::path::PathBuf::from("/tmp"));
        app.composer.open_dropdown(Focus::Folder);
        let (layout, rest) = split_composer(&mut app, Rect::new(0, 0, 100, 24));
        assert_eq!(rest.height, 20, "the panes kept their rows");
        assert_eq!(layout.folder.height, 4 + 2, "the box holds both rows");
        assert_eq!(layout.dropdown_rows, vec![4, 5]);

        let buffer = draw(&mut app, 100, 24);
        let rule_row = row_text(&buffer, layout.value_row + 1, layout.folder.width);
        assert!(
            rule_row.trim_start().starts_with("├"),
            "the rule: {rule_row}"
        );
        let first = row_text(&buffer, 4, 100);
        assert!(first.contains("tmp"), "the first row: {first}");
        let second = row_text(&buffer, 5, 100);
        assert!(second.contains("herdr"), "the second row: {second}");
    }

    #[test]
    fn a_path_being_typed_lists_what_it_could_mean_under_it() {
        let root = std::env::temp_dir().join(format!(
            "herdr-composer-draw-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        for child in ["herdr", "herdr-old"] {
            std::fs::create_dir_all(root.join(child)).unwrap();
        }
        let root = root.canonicalize().unwrap();

        let mut app = band_state();
        app.composer.open_dropdown(Focus::Folder);
        app.composer
            .edit_path(|path| path.set_text(&format!("{}/her", root.display())));
        let (layout, _) = split_composer(&mut app, Rect::new(0, 0, 160, 24));
        let buffer = draw(&mut app, 160, 24);

        assert_eq!(layout.dropdown_rows.len(), 2, "one row per folder offered");
        let first = row_text(&buffer, layout.dropdown_rows[0], 160);
        assert!(first.contains("herdr"), "the first offer: {first}");
        assert_ne!(
            buffer[(layout.dropdown.x + 1, layout.dropdown_rows[0])]
                .style()
                .fg,
            Some(app.palette.accent),
            "and no row is lit until the list is stepped into"
        );

        app.composer.point(1);
        let buffer = draw(&mut app, 160, 24);
        assert_eq!(
            buffer[(layout.dropdown.x + 1, layout.dropdown_rows[0])]
                .style()
                .fg,
            Some(app.palette.accent)
        );
    }

    #[test]
    fn the_pointed_row_of_a_dropdown_is_the_lit_one() {
        let mut app = band_state();
        app.composer.add_folder(std::path::PathBuf::from("/tmp"));
        app.composer.open_dropdown(Focus::Folder);
        app.composer.point(1);
        let (layout, _) = split_composer(&mut app, Rect::new(0, 0, 100, 24));
        let buffer = draw(&mut app, 100, 24);

        let pointed_y = layout.dropdown_rows[1];
        let pointed_row = row_text(&buffer, pointed_y, 40);
        assert!(pointed_row.contains("herdr"), "second row: {pointed_row}");
        assert_eq!(
            buffer[(layout.dropdown.x + 1, pointed_y)].style().fg,
            Some(app.palette.accent)
        );
        assert_ne!(
            buffer[(layout.dropdown.x + 1, layout.dropdown_rows[0])]
                .style()
                .fg,
            Some(app.palette.accent)
        );
    }

    #[test]
    fn the_hovered_row_of_a_dropdown_has_a_filled_highlight() {
        let mut app = band_state();
        app.composer.add_folder(std::path::PathBuf::from("/tmp"));
        app.composer.open_dropdown(Focus::Folder);
        app.composer.hover = Some(1);
        let (layout, _) = split_composer(&mut app, Rect::new(0, 0, 100, 24));
        let buffer = draw(&mut app, 100, 24);

        let hovered = buffer[(layout.dropdown.x + 1, layout.dropdown_rows[1])].style();
        assert_eq!(hovered.bg, Some(app.palette.accent));
        assert_eq!(hovered.fg, Some(panel_contrast_fg(&app.palette)));
    }

    #[test]
    fn a_path_too_long_for_its_box_keeps_the_end_that_names_the_folder() {
        assert_eq!(elide("~/lab/herdr", 20), "~/lab/herdr");
        assert_eq!(elide("~/a/very/long/path/to/herdr", 10), "…/to/herdr");
        assert_eq!(elide("~/a/very/long/path/to/herdr", 16), "…/path/to/herdr");
        assert_eq!(elide("~/lab/averylongfoldername", 8), "…dername");
    }

    #[test]
    fn clearing_the_worktree_box_draws_it_empty() {
        let mut app = band_state();
        app.composer.worktree = false;
        let (layout, _) = split_composer(&mut app, Rect::new(0, 0, 100, 24));
        let buffer = draw(&mut app, 100, 24);
        let captions = row_text(&buffer, 0, 100);
        assert!(captions.contains("Worktree"), "the label stays: {captions}");
        let check: String = row_text(&buffer, 2, 100)
            .chars()
            .skip(layout.worktree.x as usize)
            .take(WORKTREE_BOX_WIDTH as usize)
            .collect();
        assert_eq!(check, "│   │", "cleared: {check}");
    }

    #[test]
    fn a_narrow_frame_gives_the_folder_box_up_before_the_task() {
        let mut app = band_state();
        let (layout, _) = split_composer(&mut app, Rect::new(0, 0, 60, 24));
        assert!(layout.task.width >= MIN_TASK_TEXT);
        assert!(
            layout.folder.width < MIN_FOLDER_TEXT,
            "the folder gave its width up: {}",
            layout.folder.width
        );
        assert!(
            layout.task.x + layout.task.width <= 60,
            "and nothing reaches past the frame"
        );
    }
}

#[cfg(test)]
mod view_tests {
    use super::*;
    use crate::app::AppState;
    use ratatui::{backend::TestBackend, layout::Rect, Terminal};

    /// The band is laid out and drawn through the same two calls the draw loop
    /// makes, because the open box is computed in one and drawn again in the
    /// other — a test that calls them directly would not notice the two
    /// disagreeing.
    #[test]
    fn an_open_dropdown_survives_the_whole_frame_and_covers_the_panes() {
        let mut app = AppState::test_new();
        app.workspaces = vec![crate::workspace::Workspace::test_new("space")];
        app.active = Some(0);
        app.mode = Mode::Composer;
        app.composer
            .add_folder(std::path::PathBuf::from("/home/nobody/lab/herdr"));
        app.composer.open_dropdown(Focus::Folder);

        let area = Rect::new(0, 0, 120, 30);
        crate::ui::compute_view(&mut app, area);
        assert!(
            !app.view.composer.dropdown_rows.is_empty(),
            "the layout carried the open list into the view"
        );

        let mut terminal = Terminal::new(TestBackend::new(120, 30)).unwrap();
        terminal
            .draw(|frame| crate::ui::render(&app, frame))
            .unwrap();
        let buffer = terminal.backend().buffer().clone();

        let row = app.view.composer.dropdown_rows[0];
        let drawn: String = (0..120)
            .map(|col| buffer[(col, row)].symbol().to_string())
            .collect();
        assert!(
            drawn.contains("herdr"),
            "the list is on top of whatever it covers: {drawn}"
        );
    }
}
