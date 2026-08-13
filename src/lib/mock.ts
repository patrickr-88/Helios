/**
 * Browser fallback for the IPC layer.
 *
 * This exists so the interface can be run, styled and reviewed with
 * `npm run dev` alone — no Rust toolchain, no app bundle. It builds a small
 * synthetic volume in memory and answers the same commands the Rust side does,
 * including a simulated scan that emits progress events.
 *
 * It is a development aid, never shipped behaviour: inside the app,
 * `window.__TAURI_INTERNALS__` exists and none of this is reachable. The
 * treemap layout here is a deliberately simple slice-and-dice, not the
 * squarified algorithm the engine uses — good enough to lay out a demo, and not
 * a second implementation anyone could mistake for the real one.
 */

import type {
  Category,
  CategorySummary,
  Entry,
  Filter,
  ScanSummary,
  SortKey,
  Tile,
  Volume,
} from "./types";

interface MockNode {
  id: number;
  name: string;
  path: string;
  parent: number;
  size: number;
  isDir: boolean;
  category: Category;
  mtime: number;
  children: number[];
  fileCount: number;
  dirCount: number;
}

const CATEGORY_BY_EXT: Record<string, Category> = {
  mp4: "videos",
  mov: "videos",
  jpg: "images",
  png: "images",
  raw: "images",
  mp3: "audio",
  flac: "audio",
  pdf: "documents",
  docx: "documents",
  zip: "archives",
  dmg: "applications",
  app: "applications",
  rs: "developer",
  ts: "developer",
  dylib: "system",
};

const SHAPE: Array<[string, number, string[]]> = [
  ["Applications", 42, ["Xcode.app", "Final Cut Pro.app", "Logic Pro.app", "Docker.app"]],
  ["Users", 380, ["alex"]],
  ["Library", 96, ["Caches", "Application Support", "Developer"]],
  ["System", 24, ["Library"]],
];

let nodes: MockNode[] = [];

function build(): void {
  if (nodes.length) return;
  const now = Math.floor(Date.now() / 1000);
  nodes = [
    {
      id: 0,
      name: "Macintosh HD",
      path: "/",
      parent: -1,
      size: 0,
      isDir: true,
      category: "other",
      mtime: now,
      children: [],
      fileCount: 0,
      dirCount: 0,
    },
  ];

  // A deterministic pseudo-random source keeps the demo stable across reloads,
  // so a screenshot taken today matches one taken tomorrow.
  let seed = 20260813;
  const rand = () => ((seed = (seed * 1103515245 + 12345) & 0x7fffffff) / 0x7fffffff);

  const add = (parent: number, name: string, isDir: boolean, size: number): number => {
    const parentNode = nodes[parent];
    const ext = name.includes(".") ? name.split(".").pop()!.toLowerCase() : "";
    const node: MockNode = {
      id: nodes.length,
      name,
      path: parentNode.path === "/" ? `/${name}` : `${parentNode.path}/${name}`,
      parent,
      size: isDir ? 0 : size,
      isDir,
      category: CATEGORY_BY_EXT[ext] ?? "other",
      mtime: now - Math.floor(rand() * 60 * 60 * 24 * 900),
      children: [],
      fileCount: 0,
      dirCount: 0,
    };
    nodes.push(node);
    parentNode.children.push(node.id);
    return node.id;
  };

  const names = ["Projects", "Movies", "Photos", "Music", "Downloads", "Documents", "Archive"];
  const files = [
    "keynote.mp4",
    "render.mov",
    "shoot-001.raw",
    "cover.png",
    "album.flac",
    "invoice.pdf",
    "backup.zip",
    "notes.docx",
    "engine.rs",
    "app.ts",
  ];

  for (const [top, weight, children] of SHAPE) {
    const topId = add(0, top, true, 0);
    for (const child of children) {
      const childId = add(topId, child, true, 0);
      const folders = 2 + Math.floor(rand() * 4);
      for (let f = 0; f < folders; f++) {
        const sub = add(childId, names[Math.floor(rand() * names.length)] + ` ${f + 1}`, true, 0);
        const count = 3 + Math.floor(rand() * 8);
        for (let i = 0; i < count; i++) {
          const name = files[Math.floor(rand() * files.length)];
          add(sub, `${name.split(".")[0]}-${i}.${name.split(".").pop()}`, false, Math.floor(rand() ** 3 * weight * 40_000_000) + 4096);
        }
      }
    }
  }

  // Roll sizes and counts up, children-before-parents (nodes are appended in
  // that order), mirroring the engine's single reverse pass.
  for (let i = nodes.length - 1; i > 0; i--) {
    const node = nodes[i];
    const parent = nodes[node.parent];
    parent.size += node.size;
    parent.fileCount += node.fileCount + (node.isDir ? 0 : 1);
    parent.dirCount += node.dirCount + (node.isDir ? 1 : 0);
  }
}

