//! `helios` — headless front end for the scan engine.
//!
//! Two jobs: it makes the engine usable and benchmarkable without a GUI (which
//! is how the scanner is profiled and how CI exercises it on real trees), and
//! it gives scripted users the same reports the app exports.
//!
//! Argument parsing is hand-rolled. `clap` is excellent and this binary needs
//! six flags — not a derive macro, a builder DSL and a 300 KB dependency tree.

use std::io::{IsTerminal, Write};
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use helios_core::fmt::{human_bytes, human_duration};
use helios_core::query::{self, Filter, SortKey};
use helios_core::report;
use helios_core::scan::{scan, ScanControl, ScanOptions, ScanState};
use helios_core::snapshot::{self, Snapshot, SnapshotMeta};
use helios_core::{platform, Category};

const USAGE: &str = "\
helios — read-only disk usage analysis

USAGE:
    helios volumes
    helios scan <path> [options]
    helios report <path> --format <csv|json|pdf> [--out <file>] [options]
    helios snapshots

OPTIONS:
    --top <n>          Entries in each top-N list          [default: 20]
    --threads <n>      Scan worker threads                 [default: cores, max 8]
    --depth <n>        Stop descending below this depth
    --format <fmt>     Report format: csv, json, pdf       [default: csv]
    --out <file>       Write the report here instead of stdout
    --exclude <path>   Skip a path (repeatable)
    --no-hidden        Skip hidden files entirely
    --no-progress      Suppress the progress line
    --cache            Reuse and update the snapshot cache for incremental rescans
    -h, --help         Show this help

Helios never modifies the filesystem it scans.
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() || args.iter().any(|a| a == "-h" || a == "--help") {
        print!("{USAGE}");
        return ExitCode::SUCCESS;
    }

    let result = match args[0].as_str() {
        "volumes" => cmd_volumes(),
        "snapshots" => cmd_snapshots(),
        "scan" => cmd_scan(&args[1..], false),
        "report" => cmd_scan(&args[1..], true),
        other => Err(format!("unknown command '{other}'\n\n{USAGE}")),
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("helios: {message}");
            ExitCode::FAILURE
        }
    }
}

#[derive(Default)]
struct Args {
    path: Option<PathBuf>,
    top: usize,
    threads: Option<usize>,
    depth: Option<u16>,
    format: String,
    out: Option<PathBuf>,
    excludes: Vec<PathBuf>,
    hidden: bool,
    progress: bool,
    cache: bool,
}

fn parse_args(argv: &[String]) -> Result<Args, String> {
    let mut args = Args {
        top: 20,
        format: "csv".into(),
        hidden: true,
        progress: true,
        ..Args::default()
    };
    let mut i = 0;
    while i < argv.len() {
        let arg = argv[i].as_str();
        let mut value = |name: &str| -> Result<String, String> {
            argv.get(i + 1)
                .cloned()
                .ok_or_else(|| format!("{name} needs a value"))
        };
        match arg {
            "--top" => {
                args.top = value("--top")?.parse().map_err(|_| "--top must be a number")?;
                i += 1;
            }
            "--threads" => {
                args.threads =
                    Some(value("--threads")?.parse().map_err(|_| "--threads must be a number")?);
                i += 1;
            }
            "--depth" => {
                args.depth = Some(value("--depth")?.parse().map_err(|_| "--depth must be a number")?);
                i += 1;
            }
            "--format" => {
                args.format = value("--format")?;
                i += 1;
            }
            "--out" => {
                args.out = Some(PathBuf::from(value("--out")?));
                i += 1;
            }
            "--exclude" => {
                args.excludes.push(PathBuf::from(value("--exclude")?));
                i += 1;
            }
            "--no-hidden" => args.hidden = false,
            "--no-progress" => args.progress = false,
            "--cache" => args.cache = true,
            other if other.starts_with('-') => return Err(format!("unknown option '{other}'")),
            other => args.path = Some(PathBuf::from(other)),
        }
        i += 1;
    }
    Ok(args)
}

