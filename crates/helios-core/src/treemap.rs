//! Squarified treemap layout.
//!
//! Layout runs in Rust, not in the renderer. A 200k-rectangle layout is ~40 ms
//! of arithmetic in Rust and several hundred milliseconds of allocation churn
//! in JavaScript, and doing it here means the UI thread only ever paints a flat
//! array of tiles onto a canvas — resizing stays smooth because the expensive
//! part happens off the main thread.
//!
//! The algorithm is Bruls, Huizing & van Wijk's squarified treemap: at each
//! step, keep adding children to the current row while doing so improves the
//! worst aspect ratio in that row, then lay the row out and recurse on what is
//! left. Rectangles come out close to square, which is what makes relative
//! areas readable at a glance.

use serde::{Deserialize, Serialize};

use crate::category::Category;
use crate::model::{NodeFlags, NodeId, Tree};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn new(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect { x, y, w, h }
    }

    pub fn area(&self) -> f32 {
        self.w.max(0.0) * self.h.max(0.0)
    }

    fn shrink(&self, by: f32) -> Rect {
        Rect {
            x: self.x + by,
            y: self.y + by,
            w: (self.w - by * 2.0).max(0.0),
            h: (self.h - by * 2.0).max(0.0),
        }
    }

    #[cfg(test)]
    fn intersects(&self, other: &Rect) -> bool {
        self.x < other.x + other.w
            && other.x < self.x + self.w
            && self.y < other.y + other.h
            && other.y < self.y + self.h
    }
}

/// One rectangle to paint.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Tile {
    pub id: u32,
    pub name: String,
    pub size: u64,
    pub category: Category,
    pub is_dir: bool,
    /// Depth relative to the treemap root, for shading and hit-testing order.
    pub depth: u16,
    pub rect: Rect,
    /// True when the node has children that were too small to draw. The UI
    /// renders these with a subtle stipple so "there is more inside" is visible.
    pub truncated: bool,
}

#[derive(Debug, Clone)]
pub struct TreemapOptions {
    /// Stop descending below this many levels.
    pub max_depth: u16,
    /// Rectangles smaller than this on either side are not emitted — below a
    /// few pixels a tile is noise the user cannot click anyway.
    pub min_side: f32,
    /// Hard cap on emitted tiles, protecting the renderer from pathological
    /// trees.
    pub max_tiles: usize,
    /// Inset applied to each nested level, which is what makes the hierarchy
    /// legible without drawing borders.
    pub padding: f32,
    pub include_hidden: bool,
}

impl Default for TreemapOptions {
    fn default() -> Self {
        TreemapOptions {
            max_depth: 6,
            min_side: 3.0,
            max_tiles: 20_000,
            padding: 1.0,
            include_hidden: true,
        }
    }
}

/// Lays out the subtree rooted at `root` inside `viewport`.
pub fn layout(tree: &Tree, root: NodeId, viewport: Rect, opts: &TreemapOptions) -> Vec<Tile> {
    let mut tiles = Vec::new();
    layout_into(tree, root, viewport, opts, 0, &mut tiles);
    tiles
}

fn layout_into(
    tree: &Tree,
    parent: NodeId,
    rect: Rect,
    opts: &TreemapOptions,
    depth: u16,
    out: &mut Vec<Tile>,
) {
    if depth >= opts.max_depth || out.len() >= opts.max_tiles {
        return;
    }
    if rect.w < opts.min_side || rect.h < opts.min_side {
        return;
    }

    let mut items: Vec<(NodeId, u64)> = tree
        .children(parent)
        .filter(|id| {
            let node = tree.node(*id);
            node.logical_size > 0
                && !node.flags.contains(NodeFlags::HARDLINK_DUP)
                && (opts.include_hidden || !node.flags.contains(NodeFlags::HIDDEN))
        })
        .map(|id| (id, tree.node(id).logical_size))
        .collect();
    if items.is_empty() {
        return;
    }
    // Squarification assumes descending input; without it aspect ratios degrade
    // badly for skewed distributions, which is exactly what disks are.
    items.sort_unstable_by_key(|(_, size)| std::cmp::Reverse(*size));

    let total: u64 = items.iter().map(|(_, s)| *s).sum();
    let placed = squarify(&items, total, rect);

    for (id, tile_rect) in placed {
        if tile_rect.w < opts.min_side || tile_rect.h < opts.min_side {
            // The first tile too small to draw means every later (smaller) one
            // is too: mark the parent truncated and stop.
            if let Some(last) = out.last_mut() {
                last.truncated = true;
            }
            break;
        }
        if out.len() >= opts.max_tiles {
            return;
        }

        let node = tree.node(id);
        out.push(Tile {
            id: id.0,
            name: tree.name(id).to_string(),
            size: node.logical_size,
            category: node.category,
            is_dir: node.is_dir(),
            depth,
            rect: tile_rect,
            truncated: false,
        });

        if node.is_dir() && !node.flags.contains(NodeFlags::PACKAGE) {
            let inner = tile_rect.shrink(opts.padding);
            layout_into(tree, id, inner, opts, depth + 1, out);
        }
    }
}

