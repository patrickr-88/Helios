# Roadmap

## Where this stands

Phases 1–4 are built and tested. The engine, the six views, search and
filtering, all three export formats, the snapshot cache and incremental rescans
are working, with 75 tests passing and totals verified byte-for-byte against
`du`. What remains before a 1.0 is mostly *proof on real hardware*: the macOS
backend has never been compiled on a Mac, and the Windows backend has never been
compiled at all.

That distinction matters more than a percentage complete. Everything portable is
done and verified; everything platform-specific is written and unverified.

## MVP — what 1.0 must contain

**Scanning**
- [x] Enumerate every mounted volume with capacity, used and free
- [x] Parallel recursive scan with rolled-up folder sizes
- [x] Symlinks recorded, never followed (loops terminate)
- [x] Hardlinks counted once
- [x] Unreadable locations skipped, recorded and surfaced
- [x] Progress with an honest ETA
- [x] Pause, resume, cancel — cancel returns a usable partial tree
- [ ] Verified on a real Mac against Finder's Get Info numbers

**Visualization**
- [x] Treemap (squarified, canvas, drill-down)
- [x] Folder tree with sizes and share-of-parent at every level
- [x] Largest files / largest folders, top 100
- [x] Category view across eight categories
- [x] Dashboard with capacity, usage and the top consumers

**Navigation and filtering**
- [x] Drill down by click, climb by breadcrumb
- [x] Details panel with metadata and Reveal in Finder
- [x] Filter by name, path, extension, size, date, hidden, system, category

**Reporting**
- [x] Top-100 files and folders, storage summary, category breakdown
- [x] CSV, JSON and PDF export

**Platform and product**
- [x] macOS backend written (firmlinks, bundles, `UF_HIDDEN`, `getfsstat`)
- [x] Native appearance, light and dark, responsive
- [x] Snapshot cache with incremental rescans
- [x] No network code, no telemetry — enforced by tests
- [x] Runs portably from external media, writing nothing to the host
- [ ] Signed, notarized `.dmg`
- [ ] Full Disk Access onboarding flow
- [x] App icon

## Phases

**Phase 1 — Engine** ✅
Arena tree, parallel walker, platform seam, categories, control and progress.
*Done: 100k files/sec, 56 bytes/node, `du`-identical totals.*

**Phase 2 — Query and layout** ✅
Filters, top-N by heap, category aggregation, squarified treemap, CLI.

**Phase 3 — Shell and UI** ✅
Tauri commands, scan lifecycle, six views, search, details, dark mode.

**Phase 4 — Persistence and reports** ✅
Binary snapshot cache with atomic writes, incremental rescan (1.16 s → 16 ms),
CSV/JSON/PDF export.

**Phase 5 — macOS hardening** ← next
Build and run on real Macs. Verify against Finder and DaisyDisk on APFS,
including a Time Machine volume and an SMB share. Full Disk Access onboarding.
Signing, notarization, `.dmg`. Test on a genuinely full 2 TB drive with a
Photos library and a Time Machine local snapshot.

**Phase 6 — Windows**
Compile `platform/windows.rs`, add a Windows CI runner, then work the known
gaps: `\\?\` long paths, `GetCompressedFileSizeW` for on-disk size (lazily),
optional hardlink de-duplication, drive-letter vs. GUID volume identity. Verify
on NTFS, ReFS, exFAT, a mapped network drive and an external SSD. The UI should
need nothing but a title-bar adjustment.

**Phase 7 — Performance on real hardware**
`getattrlistbulk` on macOS (one syscall per batch instead of per entry) — the
largest remaining win. `NtQueryDirectoryFile` on Windows. Possibly mmap-ed
snapshots once trees exceed a few hundred megabytes.

## Post-MVP

Ordered by expected value, with the reasoning that ranks them.

**Compare two snapshots.** "What grew since last week?" is the question users
ask second, right after "what is big?", and Helios already keeps timestamped
snapshots — the diff is a tree walk over two arenas. Highest value per unit of
work in the whole list.

**Duplicate detection.** Size-bucket, then hash only the collisions, then
compare only the survivors byte for byte. Genuinely useful and genuinely
dangerous to get wrong, so: report only, never a delete button, and never a
"probably identical" claim based on a hash alone.

**Cleanup suggestions, read-only.** Xcode DerivedData, npm caches, old iOS
backups, Homebrew caches, `.Trash`. Show what and where, explain what it is, and
let the user act in Finder. Helios points; it does not delete.

**Menu bar companion.** A small always-available readout of free space with a
"scan now" item, sharing the engine. The natural place to revisit SwiftUI, since
native fit matters most there and the scanning lives in the shared crate anyway.

**Scheduled background rescans.** Wanted, but at odds with "lightweight" — a
daemon touching the disk on a timer is exactly what this tool should not be. If
built: opt-in, idle-and-on-power only, and visibly interruptible.

**Saved filters and views.** "Videos over 1 GB not opened in a year" is a query
worth keeping. Cheap to add once someone asks twice.

**Sunburst / ring view.** A second visual idiom (DaisyDisk's signature). The
layout is a different function over the same arena; the canvas and hit-testing
carry over.

**Sparkline history.** Once snapshot comparison exists, a per-folder size trend
over the last N scans is nearly free and answers "when did this start growing?".

**Localization.** The UI has few strings; the formatting layer already uses
`Intl`. Worth doing when there are users to localize for.

**Accessibility pass with a real screen-reader user.** The structure is right
(semantic tables, colour never load-bearing alone), but "structurally correct"
and "usable" are different claims and only testing settles it.

## Explicitly not planned

**Deleting, moving or compressing files.** The read-only guarantee is the
product's foundation and the reason it can be trusted with a whole disk. A
delete button would make every other safety claim conditional. Helios finds the
problem; Finder solves it.

**Cloud sync or accounts.** Nothing to sync, no one to be.

**Raw MFT / catalog parsing.** An order of magnitude faster, and requires
administrator privileges plus a second scanner to maintain. Wrong trade for a
tool that markets itself as safe and unprivileged.

**Telemetry, even anonymous.** Not "off by default" — absent, and tested for.

**A plugin system.** Third-party code inside a process that reads your entire
disk would undo the security story in one release.

## Open-source readiness

- [x] MIT license
- [x] Documented architecture, engine design, API, platform seam, threat model
- [x] Tests that run on any platform with no display and no bundling
- [x] A CLI that makes the engine usable and benchmarkable on its own
- [x] Three dependencies in the engine; ~3,000 lines of readable Rust
- [x] CI matrix (macOS, Windows, Linux) with fmt, clippy and the test suite
      — the Windows job is non-blocking until Phase 6 lands
- [ ] CONTRIBUTING.md and issue templates
- [ ] Homebrew cask