fn cmd_volumes() -> Result<(), String> {
    let volumes = platform::volumes();
    println!(
        "{:<24} {:<20} {:>10} {:>10} {:>10}  {}",
        "VOLUME", "MOUNT", "SIZE", "USED", "FREE", "TYPE"
    );
    for v in &volumes {
        let mut kind = v.filesystem.clone();
        if v.is_network {
            kind.push_str(" (network)");
        }
        if v.is_removable {
            kind.push_str(" (removable)");
        }
        if v.is_read_only {
            kind.push_str(" (read-only)");
        }
        println!(
            "{:<24} {:<20} {:>10} {:>10} {:>10}  {}",
            truncate(&v.name, 24),
            truncate(&v.mount_point.to_string_lossy(), 20),
            human_bytes(v.total_bytes),
            human_bytes(v.used_bytes),
            human_bytes(v.free_bytes),
            kind
        );
    }
    if volumes.is_empty() {
        println!("(no volumes reported)");
    }
    Ok(())
}

fn cmd_snapshots() -> Result<(), String> {
    let snapshots = snapshot::list();
    if snapshots.is_empty() {
        println!("No cached snapshots in {}", snapshot::cache_dir().display());
        return Ok(());
    }
    for meta in snapshots {
        println!(
            "{:<40} {:>12} scanned {}",
            meta.root_path.to_string_lossy(),
            human_bytes(meta.stats.bytes_seen),
            helios_core::fmt::format_timestamp(meta.scanned_at)
        );
    }
    Ok(())
}

fn cmd_scan(argv: &[String], as_report: bool) -> Result<(), String> {
    let args = parse_args(argv)?;
    let path = args.path.clone().ok_or("a path to scan is required")?;
    let path = path
        .canonicalize()
        .map_err(|e| format!("{}: {e}", path.display()))?;

    let volume = platform::volumes()
        .into_iter()
        .filter(|v| path.starts_with(&v.mount_point))
        // The longest matching mount point is the volume the path is really on.
        .max_by_key(|v| v.mount_point.as_os_str().len());

    let mut options = ScanOptions::new(&path);
    options.expected_bytes = volume.as_ref().map(|v| v.used_bytes);
    options.skip_hidden = !args.hidden;
    options.max_depth = args.depth;
    options.exclusions = args.excludes.clone();
    if let Some(threads) = args.threads {
        options.threads = threads;
    }
    if args.cache {
        if let Some(v) = &volume {
            if let Ok(previous) = snapshot::load(&v.id) {
                if previous.tree.root_path == path {
                    options.previous = Some(Arc::new(previous.tree));
                }
            }
        }
    }

    let control = ScanControl::new();
    install_interrupt_handler(control.clone());

    let show_progress = args.progress && std::io::stderr().is_terminal();
    let mut last_len = 0usize;
    let outcome = scan(&options, control, |p| {
        if !show_progress {
            return;
        }
        let eta = p
            .eta_ms
            .map(|ms| format!(", ~{} left", human_duration(ms)))
            .unwrap_or_default();
        let line = format!(
            "  scanning {} files, {} in {} folders{}  {}",
            p.files_seen,
            human_bytes(p.bytes_seen),
            p.dirs_seen,
            eta,
            truncate(&p.current_path, 48)
        );
        // Overwrite in place; pad to erase the previous, longer line.
        eprint!("\r{line:<last_len$}");
        last_len = line.len().max(last_len);
        let _ = std::io::stderr().flush();
    });
    if show_progress {
        eprint!("\r{:<width$}\r", "", width = last_len);
    }

    if outcome.state == ScanState::Cancelled {
        eprintln!("scan cancelled — showing partial results");
    }

    let meta = SnapshotMeta {
        volume_id: volume.as_ref().map(|v| v.id.clone()).unwrap_or_default(),
        root_path: path.clone(),
        scanned_at: snapshot::now_unix(),
        stats: outcome.stats.clone(),
    };

    if args.cache && !meta.volume_id.is_empty() {
        let snap = Snapshot {
            meta: meta.clone(),
            tree: outcome.tree.clone(),
        };
        match snapshot::save(&snap) {
            Ok(p) => eprintln!("cached snapshot: {}", p.display()),
            Err(e) => eprintln!("warning: could not cache snapshot: {e}"),
        }
    }

    let filter = Filter::permissive();
    if as_report {
        let report = report::build(&outcome.tree, &meta, volume.as_ref(), &filter, args.top);
        let bytes = match args.format.as_str() {
            "csv" => report::to_csv(&report).into_bytes(),
            "json" => report::to_json(&report).into_bytes(),
            "pdf" => report::to_pdf(&report),
            other => return Err(format!("unknown format '{other}' (csv, json, pdf)")),
        };
        match &args.out {
            Some(out) => {
                std::fs::write(out, &bytes).map_err(|e| format!("{}: {e}", out.display()))?;
                eprintln!("wrote {} ({})", out.display(), human_bytes(bytes.len() as u64));
            }
            None if args.format == "pdf" => {
                return Err("--out is required for PDF output".into());
            }
            None => std::io::stdout().write_all(&bytes).map_err(|e| e.to_string())?,
        }
        return Ok(());
    }

    print_summary(&outcome.tree, &outcome.stats, volume.as_ref());
    print_table(
        "Largest files",
        &query::largest(&outcome.tree, &filter, args.top, false),
    );
    print_table(
        "Largest folders",
        &query::largest(&outcome.tree, &filter, args.top, true),
    );
    print_categories(&outcome.tree, &filter);

    if !outcome.tree.errors.is_empty() {
        println!(
            "\n{} path(s) could not be read; totals are a lower bound. First few:",
            outcome.tree.errors.len()
        );
        for error in outcome.tree.errors.iter().take(5) {
            println!("  {} — {}", error.path.display(), error.message);
        }
    }
    Ok(())
}

