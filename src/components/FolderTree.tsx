/**
 * Expandable folder hierarchy with a size on every row.
 *
 * Children are fetched per folder on first expand and cached, so opening a
 * volume's tree never costs more than the folders the user actually opened —
 * the alternative, shipping the whole hierarchy to the webview up front, is
 * exactly the thing this architecture exists to avoid.
 */

import { useEffect, useState } from "react";

import { api } from "../lib/api";
import { CATEGORY_COLORS, humanBytes, percent } from "../lib/format";
import type { Entry, Filter } from "../lib/types";

interface Props {
  scanId: string;
  rootId: number;
  filter: Filter;
  selectedId: number | null;
  onSelect: (entry: Entry) => void;
}

export function FolderTree({ scanId, rootId, filter, selectedId, onSelect }: Props) {
  const [children, setChildren] = useState<Record<number, Entry[]>>({});
  const [expanded, setExpanded] = useState<Set<number>>(new Set([rootId]));
  const [loading, setLoading] = useState<Set<number>>(new Set());

  // Filter or root changes invalidate everything that was cached.
  useEffect(() => {
    setChildren({});
    setExpanded(new Set([rootId]));
  }, [scanId, rootId, filter]);

  useEffect(() => {
    const missing = [...expanded].filter((id) => !(id in children) && !loading.has(id));
    if (missing.length === 0) return;

    setLoading((current) => new Set([...current, ...missing]));
    Promise.all(
      missing.map((id) =>
        api
          .listChildren(scanId, id, filter, "size", true, 2000)
          .then((rows) => [id, rows] as const)
          .catch(() => [id, [] as Entry[]] as const),
      ),
    ).then((results) => {
      setChildren((current) => {
        const next = { ...current };
        for (const [id, rows] of results) next[id] = rows;
        return next;
      });
      setLoading((current) => {
        const next = new Set(current);
        for (const [id] of results) next.delete(id);
        return next;
      });
    });
  }, [expanded, children, loading, scanId, filter]);

  const toggle = (id: number) => {
    setExpanded((current) => {
      const next = new Set(current);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  };

  const renderRows = (parentId: number, depth: number): JSX.Element[] => {
    const rows = children[parentId];
    if (!rows) {
      return loading.has(parentId)
        ? [
            <tr key={`loading-${parentId}`}>
              <td colSpan={4} style={{ paddingLeft: 14 + depth * 16 }} className="tertiary">
                Loading…
              </td>
            </tr>,
          ]
        : [];
    }

    return rows.flatMap((entry) => {
      const isOpen = expanded.has(entry.id);
      const row = (
        <tr
          key={entry.id}
          className={entry.id === selectedId ? "selected" : ""}
          onClick={() => onSelect(entry)}
          onDoubleClick={() => entry.isDir && toggle(entry.id)}
        >
          <td>
            <div className="name-cell" style={{ paddingLeft: depth * 16 }}>
              {entry.isDir ? (
                <button
                  className="tertiary"
                  style={{ width: 14 }}
                  onClick={(event) => {
                    event.stopPropagation();
                    toggle(entry.id);
                  }}
                  aria-label={isOpen ? "Collapse" : "Expand"}
                >
                  {isOpen ? "▾" : "▸"}
                </button>
              ) : (
                <span style={{ width: 14 }} />
              )}
              <span className="swatch" style={{ background: CATEGORY_COLORS[entry.category] }} />
              <span className="name">{entry.name}</span>
              {!entry.isAccessible && <span className="badge">partial</span>}
            </div>
          </td>
          <td className="right">{humanBytes(entry.size)}</td>
          <td className="right" style={{ width: 120 }}>
            <div className="bar">
              <span
                style={{
                  width: `${Math.min(100, entry.fractionOfParent * 100)}%`,
                  background: CATEGORY_COLORS[entry.category],
                }}
              />
            </div>
          </td>
          <td className="right muted" style={{ width: 64 }}>
            {percent(entry.fractionOfParent, 0)}
          </td>
        </tr>
      );

      return isOpen && entry.isDir ? [row, ...renderRows(entry.id, depth + 1)] : [row];
    });
  };

  return (
    <div style={{ height: "100%", overflow: "auto" }}>
      <table className="rows">
        <thead>
          <tr>
            <th>Folder</th>
            <th className="right">Size</th>
            <th className="right">Share of parent</th>
            <th className="right" />
          </tr>
        </thead>
        <tbody>{renderRows(rootId, 0)}</tbody>
      </table>
    </div>
  );
}
