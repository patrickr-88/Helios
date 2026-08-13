/**
 * Types mirroring the Rust IPC surface.
 *
 * These are written by hand rather than generated, and kept in the same order
 * as the structs in `src-tauri/src/commands.rs` and `helios-core`. The engine
 * serializes with `#[serde(rename_all = "camelCase")]`, so field names match
 * one-for-one; when a struct changes on either side, this file is the single
 * place the other side needs to follow.
 */

export type Category =
  | "documents"
  | "images"
  | "videos"
  | "audio"
  | "archives"
  | "applications"
  | "developer"
  | "system"
  | "other";

export const CATEGORIES: Category[] = [
  "documents",
  "images",
  "videos",
  "audio",
  "archives",
  "applications",
  "developer",
  "system",
  "other",
];

export type ScanState = "running" | "paused" | "cancelled" | "done";

export interface Volume {
  id: string;
  name: string;
  mount_point: string;
  filesystem: string;
  total_bytes: number;
  free_bytes: number;
  used_bytes: number;
  is_removable: boolean;
  is_network: boolean;
  is_read_only: boolean;
  is_root: boolean;
}

/** One row in any list view. */
export interface Entry {
  id: number;
  name: string;
  path: string;
  size: number;
  physicalSize: number;
  category: Category;
  isDir: boolean;
  isSymlink: boolean;
  isHidden: boolean;
  isSystem: boolean;
  isPackage: boolean;
  isAccessible: boolean;
  /** Unix seconds. */
  mtime: number;
  fileCount: number;
  dirCount: number;
  /** 0–1 share of the parent folder. */
  fractionOfParent: number;
}

export interface CategorySummary {
  category: Category;
  bytes: number;
  files: number;
  fraction: number;
}

export interface Rect {
  x: number;
  y: number;
  w: number;
  h: number;
}

export interface Tile {
  id: number;
  name: string;
  size: number;
  category: Category;
  isDir: boolean;
  depth: number;
  rect: Rect;
  truncated: boolean;
}

export interface ScanProgress {
  state: ScanState;
  files_seen: number;
  dirs_seen: number;
  bytes_seen: number;
  dirs_reused: number;
  errors: number;
  current_path: string;
  elapsed_ms: number;
  eta_ms: number | null;
  fraction: number | null;
}

export interface ScanSummary {
  scanId: string;
  rootPath: string;
  state: ScanState;
  totalBytes: number;
  physicalBytes: number;
  fileCount: number;
  dirCount: number;
  scannedAt: number;
  elapsedMs: number;
  nodeCount: number;
  memoryBytes: number;
  dirsReused: number;
  errorCount: number;
  fromCache: boolean;
  volume: Volume | null;
}

export interface SnapshotMeta {
  volumeId: string;
  rootPath: string;
  scannedAt: number;
  stats: {
    filesScanned: number;
    dirsScanned: number;
    bytesSeen: number;
    elapsedMs: number;
  };
}

export interface ScanIssue {
  path: string;
  message: string;
}

/** Mirrors `helios_core::query::Filter`; every field is optional. */
export interface Filter {
  minSize?: number | null;
  maxSize?: number | null;
  extensions?: string[];
  categories?: Category[];
  modifiedAfter?: number | null;
  modifiedBefore?: number | null;
  pathContains?: string | null;
  nameContains?: string | null;
  includeHidden?: boolean;
  includeSystem?: boolean;
  onlyFiles?: boolean;
  onlyDirs?: boolean;
}

export type SortKey = "size" | "name" | "modified" | "count";

export interface ScanRequest {
  path: string;
  incremental?: boolean;
  skipHidden?: boolean;
  crossFilesystem?: boolean;
  exclusions?: string[];
  threads?: number | null;
}

export interface AppInfo {
  version: string;
  engineVersion: string;
  cacheDirectory: string;
  defaultThreads: number;
  offline: boolean;
  readOnly: boolean;
}
