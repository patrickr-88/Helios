//! The IPC surface.
//!
//! Design rules, applied to every command here:
//!
//! 1. **The tree never crosses the bridge.** Commands return the page the UI is
//!    about to draw — a few hundred rows at most — so IPC cost is independent
//!    of scan size.
//! 2. **Nothing mutates the user's disk.** The only writes are exports the user
//!    explicitly requested and Helios's own snapshot cache.
//! 3. **Long work leaves the UI thread.** Scans run on their own thread and
//!    report through events; queries are fast enough to answer inline.
//! 4. **Errors are strings the UI can show.** There is no failure here that the
//!    user cannot be told about in one sentence.

use std::path::PathBuf;
use std::sync::Arc;

use helios_core::model::{NodeId, Tree};
use helios_core::query::{self, CategorySummary, Entry, Filter, SortKey};
use helios_core::report;
use helios_core::scan::{self, ScanControl, ScanOptions, ScanState};
use helios_core::snapshot::{self, Snapshot, SnapshotMeta};
use helios_core::treemap::{self, Rect, Tile, TreemapOptions};
use helios_core::{fmt, platform, Volume};
use serde::{Deserialize, Serialize};
// `Manager` is what puts `AppHandle::state` in scope on the driver thread.
use tauri::{AppHandle, Emitter, Manager, State};

use crate::state::{AppState, LoadedScan};

/// Events emitted to the webview.
pub const EVENT_PROGRESS: &str = "scan://progress";
pub const EVENT_FINISHED: &str = "scan://finished";
pub const EVENT_FAILED: &str = "scan://failed";

