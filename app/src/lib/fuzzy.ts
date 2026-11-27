/**
 * Subsequence fuzzy matching for the quick-tag palette. Returns null when
 * `query` isn't a subsequence of `text` (ignoring case), otherwise a score
 * where higher is better. Consecutive characters, matches at the start of a
 * level or word, and a matching prefix all score higher.
 */
export function fuzzyScore(query: string, text: string): number | null {
  const q = query.trim().toLowerCase().replace(/\s+/g, " ");
  if (!q) return 0;
  const t = text.toLowerCase();
  if (t === q) return 1000;
  const isBoundary = (index: number) =>
    index === 0 || "/ -_".includes(t[index - 1]);
  // A contiguous match beats any scattered one; prefer it at a level or
  // word start, then nearer the start.
  const substring = t.indexOf(q);
  if (substring >= 0) {
    let best = substring;
    for (let i = substring; i >= 0; i = t.indexOf(q, i + 1)) {
      if (isBoundary(i)) {
        best = i;
        break;
      }
    }
    return (
      200 +
      (best === 0 ? 40 : 0) +
      (isBoundary(best) ? 20 : 0) -
      Math.min(best, 40) * 0.1 -
      t.length * 0.05
    );
  }
  let score = 0;
  let position = 0;
  let previous = -2;
  for (const char of q) {
    const index = t.indexOf(char, position);
    if (index < 0) return null;
    if (index === previous + 1) score += 5;
    if (isBoundary(index)) score += 8;
    score -= Math.min(index - position, 10) * 0.5;
    previous = index;
    position = index + 1;
  }
  // Shorter texts win ties, so `Deep` ranks above `Deep/Late Night`.
  return score - t.length * 0.05;
}

/** Items whose text matches, best first; ties keep their input order. */
export function fuzzyRank<T>(
  query: string,
  items: readonly T[],
  text: (item: T) => string,
): T[] {
  if (!query.trim()) return [...items];
  return items
    .map((item, index) => ({
      item,
      index,
      score: fuzzyScore(query, text(item)),
    }))
    .filter(
      (entry): entry is { item: T; index: number; score: number } =>
        entry.score !== null,
    )
    .sort((a, b) => b.score - a.score || a.index - b.index)
    .map((entry) => entry.item);
}
