import type { Vibe, VibeId } from "./types";

/** Matches `MAX_VIBE_PATH_CHARS` in `crates/model/src/vibe.rs`. */
export const MAX_VIBE_PATH_CHARS = 200;

export type VibePathResult =
  { ok: true; path: string; segments: string[] } | { ok: false; error: string };

// eslint-disable-next-line no-control-regex
const CONTROL = /[\u0000-\u001f\u007f-\u009f]/;

/**
 * Validates and normalizes a typed vibe path the way the backend's
 * `VibePath::parse` does, so the UI can explain problems before calling it.
 */
export function parseVibePath(input: string): VibePathResult {
  const trimmed = input.trim();
  if (!trimmed) return { ok: false, error: "Vibe name is empty" };
  if ([...trimmed].length > MAX_VIBE_PATH_CHARS) {
    return {
      ok: false,
      error: `Vibe path is longer than ${MAX_VIBE_PATH_CHARS} characters`,
    };
  }
  const segments: string[] = [];
  for (const raw of trimmed.split("/")) {
    const segment = raw.split(/\s+/).filter(Boolean).join(" ");
    if (!segment) return { ok: false, error: "Vibe path has an empty level" };
    const bad = forbiddenText(segment);
    if (bad) return { ok: false, error: `Vibe names can't contain ${bad}` };
    segments.push(segment);
  }
  return { ok: true, path: segments.join("/"), segments };
}

function forbiddenText(segment: string): string | null {
  if (segment.includes("%%")) return "%%";
  for (const char of segment) {
    if (char === "#" || char === ":" || char === "\\") return char;
    if (CONTROL.test(char)) return "control characters";
  }
  return null;
}

/** Case- and whitespace-insensitive identity, like the backend's slug column. */
export function vibeSlug(path: string): string {
  return path
    .split("/")
    .map((segment) =>
      segment.trim().split(/\s+/).filter(Boolean).join("-").toLowerCase(),
    )
    .join("/");
}

export function vibeName(path: string): string {
  const index = path.lastIndexOf("/");
  return index >= 0 ? path.slice(index + 1) : path;
}

export function vibeDepth(path: string): number {
  return path.split("/").length - 1;
}

/** The vibe and every vibe nested below it. */
export function vibeWithDescendants(
  vibes: readonly Vibe[],
  id: VibeId,
): Set<VibeId> {
  const root = vibes.find((vibe) => vibe.id === id);
  const ids = new Set<VibeId>([id]);
  if (!root) return ids;
  const prefix = `${vibeSlug(root.path)}/`;
  for (const vibe of vibes) {
    if (vibeSlug(vibe.path).startsWith(prefix)) ids.add(vibe.id);
  }
  return ids;
}

const collator = new Intl.Collator(undefined, {
  numeric: true,
  sensitivity: "base",
});

/** Compares level by level, so children always follow their parent. */
export function comparePaths(a: string, b: string): number {
  const left = a.split("/");
  const right = b.split("/");
  for (let i = 0; i < Math.min(left.length, right.length); i += 1) {
    const order = collator.compare(left[i].trim(), right[i].trim());
    if (order !== 0) return order;
  }
  return left.length - right.length;
}

export interface VibeNode {
  vibe: Vibe;
  depth: number;
  hasChildren: boolean;
}

/**
 * Flattens vibes into tree order (parents before children, siblings by
 * name). The backend creates every ancestor, so each nested vibe's parent
 * is in the list; an orphan is shown at the deepest level whose parent exists.
 */
export function vibeTree(vibes: readonly Vibe[]): VibeNode[] {
  const sorted = [...vibes].sort((a, b) => comparePaths(a.path, b.path));
  const slugs = new Set(sorted.map((vibe) => vibeSlug(vibe.path)));
  return sorted.map((vibe, index) => {
    const slug = vibeSlug(vibe.path);
    const parts = slug.split("/");
    let depth = 0;
    for (let level = 1; level < parts.length; level += 1) {
      if (slugs.has(parts.slice(0, level).join("/"))) depth += 1;
    }
    const next = sorted[index + 1];
    return {
      vibe,
      depth,
      hasChildren: next ? vibeSlug(next.path).startsWith(`${slug}/`) : false,
    };
  });
}

/** Direct track counts per vibe, from the tracks in memory. */
export function vibeCounts(
  tracks: Iterable<{ vibes: readonly VibeId[] }>,
): Map<VibeId, number> {
  const counts = new Map<VibeId, number>();
  for (const track of tracks) {
    for (const id of track.vibes) counts.set(id, (counts.get(id) ?? 0) + 1);
  }
  return counts;
}
