//! Generic-Unix / Linux platform backend.
//!
//! Not a shipping target — Helios ships macOS first and Windows second — but a
//! first-class one for development and CI: the engine's tests run here, which
//! keeps the platform seam honest and means a regression in the scanner is
//! caught by a Linux runner long before it reaches a Mac.

use std::fs;
use std::path::{Path, PathBuf};

use super::{EntryMeta, Volume};

#[path = "unix_shared.rs"]
mod unix_shared;

pub const SYSTEM_PREFIXES: &[&str] = &["/usr", "/lib", "/lib64", "/bin", "/sbin", "/boot", "/etc"];

pub const DEFAULT_EXCLUSIONS: &[&str] = &["/proc", "/sys", "/dev", "/run"];

/// Filesystems that hold no user bytes on disk; counting them would inflate
/// every total.
const PSEUDO_FILESYSTEMS: &[&str] = &[
    "proc",
    // tmpfs is RAM, not disk: /dev/shm and the cgroup mounts would otherwise
    // show up in the sidebar as volumes with capacity the user cannot fill.
    "tmpfs",
    "sysfs",
    "devtmpfs",
    "devpts",
    "cgroup",
    "cgroup2",
    "securityfs",
    "debugfs",
    "tracefs",
    "pstore",
    "bpf",
    "configfs",
    "fusectl",
    "hugetlbfs",
    "mqueue",
    "autofs",
    "binfmt_misc",
    "rpc_pipefs",
    "selinuxfs",
    "nsfs",
    "ramfs",
];

const NETWORK_FILESYSTEMS: &[&str] = &["nfs", "nfs4", "cifs", "smbfs", "sshfs", "ceph", "9p"];

fn statvfs(path: &Path) -> Option<libc::statvfs> {
    let c_path = std::ffi::CString::new(path.as_os_str().as_encoded_bytes()).ok()?;
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: `c_path` is a valid NUL-terminated string and `st` is a valid
    // out-pointer for the duration of the call.
    let rc = unsafe { libc::statvfs(c_path.as_ptr(), &mut st) };
    (rc == 0).then_some(st)
}

/// Unescapes the octal sequences `/proc/mounts` uses for spaces and tabs.
fn unescape_mount_field(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        let octal: String = chars.clone().take(3).collect();
        match u8::from_str_radix(&octal, 8) {
            Ok(byte) if octal.len() == 3 => {
                out.push(byte as char);
                chars.nth(2);
            }
            _ => out.push('\\'),
        }
    }
    out
}

pub fn volumes() -> Vec<Volume> {
    let Ok(mounts) = fs::read_to_string("/proc/mounts") else {
        return Vec::new();
    };
    let mut seen: Vec<PathBuf> = Vec::new();
    let mut out = Vec::new();

    for line in mounts.lines() {
        let mut fields = line.split_whitespace();
        let (Some(device), Some(mount_point), Some(fstype), Some(opts)) =
            (fields.next(), fields.next(), fields.next(), fields.next())
        else {
            continue;
        };
        if PSEUDO_FILESYSTEMS.contains(&fstype) || fstype.starts_with("fuse.") && device == "none" {
            continue;
        }
        let mount_point = PathBuf::from(unescape_mount_field(mount_point));
        if seen.contains(&mount_point) {
            continue;
        }
        let Some(st) = statvfs(&mount_point) else {
            continue;
        };
        let block: u64 = if st.f_frsize > 0 {
            st.f_frsize
        } else {
            st.f_bsize
        };
        let total = st.f_blocks * block;
        if total == 0 {
            continue;
        }
        let free = st.f_bavail * block;
        seen.push(mount_point.clone());

        out.push(Volume {
            id: device.to_string(),
            name: mount_point
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "Root".to_string()),
            is_root: mount_point == Path::new("/"),
            filesystem: fstype.to_string(),
            total_bytes: total,
            free_bytes: free,
            used_bytes: total.saturating_sub(free),
            is_removable: mount_point.starts_with("/media") || mount_point.starts_with("/mnt"),
            is_network: NETWORK_FILESYSTEMS.contains(&fstype),
            is_read_only: opts.split(',').any(|o| o == "ro"),
            mount_point,
        });
    }
    out
}

pub fn entry_meta(path: &Path) -> std::io::Result<EntryMeta> {
    Ok(unix_shared::from_metadata(&fs::symlink_metadata(path)?))
}

pub fn meta_from_dir_entry(entry: &fs::DirEntry) -> std::io::Result<EntryMeta> {
    // Does not follow symlinks, and reuses the open directory handle.
    Ok(unix_shared::from_metadata(&entry.metadata()?))
}

pub fn is_package(_name: &str, _is_dir: bool) -> bool {
    false
}

pub fn app_data_dir() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("helios")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unescapes_octal_mount_paths() {
        assert_eq!(unescape_mount_field(r"/mnt/My\040Disk"), "/mnt/My Disk");
        assert_eq!(unescape_mount_field("/plain/path"), "/plain/path");
    }
}
