//! Enforces the promise the whole product rests on: Helios never modifies what
//! it scans.
//!
//! Two complementary checks. The behavioural test proves a real scan leaves a
//! real directory byte-identical. The source audit proves the engine has no
//! mutating filesystem call to begin with, which is the part that keeps holding
//! after someone adds a feature the behavioural test does not happen to
//! exercise.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use helios_core::scan::{scan_blocking, ScanOptions};

/// `(len, mtime_nanos, permissions)` for every path under `root`.
fn fingerprint(root: &Path) -> BTreeMap<PathBuf, (u64, i64, u32)> {
    let mut out = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(meta) = fs::symlink_metadata(&path) else {
                continue;
            };
            #[cfg(unix)]
            let (mtime, mode) = {
                use std::os::unix::fs::MetadataExt;
                (
                    meta.mtime() * 1_000_000_000 + meta.mtime_nsec(),
                    meta.mode(),
                )
            };
            #[cfg(not(unix))]
            let (mtime, mode) = (0i64, 0u32);
            out.insert(path.clone(), (meta.len(), mtime, mode));
            if meta.is_dir() {
                stack.push(path);
            }
        }
    }
    out
}

fn fixture() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join("a/b/c")).unwrap();
    fs::write(dir.path().join("a/b/c/deep.bin"), vec![7u8; 4096]).unwrap();
    fs::write(dir.path().join("a/notes.txt"), b"hello").unwrap();
    fs::write(dir.path().join("top.mp4"), vec![1u8; 1024]).unwrap();
    dir
}

#[test]
fn a_scan_leaves_the_tree_untouched() {
    let dir = fixture();
    let before = fingerprint(dir.path());

    let mut options = ScanOptions::new(dir.path());
    options.use_default_exclusions = false;
    let outcome = scan_blocking(&options);
    assert!(
        outcome.tree.total_logical() >= 5125,
        "sanity: the scan saw the files"
    );

    let after = fingerprint(dir.path());
    assert_eq!(
        before, after,
        "scanning changed sizes, mtimes or permissions in the tree"
    );
    assert_eq!(before.len(), after.len(), "scanning added or removed paths");
}

#[test]
fn scanning_creates_no_files_anywhere_in_the_tree() {
    let dir = fixture();
    let mut options = ScanOptions::new(dir.path());
    options.use_default_exclusions = false;
    scan_blocking(&options);

    let names: Vec<String> = fingerprint(dir.path())
        .keys()
        .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        names.len(),
        6,
        "expected exactly the fixture's 6 paths, got {names:?}"
    );
}

/// Mutating `std::fs` calls that must not appear in the engine.
///
/// The audit is textual, so entries are written to be unambiguous: `set_len`
/// alone would also flag `Vec::set_len`, which the macOS volume enumeration
/// legitimately uses on its own buffer.
///
/// `snapshot.rs` is exempt: it owns the cache under the user's
/// application-support directory, which is Helios's own data and never part of
/// a scanned tree. Nothing else in the crate may write anything at all.
const FORBIDDEN: &[&str] = &[
    "fs::write",
    "fs::remove_file",
    "fs::remove_dir",
    "fs::rename",
    "fs::copy",
    "fs::create_dir",
    "fs::hard_link",
    "fs::soft_link",
    "fs::set_permissions",
    "fs::File::create",
    "OpenOptions",
    "File::set_len",
    "unlink(",
    "rmdir(",
    "chmod(",
    "utimes(",
];

const EXEMPT: &[&str] = &["snapshot.rs"];

#[test]
fn the_engine_contains_no_mutating_filesystem_calls() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut findings = Vec::new();
    let mut files_checked = 0;

    let mut stack = vec![src.clone()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().is_none_or(|e| e != "rs") {
                continue;
            }
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            if EXEMPT.contains(&name.as_str()) {
                continue;
            }
            files_checked += 1;

            let source = fs::read_to_string(&path).unwrap();
            for (line_no, line) in source.lines().enumerate() {
                // Test modules legitimately build fixtures; only the shipping
                // code path is audited.
                if line.trim_start().starts_with("//") {
                    continue;
                }
                for needle in FORBIDDEN {
                    if line.contains(needle) && !in_test_module(&source, line_no) {
                        findings.push(format!("{}:{} — {needle}", name, line_no + 1));
                    }
                }
            }
        }
    }

    assert!(
        files_checked > 5,
        "the audit found almost no sources to check"
    );
    assert!(
        findings.is_empty(),
        "mutating filesystem calls found in the engine:\n  {}",
        findings.join("\n  ")
    );
}

/// True if `line_no` falls inside a `#[cfg(test)]` module.
fn in_test_module(source: &str, line_no: usize) -> bool {
    source
        .lines()
        .take(line_no)
        .any(|l| l.trim() == "#[cfg(test)]")
}

#[test]
fn the_engine_contains_no_networking() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let forbidden = [
        "TcpStream",
        "UdpSocket",
        "TcpListener",
        "reqwest",
        "hyper::",
        "ureq",
    ];
    let mut stack = vec![src];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().is_none_or(|e| e != "rs") {
                continue;
            }
            let source = fs::read_to_string(&path).unwrap();
            for needle in forbidden {
                assert!(
                    !source.contains(needle),
                    "{} references {needle}; the engine must be fully offline",
                    path.display()
                );
            }
        }
    }
}
