# Running Helios from a flash drive

Helios can run entirely from a USB stick, scan the machine it is plugged into,
and leave nothing behind on it. This is the natural way to use a read-only disk
tool: you carry it to the computer with the full disk instead of installing
software on someone else's machine. It is one 756 KB binary with no runtime, so
there is nothing else to carry.

Nothing about scanning changes in portable mode. The only thing that moves is
where Helios keeps *its own* data — the snapshot cache — which goes onto the
drive instead of into the host's application-support directory.

## Set up the drive

```sh
cargo build --release
./scripts/make-portable-drive.sh /Volumes/HELIOS
```

The script copies the binary, drops the portable marker beside it, creates the
data folder, and clears the macOS quarantine flag. It only ever copies into the
destination you name; it deletes nothing.

The result:

```
/Volumes/HELIOS/
├── helios                 the program
├── helios-portable.txt    the marker that turns portable mode on
└── HeliosData/
    └── snapshots/         every scan is cached here
```

Doing it by hand is two commands:

```sh
cp target/release/helios /Volumes/HELIOS/
touch /Volumes/HELIOS/helios-portable
```

## Confirm it is actually portable

```sh
/Volumes/HELIOS/helios --info
```

```
helios 0.1.0
mode        portable — data stays next to the program
program in  /Volumes/HELIOS
data        /Volumes/HELIOS/HeliosData
writable    yes

Cached scans (0)
  (none yet — run 'helios <path> --cache')
```

If `mode` says *standard*, the marker is not where Helios is looking: it has to
sit in the same directory as the binary.

## How Helios decides where to write

In order:

1. **`HELIOS_DATA_DIR`**, if set — an explicit redirect that wins over
   everything else. Useful for one-offs and scripts:
   ```sh
   HELIOS_DATA_DIR=/Volumes/HELIOS/HeliosData helios / --cache
   ```
2. **Portable mode**, if a `helios-portable` (or `helios-portable.txt`) file, or
   an existing `HeliosData` folder, sits beside the binary. Either trigger is
   enough — the folder alone keeps the drive portable even if the marker gets
   deleted, so a drive never silently starts writing to the host.
3. **The platform default** otherwise:
   `~/Library/Application Support/Helios` on macOS,
   `%LOCALAPPDATA%\Helios` on Windows.

## What ends up on the host machine

Nothing that Helios writes. Specifically:

| | Written where |
|---|---|
| Snapshot cache | The drive, under `HeliosData/snapshots/` |
| Exported reports | Wherever you point `-o` — put them on the drive |
| Preferences | Helios has none |
| Login items, daemons, receipts | Helios installs none |

To verify rather than take this on faith, watch the host's application-support
directory across a scan:

```sh
ls -la ~/Library/Application\ Support/ | grep -i helios   # before
/Volumes/HELIOS/helios / --cache
ls -la ~/Library/Application\ Support/ | grep -i helios   # after — unchanged
```

macOS still keeps its own records of what ran — Gatekeeper, `launchservices`,
shell history — none of which is Helios, and all of which apply to anything you
run from a drive.
Portable mode is about Helios not storing your data on someone else's machine;
it is not an anti-forensics tool, and should not be sold to anyone as one.

## Carrying the drive between machines

The drive accumulates one cache per machine, and Helios keeps them apart.

This matters more than it sounds. Volume identifiers are only unique on the
machine that issued them — every Mac has a `/dev/disk3s1s1`, and every Windows
box has a `C:`. A cache that keyed on the volume id alone would happily show you
last Tuesday's scan of a *different* computer's disk, which looks entirely
convincing and is entirely wrong.

So every snapshot records the machine that produced it and enough of the volume
to recognise it again — name, filesystem and capacity. Snapshot files are named
`<host>-<volume>.helios`, and a cached scan is only reused when the host id and
all three volume facts match. Anything else is treated as a cache miss, and the
volume is simply rescanned. The failure mode is a slower scan, never a wrong
number.

