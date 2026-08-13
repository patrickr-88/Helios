# Roadmap

## Where this stands

Phases 1–5 are built and tested: the engine, every view, search and filtering,
all three export formats, the snapshot cache, incremental rescans and portable
mode — 78 tests passing, totals verified byte-for-byte against `du`.

What remains before a 1.0 is mostly *proof on real hardware*: the macOS backend
has never been compiled on a Mac, and the Windows backend has never been
compiled at all. That distinction matters more than a percentage complete.
Everything portable is done and verified; everything platform-specific is
written and unverified.

**A graphical version was built and then removed** — a Tauri + React app with a
treemap, six views and a drill-down interface. It worked; it cost a webview, a
Node toolchain and an IPC bridge, and it made the product twenty times larger to
ship and to build. [TECHNOLOGY-CHOICE.md](TECHNOLOGY-CHOICE.md) records the
reasoning, and `git log` has the implementation if it is ever wanted back.

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

**Views**
- [x] Volume list with capacity, used, free and a usage bar
- [x] Largest files and largest folders, with proportional bars
- [x] Folder tree with sizes and share-of-parent at every level
- [x] Category breakdown across eight categories
- [x] Search across the whole tree

**Filtering**
- [x] By name, path, extension, size, hidden and system
- [x] Depth and per-list limits so no invocation floods the terminal

**Reporting**
- [x] Top-100 files and folders, storage summary, category breakdown
- [x] CSV, JSON and PDF export

**Platform and product**
- [x] macOS backend written (firmlinks, bundles, `UF_HIDDEN`, `getfsstat`)
- [x] Snapshot cache with incremental rescans
- [x] No network code, no telemetry — enforced by tests
- [x] Runs portably from external media, writing nothing to the host
- [x] One binary, one build command, no runtime and no installer
- [ ] A universal (Intel + Apple silicon) binary published somewhere
- [ ] Homebrew formula

## Phases

**Phase 1 — Engine** ✅
Arena tree, parallel walker, platform seam, categories, control and progress.
*Done: 100k files/sec, 56 bytes/node, `du`-identical totals.*

**Phase 2 — Query and views** ✅
Filters, top-N by heap, category aggregation, search.

**Phase 3 — Interface** ✅
Built as a desktop app (Tauri + React), then replaced by the command-line
program: volume list, top lists, folder tree, categories, search, progress.

**Phase 4 — Persistence and reports** ✅
Binary snapshot cache with atomic writes, incremental rescan (1.16 s → 16 ms),
CSV/JSON/PDF export.

**Phase 5 — Portable mode** ✅
Data beside the binary, per-host snapshot identity so a drive carried between
machines cannot show one computer's scan as another's, and a setup script.

**Phase 6 — macOS hardening** ← next
Build and run on real Macs. Verify against Finder and `du` on APFS, including a
Time Machine volume and an SMB share. Test on a genuinely full 2 TB drive with a
Photos library and local snapshots. Publish a universal binary.

**Phase 7 — Windows**
Compile `platform/windows.rs`, add a Windows CI runner, then work the known
gaps: `\\?\` long paths, `GetCompressedFileSizeW` for on-disk size (lazily),
optional hardlink de-duplication, drive-letter vs. GUID volume identity. Verify
on NTFS, ReFS, exFAT, a mapped network drive and an external SSD. The UI should
need nothing but a title-bar adjustment.

**Phase 8 — Performance on real hardware**
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

**Watch mode.** `helios / --watch` re-running a cached scan on an interval and
printing what changed. Cheap once snapshot comparison exists, and it is the
closest thing to a GUI that a terminal does well.

**Scheduled background rescans.** Wanted, but at odds with "lightweight" — a
daemon touching the disk on a timer is exactly what this tool should not be. If
built: opt-in, idle-and-on-power only, and visibly interruptible.

**Saved filters and views.** "Videos over 1 GB not opened in a year" is a query
worth keeping. Cheap to add once someone asks twice.

**A graphical version, again.** Only if a treemap turns out to be the thing
people actually want — it is the one view a terminal cannot reproduce. The
engine is ready for it; see [TECHNOLOGY-CHOICE.md](TECHNOLOGY-CHOICE.md).

**Sparkline history.** Once snapshot comparison exists, a per-folder size trend
over the last N scans is nearly free and answers "when did this start growing?".

**Shell completions.** `helios --completions zsh`. Twenty lines, and the kind of
thing people notice.

**A `--json` mode for stdout**, so the tool composes with `jq` without writing a
file first.

## Explicitly not planned

**Deleting, moving or compressing files.** The read-only guarantee is the
product's foundation and the reason it can be trusted with a whole disk. A
`--delete` flag would make every other safety claim conditional. Helios finds
the problem; `rm` solves it, deliberately, with you watching.

**Cloud sync or accounts.** Nothing to sync, no one to be.

**Raw MFT / catalog parsing.** An order of magnitude faster, and requires
administrator privileges plus a second scanner to maintain. Wrong trade for a
tool that markets itself as safe and unprivileged.

**Telemetry, even anonymous.** Not "off by default" — absent, and tested for.

**A plugin system.** Third-party code inside a process that reads your entire
disk would undo the security story in one release.

**An interactive terminal UI.** A full-screen ncurses-style browser is a real
option and a real weight increase — another dependency, an alternate screen
buffer, key handling, and output that no longer pipes. The tree view covers most
of what it would offer.

## Open-source readiness

- [x] MIT license
- [x] Documented architecture, engine design, API, platform seam, threat model
- [x] Tests that run on any platform with no display and no bundling
- [x] A CLI that makes the engine usable and benchmarkable on its own
- [x] Four dependencies; ~3,500 lines of readable Rust
- [x] CI matrix (macOS, Windows, Linux) with fmt, clippy and the test suite
      — the Windows job is non-blocking until Phase 7 lands
- [ ] CONTRIBUTING.md and issue templates
- [ ] Homebrew formula
