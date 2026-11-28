import { create } from "zustand";
import { backend, errorMessage } from "../lib/backend";
import type {
  ExportSettings,
  MetadataField,
  MetadataSuggestion,
  TrackDetails,
  TrackId,
} from "../lib/types";
import { useJobs } from "./jobs";
import { reloadTracks } from "./library";
import { notify, notifyError } from "./toasts";

/**
 * Per-track details (field sources, suggestions, loudness, integrity,
 * conversion) and the background jobs that change them: lookups,
 * integrity checks, exports, and conversions. Details are cached by track
 * so the inspector draws the last known values at once.
 */
interface DetailsState {
  details: ReadonlyMap<TrackId, TrackDetails>;
}

export const useDetails = create<DetailsState>(() => ({ details: new Map() }));

export function resetDetails(): void {
  useDetails.setState({ details: new Map() });
}

export async function loadDetails(trackId: TrackId): Promise<void> {
  try {
    const found = await backend.trackDetails(trackId);
    useDetails.setState((s) => {
      const next = new Map(s.details);
      next.set(trackId, found);
      return { details: next };
    });
  } catch (error) {
    notifyError(`Couldn't load track details: ${errorMessage(error)}`);
  }
}

/** Reloads every cached track's details, keeping the old ones on screen
 * until the new ones arrive. */
async function refreshCached(): Promise<void> {
  const ids = [...useDetails.getState().details.keys()];
  await Promise.all(ids.map((id) => loadDetails(id)));
}

export async function setField(
  trackId: TrackId,
  field: MetadataField,
  value: string | null,
): Promise<void> {
  try {
    await backend.setTrackField(trackId, field, value);
  } catch (error) {
    notifyError(`Couldn't save ${field}: ${errorMessage(error)}`);
  }
  await loadDetails(trackId);
}

export async function resetField(
  trackId: TrackId,
  field: MetadataField,
): Promise<void> {
  try {
    await backend.resetTrackField(trackId, field);
  } catch (error) {
    notifyError(`Couldn't reset ${field}: ${errorMessage(error)}`);
  }
  await loadDetails(trackId);
}

export async function decideSuggestion(
  suggestion: MetadataSuggestion,
  decision: "accept" | "replace" | "reject",
): Promise<void> {
  try {
    if (decision === "reject") await backend.rejectSuggestion(suggestion.id);
    else await backend.acceptSuggestion(suggestion.id, decision === "replace");
  } catch (error) {
    notifyError(errorMessage(error));
  }
  await loadDetails(suggestion.track_id);
}

export type BackgroundKind = "lookup" | "integrity" | "export" | "convert";

const LABEL: Record<BackgroundKind, string> = {
  lookup: "Looking up",
  integrity: "Checking files",
  export: "Exporting",
  convert: "Converting",
};

/** The running background job's progress line, or null. */
export function backgroundLabel(): string | null {
  const job = useJobs.getState().background;
  if (!job.running || !job.kind) return null;
  const count = job.total > 0 ? ` ${job.done} of ${job.total}` : "…";
  return `${LABEL[job.kind]}${count}`;
}

async function run<T>(
  kind: BackgroundKind,
  work: (
    onProgress: (p: { done: number; total: number }) => void,
  ) => Promise<T>,
  summary: (report: T) => string,
): Promise<T | null> {
  const jobs = useJobs.getState();
  if (jobs.background.running || jobs.audit.running) {
    notifyError("Another background job is running; wait for it or cancel it.");
    return null;
  }
  useJobs.setState({
    background: { kind, running: true, done: 0, total: 0, cancelling: false },
  });
  try {
    const report = await work((p) =>
      useJobs.setState((s) => ({
        background: { ...s.background, done: p.done, total: p.total },
      })),
    );
    notify(summary(report));
    return report;
  } catch (error) {
    notifyError(`${LABEL[kind]} failed: ${errorMessage(error)}`);
    return null;
  } finally {
    useJobs.setState({
      background: {
        kind: null,
        running: false,
        done: 0,
        total: 0,
        cancelling: false,
      },
    });
    await reloadTracks();
    await refreshCached();
  }
}

export const runLookup = (trackIds: TrackId[] | null) =>
  run(
    "lookup",
    (onProgress) => backend.lookupTracks(trackIds, onProgress),
    (r) =>
      `Looked up ${r.looked_up} track(s): ${r.matched} matched, ${r.suggestions_added} suggestion(s) to review`,
  );

export const runIntegrity = (trackIds: TrackId[] | null) =>
  run(
    "integrity",
    (onProgress) => backend.checkIntegrity(trackIds, onProgress),
    (r) =>
      `Checked ${r.checked} file(s): ${r.damaged.length} with errors in the music, ${r.edge_only} with harmless junk at the edges`,
  );

export const runExport = (
  trackIds: TrackId[],
  folder: string,
  settings: ExportSettings,
) =>
  run(
    "export",
    (onProgress) =>
      backend.exportTracks(trackIds, folder, settings, onProgress),
    (r) =>
      `Exported ${r.exported.length} copy(ies)${r.failures.length ? `, ${r.failures.length} failed` : ""}`,
  );

export const runConvert = (trackIds: TrackId[] | null) =>
  run(
    "convert",
    (onProgress) => backend.convertTracks(trackIds, onProgress),
    (r) =>
      `Converted and verified ${r.converted.length} track(s) to FLAC${r.failures.length ? `, ${r.failures.length} failed` : ""}`,
  );

export async function cancelBackground(): Promise<void> {
  useJobs.setState((s) => ({
    background: { ...s.background, cancelling: true },
  }));
  try {
    await backend.cancelAudit();
  } catch (error) {
    notifyError(`Couldn't cancel: ${errorMessage(error)}`);
  }
}
