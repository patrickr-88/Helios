# Running Helios from a flash drive

Helios can run entirely from a USB stick, scan the machine it is plugged into,
and leave nothing behind on it. This is the natural way to use a read-only disk
tool: you carry it to the computer with the full disk instead of installing
software on someone else's machine.

Nothing about scanning changes in portable mode. The only thing that moves is
where Helios keeps *its own* data — the snapshot cache — which goes onto the
drive instead of into the host's application-support directory.

## Set up the drive

```sh
# Build whichever you want on the drive.
cargo build --release -p helios-cli     # the command-line tool
npm run app:build                       # the desktop app (macOS)

./scripts/make-portable-drive.sh /Volumes/HELIOS
```

The script copies what you built, drops the portable marker beside it, creates
the data folder, and clears the macOS quarantine flag from the app. It only ever
copies into the destination you name; it deletes nothing.

The result:

```
/Volumes/HELIOS/
├── Helios.app             double-click to run
├── helios                 the command-line tool
├── helios-portable.txt    the marker that turns portable mode on
└── HeliosData/
    └── snapshots/         every scan is cached here
```

Doing it by hand is equally fine — copy the app or binary onto the drive and
create either the marker file or the `HeliosData` folder next to it:

```sh
cp -R src-tauri/target/release/bundle/macos/Helios.app /Volumes/HELIOS/
cp target/release/helios /Volumes/HELIOS/
touch /Volumes/HELIOS/helios-portable
```

## Confirm it is actually portable

```sh
/Volumes/HELIOS/helios paths
```

```
mode          portable — data stays with the app
app directory /Volumes/HELIOS
data          /Volumes/HELIOS/HeliosData
snapshots     /Volumes/HELIOS/HeliosData/snapshots
host id       3d039d6c646abe5e
writable      yes
```

If `mode` says *standard*, the marker is not where Helios is looking. It must be
beside the executable — or, for the macOS app, beside `Helios.app` rather than
inside it, because writing into a bundle would break its code signature.

## How Helios decides where to write

In order:

1. **`HELIOS_DATA_DIR`**, if set — an explicit redirect that wins over
   everything else. Useful for one-offs and scripts:
   ```sh
   HELIOS_DATA_DIR=/Volumes/HELIOS/HeliosData helios scan / --cache
   ```
2. **Portable mode**, if a `helios-portable` (or `helios-portable.txt`) file, or
   an existing `HeliosData` folder, sits beside the app. Either trigger is
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
| Exported reports | Wherever you point the save panel — put them on the drive |
| Preferences | Helios has none |
| Login items, daemons, receipts | Helios installs none |

To verify rather than take this on faith, watch the host's application-support
directory across a scan:

```sh
ls -la ~/Library/Application\ Support/ | grep -i helios   # before
/Volumes/HELIOS/helios scan / --cache
ls -la ~/Library/Application\ Support/ | grep -i helios   # after — unchanged
```

macOS will still record that you *launched* an app — that is Gatekeeper and
`launchservices`, not Helios, and applies to anything you run from a drive.
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

**Gatekeeper.** An app copied onto a drive carries a quarantine flag, and an app
you built yourself is not signed by a Developer ID. The setup script clears the
flag; otherwise right-click `Helios.app` → **Open** → **Open**, or:

```sh
xattr -dr com.apple.quarantine /Volumes/HELIOS/Helios.app
```

**Full Disk Access is per-machine, and does not travel.** macOS grants it to an
app at a specific path on a specific Mac, so you have to grant it again on each
computer: System Settings → Privacy & Security → Full Disk Access → **+** →
select `Helios.app` on the drive. Without it, scans still work but skip
protected locations, and Helios tells you how much it could not see. That is the
honest trade of running portably — see [MACOS.md](MACOS.md) for the details.

**The drive shows up in its own sidebar**, because it is a mounted volume like
any other. Scanning it is harmless; it is just usually not what you came for.

**Ejecting.** Quit Helios before ejecting. The snapshot cache is written
atomically — temporary file, then rename — so pulling the drive mid-write costs
you the cache, never a corrupt one. Helios validates every snapshot on load and
discards anything it cannot read.

## Windows

The same marker and folder rules apply, and `platform/windows.rs` is written
against them. Until Phase 6 of the [roadmap](ROADMAP.md) lands — that backend has
never been compiled — treat Windows portable use as untested rather than
supported.

## Read-only or full drives

Helios degrades rather than failing:

```
warning: could not cache snapshot: Read-only file system (os error 30)

/etc
  4.7 MB in 1,209 files, 152 folders
```

The scan runs, every view works, and only the cache is lost — which costs you a
full rescan next time instead of an incremental one. `helios paths` reports
`writable: no` up front so you learn this before a long scan rather than after
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