const VOLUMES: Volume[] = [
  {
    id: "disk3s1s1",
    name: "Macintosh HD",
    mount_point: "/",
    filesystem: "apfs",
    total_bytes: 1_000_000_000_000,
    free_bytes: 0,
    used_bytes: 0,
    is_removable: false,
    is_network: false,
    is_read_only: false,
    is_root: true,
  },
  {
    id: "disk5s2",
    name: "Backup",
    mount_point: "/Volumes/Backup",
    filesystem: "hfs",
    total_bytes: 4_000_000_000_000,
    free_bytes: 1_200_000_000_000,
    used_bytes: 2_800_000_000_000,
    is_removable: true,
    is_network: false,
    is_read_only: false,
    is_root: false,
  },
];

function toEntry(node: MockNode): Entry {
  const parent = node.parent >= 0 ? nodes[node.parent] : node;
  return {
    id: node.id,
    name: node.name,
    path: node.path,
    size: node.size,
    physicalSize: Math.ceil(node.size / 4096) * 4096,
    category: node.category,
    isDir: node.isDir,
    isSymlink: false,
    isHidden: node.name.startsWith("."),
    isSystem: node.path.startsWith("/System") || node.path.startsWith("/Library"),
    isPackage: node.name.endsWith(".app"),
    isAccessible: true,
    mtime: node.mtime,
    fileCount: node.fileCount,
    dirCount: node.dirCount,
    fractionOfParent: parent.size > 0 ? node.size / parent.size : 1,
  };
}

function matches(node: MockNode, filter?: Filter): boolean {
  if (!filter) return true;
  const entry = toEntry(node);
  if (filter.onlyFiles && node.isDir) return false;
  if (filter.onlyDirs && !node.isDir) return false;
  if (filter.includeHidden === false && entry.isHidden) return false;
  if (filter.includeSystem === false && entry.isSystem) return false;
  if (filter.minSize != null && node.size < filter.minSize) return false;
  if (filter.maxSize != null && node.size > filter.maxSize) return false;
  if (filter.modifiedAfter != null && node.mtime < filter.modifiedAfter) return false;
  if (filter.modifiedBefore != null && node.mtime > filter.modifiedBefore) return false;
  if (filter.categories?.length && !filter.categories.includes(node.category)) return false;
  if (filter.extensions?.length) {
    const ext = node.name.split(".").pop()?.toLowerCase() ?? "";
    if (!filter.extensions.includes(ext)) return false;
  }
  if (filter.nameContains && !node.name.toLowerCase().includes(filter.nameContains.toLowerCase()))
    return false;
  if (filter.pathContains && !node.path.toLowerCase().includes(filter.pathContains.toLowerCase()))
    return false;
  return true;
}

function sortEntries(entries: Entry[], sort: SortKey, descending: boolean): Entry[] {
  const key = (e: Entry) =>
    sort === "name" ? 0 : sort === "modified" ? e.mtime : sort === "count" ? e.fileCount : e.size;
  const sorted = [...entries].sort((a, b) =>
    sort === "name" ? a.name.localeCompare(b.name) : key(a) - key(b),
  );
  return descending ? sorted.reverse() : sorted;
}

