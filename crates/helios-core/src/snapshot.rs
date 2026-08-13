//! On-disk snapshot cache.
//!
//! Rescanning a 2 TB volume to redraw a treemap the user looked at yesterday is
//! wasted work, so every completed scan is persisted and reloaded on launch.
//!
//! The format is a hand-rolled, little-endian binary layout rather than JSON or
//! a serde codec, for three reasons:
//!
//! * **Size.** A 5-million-node tree is ~280 MB as JSON and ~50 MB here, before
//!   the OS compresses it.
//! * **Speed.** Nodes are fixed-width records, so loading is a bulk read and a
//!   tight decode loop — no parser, no per-node allocation.
//! * **No dependency.** The engine's only third-party crates are `serde` (for
//!   the IPC boundary) and `crossbeam-channel`. A cache format is not worth
//!   adding a serialization framework and its version churn to an app whose
//!   pitch is that it is small and auditable.
//!
//! Files are written to a temporary path and renamed into place, so an
//! interrupted write can never leave a half-written snapshot behind.

use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::category::Category;
use crate::model::{Node, NodeFlags, NodeId, ScanError, Tree};
use crate::scan::ScanStats;

const MAGIC: &[u8; 8] = b"HELIOSNP";
const FORMAT_VERSION: u32 = 1;
/// Refuse absurd headers rather than trying to allocate from them — a
/// corrupted length field must not become a multi-gigabyte allocation.
const MAX_NODES: u64 = 500_000_000;
const MAX_NAMES_BYTES: u64 = 8 << 30;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotMeta {
    /// Volume id this snapshot belongs to (see [`crate::platform::Volume::id`]).
    pub volume_id: String,
    pub root_path: PathBuf,
    /// Unix seconds.
    pub scanned_at: i64,
    pub stats: ScanStats,
}

#[derive(Debug)]
pub struct Snapshot {
    pub meta: SnapshotMeta,
    pub tree: Tree,
}

/// Directory holding cached snapshots.
pub fn cache_dir() -> PathBuf {
    crate::platform::app_data_dir().join("snapshots")
}

/// Path for a volume's snapshot. The id is sanitized because it comes from the
/// OS (`/dev/disk3s1s1`, `\\?\Volume{…}`) and must not escape the cache
/// directory.
pub fn snapshot_path(volume_id: &str) -> PathBuf {
    let safe: String = volume_id
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    cache_dir().join(format!("{safe}.helios"))
}

struct Writer<W: Write> {
    inner: W,
}

impl<W: Write> Writer<W> {
    fn u16(&mut self, v: u16) -> io::Result<()> {
        self.inner.write_all(&v.to_le_bytes())
    }
    fn u32(&mut self, v: u32) -> io::Result<()> {
        self.inner.write_all(&v.to_le_bytes())
    }
    fn u64(&mut self, v: u64) -> io::Result<()> {
        self.inner.write_all(&v.to_le_bytes())
    }
    fn i64(&mut self, v: i64) -> io::Result<()> {
        self.inner.write_all(&v.to_le_bytes())
    }
    fn bytes(&mut self, v: &[u8]) -> io::Result<()> {
        self.u64(v.len() as u64)?;
        self.inner.write_all(v)
    }
    fn str(&mut self, v: &str) -> io::Result<()> {
        self.bytes(v.as_bytes())
    }
}

struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> io::Result<&'a [u8]> {
        if self.pos + n > self.buf.len() {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "snapshot truncated",
            ));
        }
        let slice = &self.buf[self.pos..self.pos + n];
        self.pos += n;
        Ok(slice)
    }
    fn u16(&mut self) -> io::Result<u16> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }
    fn u32(&mut self) -> io::Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn u64(&mut self) -> io::Result<u64> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn i64(&mut self) -> io::Result<i64> {
        Ok(i64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn bytes(&mut self) -> io::Result<&'a [u8]> {
        let len = self.u64()?;
        if len > MAX_NAMES_BYTES {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "length out of range"));
        }
        self.take(len as usize)
    }
    fn string(&mut self) -> io::Result<String> {
        Ok(String::from_utf8_lossy(self.bytes()?).into_owned())
    }
}

