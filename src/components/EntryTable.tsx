/**
 * The shared table used by the Folder, Largest and Search views.
 *
 * Rows are windowed: only the slice currently on screen is rendered, with
 * spacer rows standing in for everything above and below. Every row is a fixed
 * height (`--row-height`), which is what makes the arithmetic exact and lets a
 * 100,000-row result set scroll at the same cost as a 20-row one — without
 * pulling in a virtualization library.
 */

import { useMemo, useRef, useState } from "react";

import { CATEGORY_COLORS, CATEGORY_LABELS, formatDate, humanBytes, percent } from "../lib/format";
import type { Entry, SortKey } from "../lib/types";

interface Props {
  entries: Entry[];
  selectedId: number | null;
  onSelect: (entry: Entry) => void;
  onOpen?: (entry: Entry) => void;
  /** Show a rank column (used by the top-100 lists). */
  ranked?: boolean;
  /** Show the full path instead of just the name. */
  showPath?: boolean;
  sort?: SortKey;
  descending?: boolean;
  onSortChange?: (sort: SortKey) => void;
  emptyMessage?: string;
}

const ROW_HEIGHT = 28;
const OVERSCAN = 12;

export function EntryTable({
  entries,
  selectedId,
  onSelect,
  onOpen,
  ranked = false,
  showPath = false,
  sort = "size",
  descending = true,
  onSortChange,
  emptyMessage = "Nothing matches these filters.",
}: Props) {
  const scrollRef = useRef<HTMLDivElement>(null);
  const [scrollTop, setScrollTop] = useState(0);
  const [viewportHeight, setViewportHeight] = useState(600);

  const { start, end } = useMemo(() => {
    const first = Math.max(0, Math.floor(scrollTop / ROW_HEIGHT) - OVERSCAN);
    const visible = Math.ceil(viewportHeight / ROW_HEIGHT) + OVERSCAN * 2;
    return { start: first, end: Math.min(entries.length, first + visible) };
  }, [scrollTop, viewportHeight, entries.length]);

  const header = (key: SortKey, label: string, right = false) => (
    <th
      className={`${right ? "right " : ""}${onSortChange ? "sortable" : ""}`}
      onClick={onSortChange ? () => onSortChange(key) : undefined}
    >
      {label}
      {sort === key && <span className="tertiary"> {descending ? "▾" : "▴"}</span>}
    </th>
  );

  if (entries.length === 0) {
    return (
      <div className="empty">
        <h2>No results</h2>
        <p className="muted">{emptyMessage}</p>
      </div>
    );
  }

  return (
    <div
      ref={scrollRef}
      style={{ height: "100%", overflow: "auto" }}
      onScroll={(event) => {
        setScrollTop(event.currentTarget.scrollTop);
        setViewportHeight(event.currentTarget.clientHeight);
      }}
    >
      <table className="rows">
        <thead>
          <tr>
            {ranked && <th style={{ width: 42 }}>#</th>}
            {header("name", showPath ? "Path" : "Name")}
            {header("size", "Size", true)}
            <th className="right" style={{ width: 110 }}>
              Share
            </th>
            <th style={{ width: 110 }}>Kind</th>
            {header("modified", "Modified", true)}
          </tr>
        </thead>
        <tbody>
          {start > 0 && <tr style={{ height: start * ROW_HEIGHT }} aria-hidden />}
          {entries.slice(start, end).map((entry, index) => (
            <tr
              key={entry.id}
              className={entry.id === selectedId ? "selected" : ""}
              onClick={() => onSelect(entry)}
              onDoubleClick={() => onOpen?.(entry)}
            >
              {ranked && <td className="tertiary">{start + index + 1}</td>}
              <td>
                <div className="name-cell">
                  <span
                    className="swatch"
                    style={{ background: CATEGORY_COLORS[entry.category] }}
                    title={CATEGORY_LABELS[entry.category]}
                  />
                  <span className="name">
                    {entry.isDir ? "📁 " : ""}
                    {showPath ? entry.path : entry.name}
                  </span>
                  {!entry.isAccessible && (
                    <span className="badge" title="Helios could not read inside this folder">
                      partial
                    </span>
                  )}
                  {entry.isSymlink && <span className="badge">link</span>}
                </div>
              </td>
              <td className="right">{humanBytes(entry.size)}</td>
              <td className="right">
                <div className="bar" title={percent(entry.fractionOfParent)}>
                  <span
                    style={{
                      width: `${Math.min(100, entry.fractionOfParent * 100)}%`,
                      background: CATEGORY_COLORS[entry.category],
                    }}
                  />
                </div>
              </td>
              <td className="muted">
                {entry.isDir
                  ? `${entry.fileCount.toLocaleString()} items`
                  : CATEGORY_LABELS[entry.category]}
              </td>
              <td className="right muted">{formatDate(entry.mtime)}</td>
            </tr>
          ))}
          {end < entries.length && (
            <tr style={{ height: (entries.length - end) * ROW_HEIGHT }} aria-hidden />
          )}
        </tbody>
      </table>
    </div>
  );
}
