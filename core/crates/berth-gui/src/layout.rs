//! Pixel geometry for the portable window.
//!
//! The core's split tree is interpreted here and nowhere else: leaves become
//! pane rectangles, split boundaries become draggable dividers. Weights come
//! from the core, so the same tree lays out identically after a restore.

use berth_core::PaneNode;
use berth_core::model::Orientation;

/// Width of the gap between two children of a split.
pub const DIVIDER: usize = 2;
/// How far from a divider a drag still grabs it.
pub const DIVIDER_GRAB: usize = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: usize,
    pub y: usize,
    pub w: usize,
    pub h: usize,
}

impl Rect {
    pub const fn new(x: usize, y: usize, w: usize, h: usize) -> Self {
        Self { x, y, w, h }
    }

    pub const fn right(self) -> usize {
        self.x.saturating_add(self.w)
    }

    pub const fn bottom(self) -> usize {
        self.y.saturating_add(self.h)
    }

    pub fn inset(self, padding: usize) -> Self {
        Self {
            x: self.x.saturating_add(padding),
            y: self.y.saturating_add(padding),
            w: self.w.saturating_sub(padding.saturating_mul(2)),
            h: self.h.saturating_sub(padding.saturating_mul(2)),
        }
    }

    pub fn contains(self, x: usize, y: usize) -> bool {
        self.w > 0
            && self.h > 0
            && x >= self.x
            && y >= self.y
            && x < self.right()
            && y < self.bottom()
    }

    /// Grow the rectangle on every side, staying inside the window.
    pub fn expand(self, by: usize) -> Self {
        Self {
            x: self.x.saturating_sub(by),
            y: self.y.saturating_sub(by),
            w: self.w.saturating_add(by.saturating_mul(2)),
            h: self.h.saturating_add(by.saturating_mul(2)),
        }
    }

    pub fn intersect(self, other: Self) -> Option<Self> {
        let x = self.x.max(other.x);
        let y = self.y.max(other.y);
        let right = self.right().min(other.right());
        let bottom = self.bottom().min(other.bottom());
        (right > x && bottom > y).then(|| Self::new(x, y, right - x, bottom - y))
    }
}

/// One terminal pane's placement in the window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Leaf {
    pub pane_id: String,
    pub rect: Rect,
}

/// A split boundary the user can drag.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Divider {
    pub split_id: String,
    /// Index of the child on the leading side of this boundary.
    pub child: usize,
    pub orientation: Orientation,
    /// The drawn band.
    pub band: Rect,
    /// The grab area, `band` grown by [`DIVIDER_GRAB`].
    pub grab: Rect,
    /// Start of the split's content along its axis.
    pub start: usize,
    /// Content extent along its axis, gap excluded.
    pub extent: usize,
}

/// Walk the split tree, assigning rectangles to leaves and dividers.
pub fn layout(root: &PaneNode, area: Rect) -> (Vec<Leaf>, Vec<Divider>) {
    let mut leaves = Vec::new();
    let mut dividers = Vec::new();
    walk(root, area, &mut leaves, &mut dividers);
    (leaves, dividers)
}

