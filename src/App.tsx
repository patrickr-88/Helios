/**
 * Application shell: volume selection, scan lifecycle, view switching.
 *
 * All view state lives here and flows down as props. The app has one meaningful
 * piece of async state — the scan — and a handful of derived lists, which is
 * comfortably below the threshold where a state library earns its keep.
 */

import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { Dashboard } from "./components/Dashboard";
import { DetailsPanel } from "./components/DetailsPanel";
import { EntryTable } from "./components/EntryTable";
import { FilterBar } from "./components/FilterBar";
import { FolderTree } from "./components/FolderTree";
import { ReportsPanel } from "./components/ReportsPanel";
import { ScanBar } from "./components/ScanBar";
import { Sidebar } from "./components/Sidebar";
import { Treemap } from "./components/Treemap";
import { api, chooseFolder, EVENTS, isTauri, listen } from "./lib/api";
import { CATEGORY_COLORS, CATEGORY_LABELS, humanBytes, percent } from "./lib/format";
import type {
  CategorySummary,
  Entry,
  Filter,
  ScanProgress,
  ScanSummary,
  SortKey,
  Volume,
} from "./lib/types";

type View = "dashboard" | "treemap" | "folders" | "largest" | "categories" | "reports";

const VIEWS: Array<{ id: View; label: string }> = [
  { id: "dashboard", label: "Dashboard" },
  { id: "treemap", label: "Treemap" },
  { id: "folders", label: "Folders" },
  { id: "largest", label: "Largest" },
  { id: "categories", label: "Categories" },
  { id: "reports", label: "Reports" },
];

const DEFAULT_FILTER: Filter = {
  includeHidden: true,
  includeSystem: true,
  categories: [],
  extensions: [],
};

