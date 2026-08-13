//! Scan orchestration: options, statistics, and the public entry point.

pub mod control;
pub mod incremental;
pub mod progress;
mod walker;

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::model::Tree;
use crate::platform;

pub use control::{ScanControl, ScanState};
pub use progress::ScanProgress;

/// Everything that shapes a scan. Constructed by the UI, defaulted sensibly for
/// headless callers.
#[derive(Debug, Clone)]
pub struct ScanOptions {
    /// Volume mount point or folder to scan.
    pub root: PathBuf,
    /// Worker threads. Defaults to the core count capped at 8 — beyond that,
    /// directory scanning is queue-bound rather than CPU-bound, and extra
    /// threads only make the machine feel busier to the user.
    pub threads: usize,
    /// Descend into other filesystems mounted below the root. Off by default,
    /// so scanning `/` does not silently pull in a 4 TB Time Machine drive.
    pub cross_filesystem: bool,
    /// Omit hidden entries entirely rather than flagging them.
    pub skip_hidden: bool,
    /// Tag entries under OS-owned prefixes as system files.
    pub mark_system: bool,
    /// Attribute a hardlinked inode's bytes to the first path that reaches it.
    pub deduplicate_hardlinks: bool,
    /// Deepest directory level to expand. `Some(1)` sizes the top-level
    /// folders of a volume without descending further, which is how the UI
    /// renders a fast first pass.
    pub max_depth: Option<u16>,
    /// Skip the platform's pseudo-filesystems and firmlink mounts.
    pub use_default_exclusions: bool,
    /// Additional user-supplied paths to skip.
    pub exclusions: Vec<PathBuf>,
    /// Volume used-bytes, from the OS, used to estimate completion.
    pub expected_bytes: Option<u64>,
    /// Minimum wall time between progress callbacks. 100 ms keeps the UI
    /// smooth at 10 Hz without flooding the IPC bridge.
    pub progress_interval: Duration,
    /// Cap on errors kept in the tree, so a systematically unreadable volume
    /// cannot grow the snapshot without bound.
    pub max_recorded_errors: usize,
    /// Previous snapshot for an incremental rescan.
    pub previous: Option<std::sync::Arc<Tree>>,
}

impl ScanOptions {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        ScanOptions {
            root: root.into(),
            threads: default_threads(),
            cross_filesystem: false,
            skip_hidden: false,
            mark_system: true,
            deduplicate_hardlinks: true,
            max_depth: None,
            use_default_exclusions: true,
            exclusions: Vec::new(),
            expected_bytes: None,
            progress_interval: Duration::from_millis(100),
            max_recorded_errors: 1000,
            previous: None,
        }
    }

    pub fn with_previous(mut self, previous: std::sync::Arc<Tree>) -> Self {
        self.previous = Some(previous);
        self
    }

    pub(crate) fn is_excluded(&self, path: &Path) -> bool {
        if self.use_default_exclusions && platform::is_excluded(path) {
            return true;
        }
        self.exclusions.iter().any(|p| path == p || path.starts_with(p))
    }
}

pub fn default_threads() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .clamp(1, 8)
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ScanStats {
    pub files_scanned: u64,
    pub dirs_scanned: u64,
    pub bytes_seen: u64,
    pub dirs_reused: u64,
    pub errors: u64,
    pub elapsed_ms: u64,
    pub nodes: u64,
    pub memory_bytes: u64,
}

#[derive(Debug)]
pub struct ScanOutcome {
    pub tree: Tree,
    pub stats: ScanStats,
    /// `Done` for a complete scan, `Cancelled` if the user stopped it — in
    /// which case the tree is a valid partial result, not garbage.
    pub state: ScanState,
}

/// Scans `options.root`, reporting progress through `on_progress`.
///
/// Read-only by construction: this crate opens directories for enumeration and
/// calls `stat`. It contains no `write`, `remove`, `rename`, or `create` call
/// against the scanned volume — see the `no_write_syscalls` test in
/// `tests/read_only.rs`, which enforces that at the source level.
pub fn scan(
    options: &ScanOptions,
    control: ScanControl,
    on_progress: impl FnMut(&ScanProgress),
) -> ScanOutcome {
    walker::scan(options, control, on_progress)
}