type Result<T> = std::result::Result<T, String>;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanRequest {
    pub path: String,
    /// Reuse the cached snapshot for an incremental rescan.
    #[serde(default)]
    pub incremental: bool,
    #[serde(default)]
    pub skip_hidden: bool,
    #[serde(default)]
    pub cross_filesystem: bool,
    #[serde(default)]
    pub exclusions: Vec<String>,
    #[serde(default)]
    pub threads: Option<usize>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanSummary {
    pub scan_id: String,
    pub root_path: String,
    pub state: ScanState,
    pub total_bytes: u64,
    pub physical_bytes: u64,
    pub file_count: u32,
    pub dir_count: u32,
    pub scanned_at: i64,
    pub elapsed_ms: u64,
    pub node_count: u64,
    pub memory_bytes: u64,
    pub dirs_reused: u64,
    pub error_count: usize,
    pub from_cache: bool,
    pub volume: Option<Volume>,
}

fn summarize(scan: &LoadedScan, volume: Option<Volume>) -> ScanSummary {
    let root = scan.tree.node(NodeId::ROOT);
    ScanSummary {
        scan_id: scan.meta.volume_id.clone(),
        root_path: scan.tree.root_path.to_string_lossy().into_owned(),
        state: ScanState::Done,
        total_bytes: scan.tree.total_logical(),
        physical_bytes: scan.tree.total_physical(),
        file_count: root.file_count,
        dir_count: root.dir_count,
        scanned_at: scan.meta.scanned_at,
        elapsed_ms: scan.stats.elapsed_ms,
        node_count: scan.stats.nodes,
        memory_bytes: scan.stats.memory_bytes,
        dirs_reused: scan.stats.dirs_reused,
        error_count: scan.tree.errors.len(),
        from_cache: scan.from_cache,
        volume,
    }
}

/// Volumes for the sidebar.
#[tauri::command]
pub fn list_volumes() -> Vec<Volume> {
    platform::volumes()
}

/// The volume a path lives on: the longest matching mount point wins, so
/// `/Volumes/Backup/x` resolves to `Backup` and not to `/`.
fn volume_for(path: &std::path::Path) -> Option<Volume> {
    platform::volumes()
        .into_iter()
        .filter(|v| path.starts_with(&v.mount_point))
        .max_by_key(|v| v.mount_point.as_os_str().len())
}

/// A scan id that is stable across runs for a volume, and unique per path for
/// folder scans.
fn scan_id_for(path: &std::path::Path, volume: Option<&Volume>) -> String {
    match volume {
        Some(v) if v.mount_point == path => v.id.clone(),
        _ => path.to_string_lossy().into_owned(),
    }
}

/// Starts a scan on a background thread and returns its id immediately.
///
/// Progress arrives as `scan://progress` events and the result as
/// `scan://finished`; the command itself never blocks the UI.
#[tauri::command]
pub fn start_scan(app: AppHandle, state: State<'_, AppState>, request: ScanRequest) -> Result<String> {
    let path = PathBuf::from(&request.path)
        .canonicalize()
        .map_err(|e| format!("Cannot open {}: {e}", request.path))?;
    if !path.is_dir() {
        return Err(format!("{} is not a folder", path.display()));
    }

    let volume = volume_for(&path);
    let scan_id = scan_id_for(&path, volume.as_ref());

    let mut options = ScanOptions::new(&path);
    options.expected_bytes = volume.as_ref().map(|v| v.used_bytes);
    options.skip_hidden = request.skip_hidden;
    options.cross_filesystem = request.cross_filesystem;
    options.exclusions = request.exclusions.iter().map(PathBuf::from).collect();
    if let Some(threads) = request.threads {
        options.threads = threads.clamp(1, 32);
    }
    if request.incremental {
        // Prefer the in-memory tree; fall back to the on-disk snapshot.
        if let Some(previous) = state.get(&scan_id) {
            options.previous = Some(previous.tree.clone());
        } else if let Ok(cached) = snapshot::load(&scan_id) {
            if cached.tree.root_path == path {
                options.previous = Some(Arc::new(cached.tree));
            }
        }
    }

    let control = ScanControl::new();
    state.begin(scan_id.clone(), control.clone());

    let handle = app.clone();
    let id = scan_id.clone();
    std::thread::Builder::new()
        .name("helios-scan-driver".into())
        .spawn(move || {
            let emitter = handle.clone();
            let outcome = scan::scan(&options, control, move |progress| {
                // A dropped event is not worth interrupting a scan for: the
                // next sample is 100 ms away.
                let _ = emitter.emit(EVENT_PROGRESS, progress);
            });

            let meta = SnapshotMeta {
                volume_id: id.clone(),
                root_path: options.root.clone(),
                scanned_at: snapshot::now_unix(),
                stats: outcome.stats.clone(),
            };
            let state: State<'_, AppState> = handle.state();
            let tree = Arc::new(outcome.tree);
            let loaded = LoadedScan {
                meta: meta.clone(),
                tree: tree.clone(),
                stats: outcome.stats.clone(),
                from_cache: false,
            };
            state.insert(id.clone(), loaded.clone());
            state.end(&id);

            // Only complete scans are cached: a cancelled tree is a partial
            // view, and persisting it would make the next incremental rescan
            // inherit the gap.
            if outcome.state == ScanState::Done {
                let snapshot = Snapshot {
                    meta,
                    tree: (*tree).clone(),
                };
                if let Err(err) = snapshot::save(&snapshot) {
                    eprintln!("helios: could not cache snapshot: {err}");
                }
            }

            let mut summary = summarize(&loaded, volume_for(&options.root));
            summary.state = outcome.state;
            let _ = handle.emit(EVENT_FINISHED, summary);
        })
        .map_err(|e| {
            let _ = app.emit(EVENT_FAILED, e.to_string());
            format!("Could not start the scan: {e}")
        })?;

    Ok(scan_id)
}

#[tauri::command]
pub fn pause_scan(state: State<'_, AppState>) -> Result<()> {
    state.active_control().ok_or("No scan is running")?.pause();
    Ok(())
}

#[tauri::command]
pub fn resume_scan(state: State<'_, AppState>) -> Result<()> {
    state.active_control().ok_or("No scan is running")?.resume();
    Ok(())
}

#[tauri::command]
pub fn cancel_scan(state: State<'_, AppState>) -> Result<()> {
    state.active_control().ok_or("No scan is running")?.cancel();
    Ok(())
}

#[tauri::command]
pub fn active_scan(state: State<'_, AppState>) -> Option<String> {
    state.active_id()
}

fn tree_of(state: &State<'_, AppState>, scan_id: &str) -> Result<Arc<Tree>> {
    state
        .get(scan_id)
        .map(|s| s.tree)
        .ok_or_else(|| format!("No scan loaded for '{scan_id}' — run a scan first"))
}

#[tauri::command]
pub fn scan_summary(state: State<'_, AppState>, scan_id: String) -> Result<ScanSummary> {
    let scan = state
        .get(&scan_id)
        .ok_or_else(|| format!("No scan loaded for '{scan_id}'"))?;
    let volume = volume_for(&scan.tree.root_path);
    Ok(summarize(&scan, volume))
}

#[tauri::command]
pub fn loaded_scans(state: State<'_, AppState>) -> Vec<String> {
    state.ids()
}

/// Children of one folder, for the Folder Tree view. Paged by `limit`, which is
/// what keeps the row list flat regardless of how many files a folder holds.
#[tauri::command]
pub fn list_children(
    state: State<'_, AppState>,
    scan_id: String,
    node_id: u32,
    filter: Option<Filter>,
    sort: Option<SortKey>,
    descending: Option<bool>,
    limit: Option<usize>,
) -> Result<Vec<Entry>> {
    let tree = tree_of(&state, &scan_id)?;
    let node = NodeId(node_id);
    if node.index() >= tree.len() {
        return Err("That folder is not part of this scan".into());
    }
    Ok(query::children(
        &tree,
        node,
        &filter.unwrap_or_default(),
        sort.unwrap_or(SortKey::Size),
        descending.unwrap_or(true),
        limit.unwrap_or(1000).min(10_000),
    ))
}

/// The path from the tree root down to a node, for the breadcrumb bar.
#[tauri::command]
pub fn ancestors(state: State<'_, AppState>, scan_id: String, node_id: u32) -> Result<Vec<Entry>> {
    let tree = tree_of(&state, &scan_id)?;
    let mut chain = Vec::new();
    let mut current = NodeId(node_id);
    if current.index() >= tree.len() {
        return Err("That folder is not part of this scan".into());
    }
    loop {
        let parent = tree.node(current).parent;
        let parent_size = if parent.is_none() {
            tree.total_logical()
        } else {
            tree.node(parent).logical_size
        };
        chain.push(entry_for(&tree, current, parent_size));
        if parent.is_none() {
            break;
        }
        current = parent;
    }
    chain.reverse();
    Ok(chain)
}

/// Builds one row without enumerating its siblings.
fn entry_for(tree: &Tree, id: NodeId, parent_size: u64) -> Entry {
    use helios_core::model::NodeFlags;
    let node = tree.node(id);
    Entry {
        id: id.0,
        name: tree.name(id).to_string(),
        path: tree.path_of(id).to_string_lossy().into_owned(),
        size: node.logical_size,
        physical_size: node.physical_size,
        category: node.category,
        is_dir: node.is_dir(),
        is_symlink: node.is_symlink(),
        is_hidden: node.flags.contains(NodeFlags::HIDDEN),
        is_system: node.flags.contains(NodeFlags::SYSTEM),
        is_package: node.flags.contains(NodeFlags::PACKAGE),
        is_accessible: !node.flags.contains(NodeFlags::INACCESSIBLE),
        mtime: node.mtime,
        file_count: node.file_count,
        dir_count: node.dir_count,
        // The root has no parent, so it is 100% of itself.
        fraction_of_parent: if parent_size > 0 {
            (node.logical_size as f64 / parent_size as f64) as f32
        } else {
            1.0
        },
    }
}

/// Treemap rectangles for a viewport, laid out in Rust.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub fn treemap_layout(
    state: State<'_, AppState>,
    scan_id: String,
    node_id: u32,
    width: f32,
    height: f32,
    max_depth: Option<u16>,
    max_tiles: Option<usize>,
    include_hidden: Option<bool>,
) -> Result<Vec<Tile>> {
    let tree = tree_of(&state, &scan_id)?;
    let node = NodeId(node_id);
    if node.index() >= tree.len() {
        return Err("That folder is not part of this scan".into());
    }
    let options = TreemapOptions {
        max_depth: max_depth.unwrap_or(6).clamp(1, 12),
        max_tiles: max_tiles.unwrap_or(20_000).clamp(100, 200_000),
        include_hidden: include_hidden.unwrap_or(true),
        ..TreemapOptions::default()
    };
    Ok(treemap::layout(
        &tree,
        node,
        Rect::new(0.0, 0.0, width.max(1.0), height.max(1.0)),
        &options,
    ))
}

