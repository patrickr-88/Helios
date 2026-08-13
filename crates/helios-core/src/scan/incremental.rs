//! Incremental rescan support.
//!
//! A rescan of an unchanged volume should cost seconds, not minutes. The trick
//! is that a directory's mtime changes whenever an entry is added, removed or
//! renamed in it — so if a directory's mtime matches the previous snapshot, we
//! can graft the cached subtree into the new tree without a single `read_dir`.
//!
//! Caveat, stated plainly because it is a real accuracy trade-off: mtime does
//! **not** change when a file *inside* the directory merely grows. A rescan is
//! therefore exact about structure and can lag on size for files that were
//! rewritten in place. The UI exposes "Full rescan" for when that matters, and
//! reused directories are counted and reported in the scan summary.
//!
//! Directories are keyed by a hash chained down from the root
//! (`hash(parent, name)`) rather than by their reconstructed path string, which
//! keeps index construction O(nodes) instead of O(nodes × depth) and allocates
//! nothing per entry.

use std::collections::HashMap;

use crate::model::{NodeId, Tree};

/// FNV-1a. Chosen over SipHash for the same reason `rustc` uses FxHash
/// internally: this is a non-adversarial, in-process index where hashing shows
/// up in profiles, and FNV is a few instructions per byte.
#[inline]
pub fn hash_child(parent: u64, name: &str) -> u64 {
    const PRIME: u64 = 0x100_0000_01b3;
    let mut h = parent ^ 0xcbf2_9ce4_8422_2325;
    for b in name.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(PRIME);
    }
    h
}

pub fn root_hash(path: &std::path::Path) -> u64 {
    hash_child(0, &path.to_string_lossy())
}

/// Directory hash → `(mtime, node)` for a previous snapshot.
#[derive(Debug)]
pub struct DirIndex {
    map: HashMap<u64, (i64, NodeId)>,
}

impl DirIndex {
    /// Walks the snapshot once, hashing paths incrementally.
    pub fn build(tree: &Tree) -> Self {
        let mut map = HashMap::with_capacity(tree.len() / 8);
        let mut stack = vec![(NodeId::ROOT, root_hash(&tree.root_path))];
        map.insert(root_hash(&tree.root_path), (tree.node(NodeId::ROOT).mtime, NodeId::ROOT));

        while let Some((id, hash)) = stack.pop() {
            for child in tree.children(id) {
                let node = tree.node(child);
                if !node.is_dir() || node.is_symlink() {
                    continue;
                }
                let child_hash = hash_child(hash, tree.name(child));
                map.insert(child_hash, (node.mtime, child));
                stack.push((child, child_hash));
            }
        }
        DirIndex { map }
    }

    /// Returns the cached node for a directory whose mtime is unchanged.
    pub fn reusable(&self, hash: u64, mtime: i64) -> Option<NodeId> {
        match self.map.get(&hash) {
            Some((cached_mtime, id)) if *cached_mtime == mtime => Some(*id),
            _ => None,
        }
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

/// Statistics from grafting a cached subtree.
#[derive(Debug, Default, Clone, Copy)]
pub struct GraftStats {
    pub nodes: u64,
    pub files: u64,
    pub dirs: u64,
    pub bytes: u64,
}

/// Copies `src_root`'s children into `dst` beneath `dst_parent`.
///
/// Directory sizes are deliberately reset to zero on the way in: the caller's
/// [`Tree::rollup`](crate::model::Tree::rollup) recomputes every directory
/// total from its files, so carrying the old rolled-up values across would
/// double-count the whole subtree.
pub fn graft_subtree(
    dst: &mut Tree,
    dst_parent: NodeId,
    src: &Tree,
    src_root: NodeId,
) -> GraftStats {
    let mut stats = GraftStats::default();
    let base_depth = dst.node(dst_parent).depth;
    let src_base_depth = src.node(src_root).depth;
    let mut stack = vec![(src_root, dst_parent)];

    while let Some((src_id, dst_id)) = stack.pop() {
        for child in src.children(src_id) {
            let node = *src.node(child);
            let is_dir = node.is_dir();
            let depth = base_depth + (node.depth - src_base_depth);
            let (logical, physical) = if is_dir {
                (0, 0)
            } else {
                (node.logical_size, node.physical_size)
            };

            let new_id = dst.push_node(
                src.name(child),
                dst_id,
                depth,
                node.flags,
                node.category,
                logical,
                physical,
                node.mtime,
            );

            stats.nodes += 1;
            if is_dir {
                stats.dirs += 1;
                stack.push((child, new_id));
            } else {
                stats.files += 1;
                stats.bytes += node.logical_size;
            }
        }
    }
    stats
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::category::Category;
    use crate::model::NodeFlags;

    fn sample_tree() -> Tree {
        let mut t = Tree::new("/root");
        let a = t.push_node("a", NodeId::ROOT, 1, NodeFlags::DIRECTORY, Category::Other, 0, 0, 100);
        let b = t.push_node("b", a, 2, NodeFlags::DIRECTORY, Category::Other, 0, 0, 200);
        t.push_node("f.bin", b, 3, NodeFlags::empty(), Category::Other, 64, 64, 0);
        t.push_node("g.bin", a, 2, NodeFlags::empty(), Category::Other, 32, 32, 0);
        t.rollup();
        t
    }

    #[test]
    fn index_matches_only_unchanged_directories() {
        let tree = sample_tree();
        let index = DirIndex::build(&tree);
        let a_hash = hash_child(root_hash(&tree.root_path), "a");

        assert!(index.reusable(a_hash, 100).is_some());
        assert!(index.reusable(a_hash, 101).is_none(), "changed mtime must miss");
        assert!(index.reusable(hash_child(a_hash, "ghost"), 100).is_none());
    }

    #[test]
    fn grafting_reproduces_sizes_without_double_counting() {
        let old = sample_tree();
        let a = old.find(std::path::Path::new("/root/a")).unwrap();

        let mut fresh = Tree::new("/root");
        let stats = graft_subtree(&mut fresh, NodeId::ROOT, &old, a);
        fresh.rollup();

        assert_eq!(stats.files, 2);
        assert_eq!(stats.dirs, 1);
        assert_eq!(fresh.total_logical(), 96, "96 bytes of files, counted once");
        assert_eq!(fresh.node(NodeId::ROOT).file_count, 2);
        // Depths are rebased onto the new parent.
        let b = fresh.find(std::path::Path::new("/root/b")).unwrap();
        assert_eq!(fresh.node(b).depth, 1);
    }
}
