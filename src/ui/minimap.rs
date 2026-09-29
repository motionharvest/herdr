//! A tab's pane layout drawn small, for the tab rows in the sidebar.
//!
//! Each cell is split into a 2x2 grid of quarter cells, and the split tree is
//! laid out again on that finer grid: every split gives its first side a share
//! by its ratio, keeps one quarter cell clear as the gap, and hands the rest to
//! its second side. Each 2x2 block then maps to the one quadrant glyph that
//! draws exactly that pattern. Walking the tree, rather than scaling the pane
//! rects the real layout produced, keeps the preview from rounding twice, which
//! is what swallows small panes and leaves holes where two gaps meet.
//!
//! One cell never holds quarters of two different panes: the gap between any
//! two panes runs the full length of the split that made them, so a cell can
//! reach across it only by landing a whole row or column of quarters on the
//! gap. That is what lets the focused pane take its own color cell by cell.

use ratatui::{buffer::Buffer, layout::Direction, layout::Rect, style::Style};

use crate::layout::{Node, PaneId};

/// Quadrant glyphs indexed by which quarters are filled: top-left is 8,
/// top-right 4, bottom-left 2, bottom-right 1.
const QUADRANTS: [&str; 16] = [
    " ", "▗", "▖", "▄", "▝", "▐", "▞", "▟", "▘", "▚", "▌", "▙", "▀", "▜", "▛", "█",
];

/// How many rows a preview `width` columns wide takes to keep the shape of
/// `source`, the area the tab's panes are laid out in. Both are measured in
/// cells, so scaling each side by the same factor keeps the on-screen shape.
/// Returns 0 when there is nothing to size against.
pub(crate) fn minimap_rows(width: u16, source: Rect) -> u16 {
    if width == 0 || source.width == 0 || source.height == 0 {
        return 0;
    }
    let rows = (f32::from(width) * f32::from(source.height) / f32::from(source.width)).round();
    (rows as u16).max(1)
}

/// Draws `root` into `area`, filling each pane with `style`'s foreground and
/// the `focused` pane, when there is one, with `focused_style`'s. Cells no pane
/// reaches are left as they are, so the background shows through.
pub(crate) fn render_minimap(
    root: &Node,
    focused: Option<PaneId>,
    area: Rect,
    style: Style,
    focused_style: Style,
    buf: &mut Buffer,
) {
    let grid = Grid::lay_out(root, area);
    for cy in 0..usize::from(area.height) {
        for cx in 0..usize::from(area.width) {
            let quarters = grid.cell(cx, cy);
            let mask = quarters.iter().fold(0, |mask, quarter| {
                mask << 1 | usize::from(quarter.is_some())
            });
            if mask == 0 {
                continue;
            }
            let style = if focused.is_some() && quarters.contains(&focused) {
                focused_style
            } else {
                style
            };
            let pos = (area.x + cx as u16, area.y + cy as u16);
            if let Some(cell) = buf.cell_mut(pos) {
                cell.set_symbol(QUADRANTS[mask]).set_style(style);
            }
        }
    }
}

/// The pane drawn in the cell at `col`, `row` of a preview of `root` drawn
/// into `area`, or `None` when the point is outside it or no pane reaches that
/// cell.
pub(crate) fn minimap_pane_at(root: &Node, area: Rect, col: u16, row: u16) -> Option<PaneId> {
    let inside =
        col >= area.x && col < area.x + area.width && row >= area.y && row < area.y + area.height;
    if !inside {
        return None;
    }
    let grid = Grid::lay_out(root, area);
    grid.cell(usize::from(col - area.x), usize::from(row - area.y))
        .into_iter()
        .flatten()
        .next()
}

/// The preview's quarter cells, each holding the pane drawn there.
struct Grid {
    width: usize,
    quarters: Vec<Option<PaneId>>,
}

impl Grid {
    fn lay_out(root: &Node, area: Rect) -> Self {
        let w = usize::from(area.width) * 2;
        let h = usize::from(area.height) * 2;
        let mut grid = Grid {
            width: w,
            quarters: vec![None; w * h],
        };
        lay_out(root, 0, 0, w, h, &mut grid);
        grid
    }

