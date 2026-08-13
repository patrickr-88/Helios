//! `helios` — read-only disk usage analysis.
//!
//! One binary, no runtime, no installer, no configuration file. Run it with a
//! path and it prints where the space went; run it with no arguments and it
//! lists the volumes it can see.
//!
//! Argument parsing is hand-rolled. `clap` is excellent and this program has a
//! dozen flags — not a derive macro, a builder DSL and a 300 KB dependency
//! tree. The whole point of the tool is that it is small enough to read.

use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use helios_core::fmt::{format_timestamp, human_bytes, human_duration};
use helios_core::query::{self, Entry, Filter, SortKey};
use helios_core::report;
use helios_core::scan::{scan, ScanControl, ScanOptions, ScanState};
use helios_core::snapshot::{self, Snapshot, SnapshotMeta};
use helios_core::{platform, NodeId, Tree};

const USAGE: &str = "\
helios — see where your storage went. Reads your disks; never changes them.

USAGE
    helios                     list every volume, with capacity and free space
    helios <path>              scan a folder or volume and summarize it
    helios <path> --tree       show the folder tree instead of the top lists
    helios <path> -o out.pdf   write a report (.csv, .json or .pdf)

OPTIONS
    -t, --tree            folder tree with sizes at every level
    -d, --depth <n>       how deep the tree goes                [default: 2]
    -n, --top <n>         entries per list                      [default: 15]
        --files           only the largest files
        --folders         only the largest folders
        --find <text>     list everything whose name or path contains <text>
        --ext <list>      only these extensions, e.g. mp4,mov,zip
        --min-size <size> only entries at least this big, e.g. 100MB
    -o, --out <file>      write a report; the extension picks the format
    -c, --cache           reuse and update the cached scan (fast rescans)
        --no-hidden       skip hidden files entirely
        --threads <n>     scan workers                     [default: cores, max 8]
        --exclude <path>  skip a path (repeatable)
    -q, --quiet           no progress line
        --info            where Helios keeps its data, and what it has cached
    -h, --help            this text
    -V, --version

EXAMPLES
    helios ~/Downloads
    helios / --cache                 # cache it; the next run takes milliseconds
    helios ~/Movies --tree -d 3
    helios / --find node_modules
    helios / -o storage-report.pdf

Set HELIOS_DATA_DIR, or put a 'helios-portable' file beside the binary, to keep
the cache next to the program — that is how it runs from a flash drive without
writing anything to the host machine.
";

fn main() -> ExitCode {
    match run(std::env::args().skip(1).collect()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("helios: {message}");
            ExitCode::FAILURE
        }
    }
}

fn run(argv: Vec<String>) -> Result<(), String> {
    let args = Args::parse(&argv)?;

    if args.help {
        print!("{USAGE}");
        return Ok(());
    }
    if args.version {
        println!("helios {}", helios_core::VERSION);
        return Ok(());
    }
    if args.info {
        return show_info();
    }
    match &args.path {
        Some(path) => scan_path(&args, path.clone()),
        None => list_volumes(),
    }
}

// ---------------------------------------------------------------- arguments

#[derive(Debug, Default)]
struct Args {
    path: Option<PathBuf>,
    tree: bool,
    depth: u16,
    top: usize,
    files_only: bool,
    folders_only: bool,
    find: Option<String>,
    extensions: Vec<String>,
    min_size: Option<u64>,
    out: Option<PathBuf>,
    cache: bool,
    no_hidden: bool,
    threads: Option<usize>,
    excludes: Vec<PathBuf>,
    quiet: bool,
    info: bool,
    help: bool,
    version: bool,
}

