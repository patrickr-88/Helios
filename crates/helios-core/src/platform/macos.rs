//! macOS platform backend (APFS/HFS+, `getfsstat`, bundles, firmlinks).

use std::ffi::CStr;
use std::fs;
use std::path::{Path, PathBuf};

use super::{EntryMeta, Volume};

#[path = "unix_shared.rs"]
mod unix_shared;

/// Prefixes whose contents are reported in the "System Files" category.
pub const SYSTEM_PREFIXES: &[&str] = &[
    "/System",
    "/Library",
    "/usr",
    "/bin",
    "/sbin",
    "/private/var/db",
    "/private/var/folders",
    "/private/var/vm",
    "/opt/homebrew",
];

/// Never walked. These are either synthetic filesystems (no real bytes), or
/// mounts that shadow storage already counted through the root volume — the
/// APFS firmlink pairs in particular would otherwise double-count every byte
/// on a modern macOS install.
pub const DEFAULT_EXCLUSIONS: &[&str] = &[
    "/dev",
    "/net",
    "/home",
    "/Volumes/.timemachine",
    "/System/Volumes/VM",
    "/System/Volumes/Preboot",
    "/System/Volumes/Update",
    "/System/Volumes/xarts",
    "/System/Volumes/iSCPreboot",
    "/System/Volumes/Hardware",
    "/private/var/vm",
];

/// Directory extensions macOS treats as opaque bundles.
const PACKAGE_EXTENSIONS: &[&str] = &[
    "app",
    "framework",
    "bundle",
    "kext",
    "plugin",
    "prefPane",
    "photoslibrary",
    "fcpbundle",
    "logicx",
    "sparsebundle",
    "rtfd",
    "xcodeproj",
    "xcworkspace",
];

const MNT_RDONLY: u32 = 0x0000_0001;
const MNT_LOCAL: u32 = 0x0000_1000;
const MNT_DONTBROWSE: u32 = 0x0010_0000;
const MNT_NOWAIT: libc::c_int = 2;

extern "C" {
    // The libc crate does not expose `getfsstat` uniformly across Apple
    // targets, so we bind it directly. x86_64 still needs the `$INODE64`
    // variant to get 64-bit inode fields; arm64 has only the modern ABI.
    #[cfg_attr(target_arch = "x86_64", link_name = "getfsstat$INODE64")]
    fn getfsstat(buf: *mut libc::statfs, bufsize: libc::c_int, flags: libc::c_int) -> libc::c_int;
}

fn cstr_to_string(buf: &[libc::c_char]) -> String {
    // SAFETY: the kernel NUL-terminates these fixed-size char arrays.
    unsafe { CStr::from_ptr(buf.as_ptr()) }
        .to_string_lossy()
        .into_owned()
}

pub fn volumes() -> Vec<Volume> {
    // SAFETY: the two-call pattern is the documented way to size the buffer;
    // we never read past the element count the second call returns.
    let mounts: Vec<libc::statfs> = unsafe {
        let count = getfsstat(std::ptr::null_mut(), 0, MNT_NOWAIT);
        if count <= 0 {
            return Vec::new();
        }
        // Mounts can appear between the two calls; over-allocate slightly and
        // trust the second count.
        let cap = count as usize + 8;
        let mut buf: Vec<libc::statfs> = Vec::with_capacity(cap);
        let bytes = (cap * std::mem::size_of::<libc::statfs>()) as libc::c_int;
        let got = getfsstat(buf.as_mut_ptr(), bytes, MNT_NOWAIT);
        if got <= 0 {
            return Vec::new();
        }
        buf.set_len((got as usize).min(cap));
        buf
    };

    mounts
        .iter()
        .filter_map(|m| {
            let fstype = cstr_to_string(&m.f_fstypename);
            let mount_point = PathBuf::from(cstr_to_string(&m.f_mntonname));
            let device = cstr_to_string(&m.f_mntfromname);
            let flags = m.f_flags;

            let is_network = matches!(
                fstype.as_str(),
                "smbfs" | "afpfs" | "nfs" | "webdav" | "ftp" | "cifs"
            );
            // Synthetic filesystems carry no user data; the sealed system
            // volume's helper mounts are excluded above.
            if matches!(fstype.as_str(), "devfs" | "autofs" | "kernfs" | "fdesc") {
                return None;
            }
            if flags & MNT_DONTBROWSE != 0 && !is_network && mount_point != Path::new("/") {
                return None;
            }

            // `u64::from` rather than `as`: f_bsize is u32 and f_bavail is u64
            // on current Apple targets, and the reflexive `From` impl makes the
            // widening explicit without a cast that lints as redundant.
            let block = u64::from(m.f_bsize);
            let total = m.f_blocks * block;
            // f_bavail (not f_bfree) is what the user can actually use; the
            // difference is the root reserve, which Finder also hides.
            let free = m.f_bavail * block;
            if total == 0 {
                return None;
            }

            Some(Volume {
                id: device.clone(),
                name: mount_point
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "Macintosh HD".to_string()),
                is_root: mount_point == Path::new("/"),
                mount_point,
                filesystem: fstype,
                total_bytes: total,
                free_bytes: free,
                used_bytes: total.saturating_sub(free),
                is_removable: flags & MNT_LOCAL != 0 && device.starts_with("/dev/disk") && {
                    // External media is not on the internal synthesized APFS
                    // container; disk0/disk1 are internal on every shipping Mac.
                    !device.starts_with("/dev/disk0") && !device.starts_with("/dev/disk1")
                },
                is_network,
                is_read_only: flags & MNT_RDONLY != 0,
            })
        })
        .collect()
}

pub fn entry_meta(path: &Path) -> std::io::Result<EntryMeta> {
    Ok(unix_shared::from_metadata(&fs::symlink_metadata(path)?))
}

pub fn meta_from_dir_entry(entry: &fs::DirEntry) -> std::io::Result<EntryMeta> {
    // `DirEntry::metadata` uses `fstatat(.., AT_SYMLINK_NOFOLLOW)` against the
    // already-open directory: one syscall, no path resolution, no link following.
    Ok(unix_shared::from_metadata(&entry.metadata()?))
}

pub fn is_package(name: &str, is_dir: bool) -> bool {
    is_dir
        && name.rsplit_once('.').is_some_and(|(_, ext)| {
            PACKAGE_EXTENSIONS
                .iter()
                .any(|p| p.eq_ignore_ascii_case(ext))
        })
}

pub fn app_data_dir() -> PathBuf {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    home.join("Library/Application Support/Helios")
}
