/**
 * Live scan status: progress, ETA, and the pause/resume/stop controls.
 *
 * The bar is honest about uncertainty — while the engine has no trustworthy
 * estimate yet, the track runs indeterminate and the label says "estimating"
 * rather than showing a number that will swing wildly a second later.
 */

import { humanBytes, humanEta, shortenPath } from "../lib/format";
import type { ScanProgress } from "../lib/types";

interface Props {
  progress: ScanProgress;
  paused: boolean;
  onPause: () => void;
  onResume: () => void;
  onCancel: () => void;
}

export function ScanBar({ progress, paused, onPause, onResume, onCancel }: Props) {
  const fraction = progress.fraction ?? null;

  return (
    <div className="scanbar">
      <div className="spin" style={{ animationPlayState: paused ? "paused" : "running" }} />
      <span>
        {paused ? "Paused" : "Scanning"} · {progress.files_seen.toLocaleString()} files ·{" "}
        {humanBytes(progress.bytes_seen)}
        {progress.dirs_reused > 0 && ` · ${progress.dirs_reused.toLocaleString()} reused`}
      </span>

      <div className={`track${fraction == null ? " indeterminate" : ""}`}>
        <span style={fraction == null ? undefined : { width: `${fraction * 100}%` }} />
      </div>

      <span className="muted">{paused ? "paused" : humanEta(progress.eta_ms)}</span>
      <span className="path" title={progress.current_path}>
        {shortenPath(progress.current_path, 48)}
      </span>

      {paused ? (
        <button className="button" onClick={onResume}>
          Resume
        </button>
      ) : (
        <button className="button" onClick={onPause}>
          Pause
        </button>
      )}
      <button className="button" onClick={onCancel}>
        Stop
      </button>
    </div>
  );
}
