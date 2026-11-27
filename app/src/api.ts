import { Channel, convertFileSrc, invoke } from "@tauri-apps/api/core";
import type { AppStatus } from "./bindings/AppStatus";
import type { AuditProgress } from "./bindings/AuditProgress";
import type { AuditReport } from "./bindings/AuditReport";
import type { CommandError } from "./bindings/CommandError";
import type { CommentChange } from "./bindings/CommentChange";
import type { LibraryRoot } from "./bindings/LibraryRoot";
import type { MirrorReport } from "./bindings/MirrorReport";
import type { RekordboxExportPlan } from "./bindings/RekordboxExportPlan";
import type { RekordboxExportReport } from "./bindings/RekordboxExportReport";
import type { ReviewStatus } from "./bindings/ReviewStatus";
import type { RunningApps } from "./bindings/RunningApps";
import type { ScanProgress } from "./bindings/ScanProgress";
import type { ScanReport } from "./bindings/ScanReport";
import type { SeratoSyncPlan } from "./bindings/SeratoSyncPlan";
import type { SeratoSyncReport } from "./bindings/SeratoSyncReport";
import type { SpectrogramImage } from "./bindings/SpectrogramImage";
import type { SyncRun } from "./bindings/SyncRun";
import type { Track } from "./bindings/Track";
import type { TrackId } from "./bindings/TrackId";
import type { Vibe } from "./bindings/Vibe";
import type { VibeId } from "./bindings/VibeId";
import type { WaveformPeaks } from "./bindings/WaveformPeaks";

export type {
  AppStatus,
  AuditProgress,
  AuditReport,
  CommandError,
  CommentChange,
  LibraryRoot,
  MirrorReport,
  RekordboxExportPlan,
  RekordboxExportReport,
  ReviewStatus,
  RunningApps,
  ScanProgress,
  ScanReport,
  SeratoSyncPlan,
  SeratoSyncReport,
  SpectrogramImage,
  SyncRun,
  Track,
  TrackId,
  Vibe,
  VibeId,
  WaveformPeaks,
};
export type { AudioFormat } from "./bindings/AudioFormat";
export type { AudioProperties } from "./bindings/AudioProperties";
export type { DjApp } from "./bindings/DjApp";
export type { LibrarySummary } from "./bindings/LibrarySummary";
export type { MirrorFailure } from "./bindings/MirrorFailure";
export type { QualityReport } from "./bindings/QualityReport";
export type { QualityVerdict } from "./bindings/QualityVerdict";
export type { RekordboxPlaylistSummary } from "./bindings/RekordboxPlaylistSummary";
export type { ScanFailure } from "./bindings/ScanFailure";
export type { SeratoFilePlan } from "./bindings/SeratoFilePlan";
export type { SeratoFileAction } from "./bindings/SeratoFileAction";
export type { SeratoFolderReport } from "./bindings/SeratoFolderReport";
export type { SyncStatus } from "./bindings/SyncStatus";
export type { TrackTags } from "./bindings/TrackTags";
export type { VerdictCounts } from "./bindings/VerdictCounts";
export type { VibePath } from "./bindings/VibePath";

/** Whether a rejected `invoke` value is the backend's structured error. */
export function isCommandError(value: unknown): value is CommandError {
  return (
    typeof value === "object" &&
    value !== null &&
    typeof (value as CommandError).code === "string" &&
    typeof (value as CommandError).message === "string"
  );
}

/** Human-readable text for anything an `invoke` call rejects with. */
export function errorMessage(error: unknown): string {
  if (isCommandError(error)) return error.message;
  if (error instanceof Error) return error.message;
  return String(error);
}

/** Path prefix the mock backend uses for files that don't exist. */
export const MOCK_PATH_PREFIX = "mock://";

/**
 * A URL an `<audio>` element can play for a library file. In the mock
 * backend (`mock://...` paths) this is a short silent WAV data URL.
 */
export function audioSrc(path: string): string {
  if (path.startsWith(MOCK_PATH_PREFIX)) return MOCK_SILENT_WAV;
  return convertFileSrc(path);
}

/**
 * A URL an `<img>` can show for an image path returned by the backend, such
 * as `SpectrogramImage.path`. Paths starting with `mock://spectrogram` come
 * from the mock backend and become a generated gradient data URL.
 */
export function imageSrc(path: string): string {
  if (path.startsWith(MOCK_PATH_PREFIX)) return mockSpectrogramDataUrl(path);
  return convertFileSrc(path);
}

/** Half a second of 8 kHz mono 8-bit silence. */
const MOCK_SILENT_WAV = (() => {
  const samples = 4000;
  const bytes = new Uint8Array(44 + samples).fill(128);
  const view = new DataView(bytes.buffer);
  const text = (offset: number, value: string) => {
    for (let i = 0; i < value.length; i += 1) {
      bytes[offset + i] = value.charCodeAt(i);
    }
  };
  text(0, "RIFF");
  view.setUint32(4, 36 + samples, true);
  text(8, "WAVE");
  text(12, "fmt ");
  view.setUint32(16, 16, true);
  view.setUint16(20, 1, true); // PCM
  view.setUint16(22, 1, true); // mono
  view.setUint32(24, 8000, true);
  view.setUint32(28, 8000, true);
  view.setUint16(32, 1, true);
  view.setUint16(34, 8, true);
  text(36, "data");
  view.setUint32(40, samples, true);
  let binary = "";
  for (const byte of bytes) binary += String.fromCharCode(byte);
  return `data:audio/wav;base64,${btoa(binary)}`;
})();

