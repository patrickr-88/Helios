/**
 * Inspector for the selected item.
 *
 * Everything shown here comes from metadata the scan already read — Helios
 * never opens a file to describe it. "Reveal in Finder" hands off to the OS,
 * which is the one place a user can act on what they found.
 */

import { CATEGORY_LABELS, formatDateTime, humanBytes, percent } from "../lib/format";
import type { Entry, ScanSummary } from "../lib/types";

interface Props {
  entry: Entry | null;
  summary: ScanSummary | null;
  onReveal: (path: string) => void;
}

export function DetailsPanel({ entry, summary, onReveal }: Props) {
  if (!entry) {
    return (
      <div className="details">
        <div className="empty">
          <p className="muted">Select an item to see its details.</p>
        </div>
      </div>
    );
  }

  const shareOfVolume =
    summary && summary.totalBytes > 0 ? entry.size / summary.totalBytes : null;
  // Logical vs. on-disk size diverges for sparse files, APFS clones and
  // compressed files — worth showing when it is more than rounding.
  const overhead = entry.physicalSize - entry.size;

  return (
    <div className="details">
      <h3>{entry.name}</h3>
      <div className="muted mono" style={{ wordBreak: "break-all", userSelect: "text" }}>
        {entry.path}
      </div>

      <button className="button" style={{ marginTop: 12 }} onClick={() => onReveal(entry.path)}>
        Reveal in Finder
      </button>

      <dl>
        <dt>Size</dt>
        <dd>{humanBytes(entry.size)}</dd>

        <dt>On disk</dt>
        <dd>
          {humanBytes(entry.physicalSize)}
          {Math.abs(overhead) > 65_536 && (
            <div className="tertiary">
              {overhead > 0 ? "+" : "−"}
              {humanBytes(Math.abs(overhead))} vs. logical
            </div>
          )}
        </dd>

        <dt>Kind</dt>
        <dd>
          {entry.isDir ? (entry.isPackage ? "Package" : "Folder") : CATEGORY_LABELS[entry.category]}
        </dd>

        {entry.isDir && (
          <>
            <dt>Contents</dt>
            <dd>
              {entry.fileCount.toLocaleString()} files
              <br />
              {entry.dirCount.toLocaleString()} folders
            </dd>
          </>
        )}

        <dt>Modified</dt>
        <dd>{formatDateTime(entry.mtime)}</dd>

        <dt>Share of parent</dt>
        <dd>{percent(entry.fractionOfParent)}</dd>

        {shareOfVolume != null && (
          <>
            <dt>Share of scan</dt>
            <dd>{percent(shareOfVolume, 2)}</dd>
          </>
        )}
      </dl>

      {(entry.isSymlink || entry.isHidden || entry.isSystem || !entry.isAccessible) && (
        <div style={{ display: "flex", gap: 5, flexWrap: "wrap", marginTop: 12 }}>
          {entry.isSymlink && <span className="badge">symbolic link</span>}
          {entry.isHidden && <span className="badge">hidden</span>}
          {entry.isSystem && <span className="badge">system</span>}
          {!entry.isAccessible && <span className="badge">partially readable</span>}
        </div>
      )}

      {entry.isSymlink && (
        <p className="tertiary" style={{ marginTop: 10, fontSize: 11 }}>
          Links are listed at their own size. The space their target uses is counted where the
          target actually lives, so nothing is double-counted.
        </p>
      )}

      {!entry.isAccessible && (
        <p className="tertiary" style={{ marginTop: 10, fontSize: 11 }}>
          macOS blocked part of this folder, so its total is a lower bound. Granting Helios Full
          Disk Access in System Settings → Privacy & Security lets a rescan see the rest.
        </p>
      )}
    </div>
  );
}
