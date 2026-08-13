# Scan engine

The engine has one job: turn a mount point into a correct, compact tree as fast
as the disk allows, without ever writing to it, while staying interruptible.

## Pipeline

```
ScanOptions ──▶ root lstat ──▶ [ DirIndex from previous snapshot ]
                                          │
       ┌──────────────────────────────────┴─────────────────────────┐
       │                                                            │
   job queue (unbounded)                              results queue (bounded, 512)
       │                                                            │
   ┌───▼────────────────┐                              ┌────────────▼────────────┐
   │ worker × N         │                              │ collector (1 thread)    │
   │  read_dir(dir)     │  ──── Batch(entries) ───────▶│  append nodes           │
   │  metadata(entry)   │                              │  dedup hardlinks        │
   │  (no symlink       │  ◀─── Job(subdir) ───────────│  graft cached subtrees  │
   │   following)       │                              │  emit progress @ 10 Hz  │
   └────────────────────┘                              └────────────┬────────────┘
                                                                    │
                                                            rollup() one reverse pass
                                                                    │
                                                            Tree + ScanStats
```

## Why one collector thread

The arena is owned outright by a single thread, which means:

- **No lock on the hot path.** Appending a node is a `Vec::push`. A shared arena
  behind a mutex would serialize the same work with contention on top; a
  lock-free arena would need atomics on every link field and would give up dense
  sequential ids.
- **Global invariants are cheap.** Hardlink de-duplication needs a set of
  `(dev, ino)` seen anywhere in the scan; incremental reuse needs a map of
  directory hashes. On one thread these are plain `HashSet`/`HashMap` lookups.
  Sharded across workers they would be contended structures with an ordering
  problem attached.
- **Node ids stay meaningful.** Children always land after their parent, which
  is what makes rollup a single reverse pass with no recursion — and makes deep
  trees (Node.js dependency chains, Time Machine hierarchies) safe from stack
  overflow.

The collector is not the bottleneck because directory scanning is syscall-bound:
at the measured ~100k files/sec it consumes well under half a core while the
workers block on I/O.

## Thread count

Defaults to core count, capped at 8. Beyond roughly that, extra threads stop
buying throughput — the work is `read_dir`/`lstat` latency, not computation —
and start costing responsiveness on the user's machine. A disk tool that makes
the fan spin has failed at "lightweight" even if it finishes a second sooner.

## Correctness decisions

These are the details that separate a plausible scanner from a correct one. Each
has a test.

**Symlinks are never followed.** Every metadata read is no-follow
(`fstatat(AT_SYMLINK_NOFOLLOW)` on Unix, non-reparse-following on Windows). A
link is recorded at its own small size and never descended. This is not a
preference: `~/Library` contains links into ancestors, and following them makes
a scan either double-count or never terminate. `a_symlink_loop_terminates`
builds exactly that trap and asserts the scan ends with the right total.

**Hardlinks are counted once.** A file with `nlink > 1` is charged to the first
path that reaches it; later paths are listed but weightless and flagged. This is
what `du` does, and it is why Helios's `/usr` total matches `du -sb` byte for
byte. It is defensible either way, so it is an option
(`deduplicate_hardlinks`) — but the default matches the tool users will
cross-check against.

**Logical and physical size are both carried.** `st_blocks × 512` is the truth
about how much disk a file occupies; `st_size` is the truth about how much data
it contains. They diverge for sparse files, APFS clones and compressed files,
sometimes by orders of magnitude. Both are stored, the UI shows both, and the
inspector calls out the difference when it exceeds rounding.

**Other filesystems are not crossed by default.** Scanning `/` should not
silently pull in a 4 TB Time Machine drive mounted under `/Volumes`. Mount
points are detected by device id, marked, and left unexpanded unless
`cross_filesystem` is set.

**macOS firmlinks are excluded.** `/System/Volumes/Data` and friends shadow
storage already counted through `/`. Walking them double-counts essentially the
entire volume. The exclusion list lives in `platform/macos.rs` alongside the
other synthetic mounts.

