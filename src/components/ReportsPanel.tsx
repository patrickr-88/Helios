/**
 * Report export.
 *
 * The three formats answer three different needs: CSV for a spreadsheet, JSON
 * for a script, PDF for something to send someone. All three are generated in
 * Rust from the same report structure, so they never disagree.
 */

import { useEffect, useState } from "react";

import { api, chooseSavePath } from "../lib/api";
import { formatDateTime, humanBytes, humanDuration, percent } from "../lib/format";
import type { ScanIssue, ScanSummary } from "../lib/types";

interface Props {
  summary: ScanSummary;
}

type Format = "csv" | "json" | "pdf";

const FORMATS: Array<{ id: Format; label: string; note: string }> = [
  { id: "csv", label: "CSV", note: "Opens in Numbers or Excel. Full UTF-8 names." },
  { id: "json", label: "JSON", note: "For scripts and pipelines. Same fields as the UI." },
  { id: "pdf", label: "PDF", note: "Printable summary with the top-100 tables." },
];

export function ReportsPanel({ summary }: Props) {
  const [topN, setTopN] = useState(100);
  const [status, setStatus] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [issues, setIssues] = useState<ScanIssue[]>([]);

  useEffect(() => {
    api
      .issues(summary.scanId, 200)
      .then(setIssues)
      .catch(() => setIssues([]));
  }, [summary.scanId, summary.scannedAt]);

  const exportAs = async (format: Format) => {
    setStatus(null);
    const stem = (summary.volume?.name ?? "helios-report").replace(/[^\w.-]+/g, "-");
    const destination = await chooseSavePath(`${stem}-storage-report.${format}`);
    if (!destination) return;

    setBusy(true);
    try {
      setStatus(
        await api.exportReport({ scanId: summary.scanId, format, destination, topN }),
      );
    } catch (error) {
      setStatus(`Export failed: ${String(error)}`);
    } finally {
      setBusy(false);
    }
  };

  const coverage =
    summary.volume && summary.volume.used_bytes > 0
      ? summary.totalBytes / summary.volume.used_bytes
      : null;

  return (
    <>
      <div className="panel">
        <header>
          Export a report
          <span className="hint">read-only — nothing on the scanned volume is touched</span>
        </header>
        <div className="panel-body">
          <label className="field" style={{ marginBottom: 12 }}>
            Include the top
            <select
              className="input"
              value={topN}
              onChange={(event) => setTopN(Number(event.target.value))}
            >
              {[25, 50, 100, 250, 1000].map((n) => (
                <option key={n} value={n}>
                  {n}
                </option>
              ))}
            </select>
            files and folders
          </label>

          <div style={{ display: "grid", gap: 10, gridTemplateColumns: "repeat(auto-fit, minmax(210px, 1fr))" }}>
            {FORMATS.map((format) => (
              <div key={format.id} className="card">
                <h3>{format.label}</h3>
                <p className="muted" style={{ margin: "0 0 10px", fontSize: 12 }}>
                  {format.note}
                </p>
                <button
                  className="button primary"
                  disabled={busy}
                  onClick={() => exportAs(format.id)}
                >
                  Export {format.label}
                </button>
              </div>
            ))}
          </div>

          {status && (
            <p className="muted" style={{ marginBottom: 0 }}>
              {status}
            </p>
          )}
        </div>
      </div>

      <div className="panel">
        <header>What this report will contain</header>
        <div className="panel-body">
          <dl style={{ display: "grid", gridTemplateColumns: "auto 1fr", gap: "5px 14px", margin: 0 }}>
            <dt className="muted">Location</dt>
            <dd style={{ margin: 0 }}>{summary.rootPath}</dd>
            <dt className="muted">Scanned</dt>
            <dd style={{ margin: 0 }}>{formatDateTime(summary.scannedAt)}</dd>
            <dt className="muted">Total</dt>
            <dd style={{ margin: 0 }}>
              {humanBytes(summary.totalBytes)} in {summary.fileCount.toLocaleString()} files
            </dd>
            <dt className="muted">Scan time</dt>
            <dd style={{ margin: 0 }}>
              {humanDuration(summary.elapsedMs)} · {summary.nodeCount.toLocaleString()} entries ·{" "}
              {humanBytes(summary.memoryBytes)} of memory
            </dd>
            {coverage != null && (
              <>
                <dt className="muted">Coverage</dt>
                <dd style={{ margin: 0 }}>
                  {percent(coverage, 0)} of the volume's used space
                  {coverage < 0.98 && " — some locations were unreadable"}
                </dd>
              </>
            )}
          </dl>
        </div>
      </div>

      {issues.length > 0 && (
        <div className="panel">
          <header>
            Locations that could not be read
            <span className="hint">{issues.length} shown</span>
          </header>
          <table className="rows">
            <tbody>
              {issues.map((issue) => (
                <tr key={issue.path}>
                  <td className="mono">{issue.path}</td>
                  <td className="right muted" style={{ width: 200 }}>
                    {issue.message}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </>
  );
}
