import { create } from "zustand";
import { backend, errorMessage } from "../lib/backend";
import type {
  AuditProgress,
  AuditReport,
  ScanProgress,
  ScanReport,
  TrackId,
} from "../lib/types";
import { reloadTracks } from "./library";
import { notify, notifyError } from "./toasts";

export interface JobState<Progress, Report> {
  running: boolean;
  progress: Progress | null;
  report: Report | null;
  error: string | null;
}

/** A lookup, integrity check, export, or conversion (see `store/details`). */
export interface BackgroundJob {
  kind: "lookup" | "integrity" | "export" | "convert" | "write" | null;
  running: boolean;
  done: number;
  total: number;
  cancelling: boolean;
}

export interface JobsState {
  scan: JobState<ScanProgress, ScanReport>;
  audit: JobState<AuditProgress, AuditReport> & { cancelling: boolean };
  /** Shares the backend's job slot with the audit: one runs at a time. */
  background: BackgroundJob;
}

const idleBackground: BackgroundJob = {
  kind: null,
  running: false,
  done: 0,
  total: 0,
  cancelling: false,
};

const idle = { running: false, progress: null, report: null, error: null };

export const useJobs = create<JobsState>(() => ({
  scan: idle,
  audit: { ...idle, cancelling: false },
  background: idleBackground,
}));

export function resetJobs(): void {
  useJobs.setState({
    scan: idle,
    audit: { ...idle, cancelling: false },
    background: idleBackground,
  });
}

/** Fraction done from 0 to 1, or null while the total isn't known yet. */
export function scanFraction(progress: ScanProgress | null): number | null {
  if (!progress || progress.phase === "walking") return null;
  // Processing is the long phase; saving finishes the last tenth.
  const part = progress.total > 0 ? progress.done / progress.total : 1;
  return progress.phase === "processing" ? part * 0.9 : 0.9 + part * 0.1;
}

export function scanLabel(progress: ScanProgress | null): string {
  if (!progress) return "Starting scan…";
  switch (progress.phase) {
    case "walking":
      return `Finding files… ${progress.found.toLocaleString()} found`;
    case "processing":
      return `Reading tags ${progress.done.toLocaleString()} of ${progress.total.toLocaleString()}`;
    case "saving":
      return `Saving ${progress.done.toLocaleString()} of ${progress.total.toLocaleString()}`;
  }
}

export async function runScan(refresh: boolean): Promise<void> {
  if (useJobs.getState().scan.running) return;
  useJobs.setState({
    scan: { running: true, progress: null, report: null, error: null },
  });
  try {
    const report = await backend.scanLibrary(refresh, (progress) => {
      useJobs.setState((state) => ({ scan: { ...state.scan, progress } }));
    });
    useJobs.setState({
      scan: { running: false, progress: null, report, error: null },
    });
    notify(
      `Scan finished: ${report.added} added, ${report.updated} updated, ${report.missing} missing`,
    );
  } catch (error) {
    const message = errorMessage(error);
    useJobs.setState({
      scan: { running: false, progress: null, report: null, error: message },
    });
    notifyError(`Scan failed: ${message}`);
  }
  await reloadTracks();
}

/** Audits the given tracks, or every pending track for null. */
export async function runAudit(trackIds: TrackId[] | null): Promise<void> {
  if (useJobs.getState().audit.running || useJobs.getState().background.running)
    return;
  useJobs.setState({
    audit: {
      running: true,
      cancelling: false,
      progress: null,
      report: null,
      error: null,
    },
  });
  try {
    const report = await backend.auditTracks(trackIds, (progress) => {
      useJobs.setState((state) => ({ audit: { ...state.audit, progress } }));
    });
    useJobs.setState({
      audit: {
        running: false,
        cancelling: false,
        progress: null,
        report,
        error: null,
      },
    });
    notify(
      `Audit finished: ${report.analyzed} analyzed, ${report.flagged.length} flagged`,
    );
  } catch (error) {
    const message = errorMessage(error);
    useJobs.setState({
      audit: {
        running: false,
        cancelling: false,
        progress: null,
        report: null,
        error: message,
      },
    });
    notifyError(`Audit failed: ${message}`);
  }
  await reloadTracks();
}

export async function cancelAudit(): Promise<void> {
  useJobs.setState((state) => ({
    audit: { ...state.audit, cancelling: true },
  }));
  try {
    await backend.cancelAudit();
  } catch (error) {
    notifyError(`Couldn't cancel the audit: ${errorMessage(error)}`);
    useJobs.setState((state) => ({
      audit: { ...state.audit, cancelling: false },
    }));
  }
}