/// The squarified layout kernel: places `items` (descending by size) into
/// `rect`, returning one rectangle per item.
fn squarify(items: &[(NodeId, u64)], total: u64, rect: Rect) -> Vec<(NodeId, Rect)> {
    let mut out = Vec::with_capacity(items.len());
    if total == 0 || rect.area() <= 0.0 {
        return out;
    }

    // Work in area units so no precision is lost converting sizes repeatedly.
    let scale = rect.area() as f64 / total as f64;
    let mut remaining = rect;
    let mut i = 0;

    while i < items.len() {
        let short_side = remaining.w.min(remaining.h) as f64;
        if short_side <= 0.0 {
            break;
        }

        // Grow the row while the worst aspect ratio keeps improving.
        let mut row_area = 0.0f64;
        let mut row_end = i;
        let mut best_ratio = f64::MAX;
        while row_end < items.len() {
            let area = items[row_end].1 as f64 * scale;
            let candidate = row_area + area;
            let ratio = worst_ratio(
                candidate,
                items[i].1 as f64 * scale,
                items[row_end].1 as f64 * scale,
                short_side,
            );
            if row_end > i && ratio > best_ratio {
                break;
            }
            best_ratio = ratio;
            row_area = candidate;
            row_end += 1;
        }

        // Lay the row across the shorter side, so rows stay square-ish.
        let horizontal = remaining.w <= remaining.h;
        let thickness = (row_area / short_side) as f32;
        let mut offset = 0.0f32;

        for (id, size) in &items[i..row_end] {
            let fraction = if row_area > 0.0 {
                (*size as f64 * scale / row_area) as f32
            } else {
                0.0
            };
            let along = fraction * short_side as f32;
            let tile = if horizontal {
                Rect::new(remaining.x + offset, remaining.y, along, thickness)
            } else {
                Rect::new(remaining.x, remaining.y + offset, thickness, along)
            };
            out.push((*id, tile));
            offset += along;
        }

        if horizontal {
            remaining = Rect::new(
                remaining.x,
                remaining.y + thickness,
                remaining.w,
                (remaining.h - thickness).max(0.0),
            );
        } else {
            remaining = Rect::new(
                remaining.x + thickness,
                remaining.y,
                (remaining.w - thickness).max(0.0),
                remaining.h,
            );
        }
        i = row_end;
    }
    out
}

/// Worst aspect ratio in a row of total area `row`, whose largest and smallest
/// members are `max` and `min`, laid along a side of length `side`.
fn worst_ratio(row: f64, max: f64, min: f64, side: f64) -> f64 {
    if row <= 0.0 || min <= 0.0 {
        return f64::MAX;
    }
    let side_sq = side * side;
    let row_sq = row * row;
    ((side_sq * max) / row_sq).max(row_sq / (side_sq * min))
}