function mockSpectrogramDataUrl(path: string): string {
  // `mock://spectrogram/<id>-<lin|log>?cutoff=<hz>` draws a dark image whose
  // energy stops at the cutoff, like a real lossy transcode.
  const cutoff = Number(/cutoff=(\d+)/.exec(path)?.[1] ?? 22050);
  const edge = Math.max(0, Math.min(1, 1 - cutoff / 22050)) * 100;
  const svg =
    `<svg xmlns="http://www.w3.org/2000/svg" width="1200" height="400" ` +
    `preserveAspectRatio="none"><defs><linearGradient id="g" x1="0" y1="0" x2="0" y2="1">` +
    `<stop offset="0" stop-color="#000"/><stop offset="${edge}%" stop-color="#1b1b3a"/>` +
    `<stop offset="${Math.min(100, edge + 5)}%" stop-color="#7a2a7a"/>` +
    `<stop offset="100%" stop-color="#f4c542"/></linearGradient></defs>` +
    `<rect width="1200" height="400" fill="url(#g)"/></svg>`;
  return `data:image/svg+xml;utf8,${encodeURIComponent(svg)}`;
}

/** A progress channel; the backend streams messages to `onMessage`. */
function channel<T>(onMessage?: (message: T) => void): Channel<T> {
  const ch = new Channel<T>();
  if (onMessage) ch.onmessage = onMessage;
  return ch;
}

/**
 * Typed wrappers around Tauri commands. Argument and result types come from
 * the Rust structs via ts-rs (`src/bindings/`, regenerated by `cargo test`).
 * Every promise rejects with a `CommandError` (see `isCommandError`).
 */
export const commands = {
  appStatus: () => invoke<AppStatus>("app_status"),

  /** Every track in the library, including missing ones. */
  listTracks: () => invoke<Track[]>("list_tracks"),
  listVibes: () => invoke<Vibe[]>("list_vibes"),
  listRoots: () => invoke<LibraryRoot[]>("list_roots"),
  /** Also lets the webview read audio inside the folder. */
  addRoot: (path: string) => invoke<LibraryRoot>("add_root", { path }),
  /** Stops indexing a folder; its tracks stay in the library. */
  removeRoot: (id: number) => invoke<null>("remove_root", { id }),

  /** Scans every library folder. `refresh` rereads tags of unchanged files. */
  scanLibrary: (
    refresh: boolean,
    onProgress?: (progress: ScanProgress) => void,
  ) =>
    invoke<ScanReport>("scan_library", {
      refresh,
      onProgress: channel(onProgress),
    }),

  /**
   * Analyzes the given tracks, or every track still pending when `trackIds`
   * is null. Only one audit runs at a time (rejects with code `busy`).
   */
  auditTracks: (
    trackIds: TrackId[] | null,
    onProgress?: (progress: AuditProgress) => void,
  ) =>
    invoke<AuditReport>("audit_tracks", {
      trackIds,
      onProgress: channel(onProgress),
    }),
  /** Stops the running audit after the tracks in flight finish. */
  cancelAudit: () => invoke<null>("cancel_audit"),
  setReview: (trackId: TrackId, review: ReviewStatus) =>
    invoke<Track>("set_review", { trackId, review }),
  /** `null` until the track has been analyzed. */
  trackWaveform: (trackId: TrackId) =>
    invoke<WaveformPeaks | null>("track_waveform", { trackId }),
  /** Pass `path` to `imageSrc`. */
  renderSpectrogram: (trackId: TrackId, log: boolean) =>
    invoke<SpectrogramImage>("render_spectrogram", { trackId, log }),

  /** Nested vibes use `/`, for example `Dubs/Deep`. */
  createVibe: (path: string) => invoke<Vibe>("create_vibe", { path }),
  renameVibe: (vibeId: VibeId, newPath: string) =>
    invoke<Vibe>("rename_vibe", { vibeId, newPath }),
  deleteVibe: (vibeId: VibeId) => invoke<null>("delete_vibe", { vibeId }),
  assignVibe: (trackIds: TrackId[], vibeId: VibeId) =>
    invoke<null>("assign_vibe", { trackIds, vibeId }),
  unassignVibe: (trackIds: TrackId[], vibeId: VibeId) =>
    invoke<null>("unassign_vibe", { trackIds, vibeId }),

  djAppsRunning: () => invoke<RunningApps>("dj_apps_running"),
  /** `null` plans into each volume's own `_Serato_` folder. */
  planSeratoSync: (seratoDir: string | null) =>
    invoke<SeratoSyncPlan[]>("plan_serato_sync", { seratoDir }),
  applySeratoSync: (seratoDir: string | null) =>
    invoke<SeratoSyncReport>("apply_serato_sync", { seratoDir }),
  /** `null` uses `rekordbox.xml` in Overtone's data folder. */
  planRekordboxExport: (path: string | null) =>
    invoke<RekordboxExportPlan>("plan_rekordbox_export", { path }),
  exportRekordboxXml: (path: string | null) =>
    invoke<RekordboxExportReport>("export_rekordbox_xml", { path }),
  /** `vibe` limits the mirror to one vibe; `null` covers every track. */
  planTagMirror: (vibe: string | null) =>
    invoke<CommentChange[]>("plan_tag_mirror", { vibe }),
  applyTagMirror: (vibe: string | null) =>
    invoke<MirrorReport>("apply_tag_mirror", { vibe }),
  restoreComments: () => invoke<MirrorReport>("restore_comments"),
  syncHistory: (limit: number) => invoke<SyncRun[]>("sync_history", { limit }),
};