#[tauri::command]
pub fn largest_entries(
    state: State<'_, AppState>,
    scan_id: String,
    dirs: bool,
    limit: Option<usize>,
    filter: Option<Filter>,
) -> Result<Vec<Entry>> {
    let tree = tree_of(&state, &scan_id)?;
    Ok(query::largest(
        &tree,
        &filter.unwrap_or_else(Filter::permissive),
        limit.unwrap_or(100).min(10_000),
        dirs,
    ))
}

#[tauri::command]
pub fn search_entries(
    state: State<'_, AppState>,
    scan_id: String,
    filter: Filter,
    sort: Option<SortKey>,
    limit: Option<usize>,
) -> Result<Vec<Entry>> {
    let tree = tree_of(&state, &scan_id)?;
    Ok(query::search(
        &tree,
        &filter,
        sort.unwrap_or(SortKey::Size),
        limit.unwrap_or(500).min(10_000),
    ))
}

#[tauri::command]
pub fn category_breakdown(
    state: State<'_, AppState>,
    scan_id: String,
    filter: Option<Filter>,
) -> Result<Vec<CategorySummary>> {
    let tree = tree_of(&state, &scan_id)?;
    Ok(query::category_breakdown(
        &tree,
        &filter.unwrap_or_else(Filter::permissive),
    ))
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanIssue {
    pub path: String,
    pub message: String,
}

/// Paths the scan could not read — shown in the UI so the user knows a total is
/// a floor rather than silently wrong.
#[tauri::command]
pub fn scan_issues(
    state: State<'_, AppState>,
    scan_id: String,
    limit: Option<usize>,
) -> Result<Vec<ScanIssue>> {
    let tree = tree_of(&state, &scan_id)?;
    Ok(tree
        .errors
        .iter()
        .take(limit.unwrap_or(200))
        .map(|e| ScanIssue {
            path: e.path.to_string_lossy().into_owned(),
            message: e.message.clone(),
        })
        .collect())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportRequest {
    pub scan_id: String,
    /// `csv`, `json` or `pdf`.
    pub format: String,
    pub destination: String,
    #[serde(default)]
    pub top_n: Option<usize>,
    #[serde(default)]
    pub filter: Option<Filter>,
}

/// Writes a report to a path the user picked in the save dialog.
#[tauri::command]
pub fn export_report(state: State<'_, AppState>, request: ExportRequest) -> Result<String> {
    let scan = state
        .get(&request.scan_id)
        .ok_or_else(|| format!("No scan loaded for '{}'", request.scan_id))?;
    let volume = volume_for(&scan.tree.root_path);
    let filter = request.filter.unwrap_or_else(Filter::permissive);
    let report = report::build(
        &scan.tree,
        &scan.meta,
        volume.as_ref(),
        &filter,
        request.top_n.unwrap_or(report::DEFAULT_TOP_N).min(10_000),
    );

    let bytes = match request.format.as_str() {
        "csv" => report::to_csv(&report).into_bytes(),
        "json" => report::to_json(&report).into_bytes(),
        "pdf" => report::to_pdf(&report),
        other => return Err(format!("Unknown export format '{other}'")),
    };

    let destination = PathBuf::from(&request.destination);
    // Writing outside the destination the user chose in the save panel would be
    // a surprise; the dialog plugin is the only source of this path.
    std::fs::write(&destination, &bytes)
        .map_err(|e| format!("Could not write {}: {e}", destination.display()))?;
    Ok(format!(
        "Saved {} ({})",
        destination.display(),
        fmt::human_bytes(bytes.len() as u64)
    ))
}

#[tauri::command]
pub fn list_snapshots() -> Vec<SnapshotMeta> {
    snapshot::list()
}

/// Loads a cached scan so the app has something to show immediately at launch.
#[tauri::command]
pub fn load_snapshot(state: State<'_, AppState>, volume_id: String) -> Result<ScanSummary> {
    let snapshot = snapshot::load(&volume_id).map_err(|e| format!("No usable snapshot: {e}"))?;
    let stats = snapshot.meta.stats.clone();
    let loaded = LoadedScan {
        meta: snapshot.meta,
        tree: Arc::new(snapshot.tree),
        stats,
        from_cache: true,
    };
    let volume = volume_for(&loaded.tree.root_path);
    state.insert(volume_id, loaded.clone());
    Ok(summarize(&loaded, volume))
}

#[tauri::command]
pub fn forget_scan(state: State<'_, AppState>, scan_id: String) -> Result<()> {
    state.forget(&scan_id);
    snapshot::delete(&scan_id).map_err(|e| format!("Could not delete the cached snapshot: {e}"))
}

/// Opens the enclosing folder in the system file manager, with the item
/// selected. Helios never opens or modifies the file itself — this hands the
/// user off to the tool that can.
#[tauri::command]
pub fn reveal_in_file_manager(path: String) -> Result<()> {
    let path = PathBuf::from(path);
    if !path.exists() {
        return Err("That item no longer exists on disk".into());
    }
    #[cfg(target_os = "macos")]
    let mut command = {
        let mut c = std::process::Command::new("open");
        c.arg("-R").arg(&path);
        c
    };
    #[cfg(windows)]
    let mut command = {
        let mut c = std::process::Command::new("explorer");
        c.arg(format!("/select,{}", path.display()));
        c
    };
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut command = {
        let mut c = std::process::Command::new("xdg-open");
        c.arg(path.parent().unwrap_or(&path));
        c
    };

    command
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("Could not open the file manager: {e}"))
}

/// Version and build facts for the About panel.
#[tauri::command]
pub fn app_info() -> serde_json::Value {
    serde_json::json!({
        "version": env!("CARGO_PKG_VERSION"),
        "engineVersion": helios_core::VERSION,
        "cacheDirectory": snapshot::cache_dir().to_string_lossy(),
        "defaultThreads": scan::default_threads(),
        "offline": true,
        "readOnly": true,
    })
}
