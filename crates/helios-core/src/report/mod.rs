//! Read-only report generation: top-100 lists, storage summary, category
//! breakdown, exported as CSV, JSON or PDF.
//!
//! A [`Report`] is a plain data snapshot, built once and then rendered into
//! whichever format the user asked for. Keeping the render steps pure means
//! exporting three formats costs one tree traversal, and all three agree by
//! construction.

pub mod pdf;

use std::fmt::Write as _;

use serde::{Deserialize, Serialize};

use crate::category::Category;
use crate::fmt::{format_date, format_timestamp, human_bytes, human_duration};
use crate::model::Tree;
use crate::platform::Volume;
use crate::query::{self, CategorySummary, Entry, Filter};
use crate::scan::ScanStats;
use crate::snapshot::SnapshotMeta;

pub const DEFAULT_TOP_N: usize = 100;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReportSummary {
    pub root_path: String,
    pub scanned_at: i64,
    pub total_logical_bytes: u64,
    pub total_physical_bytes: u64,
    pub file_count: u32,
    pub dir_count: u32,
    /// From the OS, not the scan — present only when the root is a volume.
    pub volume_capacity_bytes: Option<u64>,
    pub volume_free_bytes: Option<u64>,
    pub volume_used_bytes: Option<u64>,
    /// Scanned bytes as a share of the volume's used bytes. Below 1.0 means the
    /// scan could not see everything (permissions, exclusions).
    pub coverage: Option<f32>,
    pub inaccessible_paths: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub generated_at: i64,
    pub summary: ReportSummary,
    pub stats: ScanStats,
    pub volume: Option<Volume>,
    pub largest_files: Vec<Entry>,
    pub largest_folders: Vec<Entry>,
    pub categories: Vec<CategorySummary>,
}

/// Builds a report from a scanned tree.
pub fn build(
    tree: &Tree,
    meta: &SnapshotMeta,
    volume: Option<&Volume>,
    filter: &Filter,
    top_n: usize,
) -> Report {
    let root = tree.node(crate::model::NodeId::ROOT);
    let coverage = volume.and_then(|v| {
        (v.used_bytes > 0).then(|| (tree.total_logical() as f64 / v.used_bytes as f64) as f32)
    });

    Report {
        generated_at: crate::snapshot::now_unix(),
        summary: ReportSummary {
            root_path: tree.root_path.to_string_lossy().into_owned(),
            scanned_at: meta.scanned_at,
            total_logical_bytes: tree.total_logical(),
            total_physical_bytes: tree.total_physical(),
            file_count: root.file_count,
            dir_count: root.dir_count,
            volume_capacity_bytes: volume.map(|v| v.total_bytes),
            volume_free_bytes: volume.map(|v| v.free_bytes),
            volume_used_bytes: volume.map(|v| v.used_bytes),
            coverage,
            inaccessible_paths: tree.errors.len() as u64,
        },
        stats: meta.stats.clone(),
        volume: volume.cloned(),
        largest_files: query::largest(tree, filter, top_n, false),
        largest_folders: query::largest(tree, filter, top_n, true),
        categories: query::category_breakdown(tree, filter),
    }
}

/// Machine-readable export. Pretty-printed because these files are read by
/// humans as often as by scripts.
pub fn to_json(report: &Report) -> String {
    serde_json::to_string_pretty(report).unwrap_or_else(|e| format!("{{\"error\":\"{e}\"}}"))
}

/// Spreadsheet export.
///
/// Emitted as several labelled sections in one file rather than one flat table:
/// a storage report is genuinely four different tables, and splitting them into
/// four downloads is worse for the user than one file with headers that Numbers
/// and Excel both open cleanly.
pub fn to_csv(report: &Report) -> String {
    let mut out = String::with_capacity(16 * 1024);
    let s = &report.summary;

    out.push_str("# Helios storage report\n");
    let _ = writeln!(out, "# Generated,{}", format_timestamp(report.generated_at));
    out.push_str("\nSection,Metric,Value\n");
    let rows: [(&str, String); 7] = [
        ("Root path", s.root_path.clone()),
        ("Scanned at", format_timestamp(s.scanned_at)),
        ("Total size (bytes)", s.total_logical_bytes.to_string()),
        ("On disk (bytes)", s.total_physical_bytes.to_string()),
        ("Files", s.file_count.to_string()),
        ("Folders", s.dir_count.to_string()),
        ("Inaccessible paths", s.inaccessible_paths.to_string()),
    ];
    for (metric, value) in rows {
        let _ = writeln!(out, "Summary,{},{}", csv_field(metric), csv_field(&value));
    }
    if let Some(volume) = &report.volume {
        for (metric, value) in [
            ("Volume", volume.name.clone()),
            ("Filesystem", volume.filesystem.clone()),
            ("Capacity (bytes)", volume.total_bytes.to_string()),
            ("Used (bytes)", volume.used_bytes.to_string()),
            ("Free (bytes)", volume.free_bytes.to_string()),
        ] {
            let _ = writeln!(out, "Volume,{},{}", csv_field(metric), csv_field(&value));
        }
    }

    out.push_str("\nCategory,Bytes,Files,Percent\n");
    for c in &report.categories {
        let _ = writeln!(
            out,
            "{},{},{},{:.2}",
            csv_field(c.category.as_str()),
            c.bytes,
            c.files,
            c.fraction * 100.0
        );
    }

    out.push_str("\nLargest files\nRank,Name,Path,Bytes,Human size,Category,Modified\n");
    write_entries(&mut out, &report.largest_files);

    out.push_str("\nLargest folders\nRank,Name,Path,Bytes,Human size,Files,Modified\n");
    for (i, e) in report.largest_folders.iter().enumerate() {
        let _ = writeln!(
            out,
            "{},{},{},{},{},{},{}",
            i + 1,
            csv_field(&e.name),
            csv_field(&e.path),
            e.size,
            csv_field(&human_bytes(e.size)),
            e.file_count,
            csv_field(&format_date(e.mtime))
        );
    }
    out
}