/// Convenience wrapper for headless callers that do not want progress events.
pub fn scan_blocking(options: &ScanOptions) -> ScanOutcome {
    scan(options, ScanControl::new(), |_| {})
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{NodeFlags, NodeId};
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;

    /// Builds a fixture tree:
    /// ```text
    /// root/
    ///   docs/report.pdf        (3000 bytes)
    ///   media/clip.mp4         (5000 bytes)
    ///   media/nested/photo.jpg (1000 bytes)
    ///   loose.txt              (10 bytes)
    ///   link -> media          (symlink, not followed)
    /// ```
    fn fixture() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("docs")).unwrap();
        fs::create_dir_all(root.join("media/nested")).unwrap();
        fs::write(root.join("docs/report.pdf"), vec![0u8; 3000]).unwrap();
        fs::write(root.join("media/clip.mp4"), vec![0u8; 5000]).unwrap();
        fs::write(root.join("media/nested/photo.jpg"), vec![0u8; 1000]).unwrap();
        fs::write(root.join("loose.txt"), vec![0u8; 10]).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(root.join("media"), root.join("link")).unwrap();
        dir
    }

    /// Total bytes of the symlink entries themselves.
    ///
    /// A symlink occupies a real (small) amount of space for its target path,
    /// and Helios counts that — what it never does is count the *target's*
    /// bytes. The exact figure depends on the temp directory's path length, so
    /// tests add it in rather than hard-coding it.
    fn symlink_bytes(tree: &crate::model::Tree) -> u64 {
        tree.iter()
            .filter(|(_, n)| n.is_symlink())
            .map(|(_, n)| n.logical_size)
            .sum()
    }

    fn options(root: &Path) -> ScanOptions {
        let mut o = ScanOptions::new(root);
        // Fixtures live under /tmp, which some platforms list in their default
        // exclusions; the test owns its own tree, so opt out.
        o.use_default_exclusions = false;
        o
    }

    #[test]
    fn scans_a_tree_and_rolls_up_sizes() {
        let dir = fixture();
        let out = scan_blocking(&options(dir.path()));

        assert_eq!(out.state, ScanState::Done);
        assert_eq!(out.tree.total_logical(), 9010 + symlink_bytes(&out.tree));
        assert_eq!(out.stats.errors, 0);

        let media = out.tree.find(&dir.path().join("media")).unwrap();
        assert_eq!(out.tree.node(media).logical_size, 6000);
        assert_eq!(out.tree.node(media).file_count, 2);
    }

    #[test]
    #[cfg(unix)]
    fn symlinked_directories_are_recorded_but_never_followed() {
        let dir = fixture();
        let out = scan_blocking(&options(dir.path()));

        let link = out.tree.find(&dir.path().join("link")).unwrap();
        assert!(out.tree.node(link).flags.contains(NodeFlags::SYMLINK));
        assert_eq!(out.tree.children(link).count(), 0, "must not descend a link");
        // 6000 bytes of media counted exactly once, not twice: following the
        // link would have pushed the total to ~15 KB.
        assert_eq!(out.tree.total_logical(), 9010 + symlink_bytes(&out.tree));
    }

    #[test]
    #[cfg(unix)]
    fn a_symlink_loop_terminates() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("a")).unwrap();
        fs::write(dir.path().join("a/f.bin"), vec![0u8; 16]).unwrap();
        // A link pointing at its own ancestor: fatal for a naive walker.
        std::os::unix::fs::symlink(dir.path(), dir.path().join("a/loop")).unwrap();

        let out = scan_blocking(&options(dir.path()));
        assert_eq!(out.tree.total_logical(), 16 + symlink_bytes(&out.tree));
        assert!(out.tree.len() < 10, "the loop must not have expanded the tree");
    }

    #[test]
    fn categories_are_assigned_from_extensions() {
        use crate::category::Category;
        let dir = fixture();
        let out = scan_blocking(&options(dir.path()));

        let pdf = out.tree.find(&dir.path().join("docs/report.pdf")).unwrap();
        let clip = out.tree.find(&dir.path().join("media/clip.mp4")).unwrap();
        assert_eq!(out.tree.node(pdf).category, Category::Documents);
        assert_eq!(out.tree.node(clip).category, Category::Videos);
    }

    #[test]
    fn max_depth_stops_descent() {
        let dir = fixture();
        let mut opts = options(dir.path());
        opts.max_depth = Some(1);
        let out = scan_blocking(&opts);

        // The depth-2 folder is still listed — the user can see it exists —
        // but it is not expanded, so its contents are not counted.
        let nested = out.tree.find(&dir.path().join("media/nested")).unwrap();
        assert_eq!(out.tree.children(nested).count(), 0);
        assert!(out.tree.find(&dir.path().join("media/nested/photo.jpg")).is_none());
        assert_eq!(out.tree.total_logical(), 8010 + symlink_bytes(&out.tree));
    }

    #[test]
    fn user_exclusions_are_honoured() {
        let dir = fixture();
        let mut opts = options(dir.path());
        opts.exclusions = vec![dir.path().join("media")];
        let out = scan_blocking(&opts);

        assert_eq!(out.tree.total_logical(), 3010 + symlink_bytes(&out.tree));
        assert!(out.tree.find(&dir.path().join("media")).is_none());
    }

    #[test]
    fn unreadable_directories_are_reported_not_fatal() {
        let dir = fixture();
        let locked = dir.path().join("locked");
        fs::create_dir(&locked).unwrap();
        fs::write(locked.join("secret.bin"), vec![0u8; 4096]).unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
        }

        let out = scan_blocking(&options(dir.path()));

        // Running as root defeats permission bits; only assert the graceful
        // path when the directory is genuinely unreadable.
        if fs::read_dir(&locked).is_err() {
            assert_eq!(out.stats.errors, 1);
            let node = out.tree.find(&locked).unwrap();
            assert!(out.tree.node(node).flags.contains(NodeFlags::INACCESSIBLE));
        }
        // Either way the rest of the tree is intact.
        assert!(out.tree.total_logical() >= 9010);
        assert!(out.tree.find(&dir.path().join("media/clip.mp4")).is_some());

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&locked, fs::Permissions::from_mode(0o755));
        }
    }

    #[test]
    #[cfg(unix)]
    fn hardlinks_are_counted_once() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("original.bin"), vec![0u8; 2048]).unwrap();
        fs::hard_link(dir.path().join("original.bin"), dir.path().join("clone.bin")).unwrap();

        let out = scan_blocking(&options(dir.path()));
        assert_eq!(out.tree.total_logical(), 2048, "one inode, counted once");
        assert_eq!(out.tree.node(NodeId::ROOT).file_count, 2, "both paths listed");

        let mut opts = options(dir.path());
        opts.deduplicate_hardlinks = false;
        assert_eq!(scan_blocking(&opts).tree.total_logical(), 4096);
    }

    #[test]
    fn progress_is_emitted_and_monotonic() {
        let dir = fixture();
        let mut opts = options(dir.path());
        opts.progress_interval = Duration::ZERO;

        let samples = Arc::new(AtomicU64::new(0));
        let last_bytes = Arc::new(AtomicU64::new(0));
        {
            let (samples, last_bytes) = (samples.clone(), last_bytes.clone());
            scan(&opts, ScanControl::new(), move |p| {
                samples.fetch_add(1, Ordering::Relaxed);
                let prev = last_bytes.swap(p.bytes_seen, Ordering::Relaxed);
                assert!(p.bytes_seen >= prev, "byte count must never go backwards");
            });
        }
        assert!(samples.load(Ordering::Relaxed) > 0, "no progress events");
    }

    #[test]
    fn cancellation_returns_a_usable_partial_tree() {
        let dir = fixture();
        let control = ScanControl::new();
        control.cancel();

        let out = scan(&options(dir.path()), control, |_| {});
        assert_eq!(out.state, ScanState::Cancelled);
        // A cancelled scan still returns a coherent tree; it is simply short.
        assert!(out.tree.total_logical() <= 9010 + symlink_bytes(&out.tree));
    }

    #[test]
    fn incremental_rescan_reuses_untouched_directories() {
        let dir = fixture();
        let first = scan_blocking(&options(dir.path()));
        let baseline = Arc::new(first.tree);

        let opts = options(dir.path()).with_previous(baseline.clone());
        let second = scan(&opts, ScanControl::new(), |_| {});

        assert_eq!(second.tree.total_logical(), baseline.total_logical());
        assert!(
            second.stats.dirs_reused > 0,
            "unchanged directories should have been grafted, not walked"
        );
    }

    #[test]
    fn incremental_rescan_picks_up_a_changed_directory() {
        let dir = fixture();
        let baseline = Arc::new(scan_blocking(&options(dir.path())).tree);

        // Sleep past filesystem mtime granularity (1 s on HFS+ and some NFS
        // mounts) so the change is guaranteed to be observable.
        std::thread::sleep(Duration::from_millis(1100));
        fs::write(dir.path().join("docs/added.pdf"), vec![0u8; 500]).unwrap();

        let opts = options(dir.path()).with_previous(baseline.clone());
        let out = scan(&opts, ScanControl::new(), |_| {});

        assert_eq!(out.tree.total_logical(), 9510 + symlink_bytes(&out.tree));
        assert!(out.tree.find(&dir.path().join("docs/added.pdf")).is_some());
    }
}