impl Args {
    fn parse(argv: &[String]) -> Result<Args, String> {
        let mut args = Args {
            depth: 2,
            top: 15,
            ..Args::default()
        };
        let mut i = 0;

        while i < argv.len() {
            let arg = argv[i].as_str();
            // Every option below that takes a value uses this, so a missing
            // value is one error message rather than a dozen.
            let mut value = |name: &str| -> Result<String, String> {
                i += 1;
                argv.get(i)
                    .cloned()
                    .ok_or_else(|| format!("{name} needs a value"))
            };

            match arg {
                "-h" | "--help" => args.help = true,
                "-V" | "--version" => args.version = true,
                "--info" => args.info = true,
                "-t" | "--tree" => args.tree = true,
                "--files" => args.files_only = true,
                "--folders" => args.folders_only = true,
                "-c" | "--cache" => args.cache = true,
                "--no-hidden" => args.no_hidden = true,
                "-q" | "--quiet" => args.quiet = true,
                "-d" | "--depth" => args.depth = parse_number(&value("--depth")?, "--depth")?,
                "-n" | "--top" => args.top = parse_number(&value("--top")?, "--top")?,
                "--threads" => {
                    args.threads = Some(parse_number(&value("--threads")?, "--threads")?)
                }
                "--find" => args.find = Some(value("--find")?),
                "--min-size" => args.min_size = Some(parse_size(&value("--min-size")?)?),
                "--ext" => {
                    args.extensions = value("--ext")?
                        .split(',')
                        .map(|e| e.trim().trim_start_matches('.').to_lowercase())
                        .filter(|e| !e.is_empty())
                        .collect()
                }
                "-o" | "--out" => args.out = Some(PathBuf::from(value("--out")?)),
                "--exclude" => args.excludes.push(PathBuf::from(value("--exclude")?)),
                other if other.starts_with('-') && other != "-" => {
                    return Err(format!("unknown option '{other}' (try --help)"))
                }
                other => {
                    if args.path.is_some() {
                        return Err(format!("only one path at a time (got '{other}' as well)"));
                    }
                    args.path = Some(PathBuf::from(other));
                }
            }
            i += 1;
        }
        Ok(args)
    }

    fn filter(&self) -> Filter {
        Filter {
            include_hidden: !self.no_hidden,
            include_system: true,
            min_size: self.min_size,
            extensions: self.extensions.clone(),
            ..Filter::default()
        }
    }
}

fn parse_number<T: std::str::FromStr>(value: &str, flag: &str) -> Result<T, String> {
    value
        .parse()
        .map_err(|_| format!("{flag} expects a number, got '{value}'"))
}

/// Accepts plain byte counts and the obvious suffixes: `500`, `10MB`, `2.5G`.
fn parse_size(value: &str) -> Result<u64, String> {
    let trimmed = value.trim().to_uppercase();
    let digits = trimmed
        .trim_end_matches(|c: char| c.is_ascii_alphabetic())
        .trim();
    // Trimmed because "1 TB" arrives as one argument when it is quoted.
    let suffix = trimmed[digits.len()..].trim();
    let number: f64 = digits
        .parse()
        .map_err(|_| format!("--min-size expects a size, got '{value}'"))?;

    let multiplier = match suffix {
        "" | "B" => 1.0,
        "K" | "KB" => 1e3,
        "M" | "MB" => 1e6,
        "G" | "GB" => 1e9,
        "T" | "TB" => 1e12,
        other => {
            return Err(format!(
                "unknown size unit '{other}' (use KB, MB, GB or TB)"
            ))
        }
    };
    Ok((number * multiplier) as u64)
}

// ------------------------------------------------------------------ commands

fn list_volumes() -> Result<(), String> {
    let volumes = platform::volumes();
    if volumes.is_empty() {
        println!("No volumes reported.");
        return Ok(());
    }

    println!(
        "{:<22} {:<20} {:>9} {:>9} {:>9}  {:<14} TYPE",
        "VOLUME", "MOUNT", "SIZE", "USED", "FREE", "IN USE"
    );
    for v in &volumes {
        let used = if v.total_bytes > 0 {
            v.used_bytes as f64 / v.total_bytes as f64
        } else {
            0.0
        };
        let mut kind = v.filesystem.clone();
        for (flag, label) in [
            (v.is_network, "network"),
            (v.is_removable, "removable"),
            (v.is_read_only, "read-only"),
        ] {
            if flag {
                kind.push_str(&format!(", {label}"));
            }
        }
        println!(
            "{:<22} {:<20} {:>9} {:>9} {:>9}  {} {:>3.0}%  {}",
            truncate(&v.name, 22),
            truncate(&v.mount_point.to_string_lossy(), 20),
            human_bytes(v.total_bytes),
            human_bytes(v.used_bytes),
            human_bytes(v.free_bytes),
            bar(used, 8),
            used * 100.0,
            kind
        );
    }
    println!("\nScan one with:  helios <mount point>");
    Ok(())
}