fn write_entries(out: &mut String, entries: &[Entry]) {
    for (i, e) in entries.iter().enumerate() {
        let _ = writeln!(
            out,
            "{},{},{},{},{},{},{}",
            i + 1,
            csv_field(&e.name),
            csv_field(&e.path),
            e.size,
            csv_field(&human_bytes(e.size)),
            csv_field(e.category.as_str()),
            csv_field(&format_date(e.mtime))
        );
    }
}

/// RFC 4180 quoting. Filenames legitimately contain commas, quotes and
/// newlines, and a report that corrupts a spreadsheet on those is worse than no
/// report.
fn csv_field(value: &str) -> String {
    if value.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

/// Colors for the category bars in the PDF.
fn category_color(category: Category) -> (f32, f32, f32) {
    match category {
        Category::Documents => (0.29, 0.51, 0.91),
        Category::Images => (0.36, 0.72, 0.45),
        Category::Videos => (0.85, 0.44, 0.32),
        Category::Audio => (0.61, 0.44, 0.86),
        Category::Archives => (0.90, 0.68, 0.25),
        Category::Applications => (0.27, 0.68, 0.76),
        Category::Developer => (0.45, 0.49, 0.58),
        Category::System => (0.55, 0.57, 0.62),
        Category::Other => (0.70, 0.72, 0.76),
    }
}

/// Printable export.
pub fn to_pdf(report: &Report) -> Vec<u8> {
    let mut doc = pdf::PdfBuilder::new("Helios Storage Report");
    let s = &report.summary;

    doc.heading("Helios Storage Report");
    doc.paragraph(&format!("Location: {}", s.root_path));
    doc.paragraph(&format!("Scanned: {}", format_timestamp(s.scanned_at)));
    doc.paragraph(&format!(
        "Generated: {}",
        format_timestamp(report.generated_at)
    ));

    doc.subheading("Summary");
    if let Some(volume) = &report.volume {
        doc.paragraph(&format!(
            "{} — {} of {} used ({} free), {}",
            volume.name,
            human_bytes(volume.used_bytes),
            human_bytes(volume.total_bytes),
            human_bytes(volume.free_bytes),
            volume.filesystem.to_uppercase()
        ));
    }
    doc.paragraph(&format!(
        "Scanned {} across {} files in {} folders ({} on disk).",
        human_bytes(s.total_logical_bytes),
        s.file_count,
        s.dir_count,
        human_bytes(s.total_physical_bytes)
    ));
    doc.paragraph(&format!(
        "Scan took {}; {} path(s) could not be read.",
        human_duration(report.stats.elapsed_ms),
        s.inaccessible_paths
    ));
    if let Some(coverage) = s.coverage {
        if coverage < 0.98 {
            doc.paragraph(&format!(
                "Coverage: {:.0}% of the volume's used bytes. The remainder is in \
                 locations this scan could not read.",
                coverage * 100.0
            ));
        }
    }

    doc.subheading("By category");
    for c in report.categories.iter().filter(|c| c.bytes > 0) {
        doc.row(
            &[
                (c.category.as_str().to_string(), 0.0, false),
                (human_bytes(c.bytes), 200.0, true),
                (format!("{:.1}%", c.fraction * 100.0), 250.0, true),
            ],
            false,
            false,
        );
        doc.bar(270.0, 220.0, c.fraction, category_color(c.category));
    }

    doc.spacer(1.0);
    doc.subheading(&format!("Top {} files", report.largest_files.len()));
    table(&mut doc, &report.largest_files, false);

    doc.page_break();
    doc.subheading(&format!("Top {} folders", report.largest_folders.len()));
    table(&mut doc, &report.largest_folders, true);

    doc.finish()
}

fn table(doc: &mut pdf::PdfBuilder, entries: &[Entry], folders: bool) {
    doc.row(
        &[
            ("#".to_string(), 0.0, false),
            ("Name".to_string(), 22.0, false),
            (
                if folders { "Files" } else { "Type" }.to_string(),
                360.0,
                true,
            ),
            ("Size".to_string(), 440.0, true),
            ("Modified".to_string(), 499.0, true),
        ],
        true,
        false,
    );
    for (i, e) in entries.iter().enumerate() {
        doc.row(
            &[
                (format!("{}", i + 1), 0.0, false),
                (truncate(&e.name, 52), 22.0, false),
                (
                    if folders {
                        e.file_count.to_string()
                    } else {
                        e.category.as_str().to_string()
                    },
                    360.0,
                    true,
                ),
                (human_bytes(e.size), 440.0, true),
                (format_date(e.mtime), 499.0, true),
            ],
            false,
            i % 2 == 1,
        );
    }
}

fn truncate(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    // Keep the tail: extensions and version numbers are the identifying part of
    // a long filename.
    let keep_head = max_chars.saturating_sub(12);
    let head: String = text.chars().take(keep_head).collect();
    let tail: String = text
        .chars()
        .skip(text.chars().count().saturating_sub(9))
        .collect();
    format!("{head}…{tail}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::category::Category;
    use crate::model::{NodeFlags, NodeId};
    use crate::snapshot::SnapshotMeta;

    fn fixture() -> (Tree, SnapshotMeta, Volume) {
        let mut tree = Tree::new("/Volumes/Test");
        let movies = tree.push_node(
            "Movies",
            NodeId::ROOT,
            1,
            NodeFlags::DIRECTORY,
            Category::Other,
            0,
            0,
            1_700_000_000,
        );
        tree.push_node(
            "holiday, final.mp4",
            movies,
            2,
            NodeFlags::empty(),
            Category::Videos,
            8_000_000_000,
            8_000_000_000,
            1_700_000_000,
        );
        tree.push_node(
            "notes.txt",
            NodeId::ROOT,
            1,
            NodeFlags::empty(),
            Category::Documents,
            2_000,
            4_096,
            1_700_000_000,
        );
        tree.rollup();

        let mut meta = SnapshotMeta::new(
            None,
            "/Volumes/Test".into(),
            ScanStats {
                elapsed_ms: 4_200,
                ..ScanStats::default()
            },
        );
        meta.volume_id = "/dev/disk1s1".into();
        meta.scanned_at = 1_700_000_500;
        let volume = Volume {
            id: "/dev/disk1s1".into(),
            name: "Test".into(),
            mount_point: "/Volumes/Test".into(),
            filesystem: "apfs".into(),
            total_bytes: 20_000_000_000,
            free_bytes: 11_000_000_000,
            used_bytes: 9_000_000_000,
            is_removable: false,
            is_network: false,
            is_read_only: false,
            is_root: false,
        };
        (tree, meta, volume)
    }

    fn report() -> Report {
        let (tree, meta, volume) = fixture();
        build(&tree, &meta, Some(&volume), &Filter::permissive(), 10)
    }

    #[test]
    fn summarizes_the_scan_against_the_volume() {
        let r = report();
        assert_eq!(r.summary.total_logical_bytes, 8_000_002_000);
        assert_eq!(r.summary.file_count, 2);
        assert_eq!(r.summary.dir_count, 1);
        assert_eq!(r.summary.volume_free_bytes, Some(11_000_000_000));
        assert!((r.summary.coverage.unwrap() - 0.888).abs() < 0.01);
        assert_eq!(r.largest_files[0].name, "holiday, final.mp4");
        assert_eq!(r.largest_folders[0].name, "Movies");
    }

    #[test]
    fn csv_quotes_fields_containing_commas() {
        let csv = to_csv(&report());
        assert!(
            csv.contains("\"holiday, final.mp4\""),
            "comma must be quoted"
        );
        assert!(csv.contains("Category,Bytes,Files,Percent"));
        assert!(csv.contains("Largest folders"));
        assert_eq!(csv_field("say \"hi\""), "\"say \"\"hi\"\"\"");
    }

    #[test]
    fn json_round_trips() {
        let json = to_json(&report());
        let parsed: Report = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.summary.total_logical_bytes, 8_000_002_000);
        assert_eq!(parsed.categories.len(), Category::ALL.len());
        assert_eq!(parsed.volume.unwrap().filesystem, "apfs");
    }

    #[test]
    fn pdf_is_well_formed_and_contains_the_data() {
        let bytes = to_pdf(&report());
        assert!(bytes.starts_with(b"%PDF-1.4"));
        let text = String::from_utf8_lossy(&bytes);
        assert!(text.contains("Helios Storage Report"));
        assert!(text.contains("holiday, final.mp4"));
        assert!(text.contains("8.0 GB"));
    }

    #[test]
    fn long_names_keep_their_tail() {
        let name = format!("{}-final-v2.mp4", "a".repeat(80));
        let short = truncate(&name, 40);
        assert!(short.chars().count() <= 40);
        assert!(short.ends_with("final-v2.mp4".get(3..).unwrap()));
    }
}
