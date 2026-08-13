//! Compact arena representation of a scanned filesystem tree.
//!
//! Millions of nodes have to fit in a desktop app's memory budget, so the tree
//! is stored as a struct-of-arrays arena rather than a graph of heap-allocated
//! nodes:
//!
//! * Names live in one contiguous `Vec<u8>` blob; each node stores a
//!   `(u32 offset, u16 len)` slice into it instead of a `String` (which would
//!   cost 24 bytes of header plus an allocation per node).
//! * Children form an intrusive singly-linked list (`first_child` /
//!   `next_sibling`), so a directory with no children costs nothing extra and
//!   we never allocate a `Vec` per directory.
//! * Node ids are `u32`, capping a single snapshot at ~4.29 billion entries.
//!
//! The resulting per-node footprint is 56 bytes, so a 10-million-file volume
//! costs roughly 560 MB of node storage plus the name blob (typically ~15
//! bytes/entry) — about 700 MB worst case, and far less for realistic trees.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::category::Category;

/// Index into [`Tree::nodes`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct NodeId(pub u32);

impl NodeId {
    /// Sentinel used for "no parent" / "no sibling" / "no child".
    pub const NONE: NodeId = NodeId(u32::MAX);
    pub const ROOT: NodeId = NodeId(0);

    #[inline]
    pub fn is_none(self) -> bool {
        self.0 == u32::MAX
    }

    #[inline]
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

bitflags_lite! {
    /// Per-node boolean attributes, packed into a `u16`.
    pub struct NodeFlags: u16 {
        const DIRECTORY    = 1 << 0;
        /// A symlink; never traversed, and its target's bytes are not counted.
        const SYMLINK      = 1 << 1;
        /// Dot-prefixed on Unix, or carrying the hidden attribute (macOS
        /// `UF_HIDDEN`, Windows `FILE_ATTRIBUTE_HIDDEN`).
        const HIDDEN       = 1 << 2;
        /// Lives under a path the platform layer classifies as OS-owned.
        const SYSTEM       = 1 << 3;
        /// A macOS bundle (`.app`, `.framework`, …): a directory the UI should
        /// present as a single item by default.
        const PACKAGE      = 1 << 4;
        /// `read_dir`/`stat` failed — size is a floor, not a total.
        const INACCESSIBLE = 1 << 5;
        /// Additional link to an inode already counted elsewhere in the scan;
        /// its bytes are attributed to the first path we saw.
        const HARDLINK_DUP = 1 << 6;
        /// A mount point for a different filesystem than its parent.
        const MOUNT_POINT  = 1 << 7;
    }
}

/// One filesystem entry. Layout is deliberately 56 bytes; see module docs.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Node {
    pub(crate) name_off: u32,
    pub(crate) name_len: u16,
    pub flags: NodeFlags,
    pub depth: u16,
    pub category: Category,
    _pad: u8,
    pub parent: NodeId,
    pub first_child: NodeId,
    pub next_sibling: NodeId,
    /// Logical size in bytes. For directories this is the rolled-up subtree
    /// total, valid only after [`Tree::rollup`].
    pub logical_size: u64,
    /// Bytes actually allocated on disk (accounts for sparse files and
    /// compression). Falls back to `logical_size` where the platform cannot
    /// report it.
    pub physical_size: u64,
    /// Modification time, seconds since the Unix epoch.
    pub mtime: i64,
    /// Files in this subtree (0 for a leaf file itself).
    pub file_count: u32,
    /// Directories in this subtree, excluding this node.
    pub dir_count: u32,
}

impl Node {
    /// Reconstructs a node from its serialized fields. Used only by the
    /// snapshot loader, which owns the on-disk field order.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_parts(
        name_off: u32,
        name_len: u16,
        flags: NodeFlags,
        depth: u16,
        category: Category,
        parent: NodeId,
        first_child: NodeId,
        next_sibling: NodeId,
        logical_size: u64,
        physical_size: u64,
        mtime: i64,
        file_count: u32,
        dir_count: u32,
    ) -> Node {
        Node {
            name_off,
            name_len,
            flags,
            depth,
            category,
            _pad: 0,
            parent,
            first_child,
            next_sibling,
            logical_size,
            physical_size,
            mtime,
            file_count,
            dir_count,
        }
    }

    #[inline]
    pub fn is_dir(&self) -> bool {
        self.flags.contains(NodeFlags::DIRECTORY)
    }

    #[inline]
    pub fn is_symlink(&self) -> bool {
        self.flags.contains(NodeFlags::SYMLINK)
    }
}