fn show_info() -> Result<(), String> {
    println!("helios {}", helios_core::VERSION);
    println!(
        "mode        {}",
        if platform::is_portable() {
            "portable — data stays next to the program"
        } else {
            "standard — data in the usual per-user location"
        }
    );
    if let Some(dir) = platform::app_directory() {
        println!("program in  {}", dir.display());
    }
    if std::env::var_os("HELIOS_DATA_DIR").is_some() {
        println!("override    HELIOS_DATA_DIR is set and wins over everything else");
    }
    let cache = snapshot::cache_dir();
    println!("data        {}", platform::data_dir().display());

    // Portable drives are routinely read-only or full; better to say so now
    // than at the end of a long scan.
    let writable = std::fs::create_dir_all(&cache)
        .and_then(|_| {
            let probe = cache.join(".helios-write-probe");
            std::fs::write(&probe, b"")?;
            std::fs::remove_file(&probe)
        })
        .is_ok();
    println!(
        "writable    {}",
        if writable {
            "yes"
        } else {
            "no — scans still work, nothing will be cached"
        }
    );

    let cached = snapshot::list_for_this_host();
    println!("\nCached scans ({})", cached.len());
    for meta in &cached {
        println!(
            "  {:<42} {:>9}  {}",
            truncate(&meta.root_path.to_string_lossy(), 42),
            human_bytes(meta.stats.bytes_seen),
            format_timestamp(meta.scanned_at)
        );
    }
    if cached.is_empty() {
        println!("  (none yet — run 'helios <path> --cache')");
    }
    Ok(())
}

fn scan_path(args: &Args, path: PathBuf) -> Result<(), String> {
    let path = path
        .canonicalize()
        .map_err(|e| format!("cannot open {}: {e}", path.display()))?;
    if !path.is_dir() {
        return Err(format!("{} is not a folder", path.display()));
    }

    // The longest matching mount point is the volume the path is really on.
    let volume = platform::volumes()
        .into_iter()
        .filter(|v| path.starts_with(&v.mount_point))
        .max_by_key(|v| v.mount_point.as_os_str().len());

    let mut options = ScanOptions::new(&path);
    options.expected_bytes = volume.as_ref().map(|v| v.used_bytes);
    options.skip_hidden = args.no_hidden;
    options.exclusions = args.excludes.clone();
    if let Some(threads) = args.threads {
        options.threads = threads;
    }
    if args.cache {
        if let Some(previous) = volume
            .as_ref()
            // load_for_volume, not load: a cache carried on a flash drive may
            // hold another machine's scan under the same volume id.
            .and_then(|v| snapshot::load_for_volume(v).ok())
            .filter(|p| p.tree.root_path == path)
        {
            options.previous = Some(Arc::new(previous.tree));
        }
    }

    let control = ScanControl::new();
    install_interrupt_handler(control.clone());
    let outcome = scan(&options, control, progress_printer(args.quiet));

    if outcome.state == ScanState::Cancelled {
        eprintln!("stopped early — showing what was scanned so far");
    }

    let meta = SnapshotMeta::new(volume.as_ref(), path.clone(), outcome.stats.clone());
    if args.cache {
        match snapshot::save(&Snapshot {
            meta: meta.clone(),
            tree: outcome.tree.clone(),
        }) {
            Ok(p) => eprintln!("cached: {}", p.display()),
            Err(e) => eprintln!("warning: could not cache this scan: {e}"),
        }
    }

    if let Some(destination) = &args.out {
        let report = report::build(
            &outcome.tree,
            &meta,
            volume.as_ref(),
            &args.filter(),
            args.top.max(report::DEFAULT_TOP_N),
        );
        let bytes = match format_from_extension(destination)? {
            "csv" => report::to_csv(&report).into_bytes(),
            "json" => report::to_json(&report).into_bytes(),
            _ => report::to_pdf(&report),
        };
        std::fs::write(destination, &bytes)
            .map_err(|e| format!("cannot write {}: {e}", destination.display()))?;
        println!(
            "Wrote {} ({})",
            destination.display(),
            human_bytes(bytes.len() as u64)
        );
        return Ok(());
    }

    print_report(args, &outcome.tree, &outcome.stats, volume.as_ref());
    Ok(())
}

