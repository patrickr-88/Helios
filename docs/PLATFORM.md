# Platform support

Everything OS-specific in Helios lives in `crates/helios-core/src/platform/`.
Nothing else — not the engine, not the program — contains a `#[cfg(target_os)]`
or an OS assumption. That is the whole portability strategy.

## The seam

A platform backend supplies seven things:

| Item | Purpose |
|---|---|
| `volumes() -> Vec<Volume>` | Enumerate mounts with capacity, used and free bytes |
| `entry_meta(path) -> EntryMeta` | No-follow metadata for one path |
| `meta_from_dir_entry(&DirEntry) -> EntryMeta` | The hot-path variant, using the open directory handle |
| `is_package(name, is_dir) -> bool` | Directories treated as one item (macOS bundles) |
| `SYSTEM_PREFIXES: &[&str]` | Paths classified as OS-owned |
| `DEFAULT_EXCLUSIONS: &[&str]` | Pseudo-filesystems and shadow mounts never worth walking |
| `app_data_dir() -> PathBuf` | Where the snapshot cache lives |

Plus `EntryMeta`, the one struct the walker consumes:

```rust
pub struct EntryMeta {
    pub is_dir: bool,
    pub is_symlink: bool,
    pub logical_size: u64,
    pub physical_size: u64,
    pub mtime: i64,
    pub hidden: bool,
    pub file_id: Option<(u64, u64)>,  // (device, inode); None disables hardlink dedup
    pub nlink: u64,
}
```

## macOS (`platform/macos.rs`) — shipping target

**Volumes** come from `getfsstat` with `MNT_NOWAIT`, bound directly rather than
through `libc` (which does not expose it uniformly across Apple targets); the
`$INODE64` symbol variant is selected by architecture. Synthetic filesystems
(`devfs`, `autofs`, `kernfs`, `fdesc`) and `MNT_DONTBROWSE` mounts are filtered
out. Free space uses `f_bavail`, not `f_bfree`, so Helios agrees with Finder
about what is actually available.

**Firmlinks are the big one.** On APFS, the sealed system volume and the data
volume are joined by firmlinks, and `/System/Volumes/Data` re-exposes storage
already reachable through `/`. Walking both double-counts effectively the entire
disk. `DEFAULT_EXCLUSIONS` covers `/System/Volumes/{VM,Preboot,Update,xarts,
iSCPreboot,Hardware}`, `/private/var/vm`, `/dev`, `/net`, `/home`, and
`/Volumes/.timemachine`.

**Bundles** — `.app`, `.framework`, `.photoslibrary`, `.xcodeproj` and friends
are flagged `PACKAGE`, so a bundle can be reported as the one thing a user
thinks it is — Xcode is one 15 GB item, not 40,000 files.

**Hidden** means dot-prefixed *or* carrying `UF_HIDDEN` in `st_flags`, matching
Finder.

**Physical size** is `st_blocks × 512`, which is what makes APFS clones and
sparse files report their real cost rather than their nominal one.

**Permissions.** macOS blocks `~/Library/Mail`, `~/Photos Library`, TCC-guarded
paths and much of `/private/var/db` unless the app has Full Disk Access. Helios
records each denial, flags the folder and reports the count after the scan. It
never asks for elevation and never works around a denial.

**Distribution.** One binary, built from source; minimum macOS 11. Nothing to
sign or notarize, because there is no bundle.

## Windows (`platform/windows.rs`) — written, needs a machine

The backend is written against the same seam and compiles under the same engine.
What it does today:

- **Volumes** via `GetLogicalDrives` → `GetDriveTypeW` → `GetDiskFreeSpaceExW` →
  `GetVolumeInformationW`. NTFS, ReFS, exFAT, FAT32, network shares
  (`DRIVE_REMOTE`) and removable media are all enumerated; the volume serial
  number is the cache key, so a USB drive keeps its snapshot across a change of
  drive letter.
