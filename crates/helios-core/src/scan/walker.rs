//! The parallel directory walker.
//!
//! # Concurrency model
//!
//! ```text
//!   ┌──────────┐  Job(dir)   ┌───────────┐  Batch(entries)  ┌───────────┐
//!   │ collector│────────────▶│ worker ×N │─────────────────▶│ collector │
//!   │ (builds  │◀────────────│ read_dir  │                  │  arena    │
//!   │  arena)  │  new dirs   │  + stat   │                  └───────────┘
//!   └──────────┘             └───────────┘
//! ```
//!
//! Workers do only the syscall-bound work (`read_dir` plus one no-follow stat
//! per entry) and hand back owned batches. A single collector thread owns the
//! arena outright, which is the crux of the design:
//!
//! * The arena needs no lock and no atomics — appends are plain `Vec` pushes,
//!   and node ids stay stable and densely packed.
//! * Hardlink de-duplication and incremental reuse need a global view; on one
//!   thread they are a plain `HashSet` lookup instead of a contended map.
//! * Directory scanning is I/O-bound, not build-bound, so the collector is
//!   never the bottleneck — on an APFS SSD it idles well under 20% while eight
//!   workers saturate the queue.
//!
//! Termination is by pending-job count, not by channel closure: the collector
//! knows exactly how many directories it has dispatched and how many batches
//! have come back, and drops the job sender when that reaches zero, which ends
//! the workers cleanly even mid-cancel.

use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;
use std::time::Instant;

use crossbeam_channel::{bounded, unbounded, Receiver, RecvTimeoutError, Sender};

use crate::category::classify;
use crate::model::{NodeFlags, NodeId, ScanError, Tree};
use crate::platform::{self, EntryMeta};

use super::control::{ScanControl, ScanState};
use super::incremental::{self, DirIndex};
use super::progress::{EtaEstimator, ScanProgress};
use super::{ScanOptions, ScanOutcome, ScanStats};

struct Job {
    path: PathBuf,
    node: NodeId,
    depth: u16,
    hash: u64,
}

struct RawEntry {
    name: String,
    meta: EntryMeta,
}

struct Batch {
    job: Job,
    entries: Vec<RawEntry>,
    error: Option<String>,
}

/// Bounded so a fast disk cannot let workers run arbitrarily far ahead of the
/// collector and balloon memory; deep enough that workers never stall in
/// practice.
const RESULT_QUEUE_DEPTH: usize = 512;

/// How often the collector looks up from the results channel to notice a
/// cancellation.
const POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(50);

