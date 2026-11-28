import type {
  AppStatus,
  QualityReport,
  SeratoSyncPlan,
  Track,
  Vibe,
} from "../lib/types";

export const STATUS: AppStatus = {
  version: "0.1.0",
  library_path: "/tmp/library.sqlite",
  library: {
    schema_version: 1,
    track_count: 12,
    missing_count: 0,
    vibe_count: 4,
    needs_review_count: 3,
  },
};

export function quality(overrides: Partial<QualityReport> = {}): QualityReport {
  return {
    verdict: "pass",
    cutoff_hz: 20_500,
    confidence: 0.9,
    reason: "Full-band spectrum",
    analyzer_version: 1,
    analyzed_at: 1_700_000_000_000,
    review: "unreviewed",
    reviewed_at: null,
    ...overrides,
  };
}

export function track(
  id: number,
  overrides: {
    title?: string | null;
    artist?: string | null;
    album?: string | null;
    comment?: string | null;
    bpm?: number | null;
    key?: string | null;
    vibes?: number[];
    quality?: QualityReport | null;
    missing?: boolean;
    duration_ms?: number;
  } = {},
): Track {
  return {
    id,
    path: `/music/track-${id}.flac`,
    file_size: 30_000_000,
    modified_at: 1_700_000_000_000,
    audio_hash: null,
    properties: {
      format: "flac",
      sample_rate_hz: 44_100,
      bit_depth: 16,
      channels: 2,
      bitrate_kbps: null,
      duration_ms: overrides.duration_ms ?? 300_000,
    },
    tags: {
      title: overrides.title === undefined ? `Track ${id}` : overrides.title,
      artist:
        overrides.artist === undefined ? `Artist ${id}` : overrides.artist,
      album: overrides.album ?? null,
      album_artist: null,
      remixer: null,
      label: null,
      genre: null,
      year: null,
      bpm: overrides.bpm === undefined ? 120 + id : overrides.bpm,
      key: overrides.key === undefined ? "8A" : overrides.key,
      comment: overrides.comment ?? null,
      grouping: null,
      catalog_number: null,
      isrc: null,
      release_date: null,
      track_number: null,
      disc_number: null,
    },
    vibes: overrides.vibes ?? [],
    quality: overrides.quality === undefined ? null : overrides.quality,
    added_at: 1_700_000_000_000 + id,
    missing_since: overrides.missing ? 1_700_000_500_000 : null,
  };
}

export function vibe(id: number, path: string, trackCount = 0): Vibe {
  return { id, path, created_at: 1_700_000_000_000, track_count: trackCount };
}

export const VIBES: Vibe[] = [
  vibe(1, "Dubs", 1),
  vibe(2, "Dubs/Deep", 1),
  vibe(3, "Warmup", 0),
  vibe(4, "Peak Time", 0),
];

export const TRACKS: Track[] = [
  track(1, { title: "Alpha", artist: "Zed", vibes: [1], quality: quality() }),
  track(2, {
    title: "Bravo",
    artist: "Yara",
    vibes: [2],
    quality: quality({
      verdict: "suspect",
      cutoff_hz: 16_000,
      reason: "Cutoff at 16 kHz",
    }),
  }),
  track(3, {
    title: "Charlie",
    artist: "Xavi",
    quality: quality({ verdict: "warning", cutoff_hz: 19_000 }),
  }),
  track(4, { title: "Delta", artist: "Wim", missing: true }),
  track(5, {
    title: "Echo",
    artist: "Vee",
    quality: quality({ verdict: "suspect", review: "verified_ok" }),
  }),
];

export const SERATO_PLAN: SeratoSyncPlan = {
  serato_dir: "/Volumes/Music/_Serato_",
  root_crate: "Overtone",
  files: [
    {
      filename: "Overtone%%Dubs.crate",
      action: { action: "create", tracks: 2 },
      track_count: 2,
    },
    {
      filename: "Overtone%%Warmup.crate",
      action: { action: "update", added: 1, removed: 2 },
      track_count: 5,
    },
    {
      filename: "Overtone%%Old.crate",
      action: { action: "delete", tracks: 3 },
      track_count: 0,
    },
  ],
  neworder: { exists: true, added: ["Overtone%%Dubs"], removed: [] },
  skipped_tracks: [
    {
      crate_name: "Dubs",
      path: "/Volumes/Other/tune.mp3",
      serato_dir: "/Volumes/Other/_Serato_",
    },
  ],
  warnings: ["#dub is a prefix of #dubtechno"],
};
