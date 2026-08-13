/**
 * The landing view: capacity, what the last scan found, and the biggest things
 * on the volume — the four questions someone opens a disk tool to answer,
 * answered before they click anything.
 */

import { useEffect, useState } from "react";

import { api } from "../lib/api";
import {
  CATEGORY_COLORS,
  CATEGORY_LABELS,
  humanBytes,
  humanDuration,
  percent,
  relativeTime,
} from "../lib/format";
import type { CategorySummary, Entry, ScanSummary } from "../lib/types";

interface Props {
  summary: ScanSummary;
  onOpenEntry: (entry: Entry) => void;
  onShowIssues: () => void;
}

export function Dashboard({ summary, onOpenEntry, onShowIssues }: Props) {
  const [folders, setFolders] = useState<Entry[]>([]);
  const [files, setFiles] = useState<Entry[]>([]);
  const [categories, setCategories] = useState<CategorySummary[]>([]);

  useEffect(() => {
    let cancelled = false;
    const apply = <T,>(setter: (value: T) => void) => (value: T) => {
      if (!cancelled) setter(value);
    };
    api.largest(summary.scanId, true, 6).then(apply(setFolders)).catch(() => {});
    api.largest(summary.scanId, false, 6).then(apply(setFiles)).catch(() => {});
    api.categories(summary.scanId).then(apply(setCategories)).catch(() => {});
    return () => {
      cancelled = true;
    };
  }, [summary.scanId, summary.scannedAt]);

  const volume = summary.volume;
  const usedFraction = volume && volume.total_bytes > 0 ? volume.used_bytes / volume.total_bytes : 0;
  const meterClass = usedFraction > 0.92 ? "critical" : usedFraction > 0.8 ? "warn" : "";
  // Scanned bytes below the OS's used figure means we could not see everything.
  const coverage =
    volume && volume.used_bytes > 0 ? summary.totalBytes / volume.used_bytes : null;

  return (
    <>
      {summary.errorCount > 0 && (
        <div className="notice">
          <span>
            {summary.errorCount.toLocaleString()} location
            {summary.errorCount === 1 ? "" : "s"} could not be read
            {coverage != null && coverage < 0.98
              ? ` — this scan covers about ${percent(coverage, 0)} of the volume's used space.`
              : "."}
          </span>
          <button className="button" onClick={onShowIssues}>
            Show
          </button>
        </div>
      )}

      <div className="cards">
        <div className="card">
          <h3>Capacity</h3>
          <div className="value">{humanBytes(volume?.total_bytes ?? summary.totalBytes)}</div>
          <div className="sub">
            {volume ? `${volume.filesystem.toUpperCase()} · ${volume.mount_point}` : summary.rootPath}
          </div>
        </div>

        <div className="card">
          <h3>Used</h3>
          <div className="value">{humanBytes(volume?.used_bytes ?? summary.totalBytes)}</div>
          <div className={`meter ${meterClass}`}>
            <span style={{ width: `${Math.min(100, usedFraction * 100)}%` }} />
          </div>
          <div className="sub">{percent(usedFraction, 0)} of capacity</div>
        </div>

        <div className="card">
          <h3>Free</h3>
          <div className="value">{humanBytes(volume?.free_bytes ?? 0)}</div>
          <div className="sub">
            {volume ? percent(1 - usedFraction, 0) : "—"} available
          </div>
        </div>

        <div className="card">
          <h3>Last scan</h3>
          <div className="value" style={{ fontSize: 17 }}>
            {relativeTime(summary.scannedAt)}
          </div>
          <div className="sub">
            {summary.fileCount.toLocaleString()} files ·{" "}
            {summary.dirCount.toLocaleString()} folders ·{" "}
            {humanDuration(summary.elapsedMs)}
            {summary.dirsReused > 0 && ` · ${summary.dirsReused.toLocaleString()} folders reused`}
            {summary.fromCache && " · from cache"}
          </div>
        </div>
      </div>

      <div className="panel">
        <header>
          Where the space went
          <span className="hint">by file category</span>
        </header>
        <div className="panel-body">
          {categories
            .filter((c) => c.bytes > 0)
            .map((c) => (
              <div
                key={c.category}
                style={{ display: "flex", alignItems: "center", gap: 10, marginBottom: 7 }}
              >
                <span style={{ width: 108 }}>{CATEGORY_LABELS[c.category]}</span>
                <div className="bar" style={{ flex: 1 }}>
                  <span
                    style={{
                      width: `${Math.max(1, c.fraction * 100)}%`,
                      background: CATEGORY_COLORS[c.category],
                    }}
                  />
                </div>
                <span style={{ width: 76, textAlign: "right" }}>{humanBytes(c.bytes)}</span>
                <span className="muted" style={{ width: 48, textAlign: "right" }}>
                  {percent(c.fraction, 0)}
                </span>
              </div>
            ))}
          {categories.length === 0 && <div className="muted">No files in this scan.</div>}
        </div>
      </div>

      <div style={{ display: "grid", gridTemplateColumns: "repeat(auto-fit, minmax(320px, 1fr))", gap: 16 }}>
        <TopList title="Largest folders" entries={folders} onOpen={onOpenEntry} />
        <TopList title="Largest files" entries={files} onOpen={onOpenEntry} />
      </div>
    </>
  );
}

function TopList({
  title,
  entries,
  onOpen,
}: {
  title: string;
  entries: Entry[];
  onOpen: (entry: Entry) => void;
}) {
  return (
    <div className="panel">
      <header>
        {title}
        <span className="hint">click to inspect</span>
      </header>
      <table className="rows">
        <tbody>
          {entries.map((entry) => (
            <tr key={entry.id} onClick={() => onOpen(entry)}>
              <td>
                <div className="name-cell">
                  <span className="swatch" style={{ background: CATEGORY_COLORS[entry.category] }} />
                  <span className="name" title={entry.path}>
                    {entry.name}
                  </span>
                </div>
              </td>
              <td className="right" style={{ width: 88 }}>
                {humanBytes(entry.size)}
              </td>
            </tr>
          ))}
          {entries.length === 0 && (
            <tr>
              <td className="muted">Nothing here yet.</td>
            </tr>
          )}
        </tbody>
      </table>
    </div>
  );
}
