/**
 * Volume list. Removable and network drives are grouped separately, the way
 * Finder's sidebar does, because "my internal disk" and "the NAS I mounted" are
 * different kinds of thing to a user even though the OS reports them alike.
 */

import { humanBytes, percent } from "../lib/format";
import type { Volume } from "../lib/types";

interface Props {
  volumes: Volume[];
  selectedId: string | null;
  onSelect: (volume: Volume) => void;
  onScanFolder: () => void;
  busy: boolean;
}

export function Sidebar({ volumes, selectedId, onSelect, onScanFolder, busy }: Props) {
  const internal = volumes.filter((v) => !v.is_removable && !v.is_network);
  const removable = volumes.filter((v) => v.is_removable);
  const network = volumes.filter((v) => v.is_network);

  return (
    <aside className="sidebar">
      <h2>Volumes</h2>
      {internal.map((volume) => (
        <VolumeRow
          key={volume.id}
          volume={volume}
          selected={volume.id === selectedId}
          onSelect={onSelect}
        />
      ))}

      {removable.length > 0 && <h2>Removable</h2>}
      {removable.map((volume) => (
        <VolumeRow
          key={volume.id}
          volume={volume}
          selected={volume.id === selectedId}
          onSelect={onSelect}
        />
      ))}

      {network.length > 0 && <h2>Network</h2>}
      {network.map((volume) => (
        <VolumeRow
          key={volume.id}
          volume={volume}
          selected={volume.id === selectedId}
          onSelect={onSelect}
        />
      ))}

      <h2>Other</h2>
      <button className="volume" onClick={onScanFolder} disabled={busy}>
        <span className="volume-name">
          <span aria-hidden>📂</span>
          <span className="label">Scan a folder…</span>
        </span>
      </button>
    </aside>
  );
}

function VolumeRow({
  volume,
  selected,
  onSelect,
}: {
  volume: Volume;
  selected: boolean;
  onSelect: (volume: Volume) => void;
}) {
  const used = volume.total_bytes > 0 ? volume.used_bytes / volume.total_bytes : 0;
  const meterClass = used > 0.92 ? "critical" : used > 0.8 ? "warn" : "";
  const icon = volume.is_network ? "🌐" : volume.is_removable ? "💾" : volume.is_root ? "🖥️" : "🗄️";

  return (
    <button
      className={`volume${selected ? " selected" : ""}`}
      onClick={() => onSelect(volume)}
      title={`${volume.mount_point} — ${volume.filesystem.toUpperCase()}`}
    >
      <span className="volume-name">
        <span aria-hidden>{icon}</span>
        <span className="label">{volume.name}</span>
        {volume.is_read_only && <span className="badge">read-only</span>}
      </span>
      <div className={`meter ${meterClass}`}>
        <span style={{ width: `${Math.min(100, used * 100)}%` }} />
      </div>
      <div className="volume-meta">
        {humanBytes(volume.free_bytes)} free · {percent(used, 0)} used
      </div>
    </button>
  );
}
