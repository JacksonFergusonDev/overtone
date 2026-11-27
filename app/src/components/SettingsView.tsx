import { open } from "@tauri-apps/plugin-dialog";
import { FolderPlus, Trash2 } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { backend, errorMessage } from "../lib/backend";
import { formatDateTime } from "../lib/format";
import type { LibraryRoot } from "../lib/types";
import { confirmAction } from "../store/confirm";
import {
  cancelAudit,
  runAudit,
  runScan,
  scanFraction,
  scanLabel,
  useJobs,
} from "../store/jobs";
import { useLibrary } from "../store/library";
import { notify, notifyError } from "../store/toasts";
import { useUi } from "../store/ui";

export function ProgressBar({
  value,
  label,
}: {
  value: number | null;
  label: string;
}) {
  return (
    <div className="progress">
      <div
        className={`progress-track${value === null ? " indeterminate" : ""}`}
        role="progressbar"
        aria-label={label}
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={value === null ? undefined : Math.round(value * 100)}
      >
        <div
          className="progress-fill"
          style={{ width: value === null ? undefined : `${value * 100}%` }}
        />
      </div>
      <span className="progress-label num">{label}</span>
    </div>
  );
}

function Folders() {
  const visible = useUi((s) => s.view === "settings");
  const [roots, setRoots] = useState<LibraryRoot[] | null>(null);

  const refresh = useCallback(() => {
    backend
      .listRoots()
      .then(setRoots, (error: unknown) =>
        notifyError(`Couldn't list library folders: ${errorMessage(error)}`),
      );
  }, []);

  useEffect(() => {
    if (visible) refresh();
  }, [visible, refresh]);

  const add = async () => {
    let path: string | null;
    try {
      const picked = await open({
        directory: true,
        multiple: false,
        title: "Add a library folder",
      });
      path = typeof picked === "string" ? picked : null;
    } catch (error) {
      notifyError(`Couldn't open the folder picker: ${errorMessage(error)}`);
      return;
    }
    if (!path) return;
    try {
      await backend.addRoot(path);
      notify("Folder added. Scan to index its tracks.");
      refresh();
    } catch (error) {
      notifyError(`Couldn't add the folder: ${errorMessage(error)}`);
    }
  };

  const remove = async (root: LibraryRoot) => {
    const ok = await confirmAction({
      title: "Stop indexing this folder?",
      body: `${root.path}\n\nOvertone stops scanning it. Your audio files aren't touched.`,
      confirmLabel: "Remove folder",
      danger: true,
    });
    if (!ok) return;
    try {
      await backend.removeRoot(root.id);
      refresh();
    } catch (error) {
      notifyError(`Couldn't remove the folder: ${errorMessage(error)}`);
    }
  };

  return (
    <section className="card" aria-labelledby="settings-folders">
      <header className="card-header">
        <h2 id="settings-folders">Library folders</h2>
        <button type="button" className="btn" onClick={() => void add()}>
          <FolderPlus size={14} aria-hidden="true" /> Add folder
        </button>
      </header>
      {roots === null ? (
        <p className="dim">Loading folders…</p>
      ) : roots.length === 0 ? (
        <p className="dim">
          No folders yet. Add the folder that holds your music.
        </p>
      ) : (
        <ul className="root-list">
          {roots.map((root) => (
            <li key={root.id}>
              <span className="mono truncate" title={root.path}>
                {root.path}
              </span>
              <span className="dim small">
                added {formatDateTime(root.added_at)}
              </span>
              <button
                type="button"
                className="icon-button"
                aria-label={`Remove ${root.path}`}
                onClick={() => void remove(root)}
              >
                <Trash2 size={14} aria-hidden="true" />
              </button>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}

function Scan() {
  const scan = useJobs((s) => s.scan);
  const [refresh, setRefresh] = useState(false);
  const report = scan.report;
  return (
    <section className="card" aria-labelledby="settings-scan">
      <header className="card-header">
        <div>
          <h2 id="settings-scan">Scan</h2>
          <p className="dim">
            Finds new, changed, and missing files in your library folders.
          </p>
        </div>
        <div className="card-actions">
          <label className="checkbox">
            <input
              type="checkbox"
              checked={refresh}
              onChange={(event) => setRefresh(event.target.checked)}
            />
            Refresh all tags
          </label>
          <button
            type="button"
            className="btn btn-primary"
            disabled={scan.running}
            onClick={() => void runScan(refresh)}
          >
            {scan.running ? "Scanning…" : "Scan"}
          </button>
        </div>
      </header>
      {scan.running && (
        <ProgressBar
          value={scanFraction(scan.progress)}
          label={scanLabel(scan.progress)}
        />
      )}
      {report && (
        <div className="report" role="status">
          <p className="num">
            {report.added} added, {report.updated} updated, {report.unchanged}{" "}
            unchanged, {report.missing} missing in{" "}
            {(report.elapsed_ms / 1000).toFixed(1)} s
          </p>
          {report.unavailable_roots.map((root) => (
            <p key={root} className="banner banner-warn">
              Couldn't reach <span className="mono">{root}</span>. Its tracks
              were left as they were.
            </p>
          ))}
          {report.failed.length > 0 && (
            <details>
              <summary>
                {report.failed.length} files couldn't be fully read
              </summary>
              <ul className="small">
                {report.failed.slice(0, 200).map((failure) => (
                  <li key={failure.path}>
                    <span className="mono">{failure.path}</span>:{" "}
                    {failure.reason}
                  </li>
                ))}
              </ul>
            </details>
          )}
        </div>
      )}
    </section>
  );
}

function Audit() {
  const audit = useJobs((s) => s.audit);
  const progress = audit.progress;
  const report = audit.report;
  const auditAll = () => {
    const { tracks, order } = useLibrary.getState();
    void runAudit(order.filter((id) => tracks.get(id)?.missing_since === null));
  };
  return (
    <section className="card" aria-labelledby="settings-audit">
      <header className="card-header">
        <div>
          <h2 id="settings-audit">Quality audit</h2>
          <p className="dim">
            Spectral analysis flags likely transcodes for you to check by ear.
            Files are never changed.
          </p>
        </div>
        <div className="card-actions">
          {audit.running ? (
            <button
              type="button"
              className="btn"
              disabled={audit.cancelling}
              onClick={() => void cancelAudit()}
            >
              {audit.cancelling ? "Cancelling…" : "Cancel"}
            </button>
          ) : (
            <>
              <button
                type="button"
                className="btn btn-primary"
                onClick={() => void runAudit(null)}
              >
                Audit pending tracks
              </button>
              <button type="button" className="btn" onClick={auditAll}>
                Re-audit all
              </button>
            </>
          )}
        </div>
      </header>
      {audit.running && (
        <ProgressBar
          value={
            progress && progress.total > 0
              ? progress.done / progress.total
              : null
          }
          label={
            progress
              ? `Analyzed ${progress.done.toLocaleString()} of ${progress.total.toLocaleString()}`
              : "Starting audit…"
          }
        />
      )}
      {report && (
        <div className="report" role="status">
          <p className="num">
            {report.analyzed} analyzed in{" "}
            {(report.elapsed_ms / 1000).toFixed(1)} s: {report.verdicts.pass}{" "}
            pass, {report.verdicts.warning} warning, {report.verdicts.suspect}{" "}
            suspect, {report.verdicts.inconclusive} inconclusive,{" "}
            {report.verdicts.unchecked} couldn't be analyzed
          </p>
          {report.flagged.length > 0 && (
            <p>
              {report.flagged.length} tracks were flagged. Open Needs review to
              check them.
            </p>
          )}
        </div>
      )}
    </section>
  );
}

export function SettingsView() {
  const status = useLibrary((s) => s.status);
  return (
    <div className="view scroll-view">
      <header className="view-header">
        <h1 className="view-title">Settings</h1>
      </header>
      <Folders />
      <Scan />
      <Audit />
      <section className="card" aria-labelledby="settings-about">
        <h2 id="settings-about">About</h2>
        <dl className="facts">
          <div>
            <dt>Version</dt>
            <dd className="num">{status?.version}</dd>
          </div>
          <div>
            <dt>Library</dt>
            <dd className="mono truncate" title={status?.library_path}>
              {status?.library_path}
            </dd>
          </div>
          <div>
            <dt>Schema</dt>
            <dd className="num">{status?.library.schema_version}</dd>
          </div>
        </dl>
      </section>
    </div>
  );
}
