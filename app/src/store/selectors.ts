import { useMemo } from "react";
import { filterTracks, needsReview, type TrackFilter } from "../lib/filter";
import { sortTracks, type SortSpec } from "../lib/sort";
import type { Track, TrackId, Vibe, VibeId } from "../lib/types";
import { vibeCounts, vibeWithDescendants } from "../lib/vibes";
import { useLibrary, type LibraryState } from "./library";
import { useUi, type TableKey, type UiState } from "./ui";

interface SortCache {
  order: readonly TrackId[];
  tracks: ReadonlyMap<TrackId, Track>;
  sort: SortSpec;
  result: TrackId[];
}

interface FilterCache {
  sorted: TrackId[];
  tracks: ReadonlyMap<TrackId, Track>;
  vibes: readonly Vibe[];
  vibeFilter: VibeId | null;
  table: UiState["tables"][TableKey];
  result: TrackId[];
}

const sortCaches = new Map<TableKey, SortCache>();
const filterCaches = new Map<TableKey, FilterCache>();

/**
 * Whether two track maps hold the same values for the fields sorting reads.
 * Tagging a vibe replaces the map but not those fields, so the 20k-row sort
 * can be reused.
 */
function sameSortFields(
  a: ReadonlyMap<TrackId, Track>,
  b: ReadonlyMap<TrackId, Track>,
  order: readonly TrackId[],
): boolean {
  if (a.size !== b.size) return false;
  for (const id of order) {
    const x = a.get(id);
    const y = b.get(id);
    if (x === y) continue;
    if (!x || !y) return false;
    if (
      x.tags !== y.tags ||
      x.properties !== y.properties ||
      x.added_at !== y.added_at ||
      x.quality?.verdict !== y.quality?.verdict
    ) {
      return false;
    }
  }
  return true;
}

export interface VisibleInputs {
  tracks: ReadonlyMap<TrackId, Track>;
  order: readonly TrackId[];
  vibes: readonly Vibe[];
  table: UiState["tables"][TableKey];
  vibeFilter: VibeId | null;
}

function sortedIds(
  key: TableKey,
  library: VisibleInputs,
  sort: SortSpec,
): TrackId[] {
  const cached = sortCaches.get(key);
  if (
    cached &&
    cached.order === library.order &&
    cached.sort === sort &&
    (cached.tracks === library.tracks ||
      sameSortFields(cached.tracks, library.tracks, library.order))
  ) {
    cached.tracks = library.tracks;
    return cached.result;
  }
  const result = sortTracks(library.order, library.tracks, sort);
  sortCaches.set(key, {
    order: library.order,
    tracks: library.tracks,
    sort,
    result,
  });
  return result;
}

/** The filter a table applies, from its state and the sidebar vibe. */
export function tableFilter(
  key: TableKey,
  table: UiState["tables"][TableKey],
  vibes: readonly Vibe[],
  vibeFilter: VibeId | null,
): TrackFilter {
  return {
    query: table.query,
    verdicts: table.verdicts,
    missing: table.missing,
    vibeIds:
      key === "library" && vibeFilter !== null
        ? vibeWithDescendants(vibes, vibeFilter)
        : null,
    needsReview: key === "review",
  };
}

/**
 * Track ids a table shows, in display order. Memoized per table, so React
 * and keyboard handlers (via `getState()`) share one array.
 */
export function visibleIds(key: TableKey, input: VisibleInputs): TrackId[] {
  const { table } = input;
  const library = input;
  const ui = input;
  const sorted = sortedIds(key, library, table.sort);
  const cached = filterCaches.get(key);
  if (
    cached &&
    cached.sorted === sorted &&
    cached.tracks === library.tracks &&
    cached.vibes === library.vibes &&
    cached.vibeFilter === ui.vibeFilter &&
    cached.table.query === table.query &&
    cached.table.verdicts === table.verdicts &&
    cached.table.missing === table.missing
  ) {
    return cached.result;
  }
  const result = filterTracks(
    sorted,
    library.tracks,
    tableFilter(key, table, library.vibes, ui.vibeFilter),
  );
  filterCaches.set(key, {
    sorted,
    tracks: library.tracks,
    vibes: library.vibes,
    vibeFilter: ui.vibeFilter,
    table,
    result,
  });
  return result;
}

function inputsFrom(
  key: TableKey,
  library: LibraryState,
  ui: UiState,
): VisibleInputs {
  return {
    tracks: library.tracks,
    order: library.order,
    vibes: library.vibes,
    table: ui.tables[key],
    vibeFilter: ui.vibeFilter,
  };
}

/** The ids a table shows right now, for keyboard and player handlers. */
export function currentVisibleIds(key: TableKey): TrackId[] {
  return visibleIds(
    key,
    inputsFrom(key, useLibrary.getState(), useUi.getState()),
  );
}

export function useVisibleIds(key: TableKey): TrackId[] {
  const tracks = useLibrary((s) => s.tracks);
  const order = useLibrary((s) => s.order);
  const vibes = useLibrary((s) => s.vibes);
  const table = useUi((s) => s.tables[key]);
  const vibeFilter = useUi((s) => s.vibeFilter);
  return useMemo(
    () => visibleIds(key, { tracks, order, vibes, table, vibeFilter }),
    [key, tracks, order, vibes, table, vibeFilter],
  );
}

export function useNeedsReviewCount(): number {
  const tracks = useLibrary((s) => s.tracks);
  return useMemo(() => {
    let count = 0;
    for (const track of tracks.values()) if (needsReview(track)) count += 1;
    return count;
  }, [tracks]);
}

export function useVibeCounts(): Map<VibeId, number> {
  const tracks = useLibrary((s) => s.tracks);
  return useMemo(() => vibeCounts(tracks.values()), [tracks]);
}

export function useVibeMap(): Map<VibeId, Vibe> {
  const vibes = useLibrary((s) => s.vibes);
  return useMemo(() => new Map(vibes.map((vibe) => [vibe.id, vibe])), [vibes]);
}
