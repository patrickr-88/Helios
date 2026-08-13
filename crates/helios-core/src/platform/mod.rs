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

/// Per-user application support directory, as the OS defines it.
pub fn app_data_dir() -> PathBuf {
    imp::app_data_dir()
}

/// Name of the folder Helios keeps its data in when running portably.
pub const PORTABLE_DIR: &str = "HeliosData";

/// Marker file that switches Helios into portable mode.
pub const PORTABLE_MARKER: &str = "helios-portable";

/// Where Helios stores *its own* data — the snapshot cache.
///
/// Three sources, in order:
///
/// 1. `HELIOS_DATA_DIR`, for scripted and one-off use.
/// 2. Portable mode: a `helios-portable` marker file, or an existing
///    `HeliosData` folder, sitting beside the app. Data then lives on the same
///    device the app was launched from — the point being that running Helios
///    from a flash drive leaves nothing behind on the host.
/// 3. The platform's application-support directory.
///
/// Note this only ever affects where Helios writes; it never affects what it
/// reads or scans.
pub fn data_dir() -> PathBuf {
    resolve_data_dir(
        std::env::var_os("HELIOS_DATA_DIR").map(PathBuf::from),
        app_directory(),
        imp::app_data_dir(),
    )
}

/// True when Helios is storing its data next to the executable.
pub fn is_portable() -> bool {
    portable_root().is_some()
}

/// The portable data directory, if portable mode is active by marker or by an
/// existing data folder. An explicit `HELIOS_DATA_DIR` is *not* portable mode:
/// it is a redirect, and the caller asked for it by name.
pub fn portable_root() -> Option<PathBuf> {
    let app_dir = app_directory()?;
    portable_dir_for(&app_dir)
}

fn portable_dir_for(app_dir: &Path) -> Option<PathBuf> {
    let data = app_dir.join(PORTABLE_DIR);
    // Either trigger is enough: the marker is how a user opts in, and the
    // existing folder is how the drive keeps working after the first run even
    // if the marker is deleted.
    let opted_in = app_dir.join(PORTABLE_MARKER).exists()
        || app_dir.join(format!("{PORTABLE_MARKER}.txt")).exists()
        || data.is_dir();
    opted_in.then_some(data)
}

pub(crate) fn resolve_data_dir(
    env_override: Option<PathBuf>,
    app_dir: Option<PathBuf>,
    default: PathBuf,
) -> PathBuf {
    if let Some(dir) = env_override.filter(|d| !d.as_os_str().is_empty()) {
        return dir;
    }
    app_dir
        .as_deref()
        .and_then(portable_dir_for)
        .unwrap_or(default)
}

/// The directory the app was launched from.
///
/// On macOS the executable lives inside `Helios.app/Contents/MacOS/`, and
/// writing anything inside a bundle breaks its code signature — so this returns
/// the directory *containing* the bundle, which on a flash drive is the drive's
/// root.
pub fn app_directory() -> Option<PathBuf> {
    std::env::current_exe().ok().map(|exe| {
        // Resolve symlinks first, so a `helios` symlinked onto the PATH still
        // reports the drive it actually lives on.
        let exe = exe.canonicalize().unwrap_or(exe);
        bundle_aware_parent(&exe)
    })
}

fn bundle_aware_parent(exe: &Path) -> PathBuf {
    for ancestor in exe.ancestors() {
        let is_bundle = ancestor
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("app"));
        if is_bundle {
            if let Some(parent) = ancestor.parent() {
                return parent.to_path_buf();
            }
        }
    }
    exe.parent().unwrap_or(exe).to_path_buf()
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

    #[test]
    fn data_dir_prefers_an_explicit_override() {
        let chosen = resolve_data_dir(
            Some(PathBuf::from("/somewhere/else")),
            Some(PathBuf::from("/Volumes/HELIOS")),
            PathBuf::from("/home/me/.local/share/helios"),
        );
        assert_eq!(chosen, Path::new("/somewhere/else"));

        // An empty variable is treated as unset rather than as the root.
        let chosen = resolve_data_dir(
            Some(PathBuf::new()),
            None,
            PathBuf::from("/home/me/.local/share/helios"),
        );
        assert_eq!(chosen, Path::new("/home/me/.local/share/helios"));
    }

    #[test]
    fn a_marker_beside_the_app_switches_to_portable_mode() {
        let drive = tempfile::tempdir().unwrap();
        let default = PathBuf::from("/home/me/.local/share/helios");

        // No marker: the OS location wins.
        assert_eq!(
            resolve_data_dir(None, Some(drive.path().to_path_buf()), default.clone()),
            default
        );

        std::fs::write(drive.path().join(PORTABLE_MARKER), b"").unwrap();
        assert_eq!(
            resolve_data_dir(None, Some(drive.path().to_path_buf()), default.clone()),
            drive.path().join(PORTABLE_DIR)
        );
    }

    #[test]
    fn an_existing_data_folder_keeps_the_drive_portable() {
        // Someone deletes the marker but keeps the data: the drive must not
        // silently start writing to the host machine instead.
        let drive = tempfile::tempdir().unwrap();
        std::fs::create_dir(drive.path().join(PORTABLE_DIR)).unwrap();

        assert_eq!(
            resolve_data_dir(
                None,
                Some(drive.path().to_path_buf()),
                PathBuf::from("/home/me/.local/share/helios")
            ),
            drive.path().join(PORTABLE_DIR)
        );
    }

    #[test]
    fn portable_data_lands_beside_a_mac_bundle_never_inside_it() {
        // Writing inside Helios.app would invalidate its code signature.
        assert_eq!(
            bundle_aware_parent(Path::new(
                "/Volumes/HELIOS/Helios.app/Contents/MacOS/Helios"
            )),
            Path::new("/Volumes/HELIOS")
        );
        // A bare executable just uses its own directory.
        assert_eq!(
            bundle_aware_parent(Path::new("/Volumes/HELIOS/helios")),
            Path::new("/Volumes/HELIOS")
        );
    }
}