fn format_from_extension(path: &Path) -> Result<&'static str, String> {
    match path
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .as_deref()
    {
        Some("csv") => Ok("csv"),
        Some("json") => Ok("json"),
        Some("pdf") => Ok("pdf"),
        _ => Err(format!(
            "cannot tell the format of {} — end it in .csv, .json or .pdf",
            path.display()
        )),
    }
}

// -------------------------------------------------------------------- output

fn print_report(
    args: &Args,
    tree: &Tree,
    stats: &helios_core::scan::ScanStats,
    volume: Option<&platform::Volume>,
) {
    let filter = args.filter();

    println!("\n{}", tree.root_path.display());
    println!(
        "  {} in {} files, {} folders · {}",
        human_bytes(tree.total_logical()),
        stats.files_scanned.to_string_separated(),
        stats.dirs_scanned.to_string_separated(),
        human_duration(stats.elapsed_ms)
    );
    if stats.dirs_reused > 0 {
        println!(
            "  {} folders reused from the last scan",
            stats.dirs_reused.to_string_separated()
        );
    }
    if let Some(v) = volume {
        let used = if v.total_bytes > 0 {
            v.used_bytes as f64 / v.total_bytes as f64
        } else {
            0.0
        };
        println!(
            "  on {}: {} of {} used ({:.0}%), {} free",
            v.name,
            human_bytes(v.used_bytes),
            human_bytes(v.total_bytes),
            used * 100.0,
            human_bytes(v.free_bytes)
        );
    }

    if let Some(needle) = &args.find {
        let mut search = filter.clone();
        if needle.contains('/') || needle.contains('\\') {
            search.path_contains = Some(needle.clone());
        } else {
            search.name_contains = Some(needle.clone());
        }
        let hits = query::search(tree, &search, SortKey::Size, args.top);
        print_list(&format!("Matching “{needle}”"), &hits, tree.total_logical());
        return;
    }

    if args.tree {
        println!();
        let mut out = std::io::stdout().lock();
        // A closed pipe (`helios / --tree | head`) is a normal way to stop
        // reading, not an error worth a message.
        let _ = render_tree(&mut out, tree, NodeId::ROOT, &filter, args.depth, args.top);
        return;
    }

    if !args.files_only {
        print_list(
            "Largest folders",
            &query::largest(tree, &filter, args.top, true),
            tree.total_logical(),
        );
    }
    if !args.folders_only {
        print_list(
            "Largest files",
            &query::largest(tree, &filter, args.top, false),
            tree.total_logical(),
        );
    }
    if !args.files_only && !args.folders_only {
        print_categories(tree, &filter);
    }

    if !tree.errors.is_empty() {
        println!(
            "\n{} location(s) could not be read, so these totals are a floor:",
            tree.errors.len()
        );
        for error in tree.errors.iter().take(3) {
            println!("  {} — {}", error.path.display(), error.message);
        }
        if tree.errors.len() > 3 {
            println!("  … and {} more", tree.errors.len() - 3);
        }
    }
}

fn print_list(title: &str, entries: &[Entry], total: u64) {
    println!("\n{title}");
    if entries.is_empty() {
        println!("  (nothing matches)");
        return;
    }
    let width = entries
        .iter()
        .map(|e| e.name.chars().count())
        .max()
        .unwrap_or(0)
        .clamp(8, 40);

    for (i, e) in entries.iter().enumerate() {
        let share = if total > 0 {
            e.size as f64 / total as f64
        } else {
            0.0
        };
        println!(
            "  {:>2}  {:<width$}  {:>9}  {} {:>4.1}%  {}",
            i + 1,
            truncate(&e.name, width),
            human_bytes(e.size),
            bar(share, 10),
            share * 100.0,
            dim(&shorten(&e.path, 44))
        );
    }
}

