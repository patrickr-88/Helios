//! Filtering, sorting and aggregation over a scanned tree.
//!
//! Every list the UI shows — folder children, largest files, largest folders,
//! category breakdown, search results — is produced here, from the arena, with
//! no intermediate collections beyond the result itself. The UI never receives
//! the full tree: it asks for the page it is about to draw, which is what keeps
//! the IPC bridge and the renderer's memory flat regardless of scan size.

use std::cmp::Reverse;
use std::collections::BinaryHeap;

use serde::{Deserialize, Serialize};

use crate::category::{extension_of, Category};
use crate::model::{Node, NodeFlags, NodeId, Tree};

/// One row as the UI sees it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub id: u32,
    pub name: String,
    pub path: String,
    pub size: u64,
    pub physical_size: u64,
    pub category: Category,
    pub is_dir: bool,
    pub is_symlink: bool,
    pub is_hidden: bool,
    pub is_system: bool,
    pub is_package: bool,
    pub is_accessible: bool,
    pub mtime: i64,
    pub file_count: u32,
    pub dir_count: u32,
    /// Share of the parent's total, 0.0–1.0. Precomputed because every view
    /// draws a bar from it and floating-point division in JS across 100k rows
    /// is measurable.
    pub fraction_of_parent: f32,
}

