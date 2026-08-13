/**
 * Presentation helpers.
 *
 * Sizes use decimal units to match Finder — if Helios and the Get Info panel
 * disagree about how big a folder is, the user will trust Finder and stop
 * trusting Helios.
 */

import type { Category } from "./types";

export function humanBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes < 0) return "—";
  if (bytes < 1000) return `${Math.round(bytes)} bytes`;
  const units = ["KB", "MB", "GB", "TB", "PB"];
  let value = bytes;
  let unit = -1;
  while (value >= 1000 && unit < units.length - 1) {
    value /= 1000;
    unit += 1;
  }
  return `${value < 10 ? value.toFixed(1) : Math.round(value)} ${units[unit]}`;
}

export function percent(fraction: number, digits = 1): string {
  if (!Number.isFinite(fraction)) return "—";
  return `${(fraction * 100).toFixed(digits)}%`;
}

export function humanDuration(ms: number): string {
  if (!Number.isFinite(ms) || ms < 0) return "—";
  if (ms < 1000) return `${Math.round(ms)} ms`;
  const seconds = ms / 1000;
  if (seconds < 60) return `${seconds.toFixed(1)} s`;
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes} min ${Math.round(seconds % 60)} s`;
  return `${Math.floor(minutes / 60)} h ${minutes % 60} min`;
}

/** "3 minutes left" — deliberately coarse, because a precise ETA is a lie. */
export function humanEta(ms: number | null): string {
  if (ms == null) return "estimating…";
  const seconds = Math.round(ms / 1000);
  if (seconds < 10) return "almost done";
  if (seconds < 60) return `about ${Math.round(seconds / 5) * 5} seconds left`;
  const minutes = Math.round(seconds / 60);
  if (minutes < 60) return `about ${minutes} minute${minutes === 1 ? "" : "s"} left`;
  const hours = Math.round(minutes / 6) / 10;
  return `about ${hours} hours left`;
}

const dateFormat = new Intl.DateTimeFormat(undefined, {
  year: "numeric",
  month: "short",
  day: "numeric",
});

const dateTimeFormat = new Intl.DateTimeFormat(undefined, {
  dateStyle: "medium",
  timeStyle: "short",
});

export function formatDate(unixSeconds: number): string {
  if (!unixSeconds) return "—";
  return dateFormat.format(new Date(unixSeconds * 1000));
}

export function formatDateTime(unixSeconds: number): string {
  if (!unixSeconds) return "—";
  return dateTimeFormat.format(new Date(unixSeconds * 1000));
}

export function relativeTime(unixSeconds: number): string {
  if (!unixSeconds) return "never";
  const seconds = Math.floor(Date.now() / 1000) - unixSeconds;
  if (seconds < 90) return "just now";
  const minutes = Math.round(seconds / 60);
  if (minutes < 60) return `${minutes} min ago`;
  const hours = Math.round(minutes / 60);
  if (hours < 24) return `${hours} hour${hours === 1 ? "" : "s"} ago`;
  const days = Math.round(hours / 24);
  if (days < 30) return `${days} day${days === 1 ? "" : "s"} ago`;
  return formatDate(unixSeconds);
}

/**
 * Category colours.
 *
 * Chosen to stay distinguishable side by side in a dense treemap and to survive
 * the most common form of colour blindness (deuteranopia) by varying lightness
 * as well as hue — a treemap where two adjacent categories read as the same
 * block is worse than one with fewer colours.
 */
export const CATEGORY_COLORS: Record<Category, string> = {
  documents: "#4a82e8",
  images: "#5cb872",
  videos: "#d97052",
  audio: "#9c70db",
  archives: "#e5ad40",
  applications: "#45aec2",
  developer: "#737d94",
  system: "#8c919c",
  other: "#b3b7bf",
};

export const CATEGORY_LABELS: Record<Category, string> = {
  documents: "Documents",
  images: "Images",
  videos: "Videos",
  audio: "Audio",
  archives: "Archives",
  applications: "Applications",
  developer: "Developer",
  system: "System Files",
  other: "Other",
};

/** Middle-truncates a path so both the volume and the filename stay visible. */
export function shortenPath(path: string, max = 60): string {
  if (path.length <= max) return path;
  const parts = path.split("/");
  const tail = parts.slice(-2).join("/");
  return `${path.slice(0, Math.max(0, max - tail.length - 2))}…/${tail}`;
}

export function extensionOf(name: string): string {
  const stem = name.startsWith(".") ? name.slice(1) : name;
  const dot = stem.lastIndexOf(".");
  return dot > 0 ? stem.slice(dot + 1).toLowerCase() : "";
}
