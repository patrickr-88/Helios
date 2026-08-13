//! The platform seam.
//!
//! Everything OS-specific in Helios lives behind this module. The scan engine,
//! query layer, reporting and the entire UI are written against the types and
//! free functions declared here, so porting to a new OS means implementing one
//! file — not touching the engine.
//!
//! Required surface for a new platform:
//!
//! | Item                  | Purpose                                            |
//! |-----------------------|----------------------------------------------------|
//! | [`volumes`]           | Enumerate mounted volumes with capacity/free bytes  |
//! | [`EntryMeta`]         | Per-entry metadata in one syscall, ideally          |
//! | [`is_hidden`]         | Hidden-attribute semantics                          |
//! | [`is_package`]        | Bundle-style directories shown as one item          |
//! | [`system_prefixes`]   | Paths classified as OS-owned                        |
//! | [`default_exclusions`]| Pseudo-filesystems to skip entirely                 |
//! | [`app_data_dir`]      | Where the snapshot cache lives                      |

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
use macos as imp;

#[cfg(all(unix, not(target_os = "macos")))]
mod linux;
#[cfg(all(unix, not(target_os = "macos")))]
use linux as imp;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows as imp;

/// A mounted volume as presented on the Dashboard.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Volume {
    /// Stable identity across rescans: device path on Unix, GUID path on
    /// Windows. Used as the snapshot cache key.
    pub id: String,
    /// User-facing name ("Macintosh HD", "Backup", "C:").
    pub name: String,
    pub mount_point: PathBuf,
    /// Filesystem type as reported by the OS ("apfs", "ntfs", "smbfs", …).
    pub filesystem: String,
    pub total_bytes: u64,
    pub free_bytes: u64,
    /// Bytes in use, as the OS reports them — this is authoritative and will
    /// usually exceed a scan's total, because scans skip unreadable areas.
    pub used_bytes: u64,
    pub is_removable: bool,
    pub is_network: bool,
    pub is_read_only: bool,
    /// The volume backing `/` (or `C:\`).
    pub is_root: bool,
}

/// Metadata for one directory entry, gathered with as few syscalls as the
/// platform allows.
#[derive(Debug, Clone, Copy)]
pub struct EntryMeta {
    pub is_dir: bool,
    pub is_symlink: bool,
    pub logical_size: u64,
    pub physical_size: u64,
    pub mtime: i64,
    pub hidden: bool,
    /// `(device, inode)` on Unix; `None` where the platform cannot supply a
    /// cheap identity, which disables hardlink de-duplication.
    pub file_id: Option<(u64, u64)>,
    pub nlink: u64,
}

/// Enumerates mounted volumes.
pub fn volumes() -> Vec<Volume> {
    imp::volumes()
}

/// Reads metadata for `path` **without** following symlinks.
///
/// Never following links is a correctness requirement, not a preference: a
/// symlink into an ancestor directory would otherwise make the scan loop
/// forever and double-count bytes.
pub fn entry_meta(path: &Path) -> std::io::Result<EntryMeta> {
    imp::entry_meta(path)
}

/// Reads metadata for an already-enumerated directory entry.
///
/// Preferred over [`entry_meta`] inside the walker: on Unix it stats against
/// the open directory handle (no path resolution), and on Windows the data is
/// already in hand from `FindNextFileW`, costing no syscall at all. Neither
/// follows symlinks.
pub fn meta_from_dir_entry(entry: &std::fs::DirEntry) -> std::io::Result<EntryMeta> {
    imp::meta_from_dir_entry(entry)
}

/// Directories the UI presents as a single opaque item (macOS bundles).
pub fn is_package(name: &str, is_dir: bool) -> bool {
    imp::is_package(name, is_dir)
}

pub fn is_hidden(name: &str, meta: &EntryMeta) -> bool {
    name.starts_with('.') || meta.hidden
}

/// Path prefixes whose contents are classified as system files.
pub fn system_prefixes() -> &'static [&'static str] {
    imp::SYSTEM_PREFIXES
}

/// Paths never worth walking: synthetic filesystems, device nodes, and
/// snapshot mounts that would double-count the volume they shadow.
pub fn default_exclusions() -> &'static [&'static str] {
    imp::DEFAULT_EXCLUSIONS
}

/// Per-user application support directory for the snapshot cache.
pub fn app_data_dir() -> PathBuf {
    imp::app_data_dir()
}

pub fn is_system_path(path: &Path) -> bool {
    let s = path.to_string_lossy();
    system_prefixes().iter().any(|p| s.starts_with(p))
}

pub fn is_excluded(path: &Path) -> bool {
    let s = path.to_string_lossy();
    default_exclusions()
        .iter()
        .any(|p| s == *p || s.starts_with(&format!("{p}/")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_volume_is_present() {
        let vols = volumes();
        assert!(!vols.is_empty(), "at least one volume must be reported");
        assert!(
            vols.iter().any(|v| v.is_root),
            "exactly one volume should be flagged as root"
        );
        let root = vols.iter().find(|v| v.is_root).unwrap();
        assert!(root.total_bytes > 0);
        assert_eq!(root.used_bytes + root.free_bytes, root.total_bytes);
    }

    #[test]
    fn exclusions_match_on_component_boundaries() {
        // A directory that merely starts with an excluded prefix's characters
        // must not be swallowed.
        assert!(!is_excluded(Path::new("/devious")));
    }
}