The host id is derived from the account name, home directory and startup
volume's identity. It is not a fingerprint of the machine in any privacy-relevant
sense, it never leaves the drive, and a collision costs a rescan.

## What the drive now contains

Snapshots hold **file and folder names, sizes and dates** for every machine you
have scanned. Never file contents — but a directory listing of someone's laptop
is not nothing.

If the drive will leave your control, encrypt it. On macOS, right-click the
volume in Finder → **Encrypt**, or format it as APFS (Encrypted) in Disk
Utility. Or just delete `HeliosData/snapshots/` when you are done; the app
rebuilds it on the next scan.

## Formatting the drive

| Format | macOS | Windows | Use it when |
|---|---|---|---|
| **exFAT** | Read/write | Read/write | One drive for both. The right default. |
| APFS / Mac OS Extended | Read/write | Not without third-party software | Macs only |
| NTFS | **Read-only** | Read/write | Windows only — Helios cannot cache to it from a Mac |

exFAT is the practical choice, with one wrinkle: it stores no POSIX permissions,
so macOS synthesizes them. Executables still run, but if the CLI comes back as
"permission denied" after copying, `chmod +x /Volumes/HELIOS/helios` fixes it.

## macOS specifics

**Gatekeeper.** A binary copied onto a drive carries a quarantine flag. The
setup script clears it; otherwise:

```sh
xattr -d com.apple.quarantine /Volumes/HELIOS/helios
```

**Full Disk Access is per-machine, and does not travel.** macOS grants it to the
program you launch — for a command-line tool, your terminal — on one specific
Mac. So on each computer, either grant it to that machine's terminal or accept a
scan that skips protected locations and says how much it could not see. That is
the honest trade of running portably; see [MACOS.md](MACOS.md).

**Architecture.** A binary built on Apple silicon will not run on an Intel Mac.
If the drive needs to work on both, build a universal binary with `lipo` — the
recipe is in [MACOS.md](MACOS.md).

**The drive is itself a volume**, so it shows up in `helios` output like any
other. Scanning it is harmless; it is just usually not what you came for.

**Ejecting.** Let a scan finish before ejecting. The snapshot cache is written
atomically — temporary file, then rename — so pulling the drive mid-write costs
you the cache, never a corrupt one. Helios validates every snapshot on load and
discards anything it cannot read.

## Windows

The same marker and folder rules apply, and `platform/windows.rs` is written
against them. Until Phase 6 of the [roadmap](ROADMAP.md) lands — that backend has
never been compiled — treat Windows portable use as untested rather than
supported. Note also that a single drive cannot hold one binary for both
platforms: build `helios` for macOS and `helios.exe` for Windows and put both on
it.

## Read-only or full drives

Helios degrades rather than failing:

```
warning: could not cache this scan: Read-only file system (os error 30)

/etc
  4.7 MB in 1,209 files, 152 folders · 62 ms
```

The scan runs, everything prints, and only the cache is lost — which costs a
full walk next time instead of a fast one. `helios --info` reports
`writable: no` up front, so you learn this before a long scan rather than after
it.

## Performance from USB

Scanning speed is governed by the disk being *scanned*, not the one Helios runs
from, so a slow USB stick does not slow down a scan of the internal SSD. What it
does affect:

- **Launch**, once, while the binary is read from the drive. USB 2.0 sticks are
  genuinely slow here; USB 3.0 is unremarkable.
- **Saving and loading the cache** — a few megabytes per volume. A 6 MB snapshot
  is nothing on USB 3.0 and noticeable on a bargain-bin USB 2.0 stick.

If a drive is slow enough to be annoying, run without `--cache`, or point
`HELIOS_DATA_DIR` at a fast temporary location for that session — accepting that
this does write to the host.

## Going back to a normal install

Delete the marker and the data folder from the drive, and Helios reverts to
storing data in the platform's usual place:

```sh
rm /Volumes/HELIOS/helios-portable.txt
rm -rf /Volumes/HELIOS/HeliosData
```

To wipe just the scans while keeping portable mode, delete
`HeliosData/snapshots/` and leave the folder itself.