pub fn scan(
    options: &ScanOptions,
    control: ScanControl,
    mut on_progress: impl FnMut(&ScanProgress),
) -> ScanOutcome {
    let started = Instant::now();
    let mut tree = Tree::new(&options.root);
    let root_meta = platform::entry_meta(&options.root);

    if let Ok(meta) = &root_meta {
        tree.node_mut(NodeId::ROOT).mtime = meta.mtime;
    }
    if root_meta.is_err() {
        tree.node_mut(NodeId::ROOT).flags.insert(NodeFlags::INACCESSIBLE);
        tree.errors.push(ScanError {
            path: options.root.clone(),
            message: "root is not readable".into(),
        });
        return finish(tree, ScanStats::default(), ScanState::Done, started, control);
    }
    let root_device = root_meta.ok().and_then(|m| m.file_id).map(|(dev, _)| dev);

    let index = options.previous.as_ref().map(|t| DirIndex::build(t));
    let (jobs_tx, jobs_rx) = unbounded::<Job>();
    let (results_tx, results_rx) = bounded::<Batch>(RESULT_QUEUE_DEPTH);

    let threads = options.threads.max(1);
    let workers: Vec<_> = (0..threads)
        .map(|_| spawn_worker(jobs_rx.clone(), results_tx.clone(), control.clone()))
        .collect();
    // The collector must not hold a result sender, or `results_rx` would never
    // disconnect if a worker panicked.
    drop(results_tx);
    drop(jobs_rx);

    let mut stats = ScanStats::default();
    let mut eta = EtaEstimator::new(options.expected_bytes);
    let mut seen_inodes: HashSet<(u64, u64)> = HashSet::new();
    let mut last_emit = Instant::now();

    let root_hash = incremental::root_hash(&options.root);
    let mut pending: u64 = 1;
    let _ = jobs_tx.send(Job {
        path: options.root.clone(),
        node: NodeId::ROOT,
        depth: 0,
        hash: root_hash,
    });

    let mut final_state = ScanState::Done;

    while pending > 0 {
        // A blocking `recv` would deadlock on cancellation: workers that are
        // idle on the job queue never send a batch, and the collector still
        // holds the job sender, so the results channel never disconnects.
        // Polling gives cancellation a place to land even when no work is
        // moving. 50 ms is far below human perception and costs ~20 wakeups a
        // second on an otherwise idle thread.
        let batch = match results_rx.recv_timeout(POLL_INTERVAL) {
            Ok(batch) => batch,
            Err(RecvTimeoutError::Timeout) => {
                if control.is_cancelled() {
                    final_state = ScanState::Cancelled;
                    break;
                }
                continue;
            }
            // Every worker is gone; nothing more can arrive.
            Err(RecvTimeoutError::Disconnected) => break,
        };
        pending -= 1;
        stats.dirs_scanned += 1;

        if let Some(message) = batch.error {
            tree.node_mut(batch.job.node).flags.insert(NodeFlags::INACCESSIBLE);
            stats.errors += 1;
            if tree.errors.len() < options.max_recorded_errors {
                tree.errors.push(ScanError {
                    path: batch.job.path.clone(),
                    message,
                });
            }
            continue;
        }

        for entry in batch.entries {
            let child_depth = batch.job.depth + 1;
            let child_path = batch.job.path.join(&entry.name);
            let meta = entry.meta;
            let hidden = platform::is_hidden(&entry.name, &meta);

            if hidden && options.skip_hidden {
                continue;
            }
            if options.is_excluded(&child_path) {
                continue;
            }

            let system = options.mark_system && platform::is_system_path(&child_path);
            let mut flags = NodeFlags::empty();
            if hidden {
                flags.insert(NodeFlags::HIDDEN);
            }
            if system {
                flags.insert(NodeFlags::SYSTEM);
            }
            if meta.is_symlink {
                flags.insert(NodeFlags::SYMLINK);
            }

            // Symlinks are recorded at their own (tiny) size and never
            // traversed: following them would double-count bytes and, for a
            // link pointing at an ancestor, never terminate.
            if meta.is_dir && !meta.is_symlink {
                flags.insert(NodeFlags::DIRECTORY);
                if platform::is_package(&entry.name, true) {
                    flags.insert(NodeFlags::PACKAGE);
                }
                let foreign_device = match (root_device, meta.file_id) {
                    (Some(root), Some((dev, _))) => dev != root,
                    _ => false,
                };
                if foreign_device {
                    flags.insert(NodeFlags::MOUNT_POINT);
                }

                let node = tree.push_node(
                    &entry.name,
                    batch.job.node,
                    child_depth,
                    flags,
                    crate::category::Category::Other,
                    0,
                    0,
                    meta.mtime,
                );

                // `max_depth` is the deepest directory level that gets
                // expanded: with a depth of 1, the volume's top-level folders
                // are opened and their contents sized, but their subfolders are
                // listed without being walked.
                let too_deep = options.max_depth.is_some_and(|d| child_depth > d);
                let blocked = foreign_device && !options.cross_filesystem;
                if too_deep || blocked {
                    continue;
                }

                let child_hash = incremental::hash_child(batch.job.hash, &entry.name);
                if let Some((prev, cached)) = index
                    .as_ref()
                    .and_then(|i| i.reusable(child_hash, meta.mtime))
                    .and_then(|c| options.previous.as_ref().map(|p| (p, c)))
                {
                    let grafted = incremental::graft_subtree(&mut tree, node, prev, cached);
                    stats.files_scanned += grafted.files;
                    stats.dirs_scanned += grafted.dirs;
                    stats.dirs_reused += grafted.dirs + 1;
                    stats.bytes_seen += grafted.bytes;
                    continue;
                }

                pending += 1;
                if jobs_tx
                    .send(Job {
                        path: child_path,
                        node,
                        depth: child_depth,
                        hash: child_hash,
                    })
                    .is_err()
                {
                    pending -= 1;
                }
                continue;
            }

            // Regular file (or symlink, counted as the link itself).
            let mut logical = meta.logical_size;
            let mut physical = meta.physical_size;
            if options.deduplicate_hardlinks && meta.nlink > 1 && !meta.is_dir {
                if let Some(id) = meta.file_id {
                    if !seen_inodes.insert(id) {
                        // Already counted under its first path; keep the entry
                        // visible but weightless so totals stay honest.
                        flags.insert(NodeFlags::HARDLINK_DUP);
                        logical = 0;
                        physical = 0;
                    }
                }
            }

            let category = classify(&entry.name, system);
            tree.push_node(
                &entry.name,
                batch.job.node,
                child_depth,
                flags,
                category,
                logical,
                physical,
                meta.mtime,
            );
            stats.files_scanned += 1;
            stats.bytes_seen += logical;
        }

        if last_emit.elapsed() >= options.progress_interval {
            last_emit = Instant::now();
            let (eta_ms, fraction) = eta.update(stats.bytes_seen);
            on_progress(&ScanProgress {
                state: control.state(),
                files_seen: stats.files_scanned,
                dirs_seen: stats.dirs_scanned,
                bytes_seen: stats.bytes_seen,
                dirs_reused: stats.dirs_reused,
                errors: stats.errors,
                current_path: batch.job.path.to_string_lossy().into_owned(),
                elapsed_ms: eta.elapsed().as_millis() as u64,
                eta_ms,
                fraction,
            });
        }

        if control.is_cancelled() {
            final_state = ScanState::Cancelled;
            break;
        }
    }

    // Dropping the sender is what tells idle workers to exit.
    drop(jobs_tx);
    if final_state == ScanState::Cancelled {
        // Drain so no worker is blocked sending into a full queue while we join.
        while results_rx.recv().is_ok() {}
    }
    for worker in workers {
        let _ = worker.join();
    }

    finish(tree, stats, final_state, started, control)
}