fn walk(node: &PaneNode, rect: Rect, leaves: &mut Vec<Leaf>, dividers: &mut Vec<Divider>) {
    match node {
        PaneNode::Leaf { pane_id } => leaves.push(Leaf {
            pane_id: pane_id.clone(),
            rect,
        }),
        PaneNode::Split {
            id,
            orientation,
            children,
            weights,
        } => {
            let count = children.len();
            if count == 0 {
                return;
            }
            let axis_start = match orientation {
                Orientation::Horizontal => rect.x,
                Orientation::Vertical => rect.y,
            };
            let axis_extent = match orientation {
                Orientation::Horizontal => rect.w,
                Orientation::Vertical => rect.h,
            };
            let gaps = DIVIDER.saturating_mul(count.saturating_sub(1));
            let available = axis_extent.saturating_sub(gaps);
            let resolved = resolve_weights(weights, count);
            let total: f32 = resolved.iter().sum();
            let mut consumed = 0usize;
            let mut offset = 0usize;
            for (index, child) in children.iter().enumerate() {
                let extent = if index + 1 == count {
                    available.saturating_sub(consumed)
                } else {
                    let share = available as f32 * (resolved[index] / total);
                    (share.round() as usize).min(available.saturating_sub(consumed))
                };
                consumed = consumed.saturating_add(extent);
                let child_rect = match orientation {
                    Orientation::Horizontal => {
                        Rect::new(rect.x.saturating_add(offset), rect.y, extent, rect.h)
                    }
                    Orientation::Vertical => {
                        Rect::new(rect.x, rect.y.saturating_add(offset), rect.w, extent)
                    }
                };
                walk(child, child_rect, leaves, dividers);
                let next = offset.saturating_add(extent);
                if index + 1 < count {
                    let band = match orientation {
                        Orientation::Horizontal => {
                            Rect::new(rect.x.saturating_add(next), rect.y, DIVIDER, rect.h)
                        }
                        Orientation::Vertical => {
                            Rect::new(rect.x, rect.y.saturating_add(next), rect.w, DIVIDER)
                        }
                    };
                    dividers.push(Divider {
                        split_id: id.clone(),
                        child: index,
                        orientation: *orientation,
                        band,
                        grab: band.expand(DIVIDER_GRAB),
                        start: axis_start,
                        extent: available,
                    });
                }
                offset = next.saturating_add(DIVIDER);
            }
        }
    }
}

/// Weights as the core stored them: one finite positive value per child.
fn resolve_weights(weights: &[f32], count: usize) -> Vec<f32> {
    (0..count)
        .map(|index| {
            weights
                .get(index)
                .copied()
                .filter(|weight| weight.is_finite() && *weight > 0.0)
                .unwrap_or(1.0)
        })
        .collect()
}

