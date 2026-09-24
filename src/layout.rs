//! Tabs and split panes.
//!
//! Deliberately free of GUI types: a [`Rect`] here is just four `f32`s, so the
//! whole module is unit-testable without an `egui::Context`. `render.rs` walks
//! [`Layout::rects`] and paints each pane's [`Session`](crate::session::Session)
//! (looked up by [`PaneId`]) into the returned rectangle.

use std::collections::HashSet;

pub type PaneId = u64;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Axis {
    /// Side by side: `a` on the left, `b` on the right.
    X,
    /// Stacked: `a` on top, `b` on the bottom.
    Y,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FocusDir {
    Left,
    Right,
    Up,
    Down,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Node {
    Leaf(PaneId),
    Split { axis: Axis, ratio: f32, a: Box<Node>, b: Box<Node> },
}

impl Node {
    fn leaves(&self, out: &mut Vec<PaneId>) {
        match self {
            Node::Leaf(id) => out.push(*id),
            Node::Split { a, b, .. } => {
                a.leaves(out);
                b.leaves(out);
            }
        }
    }

    fn contains(&self, id: PaneId) -> bool {
        match self {
            Node::Leaf(x) => *x == id,
            Node::Split { a, b, .. } => a.contains(id) || b.contains(id),
        }
    }
}

/// Normalised rectangle (0..1 on both axes); `render.rs` scales this to the
/// actual pane area in pixels.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Rect {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
}

impl Rect {
    pub const FULL: Rect = Rect { x0: 0.0, y0: 0.0, x1: 1.0, y1: 1.0 };

    pub fn center(&self) -> (f32, f32) {
        ((self.x0 + self.x1) * 0.5, (self.y0 + self.y1) * 0.5)
    }

    fn split(&self, axis: Axis, ratio: f32) -> (Rect, Rect) {
        let ratio = ratio.clamp(0.05, 0.95);
        match axis {
            Axis::X => {
                let mid = self.x0 + (self.x1 - self.x0) * ratio;
                (Rect { x1: mid, ..*self }, Rect { x0: mid, ..*self })
            }
            Axis::Y => {
                let mid = self.y0 + (self.y1 - self.y0) * ratio;
                (Rect { y1: mid, ..*self }, Rect { y0: mid, ..*self })
            }
        }
    }
}

const MIN_RATIO: f32 = 0.05;
const MAX_RATIO: f32 = 0.95;

#[derive(Clone, Debug)]
pub struct Tab {
    pub id: u64,
    /// User-set title; `None` follows the focused pane's own title (OSC 0/2).
    pub title: Option<String>,
    root: Node,
    focused: PaneId,
    zoomed: Option<PaneId>,
}

impl Tab {
    fn new(id: u64, pane: PaneId) -> Tab {
        Tab { id, title: None, root: Node::Leaf(pane), focused: pane, zoomed: None }
    }

    pub fn focused(&self) -> PaneId {
        self.focused
    }

    pub fn is_zoomed(&self) -> bool {
        self.zoomed.is_some()
    }

    pub fn panes(&self) -> Vec<PaneId> {
        let mut v = Vec::new();
        self.root.leaves(&mut v);
        v
    }

    fn focus(&mut self, id: PaneId) {
        if self.root.contains(id) {
            self.focused = id;
            if self.zoomed.is_some() {
                self.zoomed = Some(id);
            }
        }
    }

    /// Rect of every pane, honouring zoom (the zoomed pane alone fills the tab).
    pub fn rects(&self, area: Rect) -> Vec<(PaneId, Rect)> {
        if let Some(z) = self.zoomed {
            return vec![(z, area)];
        }
        let mut out = Vec::new();
        layout_rects(&self.root, area, &mut out);
        out
    }

    fn rect_of(&self, id: PaneId) -> Option<Rect> {
        self.rects(Rect::FULL).into_iter().find(|(p, _)| *p == id).map(|(_, r)| r)
    }
}

fn layout_rects(node: &Node, area: Rect, out: &mut Vec<(PaneId, Rect)>) {
    match node {
        Node::Leaf(id) => out.push((*id, area)),
        Node::Split { axis, ratio, a, b } => {
            let (ra, rb) = area.split(*axis, *ratio);
            layout_rects(a, ra, out);
            layout_rects(b, rb, out);
        }
    }
}

/// Split the leaf `target` inside `node` into `(target, axis, new)`, `new` on
/// the trailing side (right/bottom). Returns `true` if `target` was found.
fn split_leaf(node: &mut Node, target: PaneId, axis: Axis, new: PaneId, ratio: f32) -> bool {
    match node {
        Node::Leaf(id) if *id == target => {
            *node = Node::Split { axis, ratio, a: Box::new(Node::Leaf(target)), b: Box::new(Node::Leaf(new)) };
            true
        }
        Node::Leaf(_) => false,
        Node::Split { a, b, .. } => split_leaf(a, target, axis, new, ratio) || split_leaf(b, target, axis, new, ratio),
    }
}

/// Remove leaf `target` from `node`; the returned node replaces `*node`
/// (a split collapses into its remaining child). `None` means `node` itself
/// *was* the leaf being removed — the caller (a tab) has nothing left.
fn remove_leaf(node: &Node, target: PaneId) -> Option<Node> {
    match node {
        Node::Leaf(id) => {
            if *id == target {
                None
            } else {
                Some(node.clone())
            }
        }
        Node::Split { axis, ratio, a, b } => {
            let ra = remove_leaf(a, target);
            let rb = remove_leaf(b, target);
            match (ra, rb) {
                (Some(a2), Some(b2)) => Some(Node::Split { axis: *axis, ratio: *ratio, a: Box::new(a2), b: Box::new(b2) }),
                (Some(a2), None) => Some(a2),
                (None, Some(b2)) => Some(b2),
                (None, None) => None,
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Layout: all tabs of one window
// ---------------------------------------------------------------------------

pub struct Layout {
    tabs: Vec<Tab>,
    active: usize,
    next_tab_id: u64,
}

impl Layout {
    pub fn new(first_pane: PaneId) -> Layout {
        Layout { tabs: vec![Tab::new(1, first_pane)], active: 0, next_tab_id: 2 }
    }

    pub fn tabs(&self) -> &[Tab] {
        &self.tabs
    }

    pub fn active_index(&self) -> usize {
        self.active
    }

    pub fn active_tab(&self) -> &Tab {
        &self.tabs[self.active]
    }

    fn active_tab_mut(&mut self) -> &mut Tab {
        &mut self.tabs[self.active]
    }

    pub fn focused_pane(&self) -> PaneId {
        self.active_tab().focused
    }

    /// Every pane in every tab — what the caller must keep a live [`Session`] for.
    pub fn all_panes(&self) -> HashSet<PaneId> {
        self.tabs.iter().flat_map(|t| t.panes()).collect()
    }

    // ----- tabs -------------------------------------------------------

    pub fn new_tab(&mut self, pane: PaneId) -> usize {
        let id = self.next_tab_id;
        self.next_tab_id += 1;
        self.tabs.push(Tab::new(id, pane));
        self.active = self.tabs.len() - 1;
        self.active
    }

    /// Removes the tab containing `pane` if `pane` was its *only* pane, i.e.
    /// closing a pane inside a split never closes the tab; call this only
    /// once `close_pane` reports the tab is now empty.
    fn remove_empty_tab_at(&mut self, idx: usize) {
        if self.tabs.len() <= 1 {
            return; // the last tab never closes itself — the window does
        }
        self.tabs.remove(idx);
        if self.active >= self.tabs.len() {
            self.active = self.tabs.len() - 1;
        } else if idx < self.active {
            self.active -= 1;
        }
    }

    pub fn next_tab(&mut self) {
        if !self.tabs.is_empty() {
            self.active = (self.active + 1) % self.tabs.len();
        }
    }

    pub fn prev_tab(&mut self) {
        if !self.tabs.is_empty() {
            self.active = (self.active + self.tabs.len() - 1) % self.tabs.len();
        }
    }

    /// 1-based from the UI; `usize::MAX` (or anything ≥ len) means "last tab",
    /// matching the `alt+9` = last-tab convention.
    pub fn goto_tab(&mut self, one_based: usize) {
        if self.tabs.is_empty() {
            return;
        }
        let idx = one_based.saturating_sub(1).min(self.tabs.len() - 1);
        self.active = idx;
    }

    // ----- panes --------------------------------------------------------

    pub fn split(&mut self, axis: Axis, new_pane: PaneId) {
        let target = self.focused_pane();
        let tab = self.active_tab_mut();
        tab.zoomed = None;
        if split_leaf(&mut tab.root, target, axis, new_pane, 0.5) {
            tab.focused = new_pane;
        }
    }

    /// Closes `pane`. Returns `true` if that closed the whole tab (it was the
    /// tab's only pane) — the caller should end the underlying session either
    /// way; this only reports whether the *tab* also needs cleanup elsewhere.
    pub fn close_pane(&mut self, pane: PaneId) -> bool {
        let idx = match self.tabs.iter().position(|t| t.panes().contains(&pane)) {
            Some(i) => i,
            None => return false,
        };
        let was_focused = self.tabs[idx].focused == pane;
        match remove_leaf(&self.tabs[idx].root, pane) {
            Some(new_root) => {
                self.tabs[idx].root = new_root;
                if self.tabs[idx].zoomed == Some(pane) {
                    self.tabs[idx].zoomed = None;
                }
                if was_focused {
                    let first = self.tabs[idx].panes().first().copied();
                    if let Some(p) = first {
                        self.tabs[idx].focused = p;
                    }
                }
                false
            }
            None => {
                self.remove_empty_tab_at(idx);
                true
            }
        }
    }

    pub fn focus(&mut self, pane: PaneId) {
        self.active_tab_mut().focus(pane);
    }

    pub fn toggle_zoom(&mut self) {
        let tab = self.active_tab_mut();
        tab.zoomed = if tab.zoomed.is_some() { None } else { Some(tab.focused) };
    }

    /// Move focus to the nearest pane in `dir`, by comparing rect centres
    /// (ties broken by the smaller perpendicular offset, then reading order).
    pub fn focus_direction(&mut self, dir: FocusDir) {
        let tab = self.active_tab();
        if tab.zoomed.is_some() {
            return;
        }
        let rects = tab.rects(Rect::FULL);
        let cur = match tab.rect_of(tab.focused) {
            Some(r) => r,
            None => return,
        };
        let (cx, cy) = cur.center();
        let mut best: Option<(f32, f32, PaneId)> = None;
        for (id, r) in &rects {
            if *id == tab.focused {
                continue;
            }
            let (x, y) = r.center();
            let (primary, perp, ok) = match dir {
                FocusDir::Left => (cx - x, (cy - y).abs(), x < cx - 1e-4),
                FocusDir::Right => (x - cx, (cy - y).abs(), x > cx + 1e-4),
                FocusDir::Up => (cy - y, (cx - x).abs(), y < cy - 1e-4),
                FocusDir::Down => (y - cy, (cx - x).abs(), y > cy + 1e-4),
            };
            if !ok {
                continue;
            }
            let key = (perp, primary);
            match &best {
                Some((bp, bpe, _)) if (*bp, *bpe) <= key => {}
                _ => best = Some((key.0, key.1, *id)),
            }
        }
        if let Some((_, _, id)) = best {
            self.active_tab_mut().focus(id);
        }
    }
}

/// Clamp a user-dragged divider ratio.
pub fn clamp_ratio(r: f32) -> f32 {
    r.clamp(MIN_RATIO, MAX_RATIO)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_layout_has_one_tab_one_pane() {
        let l = Layout::new(1);
        assert_eq!(l.tabs().len(), 1);
        assert_eq!(l.focused_pane(), 1);
        assert_eq!(l.active_tab().panes(), vec![1]);
    }

    #[test]
    fn split_creates_two_leaves_and_focuses_the_new_one() {
        let mut l = Layout::new(1);
        l.split(Axis::X, 2);
        assert_eq!(l.focused_pane(), 2);
        let mut panes = l.active_tab().panes();
        panes.sort();
        assert_eq!(panes, vec![1, 2]);
        let rects = l.active_tab().rects(Rect::FULL);
        assert_eq!(rects.len(), 2);
        let r1 = rects.iter().find(|(p, _)| *p == 1).unwrap().1;
        let r2 = rects.iter().find(|(p, _)| *p == 2).unwrap().1;
        assert!((r1.x1 - r2.x0).abs() < 1e-6, "panes should be adjacent");
        assert!(r1.x0 < r1.x1 && r2.x0 < r2.x1);
    }

    #[test]
    fn splitting_the_focused_pane_nests_correctly() {
        let mut l = Layout::new(1);
        l.split(Axis::X, 2); // [1 | 2], focus 2
        l.split(Axis::Y, 3); // 2 becomes [2 / 3], focus 3
        let mut panes = l.active_tab().panes();
        panes.sort();
        assert_eq!(panes, vec![1, 2, 3]);
        let rects = l.active_tab().rects(Rect::FULL);
        let r2 = rects.iter().find(|(p, _)| *p == 2).unwrap().1;
        let r3 = rects.iter().find(|(p, _)| *p == 3).unwrap().1;
        assert!((r2.y1 - r3.y0).abs() < 1e-6);
        assert!((r2.x0 - r3.x0).abs() < 1e-6, "still in the same column as each other");
    }

    #[test]
    fn close_pane_collapses_split() {
        let mut l = Layout::new(1);
        l.split(Axis::X, 2);
        assert!(!l.close_pane(2));
        assert_eq!(l.active_tab().panes(), vec![1]);
        assert_eq!(l.focused_pane(), 1, "focus falls back to the remaining pane");
    }

    #[test]
    fn closing_last_pane_in_a_non_final_tab_closes_the_tab() {
        let mut l = Layout::new(1);
        l.new_tab(2);
        assert_eq!(l.tabs().len(), 2);
        assert!(l.close_pane(2));
        assert_eq!(l.tabs().len(), 1);
        assert_eq!(l.active_index(), 0);
    }

    #[test]
    fn closing_the_only_pane_of_the_only_tab_leaves_the_tab_in_place() {
        let mut l = Layout::new(1);
        assert!(l.close_pane(1), "reports the tab as empty");
        // caller is expected to then close the whole window; the layout itself
        // does not delete the very last tab
        assert_eq!(l.tabs().len(), 1);
    }

    #[test]
    fn tab_navigation_wraps_and_goto_clamps() {
        let mut l = Layout::new(1);
        l.new_tab(2);
        l.new_tab(3);
        assert_eq!(l.active_index(), 2);
        l.next_tab();
        assert_eq!(l.active_index(), 0, "wraps around");
        l.prev_tab();
        assert_eq!(l.active_index(), 2);
        l.goto_tab(1);
        assert_eq!(l.active_index(), 0);
        l.goto_tab(99);
        assert_eq!(l.active_index(), 2, "out-of-range clamps to the last tab");
    }

    #[test]
    fn zoom_shows_only_the_focused_pane_and_restores() {
        let mut l = Layout::new(1);
        l.split(Axis::X, 2);
        l.toggle_zoom();
        assert!(l.active_tab().is_zoomed());
        let rects = l.active_tab().rects(Rect::FULL);
        assert_eq!(rects, vec![(2, Rect::FULL)]);
        l.toggle_zoom();
        assert!(!l.active_tab().is_zoomed());
        assert_eq!(l.active_tab().rects(Rect::FULL).len(), 2);
    }

    #[test]
    fn focus_direction_in_a_2x2_grid() {
        // [1 | 2]
        // [3 | 4]
        let mut l = Layout::new(1);
        l.split(Axis::X, 2); // focus 2
        l.focus(1);
        l.split(Axis::Y, 3); // 1 -> [1 / 3], focus 3
        l.focus(2);
        l.split(Axis::Y, 4); // 2 -> [2 / 4], focus 4
        l.focus(1);
        l.focus_direction(FocusDir::Right);
        assert_eq!(l.focused_pane(), 2);
        l.focus_direction(FocusDir::Down);
        assert_eq!(l.focused_pane(), 4);
        l.focus_direction(FocusDir::Left);
        assert_eq!(l.focused_pane(), 3);
        l.focus_direction(FocusDir::Up);
        assert_eq!(l.focused_pane(), 1);
        // no pane further up/left of the corner: focus does not move
        l.focus_direction(FocusDir::Up);
        assert_eq!(l.focused_pane(), 1);
        l.focus_direction(FocusDir::Left);
        assert_eq!(l.focused_pane(), 1);
    }

    #[test]
    fn zoom_disables_focus_direction() {
        let mut l = Layout::new(1);
        l.split(Axis::X, 2);
        l.toggle_zoom();
        l.focus_direction(FocusDir::Left);
        assert_eq!(l.focused_pane(), 2, "zoomed: directional focus is a no-op");
    }

    #[test]
    fn all_panes_spans_every_tab() {
        let mut l = Layout::new(1);
        l.split(Axis::X, 2);
        l.new_tab(3);
        let mut all: Vec<_> = l.all_panes().into_iter().collect();
        all.sort();
        assert_eq!(all, vec![1, 2, 3]);
    }

    #[test]
    fn ratio_is_clamped() {
        assert_eq!(clamp_ratio(-1.0), MIN_RATIO);
        assert_eq!(clamp_ratio(2.0), MAX_RATIO);
        assert_eq!(clamp_ratio(0.5), 0.5);
    }
}