/** Simple slice-and-dice layout — see the note at the top of this file. */
function layout(nodeId: number, width: number, height: number, maxDepth: number): Tile[] {
  const tiles: Tile[] = [];
  const place = (id: number, x: number, y: number, w: number, h: number, depth: number) => {
    if (depth >= maxDepth || w < 4 || h < 4) return;
    const node = nodes[id];
    const children = node.children.map((c) => nodes[c]).filter((c) => c.size > 0);
    children.sort((a, b) => b.size - a.size);
    const total = children.reduce((sum, c) => sum + c.size, 0);
    if (!total) return;

    const horizontal = w >= h;
    let offset = 0;
    for (const child of children) {
      const extent = (child.size / total) * (horizontal ? w : h);
      if (extent < 3) break;
      const rect = horizontal
        ? { x: x + offset, y, w: extent, h }
        : { x, y: y + offset, w, h: extent };
      tiles.push({
        id: child.id,
        name: child.name,
        size: child.size,
        category: child.category,
        isDir: child.isDir,
        depth,
        rect,
        truncated: false,
      });
      if (child.isDir) {
        place(child.id, rect.x + 1, rect.y + 1, rect.w - 2, rect.h - 2, depth + 1);
      }
      offset += extent;
    }
  };
  place(nodeId, 0, 0, width, height, 0);
  return tiles;
}

type Handler = (payload: unknown) => void;
const listeners = new Map<string, Set<Handler>>();

function emit(event: string, payload: unknown): void {
  listeners.get(event)?.forEach((handler) => handler(payload));
}

function summary(): ScanSummary {
  const root = nodes[0];
  const volume = { ...VOLUMES[0] };
  volume.used_bytes = root.size;
  volume.free_bytes = volume.total_bytes - root.size;
  return {
    scanId: "disk3s1s1",
    rootPath: "/",
    state: "done",
    totalBytes: root.size,
    physicalBytes: Math.ceil(root.size * 1.02),
    fileCount: root.fileCount,
    dirCount: root.dirCount,
    scannedAt: Math.floor(Date.now() / 1000),
    elapsedMs: 8_400,
    nodeCount: nodes.length,
    memoryBytes: nodes.length * 56,
    dirsReused: 0,
    errorCount: 2,
    fromCache: false,
    volume,
  };
}

/** Plays a scan back over ~3 seconds so the progress UI has something to show. */
function simulateScan(): void {
  const root = nodes[0];
  const total = root.size;
  const started = Date.now();
  let bytes = 0;
  const timer = setInterval(() => {
    bytes = Math.min(total, bytes + total / 25);
    const elapsed = Date.now() - started;
    emit("scan://progress", {
      state: "running",
      files_seen: Math.floor((bytes / total) * root.fileCount),
      dirs_seen: Math.floor((bytes / total) * root.dirCount),
      bytes_seen: bytes,
      dirs_reused: 0,
      errors: 0,
      current_path: nodes[Math.floor(Math.random() * nodes.length)].path,
      elapsed_ms: elapsed,
      eta_ms: bytes > 0 ? Math.max(0, (elapsed / bytes) * (total - bytes)) : null,
      fraction: bytes / total,
    });
    if (bytes >= total) {
      clearInterval(timer);
      emit("scan://finished", summary());
    }
  }, 120);
}