/// Serializes a snapshot into `out`.
pub fn write_to(out: impl Write, snapshot: &Snapshot) -> io::Result<()> {
    let mut w = Writer {
        inner: io::BufWriter::with_capacity(1 << 20, out),
    };
    w.inner.write_all(MAGIC)?;
    w.u32(FORMAT_VERSION)?;
    w.str(&snapshot.meta.volume_id)?;
    w.str(&snapshot.meta.root_path.to_string_lossy())?;
    w.i64(snapshot.meta.scanned_at)?;

    let stats = &snapshot.meta.stats;
    for value in [
        stats.files_scanned,
        stats.dirs_scanned,
        stats.bytes_seen,
        stats.dirs_reused,
        stats.errors,
        stats.elapsed_ms,
        stats.nodes,
        stats.memory_bytes,
    ] {
        w.u64(value)?;
    }

    w.u64(snapshot.tree.node_slice().len() as u64)?;
    for node in snapshot.tree.node_slice() {
        w.u32(node.name_off)?;
        w.u16(node.name_len)?;
        w.u16(node.flags.bits())?;
        w.u16(node.depth)?;
        w.u16(node.category as u16)?;
        w.u32(node.parent.0)?;
        w.u32(node.first_child.0)?;
        w.u32(node.next_sibling.0)?;
        w.u64(node.logical_size)?;
        w.u64(node.physical_size)?;
        w.i64(node.mtime)?;
        w.u32(node.file_count)?;
        w.u32(node.dir_count)?;
    }
    w.bytes(snapshot.tree.name_bytes())?;

    w.u32(snapshot.tree.errors.len() as u32)?;
    for error in &snapshot.tree.errors {
        w.str(&error.path.to_string_lossy())?;
        w.str(&error.message)?;
    }
    w.inner.flush()
}

/// Parses a snapshot from `buf`.
pub fn read_from(buf: &[u8]) -> io::Result<Snapshot> {
    let mut r = Reader { buf, pos: 0 };
    if r.take(8)? != MAGIC {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "not a Helios snapshot"));
    }
    let version = r.u32()?;
    if version != FORMAT_VERSION {
        // Snapshots are a cache, never a source of truth: an unreadable one is
        // discarded and the volume is rescanned.
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("snapshot format {version} is not supported (expected {FORMAT_VERSION})"),
        ));
    }

    let volume_id = r.string()?;
    let root_path = PathBuf::from(r.string()?);
    let scanned_at = r.i64()?;
    let mut stats = ScanStats::default();
    stats.files_scanned = r.u64()?;
    stats.dirs_scanned = r.u64()?;
    stats.bytes_seen = r.u64()?;
    stats.dirs_reused = r.u64()?;
    stats.errors = r.u64()?;
    stats.elapsed_ms = r.u64()?;
    stats.nodes = r.u64()?;
    stats.memory_bytes = r.u64()?;

    let count = r.u64()?;
    if count > MAX_NODES {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "node count out of range"));
    }
    let mut nodes = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let name_off = r.u32()?;
        let name_len = r.u16()?;
        let flags = NodeFlags::from_bits_truncate(r.u16()?);
        let depth = r.u16()?;
        let category = category_from_u16(r.u16()?);
        let parent = NodeId(r.u32()?);
        let first_child = NodeId(r.u32()?);
        let next_sibling = NodeId(r.u32()?);
        let logical_size = r.u64()?;
        let physical_size = r.u64()?;
        let mtime = r.i64()?;
        let file_count = r.u32()?;
        let dir_count = r.u32()?;
        nodes.push(Node::from_parts(
            name_off,
            name_len,
            flags,
            depth,
            category,
            parent,
            first_child,
            next_sibling,
            logical_size,
            physical_size,
            mtime,
            file_count,
            dir_count,
        ));
    }
    let names = r.bytes()?.to_vec();

    let error_count = r.u32()?;
    let mut errors = Vec::new();
    for _ in 0..error_count {
        let path = PathBuf::from(r.string()?);
        let message = r.string()?;
        errors.push(ScanError { path, message });
    }

    let tree = Tree::from_parts(nodes, names, root_path.clone(), errors);
    Ok(Snapshot {
        meta: SnapshotMeta {
            volume_id,
            root_path,
            scanned_at,
            stats,
        },
        tree,
    })
}

fn category_from_u16(v: u16) -> Category {
    Category::ALL.get(v as usize).copied().unwrap_or(Category::Other)
}

/// Writes a snapshot to the cache directory, atomically.
pub fn save(snapshot: &Snapshot) -> io::Result<PathBuf> {
    let path = snapshot_path(&snapshot.meta.volume_id);
    save_to_path(snapshot, &path)?;
    Ok(path)
}

pub fn save_to_path(snapshot: &Snapshot, path: &Path) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temp = path.with_extension("tmp");
    {
        let file = fs::File::create(&temp)?;
        write_to(&file, snapshot)?;
        // Durability before the rename, so a crash can't publish a truncated
        // file under the real name.
        file.sync_all()?;
    }
    fs::rename(&temp, path)
}