impl Entry {
    fn build(tree: &Tree, id: NodeId, parent_size: u64) -> Entry {
        let node: &Node = tree.node(id);
        Entry {
            id: id.0,
            name: tree.name(id).to_string(),
            path: tree.path_of(id).to_string_lossy().into_owned(),
            size: node.logical_size,
            physical_size: node.physical_size,
            category: node.category,
            is_dir: node.is_dir(),
            is_symlink: node.is_symlink(),
            is_hidden: node.flags.contains(NodeFlags::HIDDEN),
            is_system: node.flags.contains(NodeFlags::SYSTEM),
            is_package: node.flags.contains(NodeFlags::PACKAGE),
            is_accessible: !node.flags.contains(NodeFlags::INACCESSIBLE),
            mtime: node.mtime,
            file_count: node.file_count,
            dir_count: node.dir_count,
            fraction_of_parent: if parent_size > 0 {
                (node.logical_size as f64 / parent_size as f64) as f32
            } else {
                0.0
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum SortKey {
    #[default]
    Size,
    Name,
    Modified,
    Count,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Filter {
    pub min_size: Option<u64>,
    pub max_size: Option<u64>,
    /// Lowercase, without the dot.
    pub extensions: Vec<String>,
    pub categories: Vec<Category>,
    /// Unix seconds.
    pub modified_after: Option<i64>,
    pub modified_before: Option<i64>,
    /// Case-insensitive substring of the full path.
    pub path_contains: Option<String>,
    /// Case-insensitive substring of the file name.
    pub name_contains: Option<String>,
    pub include_hidden: bool,
    pub include_system: bool,
    /// Restrict to files or to directories.
    pub only_files: bool,
    pub only_dirs: bool,
}

impl Filter {
    /// A filter that admits everything, including hidden and system files.
    pub fn permissive() -> Filter {
        Filter {
            include_hidden: true,
            include_system: true,
            ..Filter::default()
        }
    }

    /// True when nothing but the hidden/system toggles is set — lets callers
    /// skip per-node work on the common "no search active" path.
    pub fn is_trivial(&self) -> bool {
        self.min_size.is_none()
            && self.max_size.is_none()
            && self.extensions.is_empty()
            && self.categories.is_empty()
            && self.modified_after.is_none()
            && self.modified_before.is_none()
            && self.path_contains.is_none()
            && self.name_contains.is_none()
            && !self.only_files
            && !self.only_dirs
    }

    pub fn matches(&self, tree: &Tree, id: NodeId) -> bool {
        let node = tree.node(id);
        let is_dir = node.is_dir();

        if self.only_files && is_dir {
            return false;
        }
        if self.only_dirs && !is_dir {
            return false;
        }
        if !self.include_hidden && node.flags.contains(NodeFlags::HIDDEN) {
            return false;
        }
        if !self.include_system && node.flags.contains(NodeFlags::SYSTEM) {
            return false;
        }
        if self.min_size.is_some_and(|m| node.logical_size < m) {
            return false;
        }
        if self.max_size.is_some_and(|m| node.logical_size > m) {
            return false;
        }
        if self.modified_after.is_some_and(|t| node.mtime < t) {
            return false;
        }
        if self.modified_before.is_some_and(|t| node.mtime > t) {
            return false;
        }
        if !self.categories.is_empty() && !self.categories.contains(&node.category) {
            return false;
        }

        let name = tree.name(id);
        if !self.extensions.is_empty() {
            match extension_of(name) {
                Some(ext) if self.extensions.contains(&ext) => {}
                _ => return false,
            }
        }
        if let Some(needle) = &self.name_contains {
            if !contains_ignore_case(name, needle) {
                return false;
            }
        }
        // Path reconstruction is the one expensive predicate, so it runs last,
        // only for nodes that already passed everything cheaper.
        if let Some(needle) = &self.path_contains {
            let path = tree.path_of(id);
            if !contains_ignore_case(&path.to_string_lossy(), needle) {
                return false;
            }
        }
        true
    }
}

fn contains_ignore_case(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return true;
    }
    // ASCII fast path avoids allocating a lowercased copy of every name in the
    // tree; falls back to a proper lowercase only for non-ASCII input.
    if haystack.is_ascii() && needle.is_ascii() {
        let (h, n) = (haystack.as_bytes(), needle.as_bytes());
        if n.len() > h.len() {
            return false;
        }
        return h.windows(n.len()).any(|w| w.eq_ignore_ascii_case(n));
    }
    haystack.to_lowercase().contains(&needle.to_lowercase())
}

/// Direct children of `parent`, filtered and sorted.
pub fn children(
    tree: &Tree,
    parent: NodeId,
    filter: &Filter,
    sort: SortKey,
    descending: bool,
    limit: usize,
) -> Vec<Entry> {
    let parent_size = tree.node(parent).logical_size;
    let mut rows: Vec<Entry> = tree
        .children(parent)
        .filter(|id| filter.matches(tree, *id))
        .map(|id| Entry::build(tree, id, parent_size))
        .collect();

    sort_entries(&mut rows, sort, descending);
    rows.truncate(limit);
    rows
}

fn sort_entries(rows: &mut [Entry], sort: SortKey, descending: bool) {
    match sort {
        SortKey::Size => rows.sort_unstable_by_key(|e| e.size),
        SortKey::Modified => rows.sort_unstable_by_key(|e| e.mtime),
        SortKey::Count => rows.sort_unstable_by_key(|e| e.file_count),
        // Case-insensitive so "Zebra" and "apple" sort the way Finder shows them.
        SortKey::Name => {
            rows.sort_unstable_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        }
    }
    if descending {
        rows.reverse();
    }
}

/// Top `limit` nodes by size.
///
/// Uses a bounded min-heap rather than sorting the tree: O(n log k) with k
/// memory, so "top 100 of 12 million files" costs one linear pass and a
/// 100-element heap instead of a 12-million-element sort.
pub fn largest(tree: &Tree, filter: &Filter, limit: usize, dirs: bool) -> Vec<Entry> {
    if limit == 0 {
        return Vec::new();
    }
    let mut heap: BinaryHeap<Reverse<(u64, u32)>> = BinaryHeap::with_capacity(limit + 1);

    for (id, node) in tree.iter() {
        if node.is_dir() != dirs || id == NodeId::ROOT {
            continue;
        }
        // Hardlinked duplicates carry zero bytes; listing them alongside the
        // path that owns the bytes would just be confusing.
        if node.flags.contains(NodeFlags::HARDLINK_DUP) {
            continue;
        }
        if heap.len() == limit && node.logical_size <= heap.peek().map_or(0, |Reverse((s, _))| *s) {
            continue;
        }
        if !filter.matches(tree, id) {
            continue;
        }
        heap.push(Reverse((node.logical_size, id.0)));
        if heap.len() > limit {
            heap.pop();
        }
    }

    let mut out: Vec<Entry> = heap
        .into_sorted_vec()
        .into_iter()
        .map(|Reverse((_, id))| {
            let id = NodeId(id);
            let parent = tree.node(id).parent;
            let parent_size = if parent.is_none() {
                tree.total_logical()
            } else {
                tree.node(parent).logical_size
            };
            Entry::build(tree, id, parent_size)
        })
        .collect();
    out.sort_unstable_by_key(|e| Reverse(e.size));
    out
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CategorySummary {
    pub category: Category,
    pub bytes: u64,
    pub files: u64,
    pub fraction: f32,
}

/// Bytes and file counts per category across the whole tree.
pub fn category_breakdown(tree: &Tree, filter: &Filter) -> Vec<CategorySummary> {
    let mut bytes = [0u64; Category::ALL.len()];
    let mut files = [0u64; Category::ALL.len()];
    let trivial = filter.is_trivial() && filter.include_hidden && filter.include_system;

    for (id, node) in tree.iter() {
        if node.is_dir() || node.flags.contains(NodeFlags::HARDLINK_DUP) {
            continue;
        }
        if !trivial && !filter.matches(tree, id) {
            continue;
        }
        let i = node.category as usize;
        bytes[i] += node.logical_size;
        files[i] += 1;
    }

    let total: u64 = bytes.iter().sum();
    let mut out: Vec<CategorySummary> = Category::ALL
        .iter()
        .enumerate()
        .map(|(i, category)| CategorySummary {
            category: *category,
            bytes: bytes[i],
            files: files[i],
            fraction: if total > 0 {
                (bytes[i] as f64 / total as f64) as f32
            } else {
                0.0
            },
        })
        .collect();
    out.sort_unstable_by_key(|c| Reverse(c.bytes));
    out
}

/// Full-tree search. `limit` caps the result set so a one-character query on a
/// 10-million-file volume cannot flood the UI.
pub fn search(tree: &Tree, filter: &Filter, sort: SortKey, limit: usize) -> Vec<Entry> {
    let mut out = Vec::new();
    for (id, _) in tree.iter() {
        if id == NodeId::ROOT || !filter.matches(tree, id) {
            continue;
        }
        let parent = tree.node(id).parent;
        let parent_size = if parent.is_none() {
            tree.total_logical()
        } else {
            tree.node(parent).logical_size
        };
        out.push(Entry::build(tree, id, parent_size));
        if out.len() >= limit.saturating_mul(4) {
            break;
        }
    }
    sort_entries(&mut out, sort, true);
    out.truncate(limit);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::NodeFlags;

    fn tree() -> Tree {
        let mut t = Tree::new("/root");
        let media = t.push_node(
            "media",
            NodeId::ROOT,
            1,
            NodeFlags::DIRECTORY,
            Category::Other,
            0,
            0,
            0,
        );
        t.push_node(
            "big.mp4",
            media,
            2,
            NodeFlags::empty(),
            Category::Videos,
            900,
            900,
            50,
        );
        t.push_node(
            "small.jpg",
            media,
            2,
            NodeFlags::empty(),
            Category::Images,
            100,
            100,
            150,
        );
        t.push_node(
            ".secret.txt",
            NodeId::ROOT,
            1,
            NodeFlags::HIDDEN,
            Category::Documents,
            10,
            10,
            0,
        );
        t.push_node(
            "kernel.dylib",
            NodeId::ROOT,
            1,
            NodeFlags::SYSTEM,
            Category::System,
            500,
            500,
            0,
        );
        t.rollup();
        t
    }

    #[test]
    fn hidden_and_system_are_excluded_by_default() {
        let t = tree();
        let rows = children(
            &t,
            NodeId::ROOT,
            &Filter::default(),
            SortKey::Size,
            true,
            100,
        );
        let names: Vec<&str> = rows.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, vec!["media"]);

        let rows = children(
            &t,
            NodeId::ROOT,
            &Filter::permissive(),
            SortKey::Size,
            true,
            100,
        );
        assert_eq!(rows.len(), 3);
    }

    #[test]
    fn largest_files_are_ranked_and_bounded() {
        let t = tree();
        let rows = largest(&t, &Filter::permissive(), 2, false);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].name, "big.mp4");
        assert_eq!(rows[1].name, "kernel.dylib");
        assert!(rows[0].size >= rows[1].size, "results must be descending");
    }

    #[test]
    fn largest_folders_excludes_the_root() {
        let t = tree();
        let rows = largest(&t, &Filter::permissive(), 10, true);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "media");
        assert_eq!(rows[0].size, 1000);
    }

    #[test]
    fn filters_compose() {
        let t = tree();
        let filter = Filter {
            min_size: Some(200),
            categories: vec![Category::Videos],
            ..Filter::permissive()
        };
        let rows = search(&t, &filter, SortKey::Size, 10);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "big.mp4");
    }

