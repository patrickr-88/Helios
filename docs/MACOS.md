# Running Helios on a Mac

Everything below assumes macOS 11 (Big Sur) or later, on Apple silicon or
Intel. Helios needs no administrator rights, no privileged helper, and no
network access to run.

> **Status, stated up front.** The macOS backend is written but has never been
> compiled on a Mac — this project was developed on Linux. The portable engine
> (78 tests, verified byte-for-byte against `du`) works; the macOS-specific
> module in `crates/helios-core/src/platform/macos.rs` is unproven. If the build
> fails, [If it doesn't compile](#if-it-doesnt-compile) lists the four places
> that are most likely at fault and what to check in each.

## Quick start

```sh
git clone https://github.com/patrickr-88/helios.git
cd helios
./install.sh          # builds one binary into ~/.local/bin
helios                # every volume on this Mac
helios ~/Downloads    # scan something
```

That is the whole installation. One binary, no bundle, no runtime, no
preferences file, no login item, nothing in `/Library`.

## Prerequisites

Two, and one of them you probably have:

| Tool | Why | Install |
|---|---|---|
| **Xcode Command Line Tools** | Apple's linker and headers | `xcode-select --install` |
| **Rust** (1.77+) | Helios is built with it | `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \| sh` |

You do **not** need full Xcode. There is no Node, no npm, no bundler and no
webview.

After installing Rust, open a new terminal or run `source "$HOME/.cargo/env"` so
`cargo` is on your `PATH`.

Verify:

```sh
xcode-select -p        # /Library/Developer/CommandLineTools
rustc --version        # 1.77 or newer
```

## Running it

```sh
helios                          # every mounted volume, with capacity and free space

helios ~/Downloads              # scan a folder
helios /                        # scan the startup disk
helios / --cache                # cache it; the next run takes milliseconds
helios / --cache                # …this one

helios ~/Movies --tree -d 3     # folder tree, three levels
helios / --find "Xcode"         # everything matching, biggest first
helios ~/Movies --files --min-size 1GB

helios ~/Movies -o movies.csv   # reports: .csv, .json, .pdf
helios / -o storage-report.pdf
helios ~/Movies -o out.json && jq .summary out.json
```

Ctrl-C stops a scan and still prints what it found. `helios --help` lists every
flag; `helios --info` shows where the cache lives and what is in it.

If you would rather not install anything, `cargo run --release -- ~/Downloads`
works straight from the repository.

## First run

Run `helios` with no arguments first: it lists every mounted volume — your
startup disk, external drives and network shares — with capacity and free space,
so you can see what there is to scan.

Then `helios /`. A progress line appears on stderr with a completion estimate
that stays quiet for the first second and a half rather than guessing. Ctrl-C
stops it and still prints what was found so far.

A first scan of a full startup disk typically takes tens of seconds. Add
`--cache` and the next run of the same path reuses every unchanged folder,
usually finishing in a few seconds — a plain run always walks everything.

Helios never modifies, moves, renames or deletes anything it scans, and it
spawns no other programs.

## Granting Full Disk Access

Without it, macOS blocks Helios from reading `~/Library/Mail`, Photos
libraries, Messages data, parts of `/private/var/db`, and — depending on your
settings — `~/Desktop`, `~/Documents` and `~/Downloads`. Helios handles this
gracefully: it records every denial and prints the count afterwards, so a short
total is labelled rather than silently wrong.

macOS grants this permission to the program you *launch*, which for a
command-line tool is your terminal, not the binary:

1. Open **System Settings → Privacy & Security → Full Disk Access**.
2. Click **+**, choose your terminal (Terminal, iTerm, Ghostty…), switch it on.
3. Quit and reopen the terminal, then run the scan again — with `--cache`, force
   a full walk by omitting the flag, or the unreadable folders get reused as-is.

Granting a terminal full disk access is a real expansion of what anything you
run in it can read. That is the honest cost of scanning a Mac's whole disk
unprivileged, and it is worth granting deliberately rather than permanently.

## Checking Helios against the system

Helios should agree with the tools you already trust. Cross-check any folder:

```sh
# macOS `du` has no --apparent-size: it reports allocated blocks, so compare it
# against Helios's **on-disk** figure, not the logical size. Both count a
# hardlinked file once.
du -sk ~/Movies                                     # KB on disk

helios ~/Movies -o /tmp/movies.json
jq '.summary | {logical: .totalLogicalBytes, onDisk: .totalPhysicalBytes}' /tmp/movies.json
```

For the logical size, use Finder: select the folder and press **⌘I** (Get Info).
Finder's "Size" is the same byte count Helios shows as **Size**, and the on-disk
figure Helios shows beside it accounts for sparse files and APFS clones.

(On Linux, where `du --apparent-size -sb` does exist, Helios's logical totals
match it byte for byte — that is the cross-check in the README.)

**Where the two will legitimately differ:**

- **Purgeable space.** Finder's "available" includes space macOS can reclaim —
  mostly Time Machine local snapshots. Those bytes belong to no file Helios can
  see. Check with `tmutil listlocalsnapshots /`; a Mac can easily hold tens of
  gigabytes this way.
- **Unreadable locations.** Anything blocked by permissions is missing from
  Helios's total and counted in the "could not be read" figure.
- **APFS clones and sparse files.** Two paths can share the same blocks. Logical
  sizes add up to more than the disk actually holds; the on-disk column is the
  honest one.
- **Firmlinks.** Helios deliberately skips `/System/Volumes/Data` and its
  siblings when scanning `/`, because those mounts re-expose storage already
  counted through `/`. Counting both would roughly double every total.

## Running from a flash drive

Helios can run from a USB stick and keep every scan on the stick, writing
nothing to the Mac it is plugged into:

```sh
./scripts/make-portable-drive.sh /Volumes/HELIOS
/Volumes/HELIOS/helios --info      # "portable — data stays next to the program"
```

[PORTABLE.md](PORTABLE.md) covers the rest — drive formats, carrying one drive
between machines, and what the drive ends up holding.

## Where Helios keeps its data

One directory, containing only scan metadata — names, sizes and dates. Never
file contents, and never anything that leaves your Mac:

```
~/Library/Application Support/Helios/snapshots/
```

(Or `HeliosData/snapshots/` on the drive, when running portably.)

To remove everything Helios has ever stored:

```sh
rm -rf ~/Library/Application\ Support/Helios
```

Uninstalling is deleting the binary and that directory. There is nothing else —
no login item, no daemon, no preference file, no receipts, nothing in
`/Library`.

## Sharing a build with someone else

Copy the binary. It has no dependencies beyond the system libc, so
`scp target/release/helios them@mac:` works, as does putting it on a flash
drive.

Two things to know. A binary that arrives over the network or on a drive carries
macOS's quarantine flag, and an unsigned binary run from Finder will be refused;
run it from a terminal, or clear the flag:

```sh
xattr -d com.apple.quarantine ./helios
```

And a binary built on Apple silicon will not run on an Intel Mac, or vice versa.
Build one that does both:

```sh
rustup target add aarch64-apple-darwin x86_64-apple-darwin
cargo build --release --target aarch64-apple-darwin
cargo build --release --target x86_64-apple-darwin
lipo -create -output helios \
    target/aarch64-apple-darwin/release/helios \
    target/x86_64-apple-darwin/release/helios
```

If you want to distribute it widely, sign it with a Developer ID
(`codesign -s "Developer ID Application: …" --options runtime helios`) and
notarize the zip with `xcrun notarytool`. For a command-line tool that people
build themselves, neither is necessary.

## Troubleshooting

| Symptom | Cause and fix |
|---|---|
| `cargo: command not found` | Open a new terminal, or `source "$HOME/.cargo/env"` |
| `xcrun: error: invalid active developer path` | `xcode-select --install` |
| `helios: command not found` after install.sh | `~/.local/bin` is not on your `PATH`. The script tells you the line to add, or run it as `~/.local/bin/helios`. |
| "cannot be opened because it is from an unidentified developer" | Quarantine on a binary that arrived over the network. `xattr -d com.apple.quarantine ./helios` |
| Box-drawing characters in `--tree` look wrong | The terminal is not in UTF-8. Terminal.app and iTerm are by default. |
| Scan totals far below Finder's "used" | Full Disk Access not granted, and/or purgeable Time Machine snapshots. See the two sections above. |
| "N location(s) could not be read" | Same — grant Full Disk Access to your terminal and scan again without `--cache`. |
| Scanning `/` seems to miss an external drive | By design: other volumes are not crossed automatically. Scan the drive by its mount point, e.g. `helios /Volumes/Backup`. |
| A cached rescan misses a file that grew | Incremental rescans key on directory mtime, which does not change when a file is rewritten in place. Run without `--cache`. |
| Slow on an old spinning drive | Reduce worker threads: `helios <path> --threads 2`. The scan is I/O-bound, so fewer threads can be faster on a hard disk. |

## If it doesn't compile

Since the macOS backend has never been built on a Mac, the first `cargo build`
may fail. The engine, the queries, the reports and the program's output are
platform-independent and tested; anything that breaks will almost certainly be
in `crates/helios-core/src/platform/macos.rs`. In likely order:

1. **The `getfsstat` binding.** The `libc` crate does not expose it uniformly
   across Apple targets, so it is declared directly, with the `$INODE64` symbol
   variant selected on x86_64. If the linker cannot find the symbol, check the
   `extern "C"` block near the top of the file against your SDK.
2. **`libc::statfs` field types.** `f_bsize`, `f_blocks`, `f_bavail` and
   `f_flags` widths have changed across libc versions. The code uses
   `u64::from` for the widening; if a type mismatch appears, that is where.
3. **`st_flags` for the hidden attribute.** `platform/unix_shared.rs` reads it
   through `std::os::macos::fs::MetadataExt`, which is macOS-only and compiled
   under `#[cfg(target_os = "macos")]`.
4. **The mount filter.** `MNT_DONTBROWSE`, `MNT_LOCAL` and `MNT_RDONLY` are
   defined as literals rather than pulled from `libc`. They are stable, but
   worth checking if volumes are missing from `helios` or duplicated.

Run the portable tests first to confirm the engine itself is fine:

```sh
cargo test -p helios-core
```

If those pass and only the platform module fails, the problem is contained to
one file. Please open an issue with the exact compiler output, your macOS
version and `rustc --version` — that is precisely the feedback Phase 5 of the
[roadmap](ROADMAP.md) needs.

## What to expect performance-wise

Measured on a 4-core Linux VM, since that is the hardware this was verified on:
about 100,000 files per second, 17 MB of memory for a 173,000-node tree, a 16 ms
cached rescan when nothing changed, and 2 ms from launch to first output. Apple silicon with APFS should do
better on the walk and worse on the first scan of a cold volume. A full startup
disk with a large Photos library is the realistic worst case — expect tens of
seconds the first time and a few seconds thereafter.

If you measure something different, `docs/PERFORMANCE.md` explains where the
time goes and includes the commands to reproduce the numbers.