pub fn load(volume_id: &str) -> io::Result<Snapshot> {
    load_from_path(&snapshot_path(volume_id))
}

pub fn load_from_path(path: &Path) -> io::Result<Snapshot> {
    let mut buf = Vec::new();
    fs::File::open(path)?.read_to_end(&mut buf)?;
    read_from(&buf)
}

/// Lists cached snapshots, newest first, skipping any that fail to parse.
pub fn list() -> Vec<SnapshotMeta> {
    let Ok(entries) = fs::read_dir(cache_dir()) else {
        return Vec::new();
    };
    let mut out: Vec<SnapshotMeta> = entries
        .filter_map(Result::ok)
        .filter(|e| e.path().extension().is_some_and(|x| x == "helios"))
        .filter_map(|e| load_from_path(&e.path()).ok().map(|s| s.meta))
        .collect();
    out.sort_unstable_by_key(|m| std::cmp::Reverse(m.scanned_at));
    out
}

pub fn delete(volume_id: &str) -> io::Result<()> {
    fs::remove_file(snapshot_path(volume_id))
}

pub fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::NodeFlags;

    fn sample() -> Snapshot {
        let mut tree = Tree::new("/Volumes/Sample");
        let dir = tree.push_node(
            "Movies",
            NodeId::ROOT,
            1,
            NodeFlags::DIRECTORY,
            Category::Other,
            0,
            0,
            42,
        );
        tree.push_node("épée.mp4", dir, 2, NodeFlags::empty(), Category::Videos, 4096, 4096, 7);
        tree.push_node(".hidden", NodeId::ROOT, 1, NodeFlags::HIDDEN, Category::Other, 1, 1, 0);
        tree.errors.push(ScanError {
            path: "/Volumes/Sample/locked".into(),
            message: "permission denied".into(),
        });
        tree.rollup();

        Snapshot {
            meta: SnapshotMeta {
                volume_id: "/dev/disk3s1".into(),
                root_path: "/Volumes/Sample".into(),
                scanned_at: 1_700_000_000,
                stats: ScanStats {
                    files_scanned: 2,
                    ..ScanStats::default()
                },
            },
            tree,
        }
    }

    #[test]
    fn round_trips_through_the_binary_format() {
        let snapshot = sample();
        let mut buf = Vec::new();
        write_to(&mut buf, &snapshot).unwrap();
        let restored = read_from(&buf).unwrap();

        assert_eq!(restored.meta.volume_id, "/dev/disk3s1");
        assert_eq!(restored.meta.scanned_at, 1_700_000_000);
        assert_eq!(restored.meta.stats.files_scanned, 2);
        assert_eq!(restored.tree.len(), snapshot.tree.len());
        assert_eq!(restored.tree.total_logical(), snapshot.tree.total_logical());
        assert_eq!(restored.tree.errors.len(), 1);

        // Structure, names (including non-ASCII) and flags all survive.
        let movie = restored
            .tree
            .find(Path::new("/Volumes/Sample/Movies/épée.mp4"))
            .expect("path lookup after reload");
        assert_eq!(restored.tree.node(movie).category, Category::Videos);
        let hidden = restored.tree.find(Path::new("/Volumes/Sample/.hidden")).unwrap();
        assert!(restored.tree.node(hidden).flags.contains(NodeFlags::HIDDEN));
    }

    #[test]
    fn rejects_foreign_and_truncated_files() {
        assert!(read_from(b"not a snapshot at all").is_err());

        let mut buf = Vec::new();
        write_to(&mut buf, &sample()).unwrap();
        assert!(read_from(&buf[..buf.len() / 2]).is_err(), "truncation must be caught");

        buf[8] = 0xFF; // bump the version
        assert!(read_from(&buf).is_err(), "unknown version must be rejected");
    }

    #[test]
    fn saves_atomically_and_reloads() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested/volume.helios");
        save_to_path(&sample(), &path).unwrap();

        assert!(path.exists());
        assert!(!path.with_extension("tmp").exists(), "temp file must be renamed away");
        assert_eq!(load_from_path(&path).unwrap().tree.total_logical(), 4097);
    }

    #[test]
    fn volume_ids_cannot_escape_the_cache_directory() {
        let path = snapshot_path("../../etc/passwd");
        assert_eq!(path.parent(), Some(cache_dir().as_path()));
        assert!(!path.to_string_lossy().contains(".."));
    }
}