fn print_categories(tree: &Tree, filter: &Filter) {
    let rows: Vec<_> = query::category_breakdown(tree, filter)
        .into_iter()
        .filter(|c| c.bytes > 0)
        .collect();
    if rows.is_empty() {
        return;
    }
    println!("\nBy category");
    for c in rows {
        println!(
            "  {:<14} {:>9}  {} {:>4.1}%  {} files",
            c.category.as_str(),
            human_bytes(c.bytes),
            bar(f64::from(c.fraction), 20),
            c.fraction * 100.0,
            c.files.to_string_separated()
        );
    }
}

/// The folder tree, which is what the graphical version's tree view showed.
///
/// Writes to a sink rather than straight to stdout so the layout can be tested;
/// the box-drawing characters here are multi-byte, and an earlier version
/// corrupted its own indent by slicing the prefix on byte offsets.
fn render_tree(
    out: &mut impl Write,
    tree: &Tree,
    root: NodeId,
    filter: &Filter,
    depth: u16,
    per_level: usize,
) -> std::io::Result<()> {
    writeln!(
        out,
        "{}  {}",
        tree.root_path.display(),
        human_bytes(tree.total_logical())
    )?;
    render_branch(out, tree, root, filter, depth, per_level, "")
}

fn render_branch(
    out: &mut impl Write,
    tree: &Tree,
    parent: NodeId,
    filter: &Filter,
    depth: u16,
    per_level: usize,
    prefix: &str,
) -> std::io::Result<()> {
    if depth == 0 {
        return Ok(());
    }
    // Directories only: a tree of every file is a listing, not a shape.
    let mut dirs_filter = filter.clone();
    dirs_filter.only_dirs = true;
    let children = query::children(tree, parent, &dirs_filter, SortKey::Size, true, per_level);

    let parent_size = tree.node(parent).logical_size;
    for (i, child) in children.iter().enumerate() {
        let last = i == children.len() - 1;
        let share = if parent_size > 0 {
            child.size as f64 / parent_size as f64
        } else {
            0.0
        };
        // Names give ground to the indent so the size column stays aligned.
        let name_width = 34usize.saturating_sub(prefix.chars().count()).max(8);

        writeln!(
            out,
            "{prefix}{}{:<name_width$}  {:>9}  {} {:>4.1}%",
            if last { "└── " } else { "├── " },
            truncate(&child.name, name_width),
            human_bytes(child.size),
            bar(share, 10),
            share * 100.0,
        )?;

        if child.dir_count > 0 {
            // A fresh string per level: no mutation, so no way to slice a
            // multi-byte character in half on the way back up.
            let nested = format!("{prefix}{}", if last { "    " } else { "│   " });
            render_branch(
                out,
                tree,
                NodeId(child.id),
                filter,
                depth - 1,
                per_level,
                &nested,
            )?;
        }
    }
    Ok(())
}

/// A proportional bar. Uses eighth-blocks so short bars still show a difference.
fn bar(fraction: f64, width: usize) -> String {
    let clamped = fraction.clamp(0.0, 1.0);
    let eighths = (clamped * width as f64 * 8.0).round() as usize;
    let full = eighths / 8;
    let remainder = eighths % 8;

    let mut out = "█".repeat(full.min(width));
    if full < width && remainder > 0 {
        out.push(['▏', '▎', '▍', '▌', '▋', '▊', '▉'][remainder - 1]);
    }
    let drawn = out.chars().count();
    out.push_str(&"·".repeat(width.saturating_sub(drawn)));
    out
}

fn progress_printer(quiet: bool) -> impl FnMut(&helios_core::ScanProgress) {
    // Only animate for a human: piping to a file should not collect carriage
    // returns and spinner frames.
    let show = !quiet && std::io::stderr().is_terminal();
    let mut last_len = 0usize;

    move |p| {
        if !show {
            return;
        }
        let eta = p
            .eta_ms
            .map(|ms| format!(" · ~{} left", human_duration(ms)))
            .unwrap_or_default();
        let line = format!(
            "  {} files · {}{}  {}",
            p.files_seen.to_string_separated(),
            human_bytes(p.bytes_seen),
            eta,
            shorten(&p.current_path, 42)
        );
        eprint!("\r{line:<last_len$}");
        last_len = line.chars().count().max(last_len);
        let _ = std::io::stderr().flush();
    }
}

