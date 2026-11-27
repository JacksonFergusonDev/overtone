import { RefreshCw } from "lucide-react";
import { useCallback, useEffect, useState, type ReactNode } from "react";
import { backend, errorMessage } from "../lib/backend";
import { fileName, formatDateTime } from "../lib/format";
import type {
  CommentChange,
  MirrorReport,
  RekordboxExportPlan,
  RekordboxExportReport,
  RunningApps,
  SeratoFileAction,
  SeratoSyncPlan,
  SeratoSyncReport,
  SyncRun,
} from "../lib/types";
import { confirmAction } from "../store/confirm";
import { reloadTracks } from "../store/library";
import { notify, notifyError } from "../store/toasts";
import { useUi } from "../store/ui";

type AppKey = keyof RunningApps;

const APP_NAMES: Record<AppKey, string> = {
  serato: "Serato DJ",
  rekordbox: "Rekordbox",
  traktor: "Traktor",
};

/** Apps Overtone syncs to and can detect. Traktor joins once its sync lands. */
const MONITORED_APPS: AppKey[] = ["serato", "rekordbox"];

/** Shared frame for one sync destination. Add a card per target app. */
export function SyncTargetCard({
  title,
  app,
  running,
  description,
  actions,
  runningNote,
  children,
}: {
  title: string;
  app?: AppKey;
  running?: RunningApps | null;
  description: ReactNode;
  actions?: ReactNode;
  /** Shown while `app` runs; defaults to asking you to quit it first. */
  runningNote?: ReactNode;
  children?: ReactNode;
}) {
  const isRunning = app && running ? running[app] : false;
  const headingId = `sync-${title.toLowerCase().replace(/\W+/g, "-")}`;
  return (
    <section className="card sync-card" aria-labelledby={headingId}>
      <header className="card-header">
        <div>
          <h2 id={headingId}>{title}</h2>
          <p className="dim">{description}</p>
        </div>
        <div className="card-actions">{actions}</div>
      </header>
      {isRunning && app && (
        <p className="banner banner-warn" role="note">
          {runningNote ??
            `${APP_NAMES[app]} is running. Quit it before applying, so it doesn't overwrite the changes.`}
        </p>
      )}
      {children}
    </section>
  );
}

function RunningIndicators({
  running,
  onRefresh,
}: {
  running: RunningApps | null;
  onRefresh: () => void;
}) {
  return (
    <div className="running" role="group" aria-label="DJ apps">
      {MONITORED_APPS.map((app) => {
        const state =
          running === null ? "unknown" : running[app] ? "running" : "closed";
        return (
          <span key={app} className={`app-pill app-${state}`}>
            <span className="badge-dot" aria-hidden="true" />
            {APP_NAMES[app]}:{" "}
            {state === "unknown"
              ? "Checking…"
              : state === "running"
                ? "Running"
                : "Closed"}
          </span>
        );
      })}
      <button
        type="button"
        className="icon-button"
        aria-label="Check DJ apps again"
        onClick={onRefresh}
      >
        <RefreshCw size={14} aria-hidden="true" />
      </button>
    </div>
  );
}

function actionText(action: SeratoFileAction): string {
  switch (action.action) {
    case "create":
      return `Create with ${action.tracks} tracks`;
    case "update":
      return `Add ${action.added}, remove ${action.removed}`;
    case "delete":
      return `Delete (${action.tracks} tracks)`;
    case "unchanged":
      return "Unchanged";
  }
}

