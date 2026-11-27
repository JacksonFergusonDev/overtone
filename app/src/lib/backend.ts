import { convertFileSrc } from "@tauri-apps/api/core";
import * as api from "../api";
import type { AppStatus } from "../bindings/AppStatus";
import type { AuditProgress } from "../bindings/AuditProgress";
import type { AuditReport } from "../bindings/AuditReport";
import type { CommentChange } from "../bindings/CommentChange";
import type { LibraryRoot } from "../bindings/LibraryRoot";
import type { MirrorReport } from "../bindings/MirrorReport";
import type { ReviewStatus } from "../bindings/ReviewStatus";
import type { RunningApps } from "../bindings/RunningApps";
import type { ScanProgress } from "../bindings/ScanProgress";
import type { ScanReport } from "../bindings/ScanReport";
import type { SeratoSyncPlan } from "../bindings/SeratoSyncPlan";
import type { SeratoSyncReport } from "../bindings/SeratoSyncReport";
import type { SpectrogramImage } from "../bindings/SpectrogramImage";
import type { SyncRun } from "../bindings/SyncRun";
import type { Track } from "../bindings/Track";
import type { TrackId } from "../bindings/TrackId";
import type { Vibe } from "../bindings/Vibe";
import type { VibeId } from "../bindings/VibeId";
import type { WaveformPeaks } from "../bindings/WaveformPeaks";

/** Human-readable text for anything a command rejects with. */
export const errorMessage: (error: unknown) => string = api.errorMessage;

/** The command surface the UI relies on (see engineering/plans/desktop-ui.md). */
export interface Backend {
  appStatus(): Promise<AppStatus>;
  listTracks(): Promise<Track[]>;
  listVibes(): Promise<Vibe[]>;
  listRoots(): Promise<LibraryRoot[]>;
  addRoot(path: string): Promise<LibraryRoot>;
  removeRoot(id: number): Promise<null>;
  scanLibrary(
    refresh: boolean,
    onProgress: (progress: ScanProgress) => void,
  ): Promise<ScanReport>;
  auditTracks(
    trackIds: TrackId[] | null,
    onProgress: (progress: AuditProgress) => void,
  ): Promise<AuditReport>;
  cancelAudit(): Promise<null>;
  setReview(trackId: TrackId, review: ReviewStatus): Promise<Track>;
  trackWaveform(trackId: TrackId): Promise<WaveformPeaks | null>;
  renderSpectrogram(trackId: TrackId, log: boolean): Promise<SpectrogramImage>;
  createVibe(path: string): Promise<Vibe>;
  renameVibe(vibeId: VibeId, newPath: string): Promise<Vibe>;
  deleteVibe(vibeId: VibeId): Promise<null>;
  assignVibe(trackIds: TrackId[], vibeId: VibeId): Promise<null>;
  unassignVibe(trackIds: TrackId[], vibeId: VibeId): Promise<null>;
  djAppsRunning(): Promise<RunningApps>;
  planSeratoSync(seratoDir: string | null): Promise<SeratoSyncPlan[]>;
  applySeratoSync(seratoDir: string | null): Promise<SeratoSyncReport>;
  planTagMirror(vibe: string | null): Promise<CommentChange[]>;
  applyTagMirror(vibe: string | null): Promise<MirrorReport>;
  restoreComments(): Promise<MirrorReport>;
  syncHistory(limit: number): Promise<SyncRun[]>;
}

interface Helpers {
  audioSrc?: (path: string) => string;
  imageSrc?: (path: string) => string;
}

// Remove the casts once api.ts lands with the full command set.
export const backend = api.commands as unknown as Backend;
const helpers = api as unknown as Helpers;

/** URL the `<audio>` element plays a library file from. */
export function audioSrc(path: string): string {
  return helpers.audioSrc ? helpers.audioSrc(path) : convertFileSrc(path);
}

/** URL for a cached image such as a spectrogram PNG. */
export function imageSrc(path: string): string {
  return helpers.imageSrc ? helpers.imageSrc(path) : convertFileSrc(path);
}
