/** Pure geometry for the waveform scrubber, kept apart from canvas code for tests. */

export function clamp(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, value));
}

/** Horizontal position of `time` on a scrubber `width` pixels wide. */
export function timeToX(time: number, duration: number, width: number): number {
  if (!(duration > 0) || !(width > 0)) return 0;
  return clamp(time / duration, 0, 1) * width;
}

/** Time under pixel `x` of a scrubber `width` pixels wide. */
export function xToTime(x: number, width: number, duration: number): number {
  if (!(duration > 0) || !(width > 0)) return 0;
  return clamp(x / width, 0, 1) * duration;
}

/** Keeps a seek target inside the track. */
export function clampTime(time: number, duration: number): number {
  if (!(duration > 0)) return 0;
  return clamp(time, 0, duration);
}

/** Backing-store size for a crisp canvas on a HiDPI display. */
export function canvasSize(
  cssWidth: number,
  cssHeight: number,
  devicePixelRatio: number,
): { width: number; height: number } {
  const ratio = devicePixelRatio > 0 ? devicePixelRatio : 1;
  return {
    width: Math.max(1, Math.round(cssWidth * ratio)),
    height: Math.max(1, Math.round(cssHeight * ratio)),
  };
}

/**
 * Resamples `[min, max]` peaks to `columns` columns: each column takes the
 * extremes of the buckets it covers, or the nearest bucket when there are
 * more columns than buckets.
 */
export function resamplePeaks(
  buckets: readonly (readonly [number, number])[],
  columns: number,
): [number, number][] {
  const count = buckets.length;
  const n = Math.max(0, Math.floor(columns));
  if (count === 0 || n === 0) return [];
  const out: [number, number][] = new Array(n);
  for (let column = 0; column < n; column += 1) {
    const start = Math.floor((column * count) / n);
    const end = Math.max(start + 1, Math.floor(((column + 1) * count) / n));
    let min = 1;
    let max = -1;
    for (let i = start; i < Math.min(end, count); i += 1) {
      const [lo, hi] = buckets[i];
      if (lo < min) min = lo;
      if (hi > max) max = hi;
    }
    out[column] = [clamp(min, -1, 1), clamp(max, -1, 1)];
  }
  return out;
}

/** Number keys 1 to 9 jump to 10% to 90%. Returns null for any other key. */
export function digitFraction(key: string): number | null {
  if (!/^[1-9]$/.test(key)) return null;
  return Number(key) / 10;
}