/// A scanned tree rooted at a single path.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tree {
    pub(crate) nodes: Vec<Node>,
    pub(crate) names: Vec<u8>,
    /// Absolute path of node 0.
    pub root_path: PathBuf,
    /// Paths that could not be read, with the reason, for the scan report.
    pub errors: Vec<ScanError>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanError {
    pub path: PathBuf,
    pub message: String,
}

impl Tree {
    pub fn new(root_path: impl Into<PathBuf>) -> Self {
        let root_path = root_path.into();
        let name = root_path
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| root_path.to_string_lossy().into_owned());
        let mut tree = Tree {
            nodes: Vec::new(),
            names: Vec::new(),
            root_path,
            errors: Vec::new(),
        };
        tree.push_node(
            &name,
            NodeId::NONE,
            0,
            NodeFlags::DIRECTORY,
            Category::Other,
            0,
            0,
            0,
        );
        tree
    }

    /// Appends a node and links it as a child of `parent`.
    ///
    /// Nodes are always appended after their parent, which the rollup and
    /// serialization passes rely on (a child's index is always greater than
    /// its parent's).
    #[allow(clippy::too_many_arguments)]
    pub fn push_node(
        &mut self,
        name: &str,
        parent: NodeId,
        depth: u16,
        flags: NodeFlags,
        category: Category,
        logical_size: u64,
        physical_size: u64,
        mtime: i64,
    ) -> NodeId {
        let name_off = self.names.len() as u32;
        let bytes = name.as_bytes();
        // Names longer than u16::MAX are not representable on any filesystem we
        // target (HFS+/APFS cap at 255 UTF-16 units, NTFS at 255 UTF-16 units).
        let name_len = bytes.len().min(u16::MAX as usize);
        self.names.extend_from_slice(&bytes[..name_len]);

        let id = NodeId(self.nodes.len() as u32);
        self.nodes.push(Node {
            name_off,
            name_len: name_len as u16,
            flags,
            depth,
            category,
            _pad: 0,
            parent,
            first_child: NodeId::NONE,
            next_sibling: NodeId::NONE,
            logical_size,
            physical_size,
            mtime,
            file_count: 0,
            dir_count: 0,
        });

        if !parent.is_none() {
            // Push-front keeps insertion O(1); sibling order is restored by the
            // UI's sort, which is always size- or name-driven anyway.
            let head = self.nodes[parent.index()].first_child;
            self.nodes[id.index()].next_sibling = head;
            self.nodes[parent.index()].first_child = id;
        }
        id
    }

    /// Rebuilds a tree from already-decoded parts (snapshot loading).
    pub(crate) fn from_parts(
        nodes: Vec<Node>,
        names: Vec<u8>,
        root_path: PathBuf,
        errors: Vec<ScanError>,
    ) -> Tree {
        Tree {
            nodes,
            names,
            root_path,
            errors,
        }
    }

    pub(crate) fn node_slice(&self) -> &[Node] {
        &self.nodes
    }

    pub(crate) fn name_bytes(&self) -> &[u8] {
        &self.names
    }

    #[inline]
    pub fn node(&self, id: NodeId) -> &Node {
        &self.nodes[id.index()]
    }

    #[inline]
    pub fn node_mut(&mut self, id: NodeId) -> &mut Node {
        &mut self.nodes[id.index()]
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    #[inline]
    pub fn name(&self, id: NodeId) -> &str {
        let n = &self.nodes[id.index()];
        let start = n.name_off as usize;
        let end = start + n.name_len as usize;
        // Names are pushed from `&str`, so the slice is valid UTF-8 unless the
        // arena was corrupted; `from_utf8_lossy` keeps that non-fatal.
        std::str::from_utf8(&self.names[start..end]).unwrap_or("<invalid>")
    }

    /// Iterates the direct children of `id`.
    pub fn children(&self, id: NodeId) -> Children<'_> {
        Children {
            tree: self,
            next: self.nodes[id.index()].first_child,
        }
    }

    /// Reconstructs the absolute path of a node by walking up to the root.
    pub fn path_of(&self, id: NodeId) -> PathBuf {
        let mut parts: Vec<&str> = Vec::with_capacity(self.nodes[id.index()].depth as usize + 1);
        let mut cur = id;
        while !cur.is_none() && cur != NodeId::ROOT {
            parts.push(self.name(cur));
            cur = self.nodes[cur.index()].parent;
        }
        let mut path = self.root_path.clone();
        for part in parts.iter().rev() {
            path.push(part);
        }
        path
    }

    /// Depth-first pre-order iteration over the whole tree.
    pub fn iter(&self) -> impl Iterator<Item = (NodeId, &Node)> {
        self.nodes
            .iter()
            .enumerate()
            .map(|(i, n)| (NodeId(i as u32), n))
    }

    /// Rolls file sizes up into directory totals.
    ///
    /// Runs in a single reverse linear pass rather than a recursive descent:
    /// because children are always appended after their parent, iterating the
    /// arena backwards visits every child before its parent. This keeps rollup
    /// O(n) with no stack, which matters for deep trees where recursion would
    /// risk overflow.
    pub fn rollup(&mut self) {
        for i in (1..self.nodes.len()).rev() {
            let (logical, physical, files, dirs, parent) = {
                let n = &self.nodes[i];
                let is_dir = n.is_dir();
                (
                    n.logical_size,
                    n.physical_size,
                    n.file_count + u32::from(!is_dir),
                    n.dir_count + u32::from(is_dir),
                    n.parent,
                )
            };
            if parent.is_none() {
                continue;
            }
            let p = &mut self.nodes[parent.index()];
            p.logical_size += logical;
            p.physical_size += physical;
            p.file_count += files;
            p.dir_count += dirs;
        }
    }

    /// Finds a node by absolute path, or `None` if it is not in this tree.
    pub fn find(&self, path: &Path) -> Option<NodeId> {
        let rel = path.strip_prefix(&self.root_path).ok()?;
        let mut cur = NodeId::ROOT;
        for component in rel.components() {
            let want = component.as_os_str().to_string_lossy();
            let mut found = None;
            for child in self.children(cur) {
                if self.name(child) == want {
                    found = Some(child);
                    break;
                }
            }
            cur = found?;
        }
        Some(cur)
    }

    pub fn total_logical(&self) -> u64 {
        self.nodes.first().map(|n| n.logical_size).unwrap_or(0)
    }

    pub fn total_physical(&self) -> u64 {
        self.nodes.first().map(|n| n.physical_size).unwrap_or(0)
    }

    /// Approximate resident size of the arena, for the diagnostics panel.
    pub fn memory_bytes(&self) -> usize {
        self.nodes.capacity() * std::mem::size_of::<Node>() + self.names.capacity()
    }
}