fn print_summary(
    tree: &helios_core::Tree,
    stats: &helios_core::scan::ScanStats,
    volume: Option<&platform::Volume>,
) {
    println!("\n{}", tree.root_path.display());
    println!(
        "  {} in {} files, {} folders",
        human_bytes(tree.total_logical()),
        stats.files_scanned,
        stats.dirs_scanned
    );
    println!(
        "  {} on disk · scanned in {} · {} of memory",
        human_bytes(tree.total_physical()),
        human_duration(stats.elapsed_ms),
        human_bytes(stats.memory_bytes)
    );
    if stats.dirs_reused > 0 {
        println!("  {} folders reused from the previous snapshot", stats.dirs_reused);
    }
    if let Some(v) = volume {
        let pct = if v.total_bytes > 0 {
            v.used_bytes as f64 / v.total_bytes as f64 * 100.0
        } else {
            0.0
        };
        println!(
            "  volume {}: {} of {} used ({pct:.0}%), {} free",
            v.name,
            human_bytes(v.used_bytes),
            human_bytes(v.total_bytes),
            human_bytes(v.free_bytes)
        );
    }
}

fn print_table(title: &str, entries: &[query::Entry]) {
    println!("\n{title}");
    if entries.is_empty() {
        println!("  (nothing to show)");
        return;
    }
    let width = entries.iter().map(|e| e.name.chars().count()).max().unwrap_or(0).min(52);
    for (i, e) in entries.iter().enumerate() {
        println!(
            "  {:>3}. {:<width$}  {:>10}  {}",
            i + 1,
            truncate(&e.name, width),
            human_bytes(e.size),
            dim(&e.path)
        );
    }
}

fn print_categories(tree: &helios_core::Tree, filter: &Filter) {
    println!("\nBy category");
    for c in query::category_breakdown(tree, filter) {
        if c.bytes == 0 {
            continue;
        }
        let filled = (c.fraction * 24.0).round() as usize;
        println!(
            "  {:<14} {:>10}  {:>5.1}%  {}{}",
            c.category.as_str(),
            human_bytes(c.bytes),
            c.fraction * 100.0,
            "█".repeat(filled),
            "·".repeat(24 - filled)
        );
    }
    let _ = Category::ALL;
}

/// Ctrl-C cancels the scan instead of killing the process, so partial results
/// are still printed. Installed with `sigaction` on Unix; on Windows the
/// default terminate behaviour applies until the console handler lands.
fn install_interrupt_handler(control: ScanControl) {
    #[cfg(unix)]
    {
        use std::sync::atomic::{AtomicBool, Ordering};
        static TRIGGERED: AtomicBool = AtomicBool::new(false);

        extern "C" fn handler(_: i32) {
            TRIGGERED.store(true, Ordering::Release);
        }
        // SAFETY: `handler` only performs an atomic store, which is
        // async-signal-safe.
        unsafe {
            libc_signal(handler);
        }
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

#[cfg(unix)]
unsafe fn libc_signal(handler: extern "C" fn(i32)) {
    extern "C" {
        fn signal(signum: i32, handler: usize) -> usize;
    }
    const SIGINT: i32 = 2;
    unsafe { signal(SIGINT, handler as usize) };
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let head: String = text.chars().take(max.saturating_sub(1)).collect();
    format!("{head}…")
}

fn dim(text: &str) -> String {
    if std::io::stdout().is_terminal() {
        format!("\x1b[2m{text}\x1b[0m")
    } else {
        text.to_string()
    }
}

fn _sort_key_is_reexported(_: SortKey) {}
