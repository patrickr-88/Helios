# Running Helios on a Mac

Everything below assumes macOS 11 (Big Sur) or later, on Apple silicon or
Intel. Helios needs no administrator rights, no privileged helper, and no
network access to run.

> **Status, stated up front.** The macOS backend is written but has never been
> compiled on a Mac — this project was developed on Linux. The portable engine
> (75 tests, verified byte-for-byte against `du`) works; the macOS-specific
> module in `crates/helios-core/src/platform/macos.rs` is unproven. If the build
> fails, [If it doesn't compile](#if-it-doesnt-compile) lists the four places
> that are most likely at fault and what to check in each.

## Quick start

```sh
git clone https://github.com/patrickr-88/helios.git
cd helios

# 1. The engine and the command-line tool — no Node, no app bundle.
cargo test
cargo build --release -p helios-cli
./target/release/helios volumes

# 2. The full desktop app, in development mode.
npm install
npm run app
```

## Prerequisites

| Tool | Why | Install |
|---|---|---|
| **Xcode Command Line Tools** | Apple's linker, headers and `codesign` | `xcode-select --install` |
| **Rust** (1.77+) | The engine and the app shell | `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \| sh` |
| **Node** (20+) | Builds the interface | `brew install node`, or [nodejs.org](https://nodejs.org) |

You do **not** need full Xcode — the Command Line Tools are enough. Helios uses
the system WebKit, so there is no browser engine to download.

After installing Rust, either open a new terminal or run `source
"$HOME/.cargo/env"` so `cargo` is on your `PATH`.

Verify:

```sh
xcode-select -p        # /Library/Developer/CommandLineTools
rustc --version        # 1.77 or newer
node --version         # v20 or newer
```

## Three ways to run it

### 1. The command-line tool

The fastest way to confirm the engine works on your Mac, and useful on its own.

```sh
cargo build --release -p helios-cli

# Every mounted volume, with capacity and free space.
./target/release/helios volumes

# Scan a folder and print the summary, top-20 lists and category breakdown.
./target/release/helios scan ~/Downloads

# Scan the whole startup disk. Expect unreadable paths without Full Disk Access.
./target/release/helios scan /

# Cache the result so the next scan of the same tree takes milliseconds.
./target/release/helios scan / --cache
./target/release/helios scan / --cache      # again — watch "folders reused"

# Reports.
./target/release/helios report ~/Movies --format csv --out movies.csv
./target/release/helios report ~/Movies --format pdf --out movies.pdf
./target/release/helios report ~/Movies --format json | jq .summary
```

Ctrl-C cancels a scan and still prints the partial result rather than killing
the process. `helios --help` lists every flag.

Optionally put it on your `PATH`:

```sh
cargo install --path crates/helios-cli    # installs to ~/.cargo/bin/helios
```

### 2. The interface in a browser

No Rust build, no app bundle — the UI runs against a synthetic volume so you
can work on the interface alone:

```sh
npm install
npm run dev            # http://localhost:5173
```

Everything renders and every view works, but the data is fake. This is the
browser fallback in `src/lib/mock.ts`, and it is never reachable inside the real
app.

### 3. The desktop app

Development, with hot reload and devtools (right-click → Inspect Element):

```sh
npm run app
```

The first run compiles Tauri and WebKit bindings and takes several minutes;
later runs are seconds. Leave it running — edits to `src/` reload instantly, and
edits to Rust rebuild automatically.

A real, installable build:

```sh
npm run app:build
```

Output lands in:

```
src-tauri/target/release/bundle/macos/Helios.app
src-tauri/target/release/bundle/dmg/Helios_0.1.0_aarch64.dmg
```

Drag `Helios.app` to `/Applications` and launch it like any other app.

To build one binary that runs natively on both Apple silicon and Intel:

```sh
rustup target add aarch64-apple-darwin x86_64-apple-darwin
npm run tauri build -- --target universal-apple-darwin
```

#### Opening an unsigned build

A build you made yourself is not signed by a Developer ID, so Gatekeeper will
refuse it on first launch ("Helios cannot be opened because it is from an
unidentified developer", or "is damaged").

Either **right-click the app → Open → Open** — the one-time override — or strip
the quarantine flag:

```sh
xattr -dr com.apple.quarantine /Applications/Helios.app
```

Only do that for a build you produced yourself or one you trust. See
[Distributing a signed build](#distributing-a-signed-build) for the real fix.

## First run

1. The sidebar lists every mounted volume: your startup disk, external drives,
   and network shares.
2. Click one. If Helios has scanned it before it loads the cached snapshot
   instantly; otherwise it starts a scan.
3. Scan progress appears in a bar under the toolbar, with an estimate that stays
   quiet for the first second and a half rather than guessing. **Pause** parks
   the scanner (no CPU at all), **Stop** keeps whatever it found so far.
4. When the scan finishes, the Dashboard fills in and every other view is
   available: Treemap, Folders, Largest, Categories, Reports.

A first scan of a full startup disk typically takes tens of seconds. Afterwards,
**Rescan** reuses unchanged folders and usually finishes in a few seconds;
**Full scan** always walks everything.

Helios never modifies, moves, renames or deletes anything it scans. Its only
outward action is **Reveal in Finder** in the details panel.

## Granting Full Disk Access

Without it, macOS blocks Helios from reading `~/Library/Mail`, Photos
libraries, Messages data, parts of `/private/var/db`, and — depending on your
settings — `~/Desktop`, `~/Documents` and `~/Downloads`. Helios handles this
gracefully: it records every denial, flags those folders as **partial**, and
tells you on the dashboard roughly what share of the volume the scan actually
covered. But the totals will be short.

To grant it:

1. Open **System Settings → Privacy & Security → Full Disk Access**.
2. Click **+**, choose `Helios.app`, and make sure its switch is on.
3. Quit and reopen Helios, then run a **Full scan** (an incremental rescan will
   keep reusing the folders it could not read before).

For the CLI, grant Full Disk Access to the *terminal application* you run it
from (Terminal, iTerm, Ghostty…), not to the `helios` binary.

While testing, you can revoke the grant and start over:

```sh
tccutil reset SystemPolicyAllFiles app.helios.diskviz
```

## Checking Helios against the system

Helios should agree with the tools you already trust. Cross-check any folder:

```sh
# macOS `du` has no --apparent-size: it reports allocated blocks, so compare it
# against Helios's **on-disk** figure, not the logical size. Both count a
# hardlinked file once.
du -sk ~/Movies                                     # KB on disk

./target/release/helios report ~/Movies --format json \
  | jq '.summary | {logical: .totalLogicalBytes, onDisk: .totalPhysicalBytes}'
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
```

Full Disk Access still has to be granted on each Mac, since macOS ties that
permission to an app at a path on a machine. [PORTABLE.md](PORTABLE.md) covers
the rest — drive formats, Gatekeeper, carrying one drive between machines, and
what the drive ends up holding.

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

Or use **Forget scan** in the app, which drops the in-memory tree and deletes
that volume's cache file. Uninstalling is just deleting `Helios.app` and that
directory; there is nothing else, no login item, no daemon, no receipts.

## Distributing a signed build

Needed only if you want other people to open the app without the Gatekeeper
override. Requires a paid Apple Developer account.

```sh
# Signing identity, from `security find-identity -v -p codesigning`
export APPLE_SIGNING_IDENTITY="Developer ID Application: Your Name (TEAMID)"

# Notarization credentials — use an app-specific password from appleid.apple.com
export APPLE_ID="you@example.com"
export APPLE_PASSWORD="abcd-efgh-ijkl-mnop"
export APPLE_TEAM_ID="TEAMID"

npm run app:build
```

Tauri signs the bundle and submits it for notarization. To do the notarization
step by hand, or to check on a submission:

```sh
xcrun notarytool submit \
  "src-tauri/target/release/bundle/dmg/Helios_0.1.0_aarch64.dmg" \
  --apple-id "$APPLE_ID" --team-id "$APPLE_TEAM_ID" --password "$APPLE_PASSWORD" \
  --wait

xcrun stapler staple "src-tauri/target/release/bundle/dmg/Helios_0.1.0_aarch64.dmg"
spctl -a -vvv -t install "src-tauri/target/release/bundle/macos/Helios.app"
```

Helios needs no special entitlements: it is an unprivileged read-only app, and
Full Disk Access is granted by the user in System Settings rather than requested
by the bundle.

## Troubleshooting

| Symptom | Cause and fix |
|---|---|
| `cargo: command not found` | Open a new terminal, or `source "$HOME/.cargo/env"` |
| `xcrun: error: invalid active developer path` | `xcode-select --install` |
| `npm run app` shows a blank window | The Vite dev server did not start, or port 5173 is taken. Run `npm run dev` in another terminal and check for errors. |
| First `npm run app` seems stuck | It is compiling Tauri from source — several minutes on the first run only. |
| "Helios is damaged and can't be opened" | Gatekeeper quarantine on an unsigned build. Right-click → Open, or `xattr -dr com.apple.quarantine /Applications/Helios.app` |
| Scan totals far below Finder's "used" | Full Disk Access not granted, and/or purgeable Time Machine snapshots. See the two sections above. |
| Many folders badged **partial** | Same — grant Full Disk Access and run a **Full scan**, not a rescan. |
| Scanning `/` seems to miss an external drive | By design. Other volumes are not crossed automatically; select the drive in the sidebar, or scan with `cross_filesystem` enabled. |
| Rescan misses a file that grew | Incremental rescans key on directory mtime, which does not change when a file is rewritten in place. Use **Full scan**. |
| App feels slow on an old spinning drive | Reduce worker threads: `helios scan <path> --threads 2`. The scan is I/O-bound, so fewer threads can be faster on a hard disk. |

## If it doesn't compile

Since the macOS backend has never been built on a Mac, the first `cargo build`
may fail. The engine, the queries, the treemap, the reports and the UI are
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
   worth checking if volumes are missing from the sidebar or duplicated.

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
about 100,000 files per second, 17 MB of memory for a 173,000-node tree, and a
16 ms incremental rescan when nothing changed. Apple silicon with APFS should do
better on the walk and worse on the first scan of a cold volume. A full startup
disk with a large Photos library is the realistic worst case — expect tens of
seconds the first time and a few seconds thereafter.

If you measure something different, `docs/PERFORMANCE.md` explains where the
time goes and includes the commands to reproduce the numbers.
