# API

Two surfaces: the IPC commands the interface calls, and the Rust library any
other front end would use.

## Design rules

1. **The tree never crosses the bridge.** Every command returns the page the UI
   is about to draw. IPC cost is independent of scan size.
2. **Long work is asynchronous, queries are not.** Scans run on their own thread
   and report through events; everything else answers fast enough to await
   inline.
3. **Errors are sentences.** Commands return `Result<T, String>` with text the
   UI can show a user directly.
4. **Nothing writes to a scanned volume.** The only writes are exports to a path
   the user chose in a save panel, and Helios's own snapshot cache.

## Events

| Event | Payload | When |
|---|---|---|
| `scan://progress` | `ScanProgress` | Up to 10×/second during a scan |
| `scan://finished` | `ScanSummary` | Scan completed **or** was cancelled — check `state` |
| `scan://failed` | `string` | The scan could not be started |

```ts
const stop = await listen<ScanProgress>("scan://progress", (p) => {
  setFiles(p.files_seen);
  setEta(p.eta_ms);
});
```

## Commands

### Volumes and scans

| Command | Arguments | Returns |
|---|---|---|
| `list_volumes` | — | `Volume[]` |
| `start_scan` | `request: ScanRequest` | `scanId` (immediately; work continues in the background) |
| `pause_scan` | — | `void` |
| `resume_scan` | — | `void` |
| `cancel_scan` | — | `void` |
| `active_scan` | — | `scanId \| null` |
| `scan_summary` | `scanId` | `ScanSummary` |
| `loaded_scans` | — | `scanId[]` |

```ts
interface ScanRequest {
  path: string;
  incremental?: boolean;      // reuse unchanged folders from the last scan
  skipHidden?: boolean;
  crossFilesystem?: boolean;  // descend into other mounted volumes
  exclusions?: string[];
  threads?: number | null;    // clamped to 1..32
}
```

Only one scan runs at a time; starting a second cancels the first. Two
concurrent walks contend for the same disk and both finish later than either
would alone.

### Querying a scan

| Command | Arguments | Returns |
|---|---|---|
| `list_children` | `scanId, nodeId, filter?, sort?, descending?, limit?` | `Entry[]` |
| `ancestors` | `scanId, nodeId` | `Entry[]` (root → node, for breadcrumbs) |
| `treemap_layout` | `scanId, nodeId, width, height, maxDepth?, maxTiles?, includeHidden?` | `Tile[]` |
| `largest_entries` | `scanId, dirs, limit?, filter?` | `Entry[]` |
| `search_entries` | `scanId, filter, sort?, limit?` | `Entry[]` |
| `category_breakdown` | `scanId, filter?` | `CategorySummary[]` |
| `scan_issues` | `scanId, limit?` | `ScanIssue[]` (paths that could not be read) |

Limits are clamped server-side (10,000 rows, 200,000 tiles). A one-character
search on a 10-million-file volume cannot flood the bridge.

### Reports, cache, system

| Command | Arguments | Returns |
|---|---|---|
| `export_report` | `request: ExportRequest` | status string |
| `list_snapshots` | — | `SnapshotMeta[]` (newest first) |
| `load_snapshot` | `volumeId` | `ScanSummary` |
| `forget_scan` | `scanId` | `void` (drops the in-memory tree and deletes the cache file) |
| `reveal_in_file_manager` | `path` | `void` |
| `app_info` | — | version, cache directory, thread default |

```ts
interface ExportRequest {
  scanId: string;
  format: "csv" | "json" | "pdf";
  destination: string;   // from the save panel — never constructed by the UI
  topN?: number;
  filter?: Filter;
}
```

`reveal_in_file_manager` opens Finder (or Explorer) with the item selected. It
is the one place Helios hands off to something that *can* modify files —
deliberately, because "show me where this is so I can deal with it" is the
natural end of the workflow, and doing the deleting itself is a different
product with a different risk profile.

## Shared types