    /// The four quarters of cell `cx`, `cy`, in glyph-mask order: top-left,
    /// top-right, bottom-left, bottom-right.
    fn cell(&self, cx: usize, cy: usize) -> [Option<PaneId>; 4] {
        [(0, 0), (1, 0), (0, 1), (1, 1)]
            .map(|(dx, dy)| self.quarters[(cy * 2 + dy) * self.width + cx * 2 + dx])
    }

    fn fill(&mut self, id: PaneId, x: usize, y: usize, w: usize, h: usize) {
        for row in y..y + h {
            let start = row * self.width + x;
            self.quarters[start..start + w].fill(Some(id));
        }
    }
}

fn lay_out(node: &Node, x: usize, y: usize, w: usize, h: usize, grid: &mut Grid) {
    if w == 0 || h == 0 {
        return;
    }
    let (direction, ratio, first, second) = match node {
        Node::Pane(id) => {
            grid.fill(*id, x, y, w, h);
            return;
        }
        Node::Split {
            direction,
            ratio,
            first,
            second,
        } => (direction, ratio, first, second),
    };

    let size = match direction {
        Direction::Horizontal => w,
        Direction::Vertical => h,
    };
    // Too small to hold a gap: the first side takes the whole span.
    let Some(room) = size.checked_sub(1).filter(|room| *room > 0) else {
        lay_out(first, x, y, w, h, grid);
        return;
    };
    // Each side keeps enough quarter cells for the panes stacked along this
    // axis inside it, so a pane only drops out once the preview has no room
    // left for it at all.
    let want = (room as f32 * ratio).round() as usize;
    let least_first = least_span(first, *direction).min(room);
    let most_first = room
        .saturating_sub(least_span(second, *direction))
        .max(least_first);
    let first_size = want.clamp(least_first, most_first);
    let second_size = room - first_size;

    match direction {
        Direction::Horizontal => {
            lay_out(first, x, y, first_size, h, grid);
            lay_out(second, x + first_size + 1, y, second_size, h, grid);
        }
        Direction::Vertical => {
            lay_out(first, x, y, w, first_size, grid);
            lay_out(second, x, y + first_size + 1, w, second_size, grid);
        }
    }
}

