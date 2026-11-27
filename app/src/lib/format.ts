import type {
  AudioProperties,
  QualityVerdict,
  ReviewStatus,
  Track,
} from "./types";

/** `3:07` or `1:02:03`. Empty for unknown durations. */
export function formatDuration(ms: number | null | undefined): string {
  if (ms == null || !Number.isFinite(ms) || ms < 0) return "";
  const total = Math.floor(ms / 1000);
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  const ss = String(s).padStart(2, "0");
  return h > 0 ? `${h}:${String(m).padStart(2, "0")}:${ss}` : `${m}:${ss}`;
}

/** Seconds as a clock for the player, such as `2:05`. */
export function formatClock(seconds: number): string {
  return formatDuration(Math.max(0, seconds) * 1000) || "0:00";
}

export function formatBpm(bpm: number | null | undefined): string {
  if (bpm == null || !Number.isFinite(bpm) || bpm <= 0) return "";
  return Number.isInteger(bpm) ? String(bpm) : bpm.toFixed(1);
}

const LOSSY = new Set(["mp3", "aac"]);

/** Codec label plus the detail a DJ cares about: bitrate for lossy, rate and depth for lossless. */
export function formatAudio(props: AudioProperties): {
  label: string;
  detail: string;
} {
  const label = props.format ? props.format.toUpperCase() : "?";
  if (props.format && LOSSY.has(props.format)) {
    return {
      label,
      detail: props.bitrate_kbps ? `${props.bitrate_kbps}k` : "",
    };
  }
  const rate = props.sample_rate_hz
    ? `${trimZero((props.sample_rate_hz / 1000).toFixed(1))}`
    : "";
  const depth = props.bit_depth ? String(props.bit_depth) : "";
  return { label, detail: [rate, depth].filter(Boolean).join("/") };
}

function trimZero(text: string): string {
  return text.endsWith(".0") ? text.slice(0, -2) : text;
}

export function formatHz(hz: number | null | undefined): string {
  if (hz == null) return "";
  return hz >= 1000 ? `${trimZero((hz / 1000).toFixed(1))} kHz` : `${hz} Hz`;
}

export function fileName(path: string): string {
  const index = path.lastIndexOf("/");
  return index >= 0 ? path.slice(index + 1) : path;
}

/** The title tag, or the file name when the file has none. */
export function trackTitle(track: Track): string {
  return track.tags.title?.trim() || fileName(track.path);
}

export const VERDICT_LABEL: Record<QualityVerdict, string> = {
  unchecked: "Unchecked",
  pass: "Pass",
  warning: "Warning",
  suspect: "Suspect",
  inconclusive: "Inconclusive",
};

export const REVIEW_LABEL: Record<ReviewStatus, string> = {
  unreviewed: "Unreviewed",
  verified_ok: "Verified OK",
  replace_wanted: "Replace wanted",
};

export function formatDateTime(ms: number | null | undefined): string {
  if (ms == null) return "";
  return new Date(ms).toLocaleString(undefined, {
    dateStyle: "medium",
    timeStyle: "short",
  });
}

export function formatPercent(fraction: number | null | undefined): string {
  if (fraction == null) return "";
  return `${Math.round(fraction * 100)}%`;
}
