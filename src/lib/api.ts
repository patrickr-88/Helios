/**
 * The single place the UI talks to the engine.
 *
 * Two reasons everything funnels through here rather than calling `invoke`
 * inline: components stay testable without a Tauri runtime, and the browser
 * fallback below lets the whole interface be developed, demoed and screenshot
 * in a plain browser (`npm run dev` with no Rust build) against a synthetic
 * tree. If `window.__TAURI_INTERNALS__` is absent, we are in a browser and the
 * mock answers instead.
 */

import type {
  AppInfo,
  CategorySummary,
  Entry,
  Filter,
  ScanIssue,
  ScanProgress,
  ScanRequest,
  ScanSummary,
  SnapshotMeta,
  SortKey,
  Tile,
  Volume,
} from "./types";
import { mock } from "./mock";

export const isTauri = (): boolean =>
  typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  if (!isTauri()) {
    return mock<T>(command, args);
  }
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<T>(command, args);
}

/** Subscribes to a scan event. Returns an unsubscribe function. */
export async function listen<T>(event: string, handler: (payload: T) => void): Promise<() => void> {
  if (!isTauri()) {
    return mock.listen(event, handler as (p: unknown) => void);
  }
  const { listen: tauriListen } = await import("@tauri-apps/api/event");
  const unlisten = await tauriListen<T>(event, (e) => handler(e.payload));
  return unlisten;
}

export const EVENTS = {
  progress: "scan://progress",
  finished: "scan://finished",
  failed: "scan://failed",
} as const;

export const api = {
  listVolumes: () => call<Volume[]>("list_volumes"),

  startScan: (request: ScanRequest) => call<string>("start_scan", { request }),
  pauseScan: () => call<void>("pause_scan"),
  resumeScan: () => call<void>("resume_scan"),
  cancelScan: () => call<void>("cancel_scan"),
  activeScan: () => call<string | null>("active_scan"),

  scanSummary: (scanId: string) => call<ScanSummary>("scan_summary", { scanId }),
  loadedScans: () => call<string[]>("loaded_scans"),

  listChildren: (
    scanId: string,
    nodeId: number,
    filter?: Filter,
    sort: SortKey = "size",
    descending = true,
    limit = 1000,
  ) => call<Entry[]>("list_children", { scanId, nodeId, filter, sort, descending, limit }),

  ancestors: (scanId: string, nodeId: number) => call<Entry[]>("ancestors", { scanId, nodeId }),

  treemap: (
    scanId: string,
    nodeId: number,
    width: number,
    height: number,
    maxDepth = 5,
    includeHidden = true,
  ) =>
    call<Tile[]>("treemap_layout", {
      scanId,
      nodeId,
      width,
      height,
      maxDepth,
      includeHidden,
    }),

  largest: (scanId: string, dirs: boolean, limit = 100, filter?: Filter) =>
    call<Entry[]>("largest_entries", { scanId, dirs, limit, filter }),

  search: (scanId: string, filter: Filter, sort: SortKey = "size", limit = 500) =>
    call<Entry[]>("search_entries", { scanId, filter, sort, limit }),

  categories: (scanId: string, filter?: Filter) =>
    call<CategorySummary[]>("category_breakdown", { scanId, filter }),

  issues: (scanId: string, limit = 200) => call<ScanIssue[]>("scan_issues", { scanId, limit }),

  exportReport: (request: {
    scanId: string;
    format: "csv" | "json" | "pdf";
    destination: string;
    topN?: number;
    filter?: Filter;
  }) => call<string>("export_report", { request }),

  listSnapshots: () => call<SnapshotMeta[]>("list_snapshots"),
  loadSnapshot: (volumeId: string) => call<ScanSummary>("load_snapshot", { volumeId }),
  forgetScan: (scanId: string) => call<void>("forget_scan", { scanId }),

  revealInFileManager: (path: string) => call<void>("reveal_in_file_manager", { path }),
  appInfo: () => call<AppInfo>("app_info"),
};

/**
 * Opens the system save panel. Falls back to a synthetic path in the browser so
 * the export flow can still be exercised there.
 */
export async function chooseSavePath(defaultName: string): Promise<string | null> {
  if (!isTauri()) {
    return `~/Downloads/${defaultName}`;
  }
  const { save } = await import("@tauri-apps/plugin-dialog");
  const extension = defaultName.split(".").pop() ?? "csv";
  return save({
    defaultPath: defaultName,
    filters: [{ name: extension.toUpperCase(), extensions: [extension] }],
  });
}

/** Opens the folder picker for "Scan a folder…". */
export async function chooseFolder(): Promise<string | null> {
  if (!isTauri()) {
    return "/Users/demo";
  }
  const { open } = await import("@tauri-apps/plugin-dialog");
  const selected = await open({ directory: true, multiple: false });
  return typeof selected === "string" ? selected : null;
}

export type { ScanProgress };