function SeratoPlanView({ plan }: { plan: SeratoSyncPlan }) {
  const changed = plan.files.filter(
    (file) => file.action.action !== "unchanged",
  );
  return (
    <div className="plan">
      <h3 className="plan-title truncate" title={plan.serato_dir}>
        {plan.serato_dir}
      </h3>
      <p className="dim">
        Root crate <strong>{plan.root_crate}</strong> · {changed.length} of{" "}
        {plan.files.length} crate files change
      </p>
      <table className="data-table">
        <thead>
          <tr>
            <th scope="col">Crate file</th>
            <th scope="col">Change</th>
            <th scope="col" className="num">
              Tracks after
            </th>
          </tr>
        </thead>
        <tbody>
          {plan.files.map((file) => (
            <tr key={file.filename} className={`action-${file.action.action}`}>
              <td className="mono truncate" title={file.filename}>
                {file.filename}
              </td>
              <td>
                <span className={`action-badge action-${file.action.action}`}>
                  {actionText(file.action)}
                </span>
              </td>
              <td className="num">{file.track_count}</td>
            </tr>
          ))}
        </tbody>
      </table>
      {(plan.neworder.added.length > 0 || plan.neworder.removed.length > 0) && (
        <p>
          Crate order (neworder.pref):{" "}
          {plan.neworder.exists
            ? `${plan.neworder.added.length} added, ${plan.neworder.removed.length} removed`
            : "no neworder.pref, so Serato sorts crates itself"}
        </p>
      )}
      {plan.skipped_tracks.length > 0 && (
        <details>
          <summary>
            {plan.skipped_tracks.length} tracks skipped because they're on
            another drive
          </summary>
          <ul className="mono small">
            {plan.skipped_tracks.slice(0, 200).map((skipped) => (
              <li
                key={`${skipped.crate_name}:${skipped.path}`}
                title={skipped.path}
              >
                {skipped.crate_name}: {fileName(skipped.path)} (belongs to{" "}
                {skipped.serato_dir})
              </li>
            ))}
          </ul>
        </details>
      )}
      {plan.warnings.map((warning) => (
        <p key={warning} className="banner banner-warn">
          {warning}
        </p>
      ))}
    </div>
  );
}

function SeratoReportView({ report }: { report: SeratoSyncReport }) {
  return (
    <div className="report" role="status">
      {report.folders.map(({ serato_dir, report: r }) => (
        <p key={serato_dir}>
          <strong className="mono">{serato_dir}</strong>: {r.created.length}{" "}
          created, {r.updated.length} updated, {r.deleted.length} deleted,{" "}
          {r.unchanged} unchanged
          {r.neworder_written && ", crate order written"}
          {r.backup_dir && (
            <>
              . Backup in <span className="mono">{r.backup_dir}</span>
            </>
          )}
        </p>
      ))}
    </div>
  );
}

function SeratoSection({
  running,
  onApplied,
}: {
  running: RunningApps | null;
  onApplied: () => void;
}) {
  const [plans, setPlans] = useState<SeratoSyncPlan[] | null>(null);
  const [report, setReport] = useState<SeratoSyncReport | null>(null);
  const [busy, setBusy] = useState<"plan" | "apply" | null>(null);
  const seratoRunning = running?.serato ?? false;

  const preview = async () => {
    setBusy("plan");
    try {
      setPlans(await backend.planSeratoSync(null));
      setReport(null);
    } catch (error) {
      notifyError(`Couldn't preview the Serato sync: ${errorMessage(error)}`);
    } finally {
      setBusy(null);
    }
  };

  const apply = async () => {
    if (!plans) return;
    const ok = await confirmAction({
      title: "Write Serato crates?",
      body: `Overtone backs up each _Serato_ folder first, then writes ${plans.length} ${
        plans.length === 1 ? "folder" : "folders"
      } as previewed. Keep Serato closed until it finishes.`,
      confirmLabel: "Apply to Serato",
    });
    if (!ok) return;
    setBusy("apply");
    try {
      setReport(await backend.applySeratoSync(null));
      setPlans(null);
      notify("Serato crates written.");
      onApplied();
    } catch (error) {
      notifyError(`Serato sync failed: ${errorMessage(error)}`);
    } finally {
      setBusy(null);
    }
  };

  const noChanges =
    plans !== null &&
    plans.every(
      (plan) =>
        plan.files.every((file) => file.action.action === "unchanged") &&
        plan.neworder.added.length === 0 &&
        plan.neworder.removed.length === 0,
    );

  return (
    <SyncTargetCard
      title="Serato"
      app="serato"
      running={running}
      description="Each vibe becomes a crate under the Overtone crate; nested vibes become subcrates."
      actions={
        <>
          <button
            type="button"
            className="btn"
            disabled={busy !== null}
            onClick={() => void preview()}
          >
            {busy === "plan" ? "Previewing…" : "Preview"}
          </button>
          <button
            type="button"
            className="btn btn-primary"
            disabled={
              busy !== null || plans === null || seratoRunning || noChanges
            }
            title={
              seratoRunning
                ? "Quit Serato first"
                : plans === null
                  ? "Preview the changes first"
                  : undefined
            }
            onClick={() => void apply()}
          >
            {busy === "apply" ? "Applying…" : "Apply"}
          </button>
        </>
      }
    >
      {plans?.length === 0 && (
        <p className="dim">No _Serato_ folders found for your library.</p>
      )}
      {noChanges && plans && plans.length > 0 && (
        <p className="dim">Serato is already up to date.</p>
      )}
      {plans?.map((plan) => (
        <SeratoPlanView key={plan.serato_dir} plan={plan} />
      ))}
      {report && <SeratoReportView report={report} />}
    </SyncTargetCard>
  );
}

