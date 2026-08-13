# Architecture

## The shape of the problem

A disk visualizer is not a UI problem with some file I/O attached. It is a data
problem — tens of millions of entries, gathered under syscall pressure, queried
interactively — with a UI attached. Every significant decision here follows from
that:

1. **The tree is the product.** Building it fast and holding it small is the
   whole engineering task. Everything else is a view over it.
2. **The tree never leaves Rust.** The webview receives the few hundred rows it
   is about to paint. This is what keeps a 10-million-file scan feeling the same
   as a 10-thousand-file one.
3. **The OS is behind one door.** All platform-specific code lives in
   `platform/`, so a second OS is a module, not a fork.
4. **Read-only is structural.** Not a policy the code follows — a property the
   test suite enforces.

## Layers

```
┌──────────────────────────────────────────────────────────────────────┐
│  React + TypeScript  (src/)                                          │
│  Dashboard · Treemap (canvas) · Folder tree · Largest · Categories    │
│  Holds: the current page of rows. Never the tree.                    │
└───────────────────────────────┬──────────────────────────────────────┘
                                │  Tauri IPC — JSON, ~20 commands,
                                │  3 events (progress / finished / failed)
┌───────────────────────────────┴──────────────────────────────────────┐
│  Desktop shell  (src-tauri/)                                         │
│  Command handlers · scan lifecycle · app state · export to disk      │
│  ~450 lines. Contains no analysis logic.                             │
└───────────────────────────────┬──────────────────────────────────────┘
                                │  plain Rust calls
┌───────────────────────────────┴──────────────────────────────────────┐
│  helios-core                                                         │
│                                                                       │
│   scan/      parallel walker, progress + ETA, pause/cancel,          │
│              incremental rescan                                       │
│   model/     the arena tree (56 bytes per node)                       │
│   query/     filters, sorting, top-N, category aggregation            │
│   treemap/   squarified layout                                        │
│   snapshot/  binary cache, atomic writes                              │
│   report/    CSV · JSON · PDF                                         │
│   platform/  ← the only OS-specific code in the entire codebase       │
└───────────────────────────────┬──────────────────────────────────────┘
                                │  read_dir · lstat · statfs
                          ┌─────┴─────┐
                     macOS / Windows / Unix
```

`crates/helios-cli` sits directly on `helios-core`, bypassing the shell
entirely. It is not a toy: it is how the engine gets profiled, how CI exercises
scanning on real trees, and how the `du` cross-check in the README is run.

## Data flow of one scan

```
  UI: startScan({ path, incremental })
        │
        ▼
  shell: spawn driver thread ──────────────────────────────┐
        │                                                   │
        │  scan::scan(options, control, on_progress)        │
        │        │                                          │
        │        ├── N workers: read_dir + stat ─┐          │
        │        │                                ▼         │
        │        └── 1 collector: owns the arena, dedups    │
        │            hardlinks, grafts cached subtrees      │
        │                     │                             │
        │        every 100 ms │ ScanProgress ──── emit ─────┼──▶ UI progress bar
        │                     ▼                             │
        │            rollup() → Tree                        │
        ▼                                                   │
  shell: store Arc<Tree> in AppState, write snapshot ───────┘
        │
        └── emit scan://finished { totals, counts, timings } ──▶ UI

  then, per view:
  UI: treemapLayout(scanId, nodeId, w, h) ──▶ Vec<Tile>   (a few thousand rects)
      listChildren(scanId, nodeId, filter) ──▶ Vec<Entry> (one folder)
      largestEntries(scanId, dirs, 100)    ──▶ Vec<Entry> (top-N via a heap)
```

The UI holds a `scanId` and a `nodeId`, nothing more. Drilling into a folder is
a new query with a different `nodeId` — there is no client-side tree to keep in
sync, and no state that can drift from what the engine actually saw.

## The tree