/// The share a dragged divider gives its leading child, keeping both sides
/// usable. `pointer` is a window coordinate along the split's axis.
pub fn dragged_weight(divider: &Divider, pointer: usize) -> f32 {
    if divider.extent == 0 {
        return 0.5;
    }
    let travelled = pointer.saturating_sub(divider.start);
    (travelled as f32 / divider.extent as f32).clamp(0.05, 0.95)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn split(orientation: Orientation, weights: Vec<f32>, children: Vec<PaneNode>) -> PaneNode {
        PaneNode::Split {
            id: "split".into(),
            orientation,
            children,
            weights,
        }
    }

    fn leaf(id: &str) -> PaneNode {
        PaneNode::leaf(id)
    }

    fn area(width: usize, height: usize) -> Rect {
        Rect::new(0, 0, width, height)
    }

    #[test]
    fn given_one_leaf_when_laid_out_then_it_fills_the_area() {
        let (leaves, dividers) = layout(&leaf("one"), area(100, 60));
        assert_eq!(leaves.len(), 1);
        assert_eq!(leaves[0].pane_id, "one");
        assert_eq!(leaves[0].rect, area(100, 60));
        assert!(dividers.is_empty());
    }

    #[test]
    fn given_a_horizontal_split_when_laid_out_then_children_share_width() {
        let node = split(
            Orientation::Horizontal,
            vec![0.7, 0.3],
            vec![leaf("left"), leaf("right")],
        );
        let (leaves, dividers) = layout(&node, area(100, 50));
        assert_eq!(leaves[0].rect, Rect::new(0, 0, 69, 50));
        assert_eq!(leaves[1].rect, Rect::new(71, 0, 29, 50));
        assert_eq!(dividers.len(), 1);
        assert_eq!(dividers[0].split_id, "split");
        assert_eq!(dividers[0].child, 0);
        assert_eq!(dividers[0].band, Rect::new(69, 0, DIVIDER, 50));
        assert_eq!(dividers[0].extent, 98);
    }

    #[test]
    fn given_a_vertical_split_when_laid_out_then_children_share_height() {
        let node = split(
            Orientation::Vertical,
            vec![1.0, 1.0],
            vec![leaf("top"), leaf("bottom")],
        );
        let (leaves, _) = layout(&node, area(80, 60));
        assert_eq!(leaves[0].rect, Rect::new(0, 0, 80, 29));
        assert_eq!(leaves[1].rect, Rect::new(0, 31, 80, 29));
    }

    #[test]
    fn given_nested_splits_when_laid_out_then_leaves_stay_inside_their_parent() {
        let node = split(
            Orientation::Horizontal,
            vec![0.5, 0.5],
            vec![
                leaf("left"),
                split(
                    Orientation::Vertical,
                    vec![0.5, 0.5],
                    vec![leaf("top-right"), leaf("bottom-right")],
                ),
            ],
        );
        let (leaves, dividers) = layout(&node, area(100, 60));
        assert_eq!(leaves.len(), 3);
        assert_eq!(leaves[0].rect, Rect::new(0, 0, 49, 60));
        assert_eq!(leaves[1].rect, Rect::new(51, 0, 49, 29));
        assert_eq!(leaves[2].rect, Rect::new(51, 31, 49, 29));
        assert_eq!(dividers.len(), 2);
        assert_eq!(dividers[1].orientation, Orientation::Vertical);
    }

    #[test]
    fn given_a_three_way_split_when_laid_out_then_every_gap_is_a_divider() {
        let node = split(
            Orientation::Horizontal,
            vec![1.0, 1.0, 1.0],
            vec![leaf("a"), leaf("b"), leaf("c")],
        );
        let (leaves, dividers) = layout(&node, area(104, 10));
        assert_eq!(dividers.len(), 2);
        assert_eq!(dividers[0].child, 0);
        assert_eq!(dividers[1].child, 1);
        assert_eq!(leaves[0].rect.w + leaves[1].rect.w + leaves[2].rect.w, 100);
    }

    #[test]
    fn given_missing_weights_when_laid_out_then_children_are_even() {
        let node = split(Orientation::Horizontal, vec![], vec![leaf("a"), leaf("b")]);
        let (leaves, _) = layout(&node, area(100, 10));
        assert_eq!(leaves[0].rect.w, 49);
        assert_eq!(leaves[1].rect.w, 49);
    }

    #[test]
    fn given_a_divider_when_dragged_then_the_share_follows_the_pointer() {
        let node = split(
            Orientation::Horizontal,
            vec![0.5, 0.5],
            vec![leaf("a"), leaf("b")],
        );
        let (_, dividers) = layout(&node, area(200, 10));
        let divider = &dividers[0];
        assert!((dragged_weight(divider, 99) - 0.5).abs() < 1e-6);
        assert!((dragged_weight(divider, 50) - (50.0 / 198.0)).abs() < 1e-6);
        assert!((dragged_weight(divider, 0) - 0.05).abs() < 1e-6);
        assert!((dragged_weight(divider, 10_000) - 0.95).abs() < 1e-6);
    }

    #[test]
    fn given_a_rect_when_expanded_then_the_grab_area_covers_the_band() {
        let rect = Rect::new(20, 10, 2, 40);
        let grab = rect.expand(DIVIDER_GRAB);
        assert!(grab.contains(17, 30));
        assert!(grab.contains(24, 30));
        assert!(rect.inset(1).w == 0);
    }

    #[test]
    fn given_two_rects_when_intersected_then_the_overlap_is_returned() {
        let a = Rect::new(0, 0, 10, 10);
        let b = Rect::new(5, 5, 10, 10);
        assert_eq!(a.intersect(b), Some(Rect::new(5, 5, 5, 5)));
        assert_eq!(a.intersect(Rect::new(20, 20, 1, 1)), None);
    }
}