/// Finds the deepest tile under a point. Tiles are emitted parent-before-child,
/// so scanning backwards returns the innermost hit first.
pub fn hit_test(tiles: &[Tile], x: f32, y: f32) -> Option<&Tile> {
    tiles.iter().rev().find(|t| {
        x >= t.rect.x && x < t.rect.x + t.rect.w && y >= t.rect.y && y < t.rect.y + t.rect.h
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::NodeFlags;

    fn tree() -> Tree {
        let mut t = Tree::new("/root");
        let a = t.push_node("a", NodeId::ROOT, 1, NodeFlags::DIRECTORY, Category::Other, 0, 0, 0);
        t.push_node("a1", a, 2, NodeFlags::empty(), Category::Videos, 300, 300, 0);
        t.push_node("a2", a, 2, NodeFlags::empty(), Category::Images, 300, 300, 0);
        t.push_node("b", NodeId::ROOT, 1, NodeFlags::empty(), Category::Audio, 300, 300, 0);
        t.push_node("c", NodeId::ROOT, 1, NodeFlags::empty(), Category::Other, 100, 100, 0);
        t.rollup();
        t
    }

    #[test]
    fn top_level_tiles_fill_the_viewport_proportionally() {
        let t = tree();
        let viewport = Rect::new(0.0, 0.0, 1000.0, 500.0);
        let tiles = layout(&t, NodeId::ROOT, viewport, &TreemapOptions::default());

        let top: Vec<&Tile> = tiles.iter().filter(|t| t.depth == 0).collect();
        assert_eq!(top.len(), 3);

        let covered: f32 = top.iter().map(|t| t.rect.area()).sum();
        assert!(
            (covered - viewport.area()).abs() / viewport.area() < 0.001,
            "top level should tile the viewport, covered {covered}"
        );

        // 'a' holds 600 of 1000 bytes, so it should hold ~60% of the area.
        let a = top.iter().find(|t| t.name == "a").unwrap();
        assert!((a.rect.area() / viewport.area() - 0.6).abs() < 0.01);
    }

    #[test]
    fn sibling_tiles_never_overlap() {
        let t = tree();
        let tiles = layout(
            &t,
            NodeId::ROOT,
            Rect::new(0.0, 0.0, 800.0, 600.0),
            &TreemapOptions::default(),
        );
        let top: Vec<&Tile> = tiles.iter().filter(|t| t.depth == 0).collect();
        for (i, a) in top.iter().enumerate() {
            for b in &top[i + 1..] {
                assert!(!a.rect.intersects(&b.rect), "{a:?} overlaps {b:?}");
            }
        }
    }

    #[test]
    fn children_are_nested_inside_their_parent() {
        let t = tree();
        let tiles = layout(
            &t,
            NodeId::ROOT,
            Rect::new(0.0, 0.0, 800.0, 600.0),
            &TreemapOptions::default(),
        );
        let parent = tiles.iter().find(|t| t.name == "a").unwrap();
        for child in tiles.iter().filter(|t| t.depth == 1) {
            assert!(child.rect.x >= parent.rect.x - 0.01);
            assert!(child.rect.y >= parent.rect.y - 0.01);
            assert!(child.rect.x + child.rect.w <= parent.rect.x + parent.rect.w + 0.01);
            assert!(child.rect.y + child.rect.h <= parent.rect.y + parent.rect.h + 0.01);
        }
    }

    #[test]
    fn aspect_ratios_stay_reasonable() {
        let t = tree();
        let tiles = layout(
            &t,
            NodeId::ROOT,
            Rect::new(0.0, 0.0, 900.0, 600.0),
            &TreemapOptions::default(),
        );
        for tile in tiles.iter().filter(|t| t.depth == 0) {
            let ratio = (tile.rect.w / tile.rect.h).max(tile.rect.h / tile.rect.w);
            assert!(ratio < 6.0, "{} has aspect ratio {ratio}", tile.name);
        }
    }

    #[test]
    fn tiny_viewports_and_empty_trees_produce_nothing() {
        let t = tree();
        let opts = TreemapOptions::default();
        assert!(layout(&t, NodeId::ROOT, Rect::new(0.0, 0.0, 1.0, 1.0), &opts).is_empty());

        let empty = Tree::new("/empty");
        assert!(layout(&empty, NodeId::ROOT, Rect::new(0.0, 0.0, 500.0, 500.0), &opts).is_empty());
    }

    #[test]
    fn hit_test_returns_the_innermost_tile() {
        let t = tree();
        let tiles = layout(
            &t,
            NodeId::ROOT,
            Rect::new(0.0, 0.0, 800.0, 600.0),
            &TreemapOptions::default(),
        );
        let child = tiles.iter().find(|t| t.depth == 1).unwrap().clone();
        let hit = hit_test(&tiles, child.rect.x + child.rect.w / 2.0, child.rect.y + child.rect.h / 2.0);
        assert_eq!(hit.map(|t| t.depth), Some(1));
    }

    #[test]
    fn tile_count_is_capped() {
        let mut t = Tree::new("/root");
        for i in 0..5000 {
            t.push_node(
                &format!("f{i}"),
                NodeId::ROOT,
                1,
                NodeFlags::empty(),
                Category::Other,
                1000,
                1000,
                0,
            );
        }
        t.rollup();
        let opts = TreemapOptions {
            max_tiles: 50,
            min_side: 0.0,
            ..TreemapOptions::default()
        };
        assert!(layout(&t, NodeId::ROOT, Rect::new(0.0, 0.0, 1000.0, 1000.0), &opts).len() <= 50);
    }
}
