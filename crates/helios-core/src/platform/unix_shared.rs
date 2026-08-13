//! Metadata conversion shared by the macOS and Linux platform backends.

use std::fs::Metadata;
use std::os::unix::fs::MetadataExt;

use super::EntryMeta;

/// macOS `chflags` bit meaning "hide from the Finder". Ignored on Linux, where
/// hidden-ness is purely a naming convention.
#[cfg(target_os = "macos")]
const UF_HIDDEN: u32 = 0x8000;

pub fn from_metadata(md: &Metadata) -> EntryMeta {
    let file_type = md.file_type();

    #[cfg(target_os = "macos")]
    let hidden = {
        use std::os::macos::fs::MetadataExt as _;
        md.st_flags() & UF_HIDDEN != 0
    };
    #[cfg(not(target_os = "macos"))]
    let hidden = false;

    EntryMeta {
        is_dir: file_type.is_dir(),
        is_symlink: file_type.is_symlink(),
        logical_size: md.size(),
        // st_blocks is always in 512-byte units regardless of the filesystem's
        // block size, which is what makes sparse and APFS-cloned files report
        // their true on-disk cost here.
        physical_size: md.blocks().saturating_mul(512),
        mtime: md.mtime(),
        hidden,
        file_id: Some((md.dev(), md.ino())),
        nlink: md.nlink(),
    }
}
