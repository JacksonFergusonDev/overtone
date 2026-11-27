import { create } from "zustand";
import { backend, errorMessage } from "../lib/backend";
import type {
  AppStatus,
  ReviewStatus,
  SpectrogramImage,
  Track,
  TrackId,
  Vibe,
  VibeId,
  WaveformPeaks,
} from "../lib/types";
import { vibeWithDescendants } from "../lib/vibes";
import { notifyError } from "./toasts";

export type LoadState = "idle" | "loading" | "ready" | "error";

export interface LibraryState {
  load: LoadState;
  /** Why the library couldn't open; shown as a full-window error. */
  error: string | null;
  status: AppStatus | null;
  /** Tracks by id. Replaced (never mutated) so row components can memoize. */
  tracks: ReadonlyMap<TrackId, Track>;
  /** Ids in the order the backend listed them. */
  order: readonly TrackId[];
  vibes: readonly Vibe[];
  /** Waveform peaks by track; null when the backend has none yet. */
  waveforms: ReadonlyMap<TrackId, WaveformPeaks | null>;
  /** Rendered spectrograms keyed by `${trackId}:${log}`. */
  spectrograms: ReadonlyMap<string, SpectrogramImage>;
}

const initialState: LibraryState = {
  load: "idle",
  error: null,
  status: null,
  tracks: new Map(),
  order: [],
  vibes: [],
  waveforms: new Map(),
  spectrograms: new Map(),
};

export const useLibrary = create<LibraryState>(() => initialState);

/** Test helper: back to an empty, unloaded library. */
export function resetLibrary(): void {
  useLibrary.setState(initialState, true);
}

function indexTracks(
  list: readonly Track[],
): Pick<LibraryState, "tracks" | "order"> {
  const tracks = new Map<TrackId, Track>();
  const order: TrackId[] = [];
  for (const track of list) {
    tracks.set(track.id, track);
    order.push(track.id);
  }
  return { tracks, order };
}

/** Loads status, tracks, and vibes once at startup, in parallel. */
export async function loadLibrary(): Promise<void> {
  if (useLibrary.getState().load !== "idle") return;
  useLibrary.setState({ load: "loading" });
  try {
    const [status, tracks, vibes] = await Promise.all([
      backend.appStatus(),
      backend.listTracks(),
      backend.listVibes(),
    ]);
    useLibrary.setState({
      load: "ready",
      status,
      vibes,
      ...indexTracks(tracks),
      error: null,
    });
  } catch (error) {
    useLibrary.setState({ load: "error", error: errorMessage(error) });
  }
}

/** Re-reads every track and the status, for example after a scan or audit. */
export async function reloadTracks(): Promise<void> {
  try {
    const [status, tracks, vibes] = await Promise.all([
      backend.appStatus(),
      backend.listTracks(),
      backend.listVibes(),
    ]);
    // An audit may have produced new reports, so drop cached spectrograms.
    useLibrary.setState({
      status,
      vibes,
      ...indexTracks(tracks),
      spectrograms: new Map(),
    });
  } catch (error) {
    notifyError(`Couldn't reload the library: ${errorMessage(error)}`);
  }
}

export async function reloadVibes(): Promise<void> {
  const vibes = await backend.listVibes();
  useLibrary.setState({ vibes });
}

export function replaceTrack(track: Track): void {
  useLibrary.setState((state) => {
    const tracks = new Map(state.tracks);
    tracks.set(track.id, track);
    return { tracks };
  });
}

function patchTracks(
  ids: readonly TrackId[],
  patch: (track: Track) => Track,
): void {
  useLibrary.setState((state) => {
    const tracks = new Map(state.tracks);
    for (const id of ids) {
      const track = tracks.get(id);
      if (track) tracks.set(id, patch(track));
    }
    return { tracks };
  });
}

/**
 * Toggles a vibe on tracks: assigns it when any of them lacks it, otherwise
 * removes it. Updates the store first and rolls back if the backend refuses.
 * Returns whether the vibe was assigned.
 */