export default function App() {
  const [volumes, setVolumes] = useState<Volume[]>([]);
  const [volumeId, setVolumeId] = useState<string | null>(null);
  const [summary, setSummary] = useState<ScanSummary | null>(null);
  const [progress, setProgress] = useState<ScanProgress | null>(null);
  const [paused, setPaused] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const [view, setView] = useState<View>("dashboard");
  const [filter, setFilter] = useState<Filter>(DEFAULT_FILTER);
  const [query, setQuery] = useState("");
  const [debouncedQuery, setDebouncedQuery] = useState("");
  const [sort, setSort] = useState<SortKey>("size");
  const [descending, setDescending] = useState(true);

  const [rootId, setRootId] = useState(0);
  const [crumbs, setCrumbs] = useState<Entry[]>([]);
  const [selected, setSelected] = useState<Entry | null>(null);
  const [rows, setRows] = useState<Entry[]>([]);
  const [categories, setCategories] = useState<CategorySummary[]>([]);
  const [showDirs, setShowDirs] = useState(false);
  const scanning = progress != null;

  // Volumes, plus any snapshot we can show before the user scans anything.
  useEffect(() => {
    api.listVolumes().then(setVolumes).catch((e) => setError(String(e)));
    api
      .listSnapshots()
      .then(async (snapshots) => {
        const newest = snapshots[0];
        if (!newest) return;
        const restored = await api.loadSnapshot(newest.volumeId);
        setSummary(restored);
        setVolumeId(restored.scanId);
      })
      .catch(() => {
        /* No cache yet — the dashboard's empty state covers this. */
      });
  }, []);

  useEffect(() => {
    const unsubscribers: Array<() => void> = [];
    listen<ScanProgress>(EVENTS.progress, setProgress).then((u) => unsubscribers.push(u));
    listen<ScanSummary>(EVENTS.finished, (finished) => {
      setProgress(null);
      setPaused(false);
      setSummary(finished);
      setVolumeId(finished.scanId);
      setRootId(0);
      setSelected(null);
    }).then((u) => unsubscribers.push(u));
    listen<string>(EVENTS.failed, (message) => {
      setProgress(null);
      setError(message);
    }).then((u) => unsubscribers.push(u));

    return () => unsubscribers.forEach((unsubscribe) => unsubscribe());
  }, []);

  // Debounce the search box: a keystroke should not start a tree-wide query.
  useEffect(() => {
    const timer = window.setTimeout(() => setDebouncedQuery(query), 220);
    return () => window.clearTimeout(timer);
  }, [query]);

  const activeFilter = useMemo<Filter>(
    () => ({
      ...filter,
      // A query with a separator is a path search; otherwise match names.
      nameContains: debouncedQuery && !debouncedQuery.includes("/") ? debouncedQuery : null,
      pathContains: debouncedQuery.includes("/") ? debouncedQuery : null,
    }),
    [filter, debouncedQuery],
  );

  const searching = debouncedQuery.trim().length > 0;
  const scanId = summary?.scanId ?? null;

  // Rows for whichever list view is on screen.
  useEffect(() => {
    if (!scanId) return;
    let cancelled = false;
    const receive = (result: Entry[]) => {
      if (!cancelled) setRows(result);
    };

    if (searching) {
      api.search(scanId, activeFilter, sort, 1000).then(receive).catch(() => receive([]));
    } else if (view === "largest") {
      api.largest(scanId, showDirs, 100, activeFilter).then(receive).catch(() => receive([]));
    } else if (view === "categories") {
      api
        .categories(scanId, activeFilter)
        .then((result) => !cancelled && setCategories(result))
        .catch(() => !cancelled && setCategories([]));
    }
    return () => {
      cancelled = true;
    };
  }, [scanId, view, showDirs, activeFilter, sort, searching, summary?.scannedAt]);

  // Breadcrumbs follow the drill-down root.
  useEffect(() => {
    if (!scanId) return;
    api
      .ancestors(scanId, rootId)
      .then(setCrumbs)
      .catch(() => setCrumbs([]));
  }, [scanId, rootId]);

  const startScan = useCallback(
    async (path: string, incremental: boolean) => {
      setError(null);
      setSelected(null);
      try {
        await api.startScan({ path, incremental, skipHidden: false });
        setProgress({
          state: "running",
          files_seen: 0,
          dirs_seen: 0,
          bytes_seen: 0,
          dirs_reused: 0,
          errors: 0,
          current_path: path,
          elapsed_ms: 0,
          eta_ms: null,
          fraction: null,
        });
      } catch (e) {
        setError(String(e));
      }
    },
    [],
  );

  const selectVolume = useCallback(
    async (volume: Volume) => {
      setVolumeId(volume.id);
      setRootId(0);
      setSelected(null);
      // Show the cached scan instantly if there is one; otherwise scan.
      try {
        setSummary(await api.loadSnapshot(volume.id));
      } catch {
        await startScan(volume.mount_point, false);
      }
    },
    [startScan],
  );

  const openEntry = useCallback((entry: Entry) => {
    setSelected(entry);
    if (entry.isDir) {
      setRootId(entry.id);
      setView((current) => (current === "dashboard" ? "treemap" : current));
    }
  }, []);

  const currentVolume = volumes.find((v) => v.id === volumeId) ?? null;
  const lastScanRoot = useRef<string | null>(null);
  useEffect(() => {
    lastScanRoot.current = summary?.rootPath ?? currentVolume?.mount_point ?? null;
  }, [summary, currentVolume]);

  return (
    <div className={`app${selected ? " with-details" : ""}`}>
      <Sidebar
        volumes={volumes}
        selectedId={volumeId}
        onSelect={selectVolume}
        busy={scanning}
        onScanFolder={async () => {
          const folder = await chooseFolder();
          if (folder) await startScan(folder, false);
        }}
      />

      <main className="main">
        <div className="titlebar">
          <div className="tabs">
            {VIEWS.map((item) => (
              <button
                key={item.id}
                className={`tab${view === item.id ? " active" : ""}`}
                onClick={() => setView(item.id)}
                disabled={!summary}
              >
                {item.label}
              </button>
            ))}
          </div>
          <span className="spacer" />
          {summary && (
            <span className="muted">
              {humanBytes(summary.totalBytes)} · {summary.fileCount.toLocaleString()} files
            </span>
          )}
          <button
            className="button"
            disabled={scanning || !lastScanRoot.current}
            onClick={() => lastScanRoot.current && startScan(lastScanRoot.current, true)}
            title="Reuses unchanged folders from the last scan"
          >
            Rescan
          </button>
          <button
            className="button primary"
            disabled={scanning || !lastScanRoot.current}
            onClick={() => lastScanRoot.current && startScan(lastScanRoot.current, false)}
          >
            Full scan
          </button>
        </div>

        {progress && (
          <ScanBar
            progress={progress}
            paused={paused}
            onPause={() => {
              api.pauseScan().catch(() => {});
              setPaused(true);
            }}
            onResume={() => {
              api.resumeScan().catch(() => {});
              setPaused(false);
            }}
            onCancel={() => {
              api.cancelScan().catch(() => {});
              setPaused(false);
            }}
          />
        )}

        {summary && view !== "dashboard" && view !== "reports" && (
          <>
            {crumbs.length > 1 && (
              <nav className="breadcrumbs">
                {crumbs.map((crumb, index) => (
                  <span key={crumb.id}>
                    {index > 0 && <span className="crumb-sep">›</span>}
                    <button
                      className={`crumb${index === crumbs.length - 1 ? " current" : ""}`}
                      onClick={() => setRootId(crumb.id)}
                    >
                      {index === 0 ? summary.volume?.name ?? crumb.name : crumb.name}
                    </button>
                  </span>
                ))}
              </nav>
            )}
            <FilterBar
              filter={filter}
              onChange={setFilter}
              query={query}
              onQueryChange={setQuery}
            />
          </>
        )}

        {error && (
          <div className="notice error" style={{ margin: 14 }}>
            <span>{error}</span>
            <button className="button" onClick={() => setError(null)}>
              Dismiss
            </button>
          </div>
        )}

        {!summary ? (
          <div className="content">
            <div className="empty">
              <h2>Pick a volume to see where your storage went</h2>
              <p className="muted">
                Helios reads your disks and never changes them: nothing is moved, renamed or
                deleted, and nothing leaves your Mac.
              </p>
              {!isTauri() && (
                <p className="tertiary">
                  Running in the browser, so this is a synthetic demo volume.
                </p>
              )}
            </div>
          </div>
        ) : searching && view !== "reports" ? (
          <div className="content flush">
            <EntryTable
              entries={rows}
              selectedId={selected?.id ?? null}
              onSelect={setSelected}
              onOpen={openEntry}
              showPath
              sort={sort}
              descending={descending}
              onSortChange={(key) => {
                setSort(key);
                setDescending((d) => (key === sort ? !d : true));
              }}
              emptyMessage={`Nothing matches “${debouncedQuery}”.`}
            />
          </div>
        ) : (
          renderView()
        )}
      </main>

      {selected && (
        <DetailsPanel
          entry={selected}
          summary={summary}
          onReveal={(path) => api.revealInFileManager(path).catch((e) => setError(String(e)))}
        />
      )}
    </div>
  );

  function renderView() {
    if (!summary || !scanId) return null;

    switch (view) {
      case "dashboard":
        return (
          <div className="content">
            <Dashboard
              summary={summary}
              onOpenEntry={openEntry}
              onShowIssues={() => setView("reports")}
            />
          </div>
        );

      case "treemap":
        return (
          <div className="content flush" style={{ display: "flex", flexDirection: "column" }}>
            <Treemap
              scanId={scanId}
              nodeId={rootId}
              includeHidden={filter.includeHidden ?? true}
              selectedId={selected?.id ?? null}
              onSelect={(tile) =>
                setSelected(
                  tile
                    ? {
                        id: tile.id,
                        name: tile.name,
                        path: `${summary.rootPath.replace(/\/$/, "")}/…/${tile.name}`,
                        size: tile.size,
                        physicalSize: tile.size,
                        category: tile.category,
                        isDir: tile.isDir,
                        isSymlink: false,
                        isHidden: false,
                        isSystem: false,
                        isPackage: false,
                        isAccessible: true,
                        mtime: 0,
                        fileCount: 0,
                        dirCount: 0,
                        fractionOfParent: summary.totalBytes ? tile.size / summary.totalBytes : 0,
                      }
                    : null,
                )
              }
              onOpen={(tile) => setRootId(tile.id)}
            />
            <div className="legend">
              {Object.entries(CATEGORY_LABELS).map(([key, label]) => (
                <span className="item" key={key}>
                  <span
                    className="swatch"
                    style={{ background: CATEGORY_COLORS[key as keyof typeof CATEGORY_COLORS] }}
                  />
                  {label}
                </span>
              ))}
            </div>
          </div>
        );

      case "folders":
        return (
          <div className="content flush">
            <FolderTree
              scanId={scanId}
              rootId={rootId}
              filter={activeFilter}
              selectedId={selected?.id ?? null}
              onSelect={setSelected}
            />
          </div>
        );

      case "largest":
        return (
          <div className="content flush" style={{ display: "flex", flexDirection: "column" }}>
            <div className="filterbar" style={{ borderBottom: "1px solid var(--separator)" }}>
              <button
                className={`chip${!showDirs ? " on" : ""}`}
                onClick={() => setShowDirs(false)}
              >
                Files
              </button>
              <button className={`chip${showDirs ? " on" : ""}`} onClick={() => setShowDirs(true)}>
                Folders
              </button>
              <span className="muted">Top 100 by size</span>
            </div>
            <div style={{ flex: 1, minHeight: 0 }}>
              <EntryTable
                entries={rows}
                selectedId={selected?.id ?? null}
                onSelect={setSelected}
                onOpen={openEntry}
                ranked
                showPath
              />
            </div>
          </div>
        );

      case "categories":
        return (
          <div className="content">
            <div className="panel">
              <header>
                Storage by category
                <span className="hint">files only — folders are counted through their contents</span>
              </header>
              <table className="rows">
                <thead>
                  <tr>
                    <th>Category</th>
                    <th className="right">Size</th>
                    <th className="right">Files</th>
                    <th style={{ width: 240 }}>Share</th>
                  </tr>
                </thead>
                <tbody>
                  {categories
                    .filter((c) => c.bytes > 0)
                    .map((c) => (
                      <tr
                        key={c.category}
                        onClick={() =>
                          setFilter((current) => ({ ...current, categories: [c.category] }))
                        }
                      >
                        <td>
                          <div className="name-cell">
                            <span
                              className="swatch"
                              style={{ background: CATEGORY_COLORS[c.category] }}
                            />
                            {CATEGORY_LABELS[c.category]}
                          </div>
                        </td>
                        <td className="right">{humanBytes(c.bytes)}</td>
                        <td className="right muted">{c.files.toLocaleString()}</td>
                        <td>
                          <div className="bar">
                            <span
                              style={{
                                width: `${Math.max(1, c.fraction * 100)}%`,
                                background: CATEGORY_COLORS[c.category],
                              }}
                            />
                          </div>
                          <span className="tertiary">{percent(c.fraction)}</span>
                        </td>
                      </tr>
                    ))}
                </tbody>
              </table>
            </div>
            <p className="tertiary">
              Click a category to filter every other view by it.
            </p>
          </div>
        );

      case "reports":
        return (
          <div className="content">
            <ReportsPanel summary={summary} />
          </div>
        );
    }
  }
}