The single most consequential decision in the codebase. A naive
`struct Node { name: String, children: Vec<Node>, .. }` costs, per entry, a
24-byte `String` header plus an allocation, a 24-byte `Vec` header plus an
allocation, and pointer-chasing on every traversal. At 10 million files that is
well over a gigabyte and a rollup pass that thrashes cache.

Helios uses an arena instead:

```rust
pub struct Tree {
    nodes: Vec<Node>,   // 56 bytes each, dense, cache-friendly
    names: Vec<u8>,     // one contiguous blob
    root_path: PathBuf,
    errors: Vec<ScanError>,
}

pub struct Node {
    name_off: u32, name_len: u16,   // slice into `names`
    flags: NodeFlags, depth: u16, category: Category,
    parent: NodeId, first_child: NodeId, next_sibling: NodeId,  // u32 each
    logical_size: u64, physical_size: u64, mtime: i64,
    file_count: u32, dir_count: u32,
}
```

What this buys:

| Property | Consequence |
|---|---|
| Fixed 56-byte records | 10M files ≈ 560 MB of nodes; measured 17 MB for 173k |
| Children as an intrusive linked list | No `Vec` per directory; leaf dirs cost nothing extra |
| `u32` node ids | Half the pointer traffic, and ids are stable across IPC |
| Children always appended after parents | Rollup is one reverse linear pass, no recursion, no stack overflow on deep trees |
| Names in one blob | One allocation for a million names instead of a million |

The cost is that nodes cannot be removed individually and the tree is
append-only during a scan. Neither matters: a scan builds one tree and then only
reads it.

## Storage design

Helios has no database. It has one file per volume.

**Why not SQLite.** The obvious design — a `files` table with a parent id and
an index on size — buys nothing here. Every query Helios asks is either a
single-folder lookup (already O(children) on the arena), a top-N by size (one
linear pass and a bounded heap, ~40 ms on 10M rows), or a full-tree aggregation
that has to touch every row anyway. SQLite would add an import step costing more
than the scan itself, a schema to migrate, and a file several times larger than
the arena — to answer the same questions more slowly. The tree is already an
index.

**The snapshot format.** A hand-rolled little-endian binary layout:

```
"HELIOSNP" | version:u32 | volume_id | root_path | scanned_at:i64 | stats[8×u64]
node_count:u64 | node_count × 56-byte fixed records
names_len:u64 | names blob
error_count:u32 | errors...
```

Measured: **6.0 MB for 84,674 nodes** (~71 bytes/node including names) versus
roughly 280 MB for the same tree as JSON. Loading is a bulk read plus a tight
decode loop, with no parser and no per-node allocation.

Snapshots are written to a temporary path and renamed into place, so an
interrupted write cannot leave a corrupt cache. They are treated strictly as a
cache: an unreadable or version-mismatched snapshot is discarded and the volume
is rescanned. Nothing a user cares about is ever *only* in a snapshot.

Location: `~/Library/Application Support/Helios/snapshots/<volume-id>.helios`
on macOS, `%LOCALAPPDATA%\Helios\snapshots\` on Windows. Volume ids are
sanitized before they become filenames — a test asserts that a hostile id
cannot escape the cache directory.

## Concurrency model

```
   ┌──────────────┐   Job(dir)    ┌───────────────┐   Batch(entries)
   │  collector   │──────────────▶│  worker × N   │───────────────────┐
   │  owns arena  │◀──────────────│  read_dir     │                   │
   │  (1 thread)  │   new dirs    │  + stat       │◀──────────────────┘
   └──────────────┘               └───────────────┘