export function mock<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  build();
  const arg = <V>(name: string, fallback: V): V => (args?.[name] as V) ?? fallback;

  switch (command) {
    case "list_volumes": {
      const volumes = VOLUMES.map((v) => ({ ...v }));
      volumes[0].used_bytes = nodes[0].size;
      volumes[0].free_bytes = volumes[0].total_bytes - nodes[0].size;
      return Promise.resolve(volumes as unknown as T);
    }
    case "start_scan":
      simulateScan();
      return Promise.resolve("disk3s1s1" as unknown as T);
    case "pause_scan":
    case "resume_scan":
    case "cancel_scan":
    case "forget_scan":
    case "reveal_in_file_manager":
      return Promise.resolve(undefined as unknown as T);
    case "active_scan":
      return Promise.resolve(null as unknown as T);
    case "scan_summary":
    case "load_snapshot":
      return Promise.resolve(summary() as unknown as T);
    case "loaded_scans":
      return Promise.resolve(["disk3s1s1"] as unknown as T);
    case "list_children": {
      const node = nodes[arg("nodeId", 0)];
      const rows = node.children
        .map((id) => nodes[id])
        .filter((child) => matches(child, args?.filter as Filter))
        .map(toEntry);
      return Promise.resolve(
        sortEntries(rows, arg<SortKey>("sort", "size"), arg("descending", true)).slice(
          0,
          arg("limit", 1000),
        ) as unknown as T,
      );
    }
    case "ancestors": {
      const chain: Entry[] = [];
      let current = nodes[arg("nodeId", 0)];
      while (current) {
        chain.unshift(toEntry(current));
        if (current.parent < 0) break;
        current = nodes[current.parent];
      }
      return Promise.resolve(chain as unknown as T);
    }
    case "treemap_layout":
      return Promise.resolve(
        layout(
          arg("nodeId", 0),
          arg("width", 800),
          arg("height", 600),
          arg("maxDepth", 5),
        ) as unknown as T,
      );
    case "largest_entries": {
      const dirs = arg("dirs", false);
      const rows = nodes
        .filter((n) => n.id !== 0 && n.isDir === dirs && matches(n, args?.filter as Filter))
        .map(toEntry)
        .sort((a, b) => b.size - a.size)
        .slice(0, arg("limit", 100));
      return Promise.resolve(rows as unknown as T);
    }
    case "search_entries": {
      const filter = args?.filter as Filter;
      const rows = nodes
        .filter((n) => n.id !== 0 && matches(n, filter))
        .map(toEntry)
        .sort((a, b) => b.size - a.size)
        .slice(0, arg("limit", 500));
      return Promise.resolve(rows as unknown as T);
    }
    case "category_breakdown": {
      const totals = new Map<Category, { bytes: number; files: number }>();
      for (const node of nodes) {
        if (node.isDir || !matches(node, args?.filter as Filter)) continue;
        const current = totals.get(node.category) ?? { bytes: 0, files: 0 };
        current.bytes += node.size;
        current.files += 1;
        totals.set(node.category, current);
      }
      const sum = [...totals.values()].reduce((acc, v) => acc + v.bytes, 0) || 1;
      const rows: CategorySummary[] = [...totals.entries()]
        .map(([category, v]) => ({ category, ...v, fraction: v.bytes / sum }))
        .sort((a, b) => b.bytes - a.bytes);
      return Promise.resolve(rows as unknown as T);
    }
    case "scan_issues":
      return Promise.resolve([
        { path: "/Library/Application Support/MobileSync", message: "permission denied" },
        { path: "/private/var/db/tccd", message: "operation not permitted" },
      ] as unknown as T);
    case "export_report":
      return Promise.resolve("Saved (demo mode — nothing was written)" as unknown as T);
    case "list_snapshots":
      return Promise.resolve([] as unknown as T);
    case "app_info":
      return Promise.resolve({
        version: "0.1.0",
        engineVersion: "0.1.0",
        cacheDirectory: "~/Library/Application Support/Helios/snapshots",
        defaultThreads: 8,
        offline: true,
        readOnly: true,
      } as unknown as T);
    default:
      return Promise.reject(new Error(`mock: unhandled command '${command}'`));
  }
}

mock.listen = (event: string, handler: Handler): (() => void) => {
  const set = listeners.get(event) ?? new Set<Handler>();
  set.add(handler);
  listeners.set(event, set);
  return () => set.delete(handler);
};
