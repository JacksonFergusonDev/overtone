import { create } from "zustand";
import { audioSrc, errorMessage } from "../lib/backend";
import { trackTitle } from "../lib/format";
import type { TrackId } from "../lib/types";
import { clampTime } from "../lib/waveform";
import { useLibrary } from "./library";
import { currentVisibleIds } from "./selectors";
import { notifyError } from "./toasts";
import { activeTable, useUi } from "./ui";

export interface PlayerState {
  currentId: TrackId | null;
  playing: boolean;
  /** Seconds; updated on `timeupdate`. The waveform reads the element directly. */
  position: number;
  duration: number;
  volume: number;
  /** Why the current track can't play, if it can't. */
  error: string | null;
}

const initialState: PlayerState = {
  currentId: null,
  playing: false,
  position: 0,
  duration: 0,
  volume: 0.8,
  error: null,
};

export const usePlayer = create<PlayerState>(() => initialState);

let audio: HTMLAudioElement | null = null;

/** The player bar registers its single `<audio>` element here. */
export function registerAudio(element: HTMLAudioElement | null): void {
  audio = element;
  if (element) element.volume = usePlayer.getState().volume;
}

export function audioElement(): HTMLAudioElement | null {
  return audio;
}

export function resetPlayer(): void {
  usePlayer.setState(initialState, true);
}

function startPlayback(): void {
  if (!audio) return;
  const result = audio.play();
  if (result && typeof result.catch === "function") {
    result.catch((error: unknown) => {
      // A newer `load()` interrupting this play isn't an error worth showing.
      if (error instanceof DOMException && error.name === "AbortError") return;
      usePlayer.setState({ playing: false, error: errorMessage(error) });
    });
  }
}

/** Why a track can't be played, or null if it can. */
export function unplayableReason(trackId: TrackId): string | null {
  const track = useLibrary.getState().tracks.get(trackId);
  if (!track) return "This track is no longer in the library.";
  if (track.missing_since !== null) {
    return `The file for “${trackTitle(track)}” is missing. Reconnect its drive or rescan.`;
  }
  return null;
}

export function playTrack(trackId: TrackId): void {
  const reason = unplayableReason(trackId);
  if (reason) {
    notifyError(reason);
    return;
  }
  const track = useLibrary.getState().tracks.get(trackId);
  if (!track) return;
  usePlayer.setState({
    currentId: trackId,
    playing: true,
    position: 0,
    duration: (track.properties.duration_ms ?? 0) / 1000,
    error: null,
  });
  if (audio) {
    audio.src = audioSrc(track.path);
    startPlayback();
  }
}

export function togglePlay(): void {
  const { currentId, playing } = usePlayer.getState();
  if (currentId === null) {
    // Nothing loaded yet: play the focused row, or the first one.
    const key = activeTable();
    if (!key) return;
    const ui = useUi.getState();
    const ids = currentVisibleIds(key);
    const target = ui.tables[key].selection.cursor ?? ids[0];
    if (target !== undefined) playTrack(target);
    return;
  }
  if (playing) {
    audio?.pause();
    usePlayer.setState({ playing: false });
  } else {
    usePlayer.setState({ playing: true });
    startPlayback();
  }
}

export function seek(seconds: number): void {
  const { duration } = usePlayer.getState();
  const length =
    audio && Number.isFinite(audio.duration) ? audio.duration : duration;
  const target = clampTime(seconds, length);
  if (audio) audio.currentTime = target;
  usePlayer.setState({ position: target });
}

export function seekBy(delta: number): void {
  const now = audio ? audio.currentTime : usePlayer.getState().position;
  seek(now + delta);
}

export function seekFraction(fraction: number): void {
  const { duration } = usePlayer.getState();
  const length =
    audio && Number.isFinite(audio.duration) ? audio.duration : duration;
  seek(length * fraction);
}

/**
 * Plays the next (+1) or previous (-1) playable track in the current table
 * order, skipping missing files. Starts from the focused row if nothing has
 * played yet.
 */
export function skip(direction: 1 | -1): void {
  const key = activeTable() ?? "library";
  const ids = currentVisibleIds(key);
  if (ids.length === 0) return;
  const { currentId } = usePlayer.getState();
  const from = currentId ?? useUi.getState().tables[key].selection.cursor;
  let index = from === null ? -1 : ids.indexOf(from);
  if (index < 0 && direction < 0) index = ids.length;
  const tracks = useLibrary.getState().tracks;
  for (let i = index + direction; i >= 0 && i < ids.length; i += direction) {
    if (tracks.get(ids[i])?.missing_since === null) {
      playTrack(ids[i]);
      return;
    }
  }
}

export function setVolume(volume: number): void {
  const value = Math.min(1, Math.max(0, volume));
  if (audio) audio.volume = value;
  usePlayer.setState({ volume: value });
}