```

Workers do the syscall-bound work and hand back owned batches; one collector
thread owns the arena outright. The arena therefore needs no lock and no
atomics, node ids stay dense and stable, and the two things that need a global
view — hardlink de-duplication and incremental subtree reuse — are plain
`HashSet`/`HashMap` lookups on one thread instead of contended shared state.

Directory scanning is I/O-bound, not build-bound, so the single collector is not
the bottleneck: the engine sustains ~100k files/sec with the collector well
under half a core.

Termination is by pending-job count rather than channel closure, and the
collector polls with a 50 ms timeout so a cancellation lands even when every
worker is idle. (An early version blocked on `recv` and deadlocked on cancel for
exactly that reason — idle workers never send, and the collector still held the
job sender.)

## Folder structure

```
Helios/
├── Cargo.toml                    workspace: core + cli (src-tauri excluded on purpose)
├── package.json                  React/Vite/Tauri front end
├── crates/
│   ├── helios-core/
│   │   ├── src/
│   │   │   ├── lib.rs            crate docs, the bitflags_lite! macro, re-exports
│   │   │   ├── model.rs          arena tree, rollup, path reconstruction
│   │   │   ├── category.rs       extension → category, allocation-free
│   │   │   ├── query.rs          filters, sorting, top-N, aggregation
│   │   │   ├── treemap.rs        squarified layout + hit testing
│   │   │   ├── snapshot.rs       binary cache, atomic write
│   │   │   ├── fmt.rs            byte/date formatting
│   │   │   ├── scan/
│   │   │   │   ├── mod.rs        options, stats, entry point
│   │   │   │   ├── walker.rs     the parallel walker
│   │   │   │   ├── control.rs    pause / resume / cancel
│   │   │   │   ├── progress.rs   counters and ETA estimation
│   │   │   │   └── incremental.rs  dir index + subtree grafting
│   │   │   ├── report/
│   │   │   │   ├── mod.rs        report assembly, CSV, JSON
│   │   │   │   └── pdf.rs        a minimal PDF writer (no dependency)
│   │   │   └── platform/
│   │   │       ├── mod.rs        the seam: types + free functions
│   │   │       ├── macos.rs      getfsstat, bundles, firmlinks, UF_HIDDEN
│   │   │       ├── windows.rs    GetLogicalDrives, NTFS/ReFS, reparse points
│   │   │       ├── linux.rs      /proc/mounts, statvfs (dev + CI)
│   │   │       └── unix_shared.rs  metadata conversion shared by macOS/Linux
│   │   └── tests/read_only.rs    the guarantee, enforced
│   └── helios-cli/src/main.rs    headless front end
├── src-tauri/
│   ├── src/{main,commands,state}.rs
│   ├── capabilities/default.json  no fs/shell/http plugins — see SECURITY.md
│   └── tauri.conf.json
├── src/
│   ├── App.tsx                   shell: views, scan lifecycle, filters
│   ├── components/               Sidebar, Treemap, FolderTree, EntryTable, …
│   ├── lib/{api,types,format,mock}.ts
│   └── styles/app.css            one stylesheet, light + dark tokens
└── docs/
```

## Dependencies

The engine has three: `serde` (IPC boundary), `serde_json`, and
`crossbeam-channel` (the walker's queues), plus `libc` or `windows-sys` per
platform. Flag sets, the snapshot codec, the PDF writer, date formatting and
argument parsing are all in-tree — each is under 250 lines, and each avoided
crate is one less thing to audit in an app whose entire pitch is that it is
small, offline and inspectable.

The front end has React and Vite. No component library, no state library, no
charting library, no virtualization library: the interface is one stylesheet and
a handful of components, and the production bundle is 186 KB (60 KB gzipped).

## Testing

75 tests, all runnable on any platform:

- **Correctness under nasty inputs** — symlink loops, hardlinks, permission
  denials, deep nesting, non-ASCII names, zero-byte and truncated snapshots.
- **The read-only guarantee** — a real scan leaves a fingerprinted tree
  byte-identical, and a source audit fails the build on any mutating `fs::` call
  outside the snapshot module or any networking type anywhere.
- **Geometry** — treemap tiles tile their viewport, never overlap, nest inside
  their parents, and keep aspect ratios readable.
- **Round-trips** — snapshots, JSON reports, and a structurally valid PDF.

The Linux backend exists mostly so this suite runs in CI on every push. Keeping
a second platform honest is what keeps the seam real.