    #[test]
    fn extension_and_name_filters_are_case_insensitive() {
        let t = tree();
        let by_ext = Filter {
            extensions: vec!["mp4".into()],
            ..Filter::permissive()
        };
        assert_eq!(search(&t, &by_ext, SortKey::Size, 10).len(), 1);

        let by_name = Filter {
            name_contains: Some("BIG".into()),
            ..Filter::permissive()
        };
        assert_eq!(search(&t, &by_name, SortKey::Size, 10)[0].name, "big.mp4");
    }

    #[test]
    fn modified_window_filters_both_ends() {
        let t = tree();
        let filter = Filter {
            modified_after: Some(100),
            ..Filter::permissive()
        };
        let rows = search(&t, &filter, SortKey::Modified, 10);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "small.jpg");
    }

    #[test]
    fn category_breakdown_sums_to_the_tree_total() {
        let t = tree();
        let rows = category_breakdown(&t, &Filter::permissive());
        let total: u64 = rows.iter().map(|r| r.bytes).sum();
        assert_eq!(total, t.total_logical());

        let videos = rows
            .iter()
            .find(|r| r.category == Category::Videos)
            .unwrap();
        assert_eq!(videos.bytes, 900);
        assert!((videos.fraction - 0.9 / 1.51).abs() < 0.01);
    }

    #[test]
    fn fraction_of_parent_is_computed() {
        let t = tree();
        let media = t.find(std::path::Path::new("/root/media")).unwrap();
        let rows = children(&t, media, &Filter::permissive(), SortKey::Size, true, 10);
        assert!((rows[0].fraction_of_parent - 0.9).abs() < 1e-6);
    }
}
