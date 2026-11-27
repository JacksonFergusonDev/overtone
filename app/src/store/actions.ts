import { errorMessage } from "../lib/backend";
import { selectedInOrder } from "../lib/selection";
import type { ReviewStatus, TrackId, VibeId } from "../lib/types";
import { setReview, toggleVibe, useLibrary } from "./library";
import { playTrack, usePlayer } from "./player";
import { currentVisibleIds } from "./selectors";
import { notify, notifyError } from "./toasts";
import {
  activeTable,
  dispatchSelection,
  openModal,
  rememberVibe,
  useUi,
  type TableKey,
} from "./ui";

/** Selected tracks in the active table, in display order. */
export function selectedTrackIds(
  key: TableKey | null = activeTable(),
): TrackId[] {
  if (!key) return [];
  return selectedInOrder(
    useUi.getState().tables[key].selection,
    currentVisibleIds(key),
  );
}

export function openPalette(): void {
  if (useUi.getState().modal) return;
  if (selectedTrackIds().length === 0) {
    notify("Select one or more tracks to tag them.");
    return;
  }
  openModal("palette");
}

/** Adds a vibe to tracks that lack it, for drag and drop. */
export async function addVibeToTracks(
  trackIds: readonly TrackId[],
  vibeId: VibeId,
): Promise<void> {
  const { tracks, vibes } = useLibrary.getState();
  const missing = trackIds.filter(
    (id) => !tracks.get(id)?.vibes.includes(vibeId),
  );
  if (missing.length === 0) return;
  try {
    await toggleVibe(missing, vibeId);
    rememberVibe(vibeId);
    const name = vibes.find((vibe) => vibe.id === vibeId)?.path ?? "vibe";
    notify(
      `Added “${name}” to ${missing.length} ${missing.length === 1 ? "track" : "tracks"}`,
    );
  } catch {
    // toggleVibe already rolled back and showed the error.
  }
}

/**
 * Records a review decision for a track. In the Needs review view the
 * track then leaves the queue, so the cursor moves to the next one, and
 * playback follows if the reviewed track was playing.
 */
export async function reviewTrack(
  key: TableKey,
  trackId: TrackId,
  review: ReviewStatus,
): Promise<void> {
  const before = currentVisibleIds(key);
  const index = before.indexOf(trackId);
  try {
    await setReview(trackId, review);
  } catch (error) {
    notifyError(`Couldn't save the review: ${errorMessage(error)}`);
    return;
  }
  if (key !== "review") return;
  const after = new Set(currentVisibleIds(key));
  if (after.has(trackId)) return;
  const next =
    before.slice(index + 1).find((id) => after.has(id)) ??
    before
      .slice(0, Math.max(0, index))
      .reverse()
      .find((id) => after.has(id));
  if (next === undefined) {
    dispatchSelection(key, { type: "clear" });
    notify("Review queue is empty.");
    return;
  }
  dispatchSelection(key, { type: "select", id: next });
  const player = usePlayer.getState();
  if (player.currentId === trackId && player.playing) playTrack(next);
}
