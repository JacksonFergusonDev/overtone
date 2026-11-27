/**
 * Subsequence fuzzy matching for the quick-tag palette. Returns null when
 * `query` isn't a subsequence of `text` (ignoring case), otherwise a score
 * where higher is better. Consecutive characters, matches at the start of a
 * level or word, and a matching prefix all score higher.
 */
export function fuzzyScore(query: string, text: string): number | null {
  const q = query.trim().toLowerCase();
  if (!q) return 0;
  const t = text.toLowerCase();
  if (t === q) return 1000;
  let score = 0;
  let position = 0;
  let previous = -2;
  for (const char of q) {
    const index = t.indexOf(char, position);
    if (index < 0) return null;
    const boundary = index === 0 || "/ -_".includes(t[index - 1]);
    if (index === previous + 1) score += 5;
    if (boundary) score += 8;
    score -= Math.min(index - position, 10) * 0.5;
    previous = index;
    position = index + 1;
  }
  if (t.startsWith(q)) score += 20;
  else if (t.includes(q)) score += 10;
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
