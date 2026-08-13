# Helios

A fast, read-only disk usage analyzer. One 756 KB binary, no runtime, no
installer, no configuration.

Helios answers one question well: **where did my storage go?** Point it at a
disk and it prints the biggest folders, the biggest files, a breakdown by kind
of file, and a folder tree with sizes at every level — and it never modifies,
deletes, moves or renames anything, and never touches the network.

```console
$ helios /opt

/opt
  2.2 GB in 37,497 files, 8,077 folders · 83 ms
  on Root: 240 GB of 271 GB used (89%), 31 GB free

Largest folders
   1  pw-browsers       968 MB  ████▍····· 43.2%  /opt/pw-browsers
   2  chrome-linux      625 MB  ██▊······· 27.9%  /opt/pw-browsers/chromium-1194/chrome-linux
   3  chromium-1194     625 MB  ██▊······· 27.9%  /opt/pw-browsers/chromium-1194

Largest files
   1  chrome             463 MB  ██▏······· 20.7%  …/chrome-linux/chrome
   2  headless_shell     306 MB  █▍········ 13.7%  …/chrome-linux/headless_shell
   3  node               125 MB  ▌·········  5.6%  /opt/node22/bin/node

By category
  other             1.7 GB  ███████████████····· 74.8%  8,968 files
  developer         374 MB  ███▍················ 16.7%  26,447 files
  applications      158 MB  █▍··················  7.0%  620 files
  system             21 MB  ▏···················  0.9%  13 files
  documents          11 MB  ▏···················  0.5%  1,278 files
  images            495 KB  ····················  0.0%  167 files
  archives       842 bytes  ····················  0.0%  4 files
```

## Install

```sh
git clone https://github.com/patrickr-88/helios.git
cd helios
./install.sh
```

That builds one binary and puts it in `~/.local/bin`. Pass a different
directory if you like (`./install.sh /usr/local/bin`), or skip the script
entirely — `cargo install --path crates/helios-cli` does the same thing, and
`cargo build --release` leaves the binary at `target/release/helios` to copy
wherever you want.

The only prerequisite is Rust. Uninstalling is deleting the binary.

## Use

```sh
helios                        # every volume, with capacity and free space
helios ~/Downloads            # scan a folder and summarize it
helios /                      # scan the whole disk
helios / --cache              # cache it — the next run takes milliseconds
helios ~/Movies --tree -d 3   # folder tree, three levels deep
helios / --find node_modules  # everything matching, biggest first
helios / --files --min-size 1GB
helios / -o storage-report.pdf   # .csv and .json work too
```

`helios --help` is the full list; there are fourteen flags and no subcommands.

Ctrl-C stops a scan and still prints what it found, which is what you want three
minutes into a full disk.

## What it does

- **Every volume** — internal, external, network — with capacity, used and free.
- **Largest files and folders**, with proportional bars.
- **Folder tree** with a size and a share-of-parent at every level.
- **Categories**: documents, images, videos, audio, archives, applications,
  developer files, system files, other.
- **Search and filter** by name, path, extension and size.
- **Reports** as CSV, JSON or PDF.
- **Live progress** with an honest estimate, and Ctrl-C to stop.
- **Incremental rescans** that reuse unchanged folders — milliseconds, not
  minutes.
- **Portable mode**: runs from a flash drive, writes nothing to the host.

## What it will never do

No writes to the volumes it scans. No network access of any kind. No telemetry,
no analytics, no crash reporting, no update pings. Everything stays on your
machine, and it works with the Wi-Fi off.

Those are architectural properties, not promises in a README:
`crates/helios-core/tests/read_only.rs` fails the build if a mutating filesystem
call or a networking type appears anywhere in the engine.

## Measured

On the Linux CI box this was developed on (4-core VM, ext4, warm page cache):

| | |
|---|---|
| Binary | **756 KB**, no runtime, no shared libraries beyond libc |
| Dependencies | **4 direct** (`serde`, `serde_json`, `crossbeam-channel`, `libc`) |
| Startup | **2 ms** |
| Scan `/usr` — 76,693 files, 7,843 folders | **1.2 s** |
| Scan `/` — 149,955 files, 22,903 folders | **1.5 s** (~100k files/sec) |
| Memory, 173k-node tree | **17 MB** |
| Incremental rescan, nothing changed | **16 ms** (7,860 folders reused) |
| Total bytes vs. `du -sb /usr` | **byte-identical** |

Agreeing exactly with `du` matters more than the speed: a fast disk tool that
quietly miscounts hardlinks, sparse files or symlinks is worse than a slow
correct one.

## How it fits together

```
crates/helios-core     the engine — scanning, queries, reports, snapshot cache
    └── platform/      the only OS-specific code: macos.rs, windows.rs, linux.rs
crates/helios-cli      the program: argument parsing and text output
docs                   architecture, engine design, API, roadmap
```

Two crates, ~3,500 lines of Rust. The engine is a library with no knowledge of
terminals, so the Windows port is one module and a future GUI — if anyone wants
one — would be a second front end rather than a rewrite. See
[docs/PLATFORM.md](docs/PLATFORM.md).

## Documentation

| Document | What's in it |
|---|---|
| [MACOS.md](docs/MACOS.md) | Running, building and troubleshooting on a Mac |
| [PORTABLE.md](docs/PORTABLE.md) | Running from a flash drive without touching the host |
| [ARCHITECTURE.md](docs/ARCHITECTURE.md) | System design, data flow, storage design, folder layout |
| [TECHNOLOGY-CHOICE.md](docs/TECHNOLOGY-CHOICE.md) | Why Rust, why a CLI, and the GUI that was removed |
| [SCAN-ENGINE.md](docs/SCAN-ENGINE.md) | The walker, threading model, correctness decisions |
| [API.md](docs/API.md) | The command line, and the Rust library behind it |
| [PLATFORM.md](docs/PLATFORM.md) | Platform-specific components and the Windows plan |
| [PERFORMANCE.md](docs/PERFORMANCE.md) | Where the time and memory go, and what was done about it |
| [SECURITY.md](docs/SECURITY.md) | Privacy, permissions, threat model |
| [ROADMAP.md](docs/ROADMAP.md) | MVP scope, phases, post-MVP |

## License

MIT.