**Unreadable locations degrade gracefully.** A failed `read_dir` marks the node
`INACCESSIBLE`, records the reason, and the scan continues. A failed `stat` on
one entry skips that entry, not the directory. The counts surface in the UI and
in every report, because a total that is silently 40 GB short is worse than one
labelled incomplete — on macOS, an app without Full Disk Access will hit this
constantly.

**Depth is a listing limit, not a hiding limit.** With `max_depth: 1`, the
volume's top-level folders are expanded and sized, and *their* subfolders are
still listed — just not walked. The user sees that more exists.

## Progress and ETA

Counters are updated by the collector (single-threaded, so no atomics) and
emitted at most every 100 ms — 10 Hz is smooth to a human and keeps the IPC
bridge idle.

The ETA is deliberately conservative:

- It is anchored on the **volume's used bytes from the OS**, not on file counts.
  Bytes are what the user is watching, and directory counts are wildly
  non-uniform across a tree.
- The rate is **exponentially smoothed** (α = 0.25), because instantaneous rates
  swing by an order of magnitude between a folder of photos and a folder of
  source files.
- Nothing is shown for the **first 1.5 seconds**. An estimate that reads "4
  hours" and then "20 seconds" is worse than no estimate, and it is the single
  fastest way to make a tool feel untrustworthy.
- The UI rounds hard — "about 2 minutes left", never "1 m 47 s".

## Pause, resume, cancel

`ScanControl` is a cloneable handle over an atomic state plus a condvar. Paused
workers park on the condvar rather than spinning, so a paused scan costs no CPU
— which is the point, since users pause to get their machine back.

Cancellation is terminal and lands within ~50 ms. The tree returned by a
cancelled scan is a valid partial result, not garbage: the UI can display it,
and it is simply labelled as incomplete. It is deliberately *not* written to the
snapshot cache, so the next incremental rescan does not inherit the gap.

## Incremental rescan

A directory's mtime changes whenever an entry is added, removed or renamed
inside it. So on a rescan, if a directory's mtime matches the previous snapshot,
its entire cached subtree is grafted into the new tree without a single
`read_dir`.

Measured on `/usr`: **1.16 s → 16 ms**, with 7,860 folders reused.

Directories are keyed by a hash chained down from the root
(`hash(parent_hash, name)`, FNV-1a) rather than by reconstructed path strings.
That keeps index construction O(nodes) instead of O(nodes × depth) and allocates
nothing per entry.

**The trade-off, stated plainly:** mtime does not change when a file *inside*
the directory merely grows. An incremental rescan is exact about structure and
can lag on the size of files rewritten in place. So it is not the default —
"Rescan" is incremental, "Full scan" is not, both are one click, and the number
of reused folders is reported in the scan summary so the user knows what they
got.

When grafting, directory sizes are reset to zero on the way in and recomputed by
the rollup pass. Carrying the old rolled-up totals across would double-count the
entire subtree — a bug the `grafting_reproduces_sizes_without_double_counting`
test exists to catch.

## Memory

| Measurement | Value |
|---|---|
| Per node | 56 bytes + ~15 bytes of name |
| Measured, 173k nodes (`/` scan) | 17 MB |
| Measured, 84k nodes (`/usr` scan) | 9.4 MB |
| Projected, 10M files | ~700 MB worst case |

The bounded results queue (512 batches) is what stops a fast SSD from letting
workers run arbitrarily far ahead of the collector and ballooning peak memory.

## Extending it

Adding a field to `Node`: update the struct, the `size_of` assertion, the
snapshot read/write pair, and bump `FORMAT_VERSION` — old snapshots are then
discarded and rescanned, which is safe because snapshots are only ever a cache.

Adding a platform: implement `volumes`, `entry_meta`, `meta_from_dir_entry`,
`is_package`, `app_data_dir`, plus the `SYSTEM_PREFIXES` and
`DEFAULT_EXCLUSIONS` tables. Nothing else in the engine changes. See
[PLATFORM.md](PLATFORM.md).