`Entry` — one row in any list:

```ts
interface Entry {
  id: number;               // NodeId, stable for the lifetime of the scan
  name: string;
  path: string;
  size: number;             // logical bytes; rolled up for folders
  physicalSize: number;     // bytes on disk
  category: Category;
  isDir: boolean; isSymlink: boolean; isHidden: boolean;
  isSystem: boolean; isPackage: boolean;
  isAccessible: boolean;    // false ⇒ size is a lower bound
  mtime: number;            // Unix seconds
  fileCount: number; dirCount: number;   // subtree counts, folders only
  fractionOfParent: number; // 0–1, precomputed
}
```

`Filter` — every field optional, all conditions AND-ed:

```ts
interface Filter {
  minSize?: number; maxSize?: number;
  extensions?: string[];        // lowercase, no dot
  categories?: Category[];
  modifiedAfter?: number; modifiedBefore?: number;   // Unix seconds
  pathContains?: string; nameContains?: string;      // case-insensitive
  includeHidden?: boolean;      // default false
  includeSystem?: boolean;      // default false
  onlyFiles?: boolean; onlyDirs?: boolean;
}
```

The predicate order inside `Filter::matches` is deliberate: cheap integer
comparisons first, then the name, and path reconstruction last — it is the only
expensive test, and it runs only for nodes that already passed everything else.

`Tile` — one treemap rectangle: `{ id, name, size, category, isDir, depth,
rect: { x, y, w, h }, truncated }`. Tiles come back parent-before-child, so
hit-testing scans backwards to find the innermost hit.

`Category` — `"documents" | "images" | "videos" | "audio" | "archives" |
"applications" | "developer" | "system" | "other"`.

## Rust library API

```rust
use helios_core::scan::{scan, ScanControl, ScanOptions};
use helios_core::query::{largest, Filter};
use helios_core::{report, snapshot, treemap};

// 1. Scan, with progress and a control handle.
let mut options = ScanOptions::new("/Volumes/Backup");
options.expected_bytes = Some(volume.used_bytes);   // enables the ETA
let control = ScanControl::new();                    // clone this to pause/cancel
let outcome = scan(&options, control.clone(), |p| {
    println!("{} files, {} bytes", p.files_seen, p.bytes_seen);
});

// 2. Query the tree.
for entry in largest(&outcome.tree, &Filter::permissive(), 100, false) {
    println!("{:>12}  {}", entry.size, entry.path);
}

// 3. Lay out a treemap.
let tiles = treemap::layout(
    &outcome.tree,
    helios_core::NodeId::ROOT,
    treemap::Rect::new(0.0, 0.0, 1200.0, 800.0),
    &treemap::TreemapOptions::default(),
);

// 4. Export a report.
let meta = snapshot::SnapshotMeta { /* volume id, root, scanned_at, stats */ };
let report = report::build(&outcome.tree, &meta, Some(&volume), &Filter::permissive(), 100);
std::fs::write("report.pdf", report::to_pdf(&report))?;

// 5. Cache it, and reuse it next time.
snapshot::save(&snapshot::Snapshot { meta, tree: outcome.tree })?;
let previous = snapshot::load(&volume.id)?;
let faster = ScanOptions::new("/Volumes/Backup").with_previous(Arc::new(previous.tree));
```

Walking the tree directly:

```rust
let tree = &outcome.tree;
for child in tree.children(NodeId::ROOT) {
    println!("{:>10}  {}", tree.node(child).logical_size, tree.name(child));
}
tree.path_of(some_node);           // reconstruct an absolute path
tree.find(Path::new("/a/b/c"));    // look a node up by path
```

## Versioning

The IPC surface is internal to the app and moves with it. The Rust crate follows
semver: adding a command or an optional field is a minor bump; changing a return
shape or `Node`'s layout is a major one. `snapshot::FORMAT_VERSION` is
independent — bumping it invalidates caches, which is always safe, because a
snapshot is never a source of truth.
