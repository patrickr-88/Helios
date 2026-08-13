/**
 * Treemap view.
 *
 * Rendered to a single `<canvas>`, not to DOM nodes. A 20,000-rectangle treemap
 * is 20,000 layout objects and 20,000 style recalculations as DOM; on canvas it
 * is one paint of a flat array, and hover means a hit-test rather than a
 * pointer event on every tile. That difference is what keeps resizing smooth on
 * a real volume.
 *
 * Layout itself comes from Rust — see `helios_core::treemap` — so this file
 * only paints, hit-tests and handles navigation.
 */

import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { api } from "../lib/api";
import { CATEGORY_COLORS, humanBytes, percent } from "../lib/format";
import type { Tile } from "../lib/types";

interface Props {
  scanId: string;
  nodeId: number;
  includeHidden: boolean;
  onOpen: (tile: Tile) => void;
  onSelect: (tile: Tile | null) => void;
  selectedId: number | null;
}

/** Depth shading: deeper tiles sit slightly lighter, so nesting reads visually. */
function tint(color: string, depth: number): string {
  const amount = Math.min(depth * 0.11, 0.44);
  const r = parseInt(color.slice(1, 3), 16);
  const g = parseInt(color.slice(3, 5), 16);
  const b = parseInt(color.slice(5, 7), 16);
  const mix = (c: number) => Math.round(c + (255 - c) * amount);
  return `rgb(${mix(r)}, ${mix(g)}, ${mix(b)})`;
}

