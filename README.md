# Helios

A fast, read-only disk usage visualizer. macOS first, Windows next.

Helios answers one question well: **where did my storage go?** It scans your
volumes, shows you the answer as a treemap, a folder tree, a list of the largest
things you own and a breakdown by kind of file — and it never modifies,
deletes, moves or renames anything, and never touches the network.

```
┌──────────────┬────────────────────────────────────────────────────────┐
│ VOLUMES      │  Dashboard  Treemap  Folders  Largest  Categories       │
│ ▸ Macintosh  │ ┌──────────────┬───────────┬────────┐                   │
│   HD         │ │              │  Photos   │ Xcode  │  Treemap: every   │
│   826 GB free│ │   Movies     ├───────────┼────────┤  rectangle sized  │
│ ▸ Backup     │ │   412 GB     │  iCloud   │ Docker │  by what it uses  │
│   1.2 TB free│ └──────────────┴───────────┴────────┘                   │
└──────────────┴────────────────────────────────────────────────────────┘
```

## What it does

- **Every volume** — internal, external, network — with capacity, used and free.
- **Treemap** sized by consumption, with drill-down into any folder.
- **Folder tree** with a size and a share-of-parent bar at every level.
- **Largest files and folders**, top 100 or top 1000, sortable and filterable.
- **Categories**: documents, images, videos, audio, archives, applications,
  developer files, system files, other.
- **Search and filter** by name, path, extension, size, date, hidden, system.
- **Reports** exported as CSV, JSON or PDF.
- **Live progress** with a real estimate, and pause / resume / stop.
- **Incremental rescans** that reuse unchanged folders — seconds, not minutes.

## What it will never do

No writes to the volumes it scans. No network access of any kind. No telemetry,
no analytics, no crash reporting, no update pings. Everything stays on your
machine, and the app works with the Wi-Fi off.

Those are architectural properties, not promises in a README:
`crates/helios-core/tests/read_only.rs` fails the build if a mutating
filesystem call or a networking type appears anywhere in the engine, and the
app's Tauri capability file grants no filesystem, shell, or HTTP plugin at all.

## Try it

The engine and CLI build anywhere Rust does:

```sh
cargo test                              # 66 tests, no platform assumptions
cargo build --release -p helios-cli

./target/release/helios volumes         # every mounted volume
./target/release/helios scan ~/Movies   # scan and summarize
./target/release/helios scan / --cache  # cache the result for fast rescans
./target/release/helios report ~/Movies --format pdf --out report.pdf
```

The interface runs in a browser against a synthetic volume, with no Rust build
and no app bundle:

```sh
npm install
npm run dev            # http://localhost:5173
```

The real app (macOS):

```sh
npm run app            # development, with devtools
npm run app:build      # signed .app and .dmg in src-tauri/target/release/bundle
```

## Measured

On the Linux CI box this was developed on (4-core VM, ext4, warm page cache):

| Workload | Result |
|---|---|
| Scan `/usr` — 76,693 files, 7,843 folders | **1.2 s** |
| Scan `/` — 149,955 files, 22,903 folders | **1.5 s** (~100k files/sec) |
| Memory, 173k-node tree | **17 MB** |
| Incremental rescan of `/usr`, nothing changed | **16 ms** (7,860 folders reused) |
| Snapshot cache, 84,674 nodes | **6.0 MB** on disk |
| Total bytes vs. `du -sb /usr` | **byte-identical** |

Helios's numbers agreeing exactly with `du` matters more than the speed: a fast
disk tool that quietly miscounts hardlinks, sparse files or symlinks is worse
than a slow correct one.

## How it fits together

```
crates/helios-core     the engine — scanning, queries, treemap, reports
    └── platform/      the only OS-specific code: macos.rs, windows.rs, linux.rs
crates/helios-cli      headless front end (also how the engine is benchmarked)
src-tauri              desktop shell: window, IPC commands, ~450 lines
src                    React interface
docs                   architecture, engine design, API, roadmap
```

The engine knows nothing about Tauri, and the UI knows nothing about
filesystems. That seam is what makes the Windows port a matter of finishing one
module rather than rewriting an app — see [docs/PLATFORM.md](docs/PLATFORM.md).

## Documentation

| Document | What's in it |
|---|---|
| [ARCHITECTURE.md](docs/ARCHITECTURE.md) | System design, data flow, storage design, folder layout |
| [TECHNOLOGY-CHOICE.md](docs/TECHNOLOGY-CHOICE.md) | Electron vs. Tauri vs. SwiftUI, with the reasoning |
| [SCAN-ENGINE.md](docs/SCAN-ENGINE.md) | The walker, threading model, correctness decisions |
| [API.md](docs/API.md) | Every IPC command and the Rust library API |
| [PLATFORM.md](docs/PLATFORM.md) | Platform-specific components and the Windows plan |
| [PERFORMANCE.md](docs/PERFORMANCE.md) | Where the time and memory go, and what was done about it |
| [SECURITY.md](docs/SECURITY.md) | Privacy, permissions, threat model |
| [WIREFRAMES.md](docs/WIREFRAMES.md) | Every screen, annotated |
| [ROADMAP.md](docs/ROADMAP.md) | MVP scope, phases, post-MVP |

## License

MIT.