- **Reparse points** (junctions *and* symlinks) are treated as links and never
  traversed — this is what stops the legacy `C:\Documents and Settings` junction
  from sending a walker into an infinite loop.
- **Hidden** is `FILE_ATTRIBUTE_HIDDEN | FILE_ATTRIBUTE_SYSTEM`.
- **Timestamps** convert FILETIME (100 ns ticks since 1601) to Unix seconds.
- **Exclusions** cover `pagefile.sys`, `hiberfil.sys`, `swapfile.sys` and
  `System Volume Information`.
- **Free metadata.** `DirEntry::metadata()` on Windows is served from the
  `WIN32_FIND_DATA` that `FindNextFileW` already returned — zero extra syscalls
  per entry, where Unix pays one `fstatat`.

Known gaps, honestly:

| Gap | Impact | Plan |
|---|---|---|
| No hardlink de-duplication (`file_id: None`) | Hardlinked files counted per path | `GetFileInformationByHandle` requires opening every file — roughly doubles scan time. Make it an option, default off. |
| Physical size = logical size | Compressed/sparse NTFS files over-reported | `GetCompressedFileSizeW` per file is the same cost problem; resolve lazily for the inspector, and for the top-N lists only. |
| Long paths | Paths over 260 chars may fail | Prefix `\\?\`, and set the long-path manifest flag. |
| Per-user junction loops | Handled (reparse points skipped) | Verify against a real Windows profile. |
| Not compiled or run | Unknown unknowns | Phase 6: CI runner + a real machine. |

The queries, the reports, the snapshot format and the program's entire output
layer need no Windows-specific work at all. The one cosmetic item is the box-
drawing characters in the folder tree, which need a UTF-8 code page in the
classic console host (Windows Terminal is fine as-is).

## Linux (`platform/linux.rs`) — development and CI

Not a shipping target, but a first-class one for keeping the seam honest: the
engine's tests run here on every push, which catches scanner regressions on a
cheap runner long before they reach a Mac. Volumes come from `/proc/mounts`
(with octal-escape unescaping for paths containing spaces) plus `statvfs`, with
a denylist of pseudo-filesystems.

If Helios ever ships on Linux, the gaps are cosmetic: no bundle concept, hidden
is name-based only, and the exclusions list would want widening for containers
and network mounts.

## Adding a platform

1. Create `platform/<os>.rs` implementing the seven items above.
2. Add the `#[cfg]` arms in `platform/mod.rs`.
3. Add the dependency under a `[target.'cfg(...)'.dependencies]` section.
4. Run `cargo test`. If the engine needed changes, the abstraction leaked and
   that is the bug to fix.

## Compatibility table

| Feature | macOS | Windows | Linux |
|---|---|---|---|
| Volume enumeration | ✅ `getfsstat` | ✅ `GetLogicalDrives` | ✅ `/proc/mounts` |
| Capacity / free | ✅ | ✅ | ✅ |
| Network volumes | ✅ smb, afp, nfs, webdav | ✅ `DRIVE_REMOTE` | ✅ nfs, cifs, sshfs |
| Removable media | ✅ | ✅ | ⚠️ heuristic (`/media`, `/mnt`) |
| Logical size | ✅ | ✅ | ✅ |
| Physical (on-disk) size | ✅ `st_blocks` | ❌ see gaps | ✅ `st_blocks` |
| Hardlink de-duplication | ✅ | ❌ see gaps | ✅ |
| Symlink / junction safety | ✅ | ✅ | ✅ |
| Hidden attribute | ✅ dot + `UF_HIDDEN` | ✅ attributes | ⚠️ dot only |
| Bundles as single items | ✅ | n/a | n/a |
| Shadow-mount exclusions | ✅ firmlinks | ✅ pagefile etc. | ✅ procfs etc. |
| Snapshot cache | ✅ | ✅ | ✅ |