fn finish(
    mut tree: Tree,
    mut stats: ScanStats,
    state: ScanState,
    started: Instant,
    _control: ScanControl,
) -> ScanOutcome {
    tree.rollup();
    stats.elapsed_ms = started.elapsed().as_millis() as u64;
    stats.nodes = tree.len() as u64;
    stats.memory_bytes = tree.memory_bytes() as u64;
    ScanOutcome { tree, stats, state }
}

fn spawn_worker(
    jobs: Receiver<Job>,
    results: Sender<Batch>,
    control: ScanControl,
) -> std::thread::JoinHandle<()> {
    std::thread::Builder::new()
        .name("helios-scan".into())
        // Workers hold no recursion and small buffers; the default 2 MiB stack
        // per thread is pure waste when we spawn one per core.
        .stack_size(256 * 1024)
        .spawn(move || {
            while let Ok(job) = jobs.recv() {
                if !control.wait_if_paused() {
                    break;
                }
                let batch = read_directory(job);
                if results.send(batch).is_err() {
                    break;
                }
            }
        })
        .expect("failed to spawn scan worker")
}

fn read_directory(job: Job) -> Batch {
    let iter = match fs::read_dir(&job.path) {
        Ok(iter) => iter,
        Err(err) => {
            return Batch {
                job,
                entries: Vec::new(),
                error: Some(err.to_string()),
            }
        }
    };

    let mut entries = Vec::new();
    for entry in iter {
        // A single unreadable entry must not sink the whole directory: a
        // permission-denied file in an otherwise readable folder is routine on
        // macOS (`~/Library/Mail`, TCC-protected paths).
        let Ok(entry) = entry else { continue };
        let Ok(meta) = platform::meta_from_dir_entry(&entry) else {
            continue;
        };
        entries.push(RawEntry {
            name: entry.file_name().to_string_lossy().into_owned(),
            meta,
        });
    }

    Batch {
        job,
        entries,
        error: None,
    }
}
