//! Windows platform backend (NTFS/ReFS, drive letters, network shares).
//!
//! Written against the same seam as macOS so the engine, query layer, reports
//! and UI compile unchanged. Ships in Phase 5 of the roadmap; the code is here
//! from day one so the abstraction is validated by a second real
//! implementation rather than by imagination.

use std::fs;
use std::os::windows::fs::MetadataExt;
use std::path::{Path, PathBuf};

use windows_sys::Win32::Storage::FileSystem::{
    GetDiskFreeSpaceExW, GetDriveTypeW, GetLogicalDrives, GetVolumeInformationW,
    FILE_ATTRIBUTE_HIDDEN, FILE_ATTRIBUTE_REPARSE_POINT, FILE_ATTRIBUTE_SYSTEM,
    DRIVE_CDROM, DRIVE_FIXED, DRIVE_REMOTE, DRIVE_REMOVABLE,
};

use super::{EntryMeta, Volume};

pub const SYSTEM_PREFIXES: &[&str] = &[
    "C:\\Windows",
    "C:\\Program Files\\WindowsApps",
    "C:\\ProgramData\\Microsoft",
    "C:\\$Recycle.Bin",
];

/// Windows equivalents of the synthetic mounts we skip on macOS: the paging and
/// hibernation files are reported by the OS but are not user data, and
/// `System Volume Information` is not readable even as an administrator.
pub const DEFAULT_EXCLUSIONS: &[&str] = &[
    "C:\\pagefile.sys",
    "C:\\hiberfil.sys",
    "C:\\swapfile.sys",
    "C:\\System Volume Information",
];

fn wide_to_string(buf: &[u16]) -> String {
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..len])
}

fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

pub fn volumes() -> Vec<Volume> {
    // SAFETY: every call below writes only into buffers we own and size.
    let mask = unsafe { GetLogicalDrives() };
    let mut out = Vec::new();

    for i in 0..26u32 {
        if mask & (1 << i) == 0 {
            continue;
        }
        let letter = (b'A' + i as u8) as char;
        let root = format!("{letter}:\\");
        let wide_root = to_wide(&root);

        let drive_type = unsafe { GetDriveTypeW(wide_root.as_ptr()) };
        if !matches!(drive_type, DRIVE_FIXED | DRIVE_REMOVABLE | DRIVE_REMOTE | DRIVE_CDROM) {
            continue;
        }

        let (mut free_to_caller, mut total, mut _total_free) = (0u64, 0u64, 0u64);
        let ok = unsafe {
            GetDiskFreeSpaceExW(
                wide_root.as_ptr(),
                &mut free_to_caller,
                &mut total,
                &mut _total_free,
            )
        };
        // A card reader with no card returns 0 here; skip rather than showing
        // an empty drive on the dashboard.
        if ok == 0 || total == 0 {
            continue;
        }

        let mut label = [0u16; 261];
        let mut fs_name = [0u16; 261];
        let mut serial = 0u32;
        let mut flags = 0u32;
        let mut max_component = 0u32;
        unsafe {
            GetVolumeInformationW(
                wide_root.as_ptr(),
                label.as_mut_ptr(),
                label.len() as u32,
                &mut serial,
                &mut max_component,
                &mut flags,
                fs_name.as_mut_ptr(),
                fs_name.len() as u32,
            );
        }

        let label = wide_to_string(&label);
        out.push(Volume {
            // The volume serial number keeps snapshot cache keys stable when a
            // removable drive is remounted under a different letter.
            id: format!("{serial:08X}"),
            name: if label.is_empty() {
                format!("{letter}:")
            } else {
                format!("{label} ({letter}:)")
            },
            mount_point: PathBuf::from(&root),
            filesystem: wide_to_string(&fs_name).to_lowercase(),
            total_bytes: total,
            free_bytes: free_to_caller,
            used_bytes: total.saturating_sub(free_to_caller),
            is_removable: drive_type == DRIVE_REMOVABLE,
            is_network: drive_type == DRIVE_REMOTE,
            is_read_only: drive_type == DRIVE_CDROM,
            is_root: letter == 'C',
        });
    }
    out
}

fn from_metadata(md: &fs::Metadata) -> EntryMeta {
    let attrs = md.file_attributes();
    EntryMeta {
        is_dir: md.is_dir(),
        // Junctions and symlinks both surface as reparse points; treating them
        // as links is what stops `C:\Documents and Settings` from sending the
        // walker into an infinite loop.
        is_symlink: attrs & FILE_ATTRIBUTE_REPARSE_POINT != 0,
        logical_size: md.file_size(),
        // On-disk size needs GetCompressedFileSizeW, an extra syscall per file
        // that would roughly double scan time. We report logical size and
        // resolve the true figure lazily when the UI inspects one file.
        physical_size: md.file_size(),
        mtime: windows_time_to_unix(md.last_write_time()),
        hidden: attrs & (FILE_ATTRIBUTE_HIDDEN | FILE_ATTRIBUTE_SYSTEM) != 0,
        // Requires opening the file for GetFileInformationByHandle; too costly
        // during a scan, so hardlink de-duplication is off on Windows.
        file_id: None,
        nlink: 1,
    }
}

/// FILETIME is 100-nanosecond ticks since 1601-01-01; Unix time is seconds
/// since 1970-01-01. 11_644_473_600 seconds separate the two epochs.
fn windows_time_to_unix(ticks: u64) -> i64 {
    (ticks / 10_000_000) as i64 - 11_644_473_600
}

pub fn entry_meta(path: &Path) -> std::io::Result<EntryMeta> {
    Ok(from_metadata(&fs::symlink_metadata(path)?))
}

pub fn meta_from_dir_entry(entry: &fs::DirEntry) -> std::io::Result<EntryMeta> {
    // Free on Windows: the metadata is already in the WIN32_FIND_DATA that
    // FindNextFileW returned, so this costs no syscall at all.
    Ok(from_metadata(&entry.metadata()?))
}

pub fn is_package(_name: &str, _is_dir: bool) -> bool {
    false
}

pub fn app_data_dir() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("C:\\Temp"))
        .join("Helios")
}