const PREVIEW_LIMIT = 300;

function MirrorReportView({ report }: { report: MirrorReport }) {
  return (
    <div className="report" role="status">
      <p>
        {report.written.length} written, {report.unchanged.length} unchanged,{" "}
        {report.failed.length} failed
      </p>
      {report.failed.length > 0 && (
        <ul className="small">
          {report.failed.map((failure) => (
            <li key={failure.track_id}>
              <span className="mono">{fileName(failure.path)}</span>:{" "}
              {failure.message}
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

function TagMirrorSection({ onApplied }: { onApplied: () => void }) {
  const [changes, setChanges] = useState<CommentChange[] | null>(null);
  const [report, setReport] = useState<MirrorReport | null>(null);
  const [busy, setBusy] = useState<"plan" | "apply" | "restore" | null>(null);

  const preview = async () => {
    setBusy("plan");
    try {
      setChanges(await backend.planTagMirror(null));
      setReport(null);
    } catch (error) {
      notifyError(`Couldn't preview Comment tags: ${errorMessage(error)}`);
    } finally {
      setBusy(null);
    }
  };

  const write = async (kind: "apply" | "restore") => {
    const ok = await confirmAction(
      kind === "apply"
        ? {
            title: "Write Comment tags?",
            body: `This rewrites the Comment tag in ${changes?.length ?? 0} audio files. Overtone keeps each original Comment so you can restore it.`,
            confirmLabel: "Write tags",
            danger: true,
          }
        : {
            title: "Restore original Comments?",
            body: "This writes back the Comment each file had before Overtone first changed it.",
            confirmLabel: "Restore",
            danger: true,
          },
    );
    if (!ok) return;
    setBusy(kind);
    try {
      const result =
        kind === "apply"
          ? await backend.applyTagMirror(null)
          : await backend.restoreComments();
      setReport(result);
      setChanges(null);
      onApplied();
      await reloadTracks();
    } catch (error) {
      notifyError(`Couldn't write Comment tags: ${errorMessage(error)}`);
    } finally {
      setBusy(null);
    }
  };

  return (
    <SyncTargetCard
      title="Comment tags"
      description="Optional: mirror vibes as hashtags such as #dubs/deep in each file's Comment, for smart crates and CDJ screens."
      actions={
        <>
          <button
            type="button"
            className="btn"
            disabled={busy !== null}
            onClick={() => void preview()}
          >
            {busy === "plan" ? "Previewing…" : "Preview"}
          </button>
          <button
            type="button"
            className="btn btn-danger"
            disabled={busy !== null || !changes || changes.length === 0}
            onClick={() => void write("apply")}
          >
            {busy === "apply" ? "Writing…" : "Apply"}
          </button>
          <button
            type="button"
            className="btn"
            disabled={busy !== null}
            onClick={() => void write("restore")}
          >
            {busy === "restore" ? "Restoring…" : "Restore originals"}
          </button>
        </>
      }
    >
      <p className="banner banner-warn">
        This writes to your audio files' Comment tags. Only Overtone's own
        #tokens change; the rest of each Comment stays as it is.
      </p>
      {changes?.length === 0 && (
        <p className="dim">Every Comment already matches your vibes.</p>
      )}
      {changes && changes.length > 0 && (
        <table className="data-table">
          <thead>
            <tr>
              <th scope="col">File</th>
              <th scope="col">Before</th>
              <th scope="col">After</th>
            </tr>
          </thead>
          <tbody>
            {changes.slice(0, PREVIEW_LIMIT).map((change) => (
              <tr key={change.track_id}>
                <td className="truncate" title={change.path}>
                  {fileName(change.path)}
                </td>
                <td className="mono">
                  {change.before ?? <span className="dim">(empty)</span>}
                </td>
                <td className="mono">
                  {change.after ?? <span className="dim">(empty)</span>}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
      {changes && changes.length > PREVIEW_LIMIT && (
        <p className="dim">And {changes.length - PREVIEW_LIMIT} more files.</p>
      )}
      {report && <MirrorReportView report={report} />}
    </SyncTargetCard>
  );
}

/** Writes vibes as playlists into a rekordbox.xml that Rekordbox imports. */
export function RekordboxSection({
  running,
  onApplied,
}: {
  running: RunningApps | null;
  onApplied: () => void;
}) {
  const [plan, setPlan] = useState<RekordboxExportPlan | null>(null);
  const [report, setReport] = useState<RekordboxExportReport | null>(null);
  const [busy, setBusy] = useState<"plan" | "export" | null>(null);

  const preview = async () => {
    setBusy("plan");
    try {
      setPlan(await backend.planRekordboxExport(null));
      setReport(null);
    } catch (error) {
      notifyError(
        `Couldn't preview the Rekordbox export: ${errorMessage(error)}`,
      );
    } finally {
      setBusy(null);
    }
  };

  const exportXml = async () => {
    if (!plan) return;
    const ok = await confirmAction({
      title: "Write rekordbox.xml?",
      body: `Overtone writes ${plan.playlist_count} playlists with ${plan.track_count} tracks to\n${plan.path}${
        plan.exists ? "\n\nThe current file is backed up first." : ""
      }`,
      confirmLabel: "Export",
    });
    if (!ok) return;
    setBusy("export");
    try {
      setReport(await backend.exportRekordboxXml(null));
      setPlan(null);
      onApplied();
    } catch (error) {
      notifyError(`Rekordbox export failed: ${errorMessage(error)}`);
    } finally {
      setBusy(null);
    }
  };

  return (
    <SyncTargetCard
      title="Rekordbox"
      app="rekordbox"
      running={running}
      runningNote="Rekordbox is running. You can still export; import the file again in Rekordbox afterwards to see the changes."
      description="Each vibe becomes a playlist under an Overtone folder in a rekordbox.xml file that Rekordbox imports."
      actions={
        <>
          <button
            type="button"
            className="btn"
            disabled={busy !== null}
            onClick={() => void preview()}
          >
            {busy === "plan" ? "Previewing…" : "Preview"}
          </button>
          <button
            type="button"
            className="btn btn-primary"
            disabled={busy !== null || plan === null || !plan.changed}
            title={plan === null ? "Preview the export first" : undefined}
            onClick={() => void exportXml()}
          >
            {busy === "export" ? "Exporting…" : "Export"}
          </button>
        </>
      }
    >
      <p className="dim">
        In Rekordbox, open Preferences &gt; Advanced &gt; Database and set
        rekordbox xml &gt; Imported Library to this file, then import the
        Overtone playlists from the rekordbox xml section of the sidebar. Import
        again after each export.
      </p>
      {plan && (
        <div className="plan">
          <h3 className="plan-title truncate" title={plan.path}>
            {plan.path}
          </h3>
          <p className="dim num">
            {plan.playlist_count} playlists, {plan.track_count} tracks ·{" "}
            {!plan.exists
              ? "new file"
              : plan.changed
                ? "differs from the current file"
                : "already up to date"}
          </p>
          {plan.playlists.length > 0 && (
            <table className="data-table">
              <thead>
                <tr>
                  <th scope="col">Playlist</th>
                  <th scope="col" className="num">
                    Tracks
                  </th>
                </tr>
              </thead>
              <tbody>
                {plan.playlists.map((playlist) => (
                  <tr key={playlist.path.join("/")}>
                    <td className="truncate" title={playlist.path.join(" / ")}>
                      {playlist.path.join(" / ")}
                    </td>
                    <td className="num">{playlist.track_count}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </div>
      )}
      {report && (
        <div className="report" role="status">
          <p>
            {report.changed
              ? `Wrote ${report.playlist_count} playlists with ${report.track_count} tracks to `
              : "Nothing changed in "}
            <span className="mono">{report.path}</span>
            {report.backup_path && (
              <>
                . Backup in <span className="mono">{report.backup_path}</span>
              </>
            )}
          </p>
          {report.changed && report.rekordbox_running && (
            <p>
              Rekordbox is running: import the file again to pick up the
              changes.
            </p>
          )}
        </div>
      )}
    </SyncTargetCard>
  );
}

function SyncHistory({ runs }: { runs: SyncRun[] | null }) {
  return (
    <section className="card" aria-labelledby="sync-history">
      <h2 id="sync-history">History</h2>
      {runs === null ? (
        <p className="dim">Loading history…</p>
      ) : runs.length === 0 ? (
        <p className="dim">No syncs yet.</p>
      ) : (
        <table className="data-table">
          <thead>
            <tr>
              <th scope="col">When</th>
              <th scope="col">App</th>
              <th scope="col">Target</th>
              <th scope="col">Result</th>
              <th scope="col">Backup</th>
            </tr>
          </thead>
          <tbody>
            {runs.map((run) => (
              <tr key={run.id}>
                <td className="num">{formatDateTime(run.started_at)}</td>
                <td>{APP_NAMES[run.app]}</td>
                <td className="mono truncate" title={run.target}>
                  {run.target}
                </td>
                <td className={`status-${run.status}`}>
                  {run.status === "succeeded"
                    ? "Succeeded"
                    : run.status === "failed"
                      ? "Failed"
                      : "Running"}
                  {run.dry_run && " (preview)"}
                </td>
                <td className="mono truncate" title={run.backup_path ?? ""}>
                  {run.backup_path ?? ""}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </section>
  );
}

export function SyncView() {
  const visible = useUi((s) => s.view === "sync");
  const [running, setRunning] = useState<RunningApps | null>(null);
  const [runs, setRuns] = useState<SyncRun[] | null>(null);

  const refresh = useCallback(() => {
    backend
      .djAppsRunning()
      .then(setRunning, (error: unknown) =>
        notifyError(
          `Couldn't check which DJ apps are running: ${errorMessage(error)}`,
        ),
      );
    backend.syncHistory(20).then(setRuns, () => setRuns([]));
  }, []);

  useEffect(() => {
    if (visible) refresh();
  }, [visible, refresh]);

  return (
    <div className="view scroll-view">
      <header className="view-header">
        <h1 className="view-title">Sync</h1>
        <RunningIndicators running={running} onRefresh={refresh} />
      </header>
      <p className="dim lead">
        Preview what changes in each DJ app, then apply. Overtone backs up every
        file before it writes.
      </p>
      <SeratoSection running={running} onApplied={refresh} />
      <RekordboxSection running={running} onApplied={refresh} />
      <TagMirrorSection onApplied={refresh} />
      <SyncHistory runs={runs} />
    </div>
  );
}
