# Architecture

## The shape of the problem

A disk analyzer is not a UI problem with some file I/O attached. It is a data
problem — tens of millions of entries, gathered under syscall pressure — with a
thin presentation layer on top. Every significant decision here follows from
that:

1. **The tree is the product.** Building it fast and holding it small is the
   whole engineering task. Everything else is a view over it.
2. **The engine is a library.** It knows nothing about terminals, argument
   parsing or output formats. The program is one file that calls it.
3. **The OS is behind one door.** All platform-specific code lives in
   `platform/`, so a second OS is a module, not a fork.
4. **Read-only is structural.** Not a policy the code follows — a property the
   test suite enforces.

## Layers

```
┌──────────────────────────────────────────────────────────────────────┐
│  helios-cli — the program                                            │
│  argument parsing · text tables, bars and the folder tree · Ctrl-C   │
│  One file. Contains no analysis logic.                               │
└───────────────────────────────┬──────────────────────────────────────┘
                                │  plain Rust calls
┌───────────────────────────────┴──────────────────────────────────────┐
│  helios-core — the engine                                            │
│                                                                       │
│   scan/      parallel walker, progress + ETA, pause/cancel,          │
│              incremental rescan                                       │
│   model/     the arena tree (56 bytes per node)                       │
│   query/     filters, sorting, top-N, category aggregation            │
│   snapshot/  binary cache, atomic writes                              │
│   report/    CSV · JSON · PDF                                         │
│   platform/  ← the only OS-specific code in the entire codebase       │
└───────────────────────────────┬──────────────────────────────────────┘
                                │  read_dir · lstat · statfs
                          ┌─────┴─────┐
                     macOS / Windows / Unix
```

Two layers, and the boundary is a plain library API — no IPC, no serialization
between them, no process boundary. A front end is whatever calls `scan()` and
then queries the tree; today that is a terminal program, and
[TECHNOLOGY-CHOICE.md](TECHNOLOGY-CHOICE.md) records the graphical one that was
built and then removed.

## Data flow of one run

```
  helios /Volumes/Backup --cache
        │
        ├─ platform::volumes()          which volume is this path on?
        ├─ snapshot::load_for_volume()  a previous scan to reuse, if any
        │
        ▼
    scan::scan(options, control, on_progress)
        │
        ├── N workers: read_dir + stat ─┐
        │                                ▼
        └── 1 collector: owns the arena, dedups hardlinks,
            grafts unchanged subtrees from the previous scan
                     │
     every 100 ms    │ ScanProgress ──▶ the progress line on stderr
                     ▼
             rollup() → Tree
                     │
        ┌────────────┼────────────────────────────┐
        ▼            ▼                            ▼
  query::largest  query::category_breakdown  snapshot::save
  query::children query::search              report::to_{csv,json,pdf}
        │
        ▼
   text tables, bars and the folder tree on stdout
```

Queries run against the arena in memory and return only the rows about to be
printed. That mattered when a webview was on the other end of an IPC bridge; it
still matters now, because "top 100 of 10 million files" is a bounded heap over
a linear pass rather than a sort of everything.

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
├── Cargo.toml                    workspace: two crates
├── install.sh                    build and put the binary on your PATH
├── crates/
│   ├── helios-core/              the engine
│   │   ├── src/
│   │   │   ├── lib.rs            crate docs, the bitflags_lite! macro, re-exports
│   │   │   ├── model.rs          arena tree, rollup, path reconstruction
│   │   │   ├── category.rs       extension → category, allocation-free
│   │   │   ├── query.rs          filters, sorting, top-N, aggregation
│   │   │   ├── snapshot.rs       binary cache, atomic write, host identity
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
│   │   │       ├── mod.rs        the seam: types, free functions, portable mode
│   │   │       ├── macos.rs      getfsstat, bundles, firmlinks, UF_HIDDEN
│   │   │       ├── windows.rs    GetLogicalDrives, NTFS/ReFS, reparse points
│   │   │       ├── linux.rs      /proc/mounts, statvfs (dev + CI)
│   │   │       └── unix_shared.rs  metadata conversion shared by macOS/Linux
│   │   └── tests/read_only.rs    the guarantee, enforced
│   └── helios-cli/src/main.rs    the program: arguments, tables, bars, tree
├── scripts/make-portable-drive.sh
└── docs/
```

## Dependencies

Four, and each earns its place:

| Crate | Why it is here |
|---|---|
| `crossbeam-channel` | The walker's job and result queues |
| `serde` + `serde_json` | The JSON report, and only that |
| `libc` / `windows-sys` | The platform layer's syscalls |

Flag sets, the snapshot codec, the PDF writer, date formatting, byte formatting
and argument parsing are all in-tree — each is under 250 lines, and each avoided
crate is one less thing to audit in a program whose pitch is that it is small,
offline and inspectable.

`serde` is the one that could plausibly go: it and its derive macro pull in
`syn`, `quote` and `proc-macro2`, which dominate both the dependency graph and
the build time. Hand-rolling JSON output would remove them. It stays because
correct JSON string escaping is easy to get subtly wrong, `serde` is among the
most-audited crates in the ecosystem, and the derive keeps the report's fields
and its serialized shape from drifting apart. Compiled size is not the argument
either way — the binary is 756 KB.

## Testing

78 tests, all runnable on any platform:

- **Correctness under nasty inputs** — symlink loops, hardlinks, permission
  denials, deep nesting, non-ASCII names, zero-byte and truncated snapshots.
- **The read-only guarantee** — a real scan leaves a fingerprinted tree
  byte-identical, and a source audit fails the build on any mutating `fs::` call
  outside the snapshot module or any networking type anywhere.
- **Round-trips** — snapshots, JSON reports, and a structurally valid PDF.
- **The program itself** — argument parsing, size suffixes, report format
  inference, and the folder tree's indentation (whose box-drawing characters are
  multi-byte, and which an earlier version sliced on byte offsets).

The Linux backend exists mostly so this suite runs in CI on every push. Keeping
a second platform honest is what keeps the seam real.