export function Treemap({ scanId, nodeId, includeHidden, onOpen, onSelect, selectedId }: Props) {
  const containerRef = useRef<HTMLDivElement>(null);
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const [tiles, setTiles] = useState<Tile[]>([]);
  const [size, setSize] = useState({ width: 0, height: 0 });
  const [hover, setHover] = useState<{ tile: Tile; x: number; y: number } | null>(null);
  const [loading, setLoading] = useState(false);

  // Track the container's real pixel size; the layout has to be recomputed on
  // resize because rectangle proportions depend on the aspect ratio.
  useEffect(() => {
    const element = containerRef.current;
    if (!element) return;
    const observer = new ResizeObserver(([entry]) => {
      const { width, height } = entry.contentRect;
      setSize({ width: Math.floor(width), height: Math.floor(height) });
    });
    observer.observe(element);
    return () => observer.disconnect();
  }, []);

  useEffect(() => {
    if (size.width < 40 || size.height < 40) return;
    let cancelled = false;
    setLoading(true);
    // Debounced so dragging a window edge issues one layout, not sixty.
    const timer = window.setTimeout(() => {
      api
        .treemap(scanId, nodeId, size.width, size.height, 5, includeHidden)
        .then((result) => {
          if (!cancelled) setTiles(result);
        })
        .catch(() => {
          if (!cancelled) setTiles([]);
        })
        .finally(() => {
          if (!cancelled) setLoading(false);
        });
    }, 90);
    return () => {
      cancelled = true;
      window.clearTimeout(timer);
    };
  }, [scanId, nodeId, size.width, size.height, includeHidden]);

  const paint = useCallback(() => {
    const canvas = canvasRef.current;
    if (!canvas || size.width === 0) return;
    const dpr = window.devicePixelRatio || 1;
    canvas.width = size.width * dpr;
    canvas.height = size.height * dpr;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);

    const styles = getComputedStyle(document.documentElement);
    const background = styles.getPropertyValue("--bg").trim() || "#fff";
    ctx.fillStyle = background;
    ctx.fillRect(0, 0, size.width, size.height);

    for (const tile of tiles) {
      const { x, y, w, h } = tile.rect;
      ctx.fillStyle = tint(CATEGORY_COLORS[tile.category] ?? "#999", tile.depth);
      ctx.fillRect(x, y, w, h);

      // Hairline borders only where there is room; below ~6px they turn the
      // map into a grey mush.
      if (w > 6 && h > 6) {
        ctx.strokeStyle = "rgba(0, 0, 0, 0.28)";
        ctx.lineWidth = 0.5;
        ctx.strokeRect(x + 0.25, y + 0.25, w - 0.5, h - 0.5);
      }

      // Labels only when they genuinely fit; clipped text is noise.
      if (w > 62 && h > 20) {
        ctx.save();
        ctx.beginPath();
        ctx.rect(x + 3, y + 2, w - 6, h - 4);
        ctx.clip();
        ctx.fillStyle = "rgba(0, 0, 0, 0.82)";
        ctx.font = '600 11px -apple-system, BlinkMacSystemFont, "SF Pro Text", system-ui, sans-serif';
        ctx.textBaseline = "top";
        ctx.fillText(tile.name, x + 5, y + 4);
        if (h > 34) {
          ctx.fillStyle = "rgba(0, 0, 0, 0.6)";
          ctx.font = '10px -apple-system, BlinkMacSystemFont, system-ui, sans-serif';
          ctx.fillText(humanBytes(tile.size), x + 5, y + 18);
        }
        ctx.restore();
      }

      if (tile.id === selectedId) {
        ctx.strokeStyle = styles.getPropertyValue("--accent").trim() || "#0a72e8";
        ctx.lineWidth = 2;
        ctx.strokeRect(x + 1, y + 1, w - 2, h - 2);
      }
    }
  }, [tiles, size, selectedId]);

  useEffect(() => {
    paint();
  }, [paint]);

  // Repaint when the system switches between light and dark.
  useEffect(() => {
    const media = window.matchMedia("(prefers-color-scheme: dark)");
    const handler = () => paint();
    media.addEventListener("change", handler);
    return () => media.removeEventListener("change", handler);
  }, [paint]);

  /** Tiles are emitted parent-first, so the last hit is the innermost one. */
  const hitTest = useCallback(
    (x: number, y: number): Tile | null => {
      for (let i = tiles.length - 1; i >= 0; i--) {
        const { rect } = tiles[i];
        if (x >= rect.x && x < rect.x + rect.w && y >= rect.y && y < rect.y + rect.h) {
          return tiles[i];
        }
      }
      return null;
    },
    [tiles],
  );

  const onMove = (event: React.MouseEvent<HTMLCanvasElement>) => {
    const bounds = event.currentTarget.getBoundingClientRect();
    const x = event.clientX - bounds.left;
    const y = event.clientY - bounds.top;
    const tile = hitTest(x, y);
    setHover(tile ? { tile, x, y } : null);
  };

  const total = useMemo(
    () => tiles.filter((t) => t.depth === 0).reduce((sum, t) => sum + t.size, 0),
    [tiles],
  );

  return (
    <div className="treemap" ref={containerRef}>
      <canvas
        ref={canvasRef}
        onMouseMove={onMove}
        onMouseLeave={() => setHover(null)}
        onClick={(event) => {
          const bounds = event.currentTarget.getBoundingClientRect();
          onSelect(hitTest(event.clientX - bounds.left, event.clientY - bounds.top));
        }}
        onDoubleClick={(event) => {
          const bounds = event.currentTarget.getBoundingClientRect();
          const tile = hitTest(event.clientX - bounds.left, event.clientY - bounds.top);
          // Double-click drills in, matching how folders open everywhere else.
          if (tile?.isDir) onOpen(tile);
        }}
      />

      {hover && (
        <div
          className="tooltip"
          style={{
            // Flip the tooltip when it would run off the right or bottom edge.
            left: Math.min(hover.x + 14, Math.max(0, size.width - 300)),
            top: Math.min(hover.y + 14, Math.max(0, size.height - 90)),
          }}
        >
          <strong>{hover.tile.name}</strong>
          <div className="muted">
            {humanBytes(hover.tile.size)}
            {total > 0 && ` · ${percent(hover.tile.size / total)} of this folder`}
          </div>
          {hover.tile.isDir && <div className="tertiary">Double-click to open</div>}
        </div>
      )}

      {loading && tiles.length === 0 && (
        <div className="empty">
          <div className="spin" />
          <span>Laying out the treemap…</span>
        </div>
      )}

      {!loading && tiles.length === 0 && (
        <div className="empty">
          <h2>Nothing to draw here</h2>
          <p className="muted">This folder is empty, or everything in it is zero bytes.</p>
        </div>
      )}
    </div>
  );
}