// -------------------------------------------------------------------- helpers

/// Ctrl-C stops the scan and prints partial results instead of killing the
/// process, which is what you want three minutes into scanning a full disk.
fn install_interrupt_handler(control: ScanControl) {
    #[cfg(unix)]
    {
        use std::sync::atomic::{AtomicBool, Ordering};
        static TRIGGERED: AtomicBool = AtomicBool::new(false);

        extern "C" fn handler(_: i32) {
            TRIGGERED.store(true, Ordering::Release);
        }
        extern "C" {
            fn signal(signum: i32, handler: usize) -> usize;
        }
        const SIGINT: i32 = 2;
        // SAFETY: the handler only performs an atomic store, which is
        // async-signal-safe.
        unsafe { signal(SIGINT, handler as *const () as usize) };

        std::thread::spawn(move || loop {
            if TRIGGERED.load(Ordering::Acquire) {
                control.cancel();
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        });
    }
    #[cfg(not(unix))]
    let _ = control;
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    format!(
        "{}…",
        text.chars().take(max.saturating_sub(1)).collect::<String>()
    )
}

/// Middle-truncates a path, keeping the volume and the last two components —
/// the parts that actually identify it.
fn shorten(path: &str, max: usize) -> String {
    if path.chars().count() <= max {
        return path.to_string();
    }
    let parts: Vec<&str> = path.split('/').collect();
    let tail = parts
        .iter()
        .rev()
        .take(2)
        .rev()
        .copied()
        .collect::<Vec<_>>()
        .join("/");
    format!("…/{}", truncate(&tail, max.saturating_sub(2)))
}

fn dim(text: &str) -> String {
    if std::io::stdout().is_terminal() {
        format!("\x1b[2m{text}\x1b[0m")
    } else {
        text.to_string()
    }
}

/// Thousands separators, because a bare `1247893` in a column of sizes is
/// unreadable and pulling in a formatting crate for it would be silly.
trait Separated {
    fn to_string_separated(&self) -> String;
}

impl Separated for u64 {
    fn to_string_separated(&self) -> String {
        let digits = self.to_string();
        let mut out = String::with_capacity(digits.len() + digits.len() / 3);
        for (i, c) in digits.chars().enumerate() {
            if i > 0 && (digits.len() - i) % 3 == 0 {
                out.push(',');
            }
            out.push(c);
        }
        out
    }
}

impl Separated for u32 {
    fn to_string_separated(&self) -> String {
        u64::from(*self).to_string_separated()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Args, String> {
        Args::parse(&args.iter().map(|a| a.to_string()).collect::<Vec<_>>())
    }

    #[test]
    fn a_bare_path_is_the_common_case() {
        let args = parse(&["/Users/me/Movies"]).unwrap();
        assert_eq!(args.path, Some(PathBuf::from("/Users/me/Movies")));
        assert_eq!(args.top, 15);
        assert!(!args.tree);
    }

    #[test]
    fn no_arguments_means_list_volumes() {
        assert_eq!(parse(&[]).unwrap().path, None);
    }

    #[test]
    fn flags_and_paths_mix_in_any_order() {
        let args = parse(&["--tree", "/data", "-d", "4", "-n", "5"]).unwrap();
        assert_eq!(args.path, Some(PathBuf::from("/data")));
        assert!(args.tree);
        assert_eq!(args.depth, 4);
        assert_eq!(args.top, 5);
    }

    #[test]
    fn mistakes_get_a_useful_message() {
        assert!(parse(&["--nope"]).unwrap_err().contains("unknown option"));
        assert!(parse(&["--top"]).unwrap_err().contains("needs a value"));
        assert!(parse(&["--top", "lots"]).unwrap_err().contains("number"));
        assert!(parse(&["/a", "/b"]).unwrap_err().contains("one path"));
    }

    #[test]
    fn sizes_accept_the_usual_suffixes() {
        assert_eq!(parse_size("500").unwrap(), 500);
        assert_eq!(parse_size("10MB").unwrap(), 10_000_000);
        assert_eq!(parse_size("2.5G").unwrap(), 2_500_000_000);
        assert_eq!(parse_size(" 1 tb ").unwrap(), 1_000_000_000_000);
        assert!(parse_size("12 furlongs").is_err());
    }

    #[test]
    fn extensions_are_normalised() {
        let args = parse(&["--ext", ".MP4, mov ,", "/x"]).unwrap();
        assert_eq!(args.extensions, vec!["mp4", "mov"]);
    }

    #[test]
    fn report_format_comes_from_the_file_name() {
        assert_eq!(format_from_extension(Path::new("r.CSV")).unwrap(), "csv");
        assert_eq!(format_from_extension(Path::new("r.pdf")).unwrap(), "pdf");
        assert!(format_from_extension(Path::new("report")).is_err());
    }

    #[test]
    fn bars_are_proportional_and_fixed_width() {
        assert_eq!(bar(0.0, 4).chars().count(), 4);
        assert_eq!(bar(1.0, 4), "████");
        assert_eq!(bar(0.5, 4).chars().next(), Some('█'));
        // Clamped rather than overflowing when a share exceeds 100%.
        assert_eq!(bar(9.9, 4), "████");
    }

    #[test]
    fn counts_get_thousands_separators() {
        assert_eq!(76_693u64.to_string_separated(), "76,693");
        assert_eq!(999u64.to_string_separated(), "999");
        assert_eq!(1_000_000u64.to_string_separated(), "1,000,000");
    }

    /// Builds `root/a/b/c` with one file at the bottom, to exercise nesting.
    fn nested_tree() -> Tree {
        use helios_core::model::NodeFlags;
        use helios_core::Category;

        let mut tree = Tree::new("/root");
        let mut parent = NodeId::ROOT;
        for (depth, name) in ["alpha", "beta", "gamma"].iter().enumerate() {
            parent = tree.push_node(
                name,
                parent,
                depth as u16 + 1,
                NodeFlags::DIRECTORY,
                Category::Other,
                0,
                0,
                0,
            );
        }
        tree.push_node(
            "big.mp4",
            parent,
            4,
            NodeFlags::empty(),
            Category::Videos,
            1_000,
            1_000,
            0,
        );
        tree.rollup();
        tree
    }

    #[test]
    fn the_tree_nests_without_mangling_its_indent() {
        // The box-drawing prefix is multi-byte; an earlier version sliced it on
        // byte offsets and panicked partway down a real filesystem.
        let tree = nested_tree();
        let mut out = Vec::new();
        render_tree(&mut out, &tree, NodeId::ROOT, &Filter::permissive(), 5, 10).unwrap();
        let text = String::from_utf8(out).unwrap();

        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 4, "root plus three folders:\n{text}");
        assert!(lines[1].starts_with("└── alpha"), "{}", lines[1]);
        assert!(lines[2].starts_with("    └── beta"), "{}", lines[2]);
        assert!(lines[3].starts_with("        └── gamma"), "{}", lines[3]);
        // Files are not listed: the tree shows shape, not contents.
        assert!(!text.contains("big.mp4"));
    }

    #[test]
    fn the_tree_stops_at_the_requested_depth() {
        let tree = nested_tree();
        let mut out = Vec::new();
        render_tree(&mut out, &tree, NodeId::ROOT, &Filter::permissive(), 2, 10).unwrap();
        let text = String::from_utf8(out).unwrap();

        assert!(text.contains("alpha") && text.contains("beta"));
        assert!(
            !text.contains("gamma"),
            "depth 2 must not reach the third level"
        );
    }

    #[test]
    fn long_paths_keep_their_tail() {
        let short = shorten("/Users/me/Movies/raw/take-001.mov", 20);
        assert!(short.chars().count() <= 20, "{short}");
        assert!(short.contains("take-001.mov") || short.contains('…'));
    }
}