/// Fewest quarter cells `node` needs along `axis` to show every pane with a
/// one-quarter gap between neighbours.
fn least_span(node: &Node, axis: Direction) -> usize {
    match node {
        Node::Pane(_) => 1,
        Node::Split {
            direction,
            first,
            second,
            ..
        } if *direction == axis => least_span(first, axis) + 1 + least_span(second, axis),
        Node::Split { first, second, .. } => least_span(first, axis).max(least_span(second, axis)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::PaneId;

    fn pane() -> Box<Node> {
        pane_with(0)
    }

    fn pane_with(id: u32) -> Box<Node> {
        Box::new(Node::Pane(PaneId::from_raw(id)))
    }

    fn split(direction: Direction, ratio: f32, first: Box<Node>, second: Box<Node>) -> Box<Node> {
        Box::new(Node::Split {
            direction,
            ratio,
            first,
            second,
        })
    }

    fn draw(root: &Node, width: u16, height: u16) -> Vec<String> {
        let area = Rect::new(0, 0, width, height);
        let mut buf = Buffer::empty(area);
        render_minimap(
            root,
            None,
            area,
            Style::default(),
            Style::default(),
            &mut buf,
        );
        (0..height)
            .map(|y| (0..width).map(|x| buf[(x, y)].symbol()).collect())
            .collect()
    }

    #[test]
    fn a_split_column_beside_a_full_pane() {
        let root = split(
            Direction::Horizontal,
            0.5,
            split(Direction::Vertical, 0.45, pane(), pane()),
            pane(),
        );
        assert_eq!(
            draw(&root, 20, 5),
            [
                "██████████▐█████████",
                "██████████▐█████████",
                "▄▄▄▄▄▄▄▄▄▄▐█████████",
                "██████████▐█████████",
                "██████████▐█████████",
            ]
        );
    }

    #[test]
    fn gaps_that_meet_inside_one_cell_leave_no_hole() {
        let root = split(
            Direction::Horizontal,
            0.5,
            split(Direction::Vertical, 0.5, pane(), pane()),
            split(Direction::Vertical, 0.5, pane(), pane()),
        );
        let rows = draw(&root, 4, 3);
        assert!(rows.iter().all(|row| !row.contains(' ')), "{rows:?}");
    }

    #[test]
    fn every_stacked_pane_stays_visible_while_there_is_room() {
        let root = split(
            Direction::Horizontal,
            0.5,
            pane(),
            split(
                Direction::Vertical,
                0.5,
                pane(),
                split(
                    Direction::Vertical,
                    0.5,
                    pane(),
                    split(Direction::Vertical, 0.5, pane(), pane()),
                ),
            ),
        );
        // Four panes and three gaps need seven of the ten quarter rows.
        let right: Vec<String> = draw(&root, 20, 5)
            .into_iter()
            .map(|row| row.chars().skip(12).collect())
            .collect();
        let quarters: Vec<bool> = right
            .iter()
            .flat_map(|row| {
                let c = row.chars().next().unwrap();
                [matches!(c, '█' | '▀'), matches!(c, '█' | '▄')]
            })
            .collect();
        let panes = quarters
            .windows(2)
            .filter(|pair| pair[1] && !pair[0])
            .count()
            + usize::from(quarters[0]);
        assert_eq!(panes, 4, "{right:?}");
    }

    #[test]
    fn the_focused_pane_takes_its_own_color() {
        use ratatui::style::Color;
        let root = split(
            Direction::Horizontal,
            0.5,
            split(Direction::Vertical, 0.45, pane_with(1), pane_with(2)),
            pane_with(3),
        );
        let area = Rect::new(0, 0, 20, 5);
        let mut buf = Buffer::empty(area);
        let muted = Style::default().fg(Color::Gray);
        let accent = Style::default().fg(Color::Blue);
        render_minimap(
            &root,
            Some(PaneId::from_raw(2)),
            area,
            muted,
            accent,
            &mut buf,
        );
        let colors: Vec<String> = (0..5)
            .map(|y| {
                (0..20)
                    .map(|x| match buf[(x, y)].fg {
                        Color::Blue => 'F',
                        Color::Gray => '.',
                        _ => ' ',
                    })
                    .collect()
            })
            .collect();
        // The bottom-left pane starts on the lower half of the third row, so
        // that row's `▄` cells are its and take the accent too.
        assert_eq!(
            colors,
            [
                "...........",
                "...........",
                "FFFFFFFFFF.",
                "FFFFFFFFFF.",
                "FFFFFFFFFF.",
            ]
            .map(|row| format!("{row:.<20}"))
        );
    }

    #[test]
    fn a_point_in_the_preview_finds_the_pane_drawn_there() {
        let root = split(
            Direction::Horizontal,
            0.5,
            split(Direction::Vertical, 0.45, pane_with(1), pane_with(2)),
            pane_with(3),
        );
        let area = Rect::new(4, 10, 20, 5);
        let at = |col, row| minimap_pane_at(&root, area, col, row).map(PaneId::raw);
        assert_eq!(at(4, 10), Some(1));
        // The `▄` row between the left panes belongs to the lower one.
        assert_eq!(at(4, 12), Some(2));
        assert_eq!(at(13, 14), Some(2));
        // The `▐` column between the halves belongs to the right pane.
        assert_eq!(at(14, 12), Some(3));
        assert_eq!(at(23, 14), Some(3));
        assert_eq!(at(24, 10), None);
        assert_eq!(at(4, 15), None);
    }

    #[test]
    fn rows_follow_width_and_keep_the_source_shape() {
        let source = Rect::new(0, 0, 160, 48);
        assert_eq!(minimap_rows(20, source), 6);
        assert_eq!(minimap_rows(30, source), 9);
        assert_eq!(minimap_rows(1, source), 1);
        assert_eq!(minimap_rows(0, source), 0);
        assert_eq!(minimap_rows(20, Rect::default()), 0);
    }
}
