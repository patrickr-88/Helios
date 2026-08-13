# API

Two surfaces: the command line, and the Rust library it is built on.

## The command line

```
helios                     list every volume, with capacity and free space
helios <path>              scan a folder or volume and summarize it
helios <path> --tree       show the folder tree instead of the top lists
helios <path> -o out.pdf   write a report (.csv, .json or .pdf)
```

| Flag | Meaning |
|---|---|
| `-t, --tree` | Folder tree with a size and share-of-parent at every level |
| `-d, --depth <n>` | How deep the tree goes (default 2) |
| `-n, --top <n>` | Entries per list (default 15) |
| `--files` / `--folders` | Only one of the two lists |
| `--find <text>` | Everything whose name — or path, if it contains a separator — matches |
| `--ext <list>` | Only these extensions: `mp4,mov,zip` |
| `--min-size <size>` | `500`, `10MB`, `2.5G`, `1 TB` |
| `-o, --out <file>` | Write a report; the extension picks the format |
| `-c, --cache` | Reuse and update the cached scan |
| `--no-hidden` | Skip hidden files entirely |
| `--threads <n>` | Scan workers (default: cores, capped at 8) |
| `--exclude <path>` | Skip a path; repeatable |
| `-q, --quiet` | No progress line |
| `--info` | Where data is kept, whether it is writable, what is cached |

Design rules, which are why there are no subcommands:

1. **The common case is one word.** `helios ~/Downloads` needs no verb; a path
   is unambiguously a request to scan it, and no path is unambiguously a request
   to list volumes.
2. **The output format follows the filename.** `-o report.csv` means CSV. A
   separate `--format` flag would be a second way to say the same thing, and a
   way to say two contradictory things.
3. **Nothing is interactive.** No prompts, no menus, no alternate screen. Output
   is a stream you can pipe, and every list is bounded so a pipe cannot be
   flooded.
4. **Progress goes to stderr, results to stdout.** `helios / > report.txt` gives
   a clean file with a live progress line still on the terminal.
5. **Exit codes are the usual ones.** 0 on success — including a scan stopped
   with Ctrl-C, which still prints partial results — and 1 on a bad argument or
   an unreadable root.

## The types the library hands back

`Entry` — one row in any list:

```rust
pub struct Entry {
    pub id: u32,                    // NodeId, stable for the lifetime of the scan
    pub name: String,
    pub path: String,
    pub size: u64,                  // logical bytes; rolled up for folders
    pub physical_size: u64,         // bytes on disk
    pub category: Category,
    pub is_dir: bool, pub is_symlink: bool, pub is_hidden: bool,
    pub is_system: bool, pub is_package: bool,
    pub is_accessible: bool,        // false ⇒ size is a lower bound
    pub mtime: i64,                 // Unix seconds
    pub file_count: u32, pub dir_count: u32,   // subtree counts, folders only
    pub fraction_of_parent: f32,    // 0–1, precomputed
}
```

`Filter` — every field optional, all conditions AND-ed:

```rust
pub struct Filter {
    pub min_size: Option<u64>, pub max_size: Option<u64>,
    pub extensions: Vec<String>,        // lowercase, no dot
    pub categories: Vec<Category>,
    pub modified_after: Option<i64>, pub modified_before: Option<i64>,
    pub path_contains: Option<String>, pub name_contains: Option<String>,
    pub include_hidden: bool,           // default false
    pub include_system: bool,
    pub only_files: bool, pub only_dirs: bool,
}
```

The predicate order inside `Filter::matches` is deliberate: cheap integer
comparisons first, then the name, and path reconstruction last — it is the only
expensive test, and it runs only for nodes that already passed everything else.

`Category` — `Documents | Images | Videos | Audio | Archives | Applications |
Developer | System | Other`.

## Rust library API

```rust
use helios_core::scan::{scan, ScanControl, ScanOptions};
use helios_core::query::{largest, Filter};
use helios_core::{report, snapshot};

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

// 3. Export a report.
let meta = snapshot::SnapshotMeta::new(Some(&volume), path, outcome.stats.clone());
let report = report::build(&outcome.tree, &meta, Some(&volume), &Filter::permissive(), 100);
std::fs::write("report.pdf", report::to_pdf(&report))?;

// 4. Cache it, and reuse it next time.
snapshot::save(&snapshot::Snapshot { meta, tree: outcome.tree })?;
let previous = snapshot::load_for_volume(&volume)?;   // verifies host and volume
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

The command line follows the usual expectations: adding a flag is a minor
change, changing what an existing flag means is a breaking one. The Rust crate
follows semver — adding a function or an optional field is a minor bump;
changing a return shape or `Node`'s layout is a major one. `snapshot::FORMAT_VERSION` is
independent — bumping it invalidates caches, which is always safe, because a
snapshot is never a source of truth.
