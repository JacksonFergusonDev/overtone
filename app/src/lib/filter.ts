import { fileName } from "./format";
import type { QualityVerdict, Track, TrackId, VibeId } from "./types";

export type MissingFilter = "any" | "missing" | "present";

export interface TrackFilter {
  /** Words that must all appear in title, artist, album, comment, or file name. */
  query: string;
  /** Verdicts to show; empty shows every verdict. */
  verdicts: readonly QualityVerdict[];
  missing: MissingFilter;
  /** A vibe and its descendants, already expanded; null shows every track. */
  vibeIds: ReadonlySet<VibeId> | null;
  /** Only Warning or Suspect tracks nobody has reviewed yet. */
  needsReview: boolean;
}

export const EMPTY_FILTER: TrackFilter = {
  query: "",
  verdicts: [],
  missing: "any",
  vibeIds: null,
  needsReview: false,
};

export function verdictOf(track: Track): QualityVerdict {
  return track.quality?.verdict ?? "unchecked";
}

/** Flagged by the analyzer and not yet decided on by the DJ. */
export function needsReview(track: Track): boolean {
  const quality = track.quality;
  return (
    quality !== null &&
    (quality.verdict === "warning" || quality.verdict === "suspect") &&
    quality.review === "unreviewed"
  );
}

const haystacks = new WeakMap<Track, string>();

function haystack(track: Track): string {
  let text = haystacks.get(track);
  if (text === undefined) {
    const { title, artist, album, comment } = track.tags;
    text = [title, artist, album, comment, fileName(track.path)]
      .filter(Boolean)
      .join("\n")
      .toLowerCase();
    haystacks.set(track, text);
  }
  return text;
}

export function queryTerms(query: string): string[] {
  return query.toLowerCase().split(/\s+/).filter(Boolean);
}

export function matchesTrack(
  track: Track,
  filter: TrackFilter,
  terms: readonly string[] = queryTerms(filter.query),
): boolean {
  if (filter.needsReview && !needsReview(track)) return false;
  if (filter.missing === "missing" && track.missing_since === null)
    return false;
  if (filter.missing === "present" && track.missing_since !== null)
    return false;
  if (
    filter.verdicts.length > 0 &&
    !filter.verdicts.includes(verdictOf(track))
  ) {
    return false;
  }
  if (filter.vibeIds && !track.vibes.some((id) => filter.vibeIds?.has(id))) {
    return false;
  }
  if (terms.length > 0) {
    const text = haystack(track);
    if (!terms.every((term) => text.includes(term))) return false;
  }
  return true;
}

/** The ids in `order` whose tracks pass `filter`, keeping their order. */
export function filterTracks(
  order: readonly TrackId[],
  tracks: ReadonlyMap<TrackId, Track>,
  filter: TrackFilter,
): TrackId[] {
  const terms = queryTerms(filter.query);
  const result: TrackId[] = [];
  for (const id of order) {
    const track = tracks.get(id);
    if (track && matchesTrack(track, filter, terms)) result.push(id);
  }
  return result;
}
