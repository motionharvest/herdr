//! BSP tree layout for tiling panes within a workspace.

use ratatui::layout::{Direction, Rect};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct PaneId(u32);

/// Global atomic counter for unique PaneId generation across all workspaces.
static NEXT_PANE_ID: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);

impl PaneId {
    /// Allocate a globally unique PaneId.
    pub fn alloc() -> Self {
        Self(NEXT_PANE_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed))
    }

    pub fn raw(self) -> u32 {
        self.0
    }

    /// Reconstruct from a saved u32 (persistence only).
    pub fn from_raw(id: u32) -> Self {
        Self(id)
    }
}

/// Which sides of a pane touch the outer workspace edge rather than a sibling.
#[derive(Debug, Clone, Copy, Default)]
pub struct ExposedSides {
    pub top: bool,
    pub bottom: bool,
    pub left: bool,
    pub right: bool,
}

impl ExposedSides {
    pub fn all() -> Self {
        Self {
            top: true,
            bottom: true,
            left: true,
            right: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum SplitSide {
    Top,
    Bottom,
    Left,
    Right,
}

/// Snapshot of a pane's position and focus state after layout.
#[derive(Clone)]
pub struct PaneInfo {
    pub id: PaneId,
    /// Outer rect (including borders if present).
    pub rect: Rect,
    /// Inner rect (terminal area inside borders and inner padding). Used for selection.
    pub inner_rect: Rect,
    /// Visible scrollbar lane, when scrollback is present. `inner_rect` may still
    /// exclude a stable hidden gutter when this is `None`.
    pub scrollbar_rect: Option<Rect>,
    pub is_focused: bool,
    pub exposed: ExposedSides,
}

pub fn placement_is_adjacent(current: Rect, candidate: Rect, side: SplitSide) -> bool {
    adjacent_overlap(current, candidate, side) > 0
}

pub fn adjacent_overlap(current: Rect, candidate: Rect, side: SplitSide) -> u16 {
    match side {
        SplitSide::Top => {
            if candidate.bottom() != current.y {
                return 0;
            }
            overlap(candidate.x, candidate.right(), current.x, current.right())
        }
        SplitSide::Bottom => {
            if current.bottom() != candidate.y {
                return 0;
            }
            overlap(candidate.x, candidate.right(), current.x, current.right())
        }
        SplitSide::Left => {
            if candidate.right() != current.x {
                return 0;
            }
            overlap(candidate.y, candidate.bottom(), current.y, current.bottom())
        }
        SplitSide::Right => {
            if current.right() != candidate.x {
                return 0;
            }
            overlap(candidate.y, candidate.bottom(), current.y, current.bottom())
        }
    }
}

/// Screen-row span `[start, end)` where `current` shares its right edge with `right_neighbor`.
pub fn adjacent_right_edge_y_range(current: Rect, right_neighbor: Rect) -> Option<(u16, u16)> {
    if current.right() != right_neighbor.x {
        return None;
    }
    let start = current.y.max(right_neighbor.y);
    let end = current.bottom().min(right_neighbor.bottom());
    if start < end {
        Some((start, end))
    } else {
        None
    }
}

pub fn y_segments_outside(start: u16, end: u16, hidden: (u16, u16)) -> Vec<(u16, u16)> {
    let mut segments = Vec::with_capacity(2);
    if start < hidden.0 {
        segments.push((start, hidden.0.min(end)));
    }
    if hidden.1 < end {
        segments.push((hidden.1.max(start), end));
    }
    segments.retain(|(segment_start, segment_end)| segment_start < segment_end);
    segments
}

fn overlap(start_a: u16, end_a: u16, start_b: u16, end_b: u16) -> u16 {
    let start = start_a.max(start_b);
    let end = end_a.min(end_b);
    end.saturating_sub(start)
}

/// Info about a split boundary, used for mouse drag resize.
#[derive(Clone)]
pub struct SplitBorder {
    /// Position of the divider line (x for horizontal split, y for vertical).
    pub pos: u16,
    /// Direction of the split that created this border.
    pub direction: Direction,
    /// Total area of the split node.
    pub area: Rect,
    /// Path from root to this split node (false=first, true=second).
    pub path: Vec<bool>,
}

/// Cardinal direction for pane navigation.
#[derive(Debug, Clone, Copy)]
pub enum NavDirection {
    Left,
    Right,
    Up,
    Down,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplitPlacement {
    Before,
    After,
}

/// A node in the BSP tree. Public for serialization.
pub enum Node {
    Pane(PaneId),
    Split {
        direction: Direction,
        ratio: f32,
        first: Box<Node>,
        second: Box<Node>,
    },
}

/// BSP tiling layout. Tracks a tree of splits and a focused pane.
pub struct TileLayout {
    root: Node,
    focus: PaneId,
}

impl TileLayout {
    /// Create a new layout with a single pane (globally unique ID).
    /// Returns (layout, root_pane_id) so the caller can create the pane.
    pub fn new() -> (Self, PaneId) {
        let root_id = PaneId::alloc();
        (
            Self {
                root: Node::Pane(root_id),
                focus: root_id,
            },
            root_id,
        )
    }

    pub fn focused(&self) -> PaneId {
        self.focus
    }

    pub fn pane_count(&self) -> usize {
        count_panes(&self.root)
    }

    /// Compute rects for all panes given the available area.
    pub fn panes(&self, area: Rect) -> Vec<PaneInfo> {
        let mut result = Vec::new();
        collect_panes(
            &self.root,
            area,
            self.focus,
            ExposedSides::all(),
            &mut result,
        );
        result
    }

    /// Collect all split boundaries for mouse drag resize.
    pub fn splits(&self, area: Rect) -> Vec<SplitBorder> {
        let mut result = Vec::new();
        collect_splits(&self.root, area, vec![], &mut result);
        result
    }

    /// Split the focused pane, placing the new pane before or after the current pane.
    pub fn split_focused_with_placement(
        &mut self,
        direction: Direction,
        placement: SplitPlacement,
    ) -> PaneId {
        let new_id = PaneId::alloc();
        let placeholder = PaneId::from_raw(0);
        let old = std::mem::replace(&mut self.root, Node::Pane(placeholder));
        self.root = split_at(old, self.focus, direction, new_id, placement);
        self.focus = new_id;
        new_id
    }

    /// Close the focused pane. Returns false if it's the last pane.
    pub fn close_focused(&mut self) -> bool {
        if self.pane_count() <= 1 {
            return false;
        }
        let target = self.focus;
        let ids = self.pane_ids();
        let pos = ids.iter().position(|id| *id == target).unwrap();
        let new_focus = if pos + 1 < ids.len() {
            ids[pos + 1]
        } else {
            ids[pos - 1]
        };
        let placeholder = PaneId::from_raw(0);
        let old = std::mem::replace(&mut self.root, Node::Pane(placeholder));
        if let Some(new_root) = remove_pane(old, target) {
            self.root = new_root;
            self.focus = new_focus;
            true
        } else {
            false
        }
    }

    pub fn focus_pane(&mut self, id: PaneId) {
        if self.pane_ids().contains(&id) {
            self.focus = id;
        }
    }

    /// Set the ratio of a split node at the given path.
    pub fn set_ratio_at(&mut self, path: &[bool], ratio: f32) {
        set_ratio_at(&mut self.root, path, ratio.clamp(0.1, 0.9));
    }

    /// Adjust the nearest split in the given direction for the focused pane.
    /// `delta` is positive to grow, negative to shrink.
    pub fn resize_focused(&mut self, nav: NavDirection, delta: f32, area: Rect) {
        let panes = self.panes(area);
        let Some(focused) = panes.iter().find(|p| p.is_focused) else {
            return;
        };
        let focused_rect = focused.rect;
        let splits = self.splits(area);

        // Find the split whose border is adjacent to the focused pane in the given direction
        let target_dir = match nav {
            NavDirection::Left | NavDirection::Right => Direction::Horizontal,
            NavDirection::Up | NavDirection::Down => Direction::Vertical,
        };
        let grows = matches!(nav, NavDirection::Right | NavDirection::Down);

        // Find the closest matching split border
        let best = splits
            .iter()
            .filter(|s| s.direction == target_dir)
            .filter(|s| match target_dir {
                Direction::Horizontal => {
                    // Border must be near the focused pane's left or right edge
                    let near_right = (s.pos as i32 - (focused_rect.x + focused_rect.width) as i32)
                        .unsigned_abs()
                        <= 1;
                    let near_left = (s.pos as i32 - focused_rect.x as i32).unsigned_abs() <= 1;
                    near_right || near_left
                }
                Direction::Vertical => {
                    let near_bottom = (s.pos as i32
                        - (focused_rect.y + focused_rect.height) as i32)
                        .unsigned_abs()
                        <= 1;
                    let near_top = (s.pos as i32 - focused_rect.y as i32).unsigned_abs() <= 1;
                    near_bottom || near_top
                }
            })
            .min_by_key(|s| {
                // Prefer the border in the direction we're resizing toward
                match (target_dir, grows) {
                    (Direction::Horizontal, true) => {
                        ((focused_rect.x + focused_rect.width) as i32 - s.pos as i32).unsigned_abs()
                    }
                    (Direction::Horizontal, false) => {
                        (focused_rect.x as i32 - s.pos as i32).unsigned_abs()
                    }
                    (Direction::Vertical, true) => ((focused_rect.y + focused_rect.height) as i32
                        - s.pos as i32)
                        .unsigned_abs(),
                    (Direction::Vertical, false) => {
                        (focused_rect.y as i32 - s.pos as i32).unsigned_abs()
                    }
                }
            });

        if let Some(split) = best {
            let path = split.path.clone();
            let current_ratio = get_ratio_at(&self.root, &path).unwrap_or(0.5);
            let adj = if grows { delta } else { -delta };
            self.set_ratio_at(&path, current_ratio + adj);
        }
    }

    pub fn pane_ids(&self) -> Vec<PaneId> {
        let mut ids = Vec::new();
        collect_ids(&self.root, &mut ids);
        ids
    }

    /// Swap the screen positions of two panes by exchanging their leaf ids.
    pub fn swap_leaf_ids(&mut self, pane_a: PaneId, pane_b: PaneId) -> bool {
        if pane_a == pane_b
            || !contains_pane_id(&self.root, pane_a)
            || !contains_pane_id(&self.root, pane_b)
        {
            return false;
        }
        swap_leaf_ids_unchecked(&mut self.root, pane_a, pane_b);
        true
    }

    /// Access the tree root for serialization.
    pub fn root(&self) -> &Node {
        &self.root
    }

    /// Reconstruct a layout from a saved tree.
    /// Reconstruct a layout from a saved tree.
    pub fn from_saved(root: Node, focus: PaneId) -> Self {
        Self { root, focus }
    }
}

// --- Directional pane navigation ---

/// Find the nearest pane in the given direction from `focused`.
pub fn find_in_direction(
    focused: &PaneInfo,
    direction: NavDirection,
    panes: &[PaneInfo],
) -> Option<PaneId> {
    let fr = focused.rect;

    panes
        .iter()
        .filter(|p| p.id != focused.id)
        .filter(|p| {
            let r = p.rect;
            match direction {
                NavDirection::Left => {
                    r.x + r.width <= fr.x && ranges_overlap(r.y, r.height, fr.y, fr.height)
                }
                NavDirection::Right => {
                    r.x >= fr.x + fr.width && ranges_overlap(r.y, r.height, fr.y, fr.height)
                }
                NavDirection::Up => {
                    r.y + r.height <= fr.y && ranges_overlap(r.x, r.width, fr.x, fr.width)
                }
                NavDirection::Down => {
                    r.y >= fr.y + fr.height && ranges_overlap(r.x, r.width, fr.x, fr.width)
                }
            }
        })
        .min_by_key(|p| {
            let r = p.rect;
            match direction {
                NavDirection::Left => fr.x.saturating_sub(r.x + r.width),
                NavDirection::Right => r.x.saturating_sub(fr.x + fr.width),
                NavDirection::Up => fr.y.saturating_sub(r.y + r.height),
                NavDirection::Down => r.y.saturating_sub(fr.y + fr.height),
            }
        })
        .map(|p| p.id)
}

fn ranges_overlap(a_start: u16, a_len: u16, b_start: u16, b_len: u16) -> bool {
    a_start < b_start + b_len && a_start + a_len > b_start
}

// --- Tree operations ---

fn count_panes(node: &Node) -> usize {
    match node {
        Node::Pane(_) => 1,
        Node::Split { first, second, .. } => count_panes(first) + count_panes(second),
    }
}

fn contains_pane_id(node: &Node, pane_id: PaneId) -> bool {
    match node {
        Node::Pane(id) => *id == pane_id,
        Node::Split { first, second, .. } => {
            contains_pane_id(first, pane_id) || contains_pane_id(second, pane_id)
        }
    }
}

fn swap_leaf_ids_unchecked(node: &mut Node, pane_a: PaneId, pane_b: PaneId) {
    match node {
        Node::Pane(id) => {
            if *id == pane_a {
                *id = pane_b;
            } else if *id == pane_b {
                *id = pane_a;
            }
        }
        Node::Split { first, second, .. } => {
            swap_leaf_ids_unchecked(first, pane_a, pane_b);
            swap_leaf_ids_unchecked(second, pane_a, pane_b);
        }
    }
}

fn collect_panes(
    node: &Node,
    area: Rect,
    focus: PaneId,
    exposed: ExposedSides,
    result: &mut Vec<PaneInfo>,
) {
    match node {
        Node::Pane(id) => {
            result.push(PaneInfo {
                id: *id,
                rect: area,
                // inner_rect is set during render when we know if borders are shown
                inner_rect: area,
                scrollbar_rect: None,
                is_focused: *id == focus,
                exposed,
            });
        }
        Node::Split {
            direction,
            ratio,
            first,
            second,
        } => {
            let (a, b) = split_rect(area, *direction, *ratio);
            let mut first_exposed = exposed;
            let mut second_exposed = exposed;
            match direction {
                Direction::Vertical => {
                    first_exposed.bottom = false;
                    second_exposed.top = false;
                }
                Direction::Horizontal => {
                    first_exposed.right = false;
                    second_exposed.left = false;
                }
            }
            collect_panes(first, a, focus, first_exposed, result);
            collect_panes(second, b, focus, second_exposed, result);
        }
    }
}

fn collect_splits(node: &Node, area: Rect, path: Vec<bool>, result: &mut Vec<SplitBorder>) {
    if let Node::Split {
        direction,
        ratio,
        first,
        second,
    } = node
    {
        let (a, b) = split_rect(area, *direction, *ratio);
        let pos = match direction {
            Direction::Horizontal => a.x + a.width,
            Direction::Vertical => a.y + a.height,
        };
        result.push(SplitBorder {
            pos,
            direction: *direction,
            area,
            path: path.clone(),
        });
        let mut lp = path.clone();
        lp.push(false);
        collect_splits(first, a, lp, result);
        let mut rp = path;
        rp.push(true);
        collect_splits(second, b, rp, result);
    }
}

fn collect_ids(node: &Node, ids: &mut Vec<PaneId>) {
    match node {
        Node::Pane(id) => ids.push(*id),
        Node::Split { first, second, .. } => {
            collect_ids(first, ids);
            collect_ids(second, ids);
        }
    }
}

fn split_at(
    node: Node,
    target: PaneId,
    direction: Direction,
    new_id: PaneId,
    placement: SplitPlacement,
) -> Node {
    match node {
        Node::Pane(id) if id == target => {
            let (first, second) = match placement {
                SplitPlacement::Before => (Node::Pane(new_id), Node::Pane(id)),
                SplitPlacement::After => (Node::Pane(id), Node::Pane(new_id)),
            };
            Node::Split {
                direction,
                ratio: 0.5,
                first: Box::new(first),
                second: Box::new(second),
            }
        }
        Node::Pane(_) => node,
        Node::Split {
            direction: d,
            ratio,
            first,
            second,
        } => Node::Split {
            direction: d,
            ratio,
            first: Box::new(split_at(*first, target, direction, new_id, placement)),
            second: Box::new(split_at(*second, target, direction, new_id, placement)),
        },
    }
}

fn remove_pane(node: Node, target: PaneId) -> Option<Node> {
    match node {
        Node::Pane(id) if id == target => None,
        Node::Pane(_) => Some(node),
        Node::Split {
            direction,
            ratio,
            first,
            second,
        } => match (remove_pane(*first, target), remove_pane(*second, target)) {
            (None, Some(s)) => Some(s),
            (Some(f), None) => Some(f),
            (Some(f), Some(s)) => Some(Node::Split {
                direction,
                ratio,
                first: Box::new(f),
                second: Box::new(s),
            }),
            (None, None) => None,
        },
    }
}

fn set_ratio_at(node: &mut Node, path: &[bool], new_ratio: f32) {
    if let Node::Split {
        ratio,
        first,
        second,
        ..
    } = node
    {
        if path.is_empty() {
            *ratio = new_ratio;
        } else if path[0] {
            set_ratio_at(second, &path[1..], new_ratio);
        } else {
            set_ratio_at(first, &path[1..], new_ratio);
        }
    }
}

fn get_ratio_at(node: &Node, path: &[bool]) -> Option<f32> {
    if let Node::Split {
        ratio,
        first,
        second,
        ..
    } = node
    {
        if path.is_empty() {
            Some(*ratio)
        } else if path[0] {
            get_ratio_at(second, &path[1..])
        } else {
            get_ratio_at(first, &path[1..])
        }
    } else {
        None
    }
}

fn split_rect(area: Rect, direction: Direction, ratio: f32) -> (Rect, Rect) {
    match direction {
        Direction::Horizontal => {
            let first_w = ((area.width as f32) * ratio).round() as u16;
            let second_w = area.width.saturating_sub(first_w);
            (
                Rect::new(area.x, area.y, first_w, area.height),
                Rect::new(area.x + first_w, area.y, second_w, area.height),
            )
        }
        Direction::Vertical => {
            let first_h = ((area.height as f32) * ratio).round() as u16;
            let second_h = area.height.saturating_sub(first_h);
            (
                Rect::new(area.x, area.y, area.width, first_h),
                Rect::new(area.x, area.y + first_h, area.width, second_h),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn horizontal_split_marks_shared_vertical_edges() {
        let (mut layout, root) = TileLayout::new();
        let right =
            layout.split_focused_with_placement(Direction::Horizontal, SplitPlacement::After);
        let panes = layout.panes(Rect::new(0, 0, 100, 20));

        let left = panes.iter().find(|pane| pane.id == root).unwrap();
        let right = panes.iter().find(|pane| pane.id == right).unwrap();
        assert!(!left.exposed.right);
        assert!(!right.exposed.left);
        assert!(left.exposed.left);
        assert!(right.exposed.right);
    }

    #[test]
    fn adjacent_right_edge_y_range_matches_shared_vertical_span() {
        let left = Rect::new(0, 2, 20, 10);
        let right = Rect::new(20, 4, 20, 8);
        assert_eq!(adjacent_right_edge_y_range(left, right), Some((4, 12)));
    }

    #[test]
    fn split_placement_controls_new_pane_side() {
        let (mut layout, root) = TileLayout::new();
        let left =
            layout.split_focused_with_placement(Direction::Horizontal, SplitPlacement::Before);
        let panes = layout.panes(Rect::new(0, 0, 100, 20));
        assert_eq!(panes.iter().find(|pane| pane.id == left).unwrap().rect.x, 0);
        assert_eq!(
            panes.iter().find(|pane| pane.id == root).unwrap().rect.x,
            50
        );

        layout.focus_pane(root);
        let right =
            layout.split_focused_with_placement(Direction::Horizontal, SplitPlacement::After);
        let panes = layout.panes(Rect::new(0, 0, 100, 20));
        let root_rect = panes.iter().find(|pane| pane.id == root).unwrap().rect;
        let right_rect = panes.iter().find(|pane| pane.id == right).unwrap().rect;
        assert!(right_rect.x > root_rect.x);
    }

    #[test]
    fn vertical_split_placement_controls_new_pane_side() {
        let (mut layout, root) = TileLayout::new();
        let top = layout.split_focused_with_placement(Direction::Vertical, SplitPlacement::Before);
        let panes = layout.panes(Rect::new(0, 0, 100, 20));
        assert_eq!(panes.iter().find(|pane| pane.id == top).unwrap().rect.y, 0);
        assert_eq!(
            panes.iter().find(|pane| pane.id == root).unwrap().rect.y,
            10
        );

        layout.focus_pane(root);
        let bottom =
            layout.split_focused_with_placement(Direction::Vertical, SplitPlacement::After);
        let panes = layout.panes(Rect::new(0, 0, 100, 20));
        let root_rect = panes.iter().find(|pane| pane.id == root).unwrap().rect;
        let bottom_rect = panes.iter().find(|pane| pane.id == bottom).unwrap().rect;
        assert!(bottom_rect.y > root_rect.y);
    }

    #[test]
    fn swap_leaf_ids_exchanges_pane_positions_in_tree() {
        let (mut layout, left) = TileLayout::new();
        let right =
            layout.split_focused_with_placement(Direction::Horizontal, SplitPlacement::After);
        let area = Rect::new(0, 0, 100, 20);
        let before_left = layout
            .panes(area)
            .iter()
            .find(|pane| pane.id == left)
            .unwrap()
            .rect;
        let before_right = layout
            .panes(area)
            .iter()
            .find(|pane| pane.id == right)
            .unwrap()
            .rect;
        assert!(before_left.x < before_right.x);

        assert!(layout.swap_leaf_ids(left, right));

        let after_left = layout
            .panes(area)
            .iter()
            .find(|pane| pane.id == left)
            .unwrap()
            .rect;
        let after_right = layout
            .panes(area)
            .iter()
            .find(|pane| pane.id == right)
            .unwrap()
            .rect;
        assert_eq!(after_left, before_right);
        assert_eq!(after_right, before_left);
    }

    #[test]
    fn swap_leaf_ids_rejects_unknown_or_identical_panes() {
        let (mut layout, left) = TileLayout::new();
        let right =
            layout.split_focused_with_placement(Direction::Horizontal, SplitPlacement::After);
        let unknown = PaneId::from_raw(999);

        assert!(!layout.swap_leaf_ids(left, left));
        assert!(!layout.swap_leaf_ids(left, unknown));

        let area = Rect::new(0, 0, 100, 20);
        let before_left = layout
            .panes(area)
            .iter()
            .find(|pane| pane.id == left)
            .unwrap()
            .rect;
        assert!(layout.swap_leaf_ids(left, right));
        let after_unknown = layout
            .panes(area)
            .iter()
            .find(|pane| pane.id == left)
            .unwrap()
            .rect;
        assert_ne!(after_unknown, before_left);
    }
}