#[derive(Debug)]
pub struct Children<'a> {
    tree: &'a Tree,
    next: NodeId,
}

impl<'a> Iterator for Children<'a> {
    type Item = NodeId;

    fn next(&mut self) -> Option<NodeId> {
        if self.next.is_none() {
            return None;
        }
        let cur = self.next;
        self.next = self.tree.nodes[cur.index()].next_sibling;
        Some(cur)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(tree: &mut Tree, parent: NodeId, name: &str, size: u64) -> NodeId {
        let depth = tree.node(parent).depth + 1;
        tree.push_node(
            name,
            parent,
            depth,
            NodeFlags::empty(),
            Category::Other,
            size,
            size,
            0,
        )
    }

    fn dir(tree: &mut Tree, parent: NodeId, name: &str) -> NodeId {
        let depth = tree.node(parent).depth + 1;
        tree.push_node(
            name,
            parent,
            depth,
            NodeFlags::DIRECTORY,
            Category::Other,
            0,
            0,
            0,
        )
    }

    #[test]
    fn node_is_56_bytes() {
        assert_eq!(std::mem::size_of::<Node>(), 56);
    }

    #[test]
    fn rollup_sums_nested_directories() {
        let mut tree = Tree::new("/Volumes/Test");
        let a = dir(&mut tree, NodeId::ROOT, "a");
        let b = dir(&mut tree, a, "b");
        file(&mut tree, b, "deep.bin", 100);
        file(&mut tree, a, "shallow.bin", 25);
        file(&mut tree, NodeId::ROOT, "top.bin", 5);
        tree.rollup();

        assert_eq!(tree.node(b).logical_size, 100);
        assert_eq!(tree.node(a).logical_size, 125);
        assert_eq!(tree.total_logical(), 130);
        assert_eq!(tree.node(NodeId::ROOT).file_count, 3);
        assert_eq!(tree.node(NodeId::ROOT).dir_count, 2);
    }

    #[test]
    fn path_and_lookup_round_trip() {
        let mut tree = Tree::new("/Volumes/Test");
        let a = dir(&mut tree, NodeId::ROOT, "a");
        let f = file(&mut tree, a, "deep.bin", 1);
        assert_eq!(tree.path_of(f), Path::new("/Volumes/Test/a/deep.bin"));
        assert_eq!(tree.find(Path::new("/Volumes/Test/a/deep.bin")), Some(f));
        assert_eq!(tree.find(Path::new("/Volumes/Test/nope")), None);
        assert_eq!(tree.find(Path::new("/elsewhere")), None);
    }
}
