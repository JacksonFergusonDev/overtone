import { trackTitle } from "./format";
import type { QualityVerdict, Track, TrackId } from "./types";

export type SortKey =
  | "title"
  | "artist"
  | "bpm"
  | "key"
  | "format"
  | "duration"
  | "quality"
  | "added";

export type SortDirection = "asc" | "desc";

export interface SortSpec {
  key: SortKey;
  dir: SortDirection;
}

/** Clicking a header sorts by it, or flips the direction if it's already the sort. */
export function nextSort(current: SortSpec, key: SortKey): SortSpec {
  if (current.key === key) {
    return { key, dir: current.dir === "asc" ? "desc" : "asc" };
  }
  return { key, dir: key === "added" ? "desc" : "asc" };
}

/** Worst first when ascending, so flagged tracks rise to the top. */
const VERDICT_RANK: Record<QualityVerdict, number> = {
  suspect: 0,
  warning: 1,
  inconclusive: 2,
  unchecked: 3,
  pass: 4,
};

/**
 * Orders Camelot (`8A`) and Open Key (`3m`) keys around the wheel; other
 * spellings sort after them by text.
 */
export function keyRank(key: string | null): number | string | null {
  if (!key) return null;
  const text = key.trim();
  const camelot = /^(\d{1,2})\s*([ABab])$/.exec(text);
  if (camelot) {
    return Number(camelot[1]) * 2 + (camelot[2].toUpperCase() === "A" ? 0 : 1);
  }
  return text.toLowerCase();
}

type SortValue = number | string | null;

function sortValue(track: Track, key: SortKey): SortValue {
  switch (key) {
    case "title":
      return trackTitle(track);
    case "artist":
      return track.tags.artist?.trim() || null;
    case "bpm":
      return track.tags.bpm ?? null;
    case "key":
      return keyRank(track.tags.key);
    case "format":
      return track.properties.format;
    case "duration":
      return track.properties.duration_ms;
    case "quality":
      return track.quality
        ? VERDICT_RANK[track.quality.verdict]
        : VERDICT_RANK.unchecked;
    case "added":
      return track.added_at;
  }
}

const collator = new Intl.Collator(undefined, {
  numeric: true,
  sensitivity: "base",
});

function compareValues(a: SortValue, b: SortValue): number {
  if (typeof a === "number" && typeof b === "number") return a - b;
  if (typeof a === "number") return -1;
  if (typeof b === "number") return 1;
  return collator.compare(a as string, b as string);
}

/**
 * Returns `ids` sorted by `spec`. Stable, and empty values always sort last
 * whichever way the column points. Ties fall back to title so the order is
 * predictable.
 */
export function sortTracks(
  ids: readonly TrackId[],
  tracks: ReadonlyMap<TrackId, Track>,
  spec: SortSpec,
): TrackId[] {
  const sign = spec.dir === "asc" ? 1 : -1;
  const rows = ids.map((id, index) => {
    const track = tracks.get(id);
    return {
      id,
      index,
      value: track ? sortValue(track, spec.key) : null,
      title: track && spec.key !== "title" ? trackTitle(track) : "",
    };
  });
  rows.sort((a, b) => {
    if (a.value === null || b.value === null) {
      if (a.value !== b.value) return a.value === null ? 1 : -1;
    } else {
      const order = compareValues(a.value, b.value);
      if (order !== 0) return order * sign;
    }
    const byTitle = collator.compare(a.title, b.title);
    return byTitle !== 0 ? byTitle : a.index - b.index;
  });
  return rows.map((row) => row.id);
}
