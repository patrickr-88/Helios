# Performance

## Targets and where things actually stand

| Target | Status |
|---|---|
| Scan a 1 TB drive efficiently | ~100k files/sec measured; a 1 TB SSD with ~2M files projects to ~30 s cold |
| Handle millions of files | 56 bytes/node ⇒ 10M files ≈ 700 MB; measured 17 MB for 173k |
| Multi-threaded scanning | ✅ N workers + a lock-free single-owner arena |
| Incremental updates | ✅ 1.16 s → **16 ms** on an unchanged `/usr` |
| Background processing | ✅ scans run off the UI thread; pause parks on a condvar |
| Minimal memory | ✅ see the arena design |
| Fast rendering with large datasets | ✅ canvas treemap, windowed tables, layout in Rust |

## Measured

4-core Linux VM, ext4, warm cache, release build:

```
$ helios scan /usr
  3.6 GB in 76,693 files, 7,843 folders
  scanned in 1.2 s · 9.4 MB of memory

$ helios scan /
  7.9 GB in 149,955 files, 22,903 folders
  scanned in 1.5 s · 17 MB of memory        ≈ 100,000 files/sec

$ helios scan /usr --cache      # second run, nothing changed
  scanned in 16 ms · 7,860 folders reused

$ du -sb /usr  →  3,569,752,578
$ helios       →  3,569,752,578            byte-identical
```

Snapshot cache: 6.0 MB for 84,674 nodes (~71 bytes/node on disk).
Frontend bundle: 186 KB, 60 KB gzipped.

Numbers on Apple silicon with APFS should be better for the walk (faster
storage, and `getattrlistbulk` is available as a future optimization) and worse
for the first scan of a cold volume.

## Where the time goes

A scan is essentially `read_dir` + one `stat` per entry. At ~150k entries in
1.5 s the engine is issuing on the order of 200k syscalls per second and
spending the great majority of wall time inside them — `sys` time is 3.5 s
across threads against 0.4 s of `user` time. That ratio is the design goal: the
scanner should be waiting on the kernel, not on itself.

Consequences:

- **More threads stop helping** past the point where the queue is saturated,
  which is why the default caps at 8. The remaining lever is fewer syscalls per
  entry, not more threads.
- **The collector is not the bottleneck** at under half a core.
- **The next real win is `getattrlistbulk`** on macOS — one syscall per batch of
  entries instead of one per entry. That is the single largest remaining
  improvement available, and it is contained entirely within
  `platform/macos.rs`.

## Optimizations that are in

**The arena.** 56-byte fixed records, names in one blob, children as an
intrusive linked list, `u32` ids. Roughly 4× smaller than the naive
`String`/`Vec<Node>` shape and far friendlier to cache.

**One reverse pass for rollup.** Children are always appended after their
parent, so summing subtree sizes is a single backwards loop over the arena — no
recursion, no stack, no risk on deep trees.

**No syscall wasted on path resolution.** The walker uses
`DirEntry::metadata()`, which stats against the already-open directory handle
(`fstatat`) instead of resolving a full path. On Windows the same call is free —
the data came back with `FindNextFileW`.

**Allocation-free classification.** Extension lookup lowercases into a 16-byte
stack buffer and binary-searches a sorted static table. No `String`, no
`HashMap` entry, per file.

**Top-N by heap, not by sort.** `largest()` keeps a bounded min-heap: O(n log k)
with k memory. Top 100 of 10M files is one linear pass and a 100-element heap
instead of a 10M-element sort. It also skips the filter predicate entirely for
nodes that cannot beat the current threshold.

**Filter predicates ordered by cost.** Integer comparisons, then category, then
name, and path reconstruction dead last — it is the only expensive test, and it
runs only for nodes that already passed everything cheaper.

**ASCII fast path for case-insensitive matching.** Substring search over
`eq_ignore_ascii_case` windows rather than allocating a lowercased copy of every
name in the tree.

**Treemap layout in Rust.** A 20,000-tile layout is tens of milliseconds of
arithmetic here versus hundreds of milliseconds of allocation churn in
JavaScript. The UI receives a flat array and paints it.

**Canvas, not DOM.** 20,000 rectangles as DOM nodes is 20,000 layout objects and
a style recalculation per hover. On canvas it is one paint and a hit-test.

**Windowed tables.** Fixed row height, only the visible slice rendered, spacer
rows for the rest. A 100,000-row result scrolls like a 20-row one, without a
virtualization dependency.

**Debounced work.** Treemap layout is debounced 90 ms against resize; search is
debounced 220 ms against typing. Dragging a window edge issues one layout, not
sixty.

**Bounded queues.** The results channel holds 512 batches, so a fast disk cannot
let workers run arbitrarily far ahead of the collector and balloon peak memory.

**Small worker stacks.** 256 KB instead of the default 2 MB — the workers hold
no recursion, and one core's worth of threads at 2 MB each is pure waste.

**Progress throttled to 10 Hz.** Smooth to a human, and the IPC bridge stays
idle between samples.

## Optimizations that are deliberately out

**`getattrlistbulk` / `NtQueryDirectoryFile`.** The biggest remaining win, and
also the biggest jump in platform-specific complexity. Deferred until the
straightforward version is measured on real hardware — it belongs in Phase 6,
not in an MVP.

**Reading the raw MFT on NTFS / the catalog on APFS.** WizTree's trick, and it
is genuinely an order of magnitude faster. It also requires administrator
privileges, filesystem-version-specific parsing, and a second scanner to
maintain. Wrong trade for a tool whose pitch is "safe and unprivileged".

**mmap-ing snapshots.** Would make loading nearly instant for huge trees. Worth
doing when snapshots exceed a few hundred megabytes; the format was designed
with fixed-width records so this stays possible later.

**A background rescan daemon.** Ruled out on principle: an always-on process
that keeps touching the disk is exactly what a lightweight tool should not be.

**SIMD anything.** There is no compute-bound loop here to vectorize. The
bottleneck is the kernel.

## Perceived performance

Raw speed is only half of feeling fast:

- **A snapshot loads at launch**, so the app opens showing data rather than an
  empty state and a "Scan" button.
- **"Rescan" is incremental**, so the common case is seconds.
- **The ETA stays quiet for 1.5 s** rather than showing a number that will
  swing.
- **Pause actually parks threads** — the machine comes back immediately.
- **Cancel returns a usable partial tree** instead of throwing away the work.
- **Every list is capped** server-side, so no interaction can hang the UI.

## Regression watch

The measurements above are reproducible with the CLI, which is why it exists:

```sh
cargo build --release -p helios-cli
time ./target/release/helios scan /usr --no-progress
time ./target/release/helios scan /usr --cache --no-progress   # incremental
du -sb /usr                                                     # cross-check
```

`Node` is pinned at 56 bytes by an assertion in `model.rs`; growing it is a
deliberate act, not an accident.