export async function toggleVibe(
  trackIds: readonly TrackId[],
  vibeId: VibeId,
): Promise<boolean> {
  const { tracks } = useLibrary.getState();
  const targets = trackIds.filter((id) => tracks.has(id));
  if (targets.length === 0) return false;
  const assign = targets.some((id) => !tracks.get(id)?.vibes.includes(vibeId));
  const before = new Map(targets.map((id) => [id, tracks.get(id) as Track]));
  patchTracks(targets, (track) => ({
    ...track,
    vibes: assign
      ? track.vibes.includes(vibeId)
        ? track.vibes
        : [...track.vibes, vibeId]
      : track.vibes.filter((id) => id !== vibeId),
  }));
  try {
    if (assign) await backend.assignVibe(targets, vibeId);
    else await backend.unassignVibe(targets, vibeId);
  } catch (error) {
    // Put back the exact previous objects, unless something newer replaced them.
    useLibrary.setState((state) => {
      const next = new Map(state.tracks);
      for (const [id, track] of before) {
        const current = next.get(id);
        if (current) next.set(id, { ...current, vibes: track.vibes });
      }
      return { tracks: next };
    });
    notifyError(
      `Couldn't ${assign ? "add" : "remove"} the vibe: ${errorMessage(error)}`,
    );
    throw error;
  }
  return assign;
}

/** Creates a vibe (and any missing parents) and returns it. */
export async function createVibe(path: string): Promise<Vibe> {
  const vibe = await backend.createVibe(path);
  // Parents may have been created too, so re-list.
  try {
    await reloadVibes();
  } catch {
    useLibrary.setState((state) => ({
      vibes: state.vibes.some((v) => v.id === vibe.id)
        ? state.vibes
        : [...state.vibes, vibe],
    }));
  }
  return vibe;
}

export async function renameVibe(
  vibeId: VibeId,
  newPath: string,
): Promise<Vibe> {
  const vibe = await backend.renameVibe(vibeId, newPath);
  await reloadVibes();
  return vibe;
}

/** Deletes a vibe and the vibes nested below it. Tracks are not touched. */
export async function deleteVibe(vibeId: VibeId): Promise<void> {
  const removed = vibeWithDescendants(useLibrary.getState().vibes, vibeId);
  await backend.deleteVibe(vibeId);
  useLibrary.setState((state) => {
    const tracks = new Map(state.tracks);
    for (const [id, track] of state.tracks) {
      if (track.vibes.some((v) => removed.has(v))) {
        tracks.set(id, {
          ...track,
          vibes: track.vibes.filter((v) => !removed.has(v)),
        });
      }
    }
    return { tracks, vibes: state.vibes.filter((v) => !removed.has(v.id)) };
  });
}

/** Records the DJ's review decision and stores the updated track. */
export async function setReview(
  trackId: TrackId,
  review: ReviewStatus,
): Promise<Track> {
  const track = await backend.setReview(trackId, review);
  replaceTrack(track);
  return track;
}

const waveformRequests = new Map<TrackId, Promise<WaveformPeaks | null>>();

/** Fetches a track's waveform once and caches it, including a null result. */
export function loadWaveform(trackId: TrackId): Promise<WaveformPeaks | null> {
  const cached = useLibrary.getState().waveforms;
  if (cached.has(trackId)) return Promise.resolve(cached.get(trackId) ?? null);
  let request = waveformRequests.get(trackId);
  if (!request) {
    request = backend
      .trackWaveform(trackId)
      .catch(() => null)
      .then((peaks) => {
        waveformRequests.delete(trackId);
        useLibrary.setState((state) => {
          const waveforms = new Map(state.waveforms);
          waveforms.set(trackId, peaks);
          return { waveforms };
        });
        return peaks;
      });
    waveformRequests.set(trackId, request);
  }
  return request;
}

export const spectrogramKey = (trackId: TrackId, log: boolean): string =>
  `${trackId}:${log}`;

export async function loadSpectrogram(
  trackId: TrackId,
  log: boolean,
): Promise<SpectrogramImage> {
  const key = spectrogramKey(trackId, log);
  const cached = useLibrary.getState().spectrograms.get(key);
  if (cached) return cached;
  const image = await backend.renderSpectrogram(trackId, log);
  useLibrary.setState((state) => {
    const spectrograms = new Map(state.spectrograms);
    spectrograms.set(key, image);
    return { spectrograms };
  });
  return image;
}
