/**
 * Frequency axis for the spectrogram. A linear image spans 0 Hz to Nyquist;
 * a logarithmic one spans 20 Hz to Nyquist.
 */

export const LOG_MIN_HZ = 20;

export interface FrequencyTick {
  hz: number;
  /** Label in kHz, such as `0.5` or `16`. */
  label: string;
  /** Height above the bottom of the image, from 0 to 1. */
  fraction: number;
}

export function nyquist(sampleRateHz: number | null | undefined): number {
  return (sampleRateHz && sampleRateHz > 0 ? sampleRateHz : 44_100) / 2;
}

/** Where `hz` sits on the axis, from 0 (bottom) to 1 (top). */
export function frequencyFraction(
  hz: number,
  sampleRateHz: number,
  log: boolean,
): number {
  const top = nyquist(sampleRateHz);
  if (log) {
    if (hz <= LOG_MIN_HZ) return 0;
    return Math.min(1, Math.log(hz / LOG_MIN_HZ) / Math.log(top / LOG_MIN_HZ));
  }
  return Math.min(1, Math.max(0, hz / top));
}

function kHzLabel(hz: number): string {
  const khz = hz / 1000;
  return Number.isInteger(khz) ? String(khz) : String(Number(khz.toFixed(2)));
}

const LOG_STEPS = [50, 100, 200, 500, 1000, 2000, 5000, 10_000, 20_000, 50_000];

/** Tick marks for the axis, bottom to top. */
export function frequencyTicks(
  sampleRateHz: number,
  log: boolean,
): FrequencyTick[] {
  const top = nyquist(sampleRateHz);
  const hzList: number[] = [];
  if (log) {
    for (const hz of LOG_STEPS) if (hz <= top) hzList.push(hz);
  } else {
    // About five to eight labels whatever the sample rate.
    const step =
      [2000, 4000, 5000, 10_000, 20_000].find((s) => top / s <= 8) ?? 20_000;
    for (let hz = 0; hz <= top; hz += step) hzList.push(hz);
  }
  return hzList.map((hz) => ({
    hz,
    label: kHzLabel(hz),
    fraction: frequencyFraction(hz, sampleRateHz, log),
  }));
}
