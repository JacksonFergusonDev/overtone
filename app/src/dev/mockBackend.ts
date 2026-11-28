import { mockIPC } from "@tauri-apps/api/mocks";
import type {
  AppStatus,
  AuditProgress,
  AuditReport,
  AudioFormat,
  CommandError,
  CommentChange,
  DjApp,
  LibraryRoot,
  MirrorReport,
  QualityReport,
  QualityVerdict,
  ReviewStatus,
  RunningApps,
  ScanProgress,
  ScanReport,
  SeratoFilePlan,
  SeratoSyncPlan,
  RekordboxExportPlan,
  RekordboxExportReport,
  SeratoSyncReport,
  RekordboxTagPlan,
  RekordboxTagReport,
  TraktorSyncPlan,
  TraktorSyncReport,
  SpectrogramImage,
  SyncRun,
  Track,
  TrackId,
  Vibe,
  VibeId,
  VerdictCounts,
  WaveformPeaks,
} from "../api";

/**
 * In-memory stand-in for the Rust backend, used by `pnpm dev` in a browser
 * and by tests. It answers every command in the IPC contract with
 * deterministic, plausible data (about 200 tracks) and applies mutations to
 * that data, so the UI behaves like the real app.
 *
 * Conventions the UI should know about:
 * - Track paths look like `mock://music/<name>.<ext>`; `audioSrc` turns them
 *   into a silent WAV so the player works.
 * - `renderSpectrogram` returns `mock://spectrogram/<id>-<lin|log>?cutoff=<hz>`;
 *   `imageSrc` turns that into a generated gradient image.
 * - Progress channels stream like the real ones (the handler is called per
 *   event, with a short delay between audit tracks).
 */

const ANALYZER_VERSION = 2;
const NOW = 1_780_000_000_000;
const DAY = 86_400_000;

/** Small deterministic PRNG (mulberry32). */
function seeded(seed: number): () => number {
  let a = seed;
  return () => {
    a = (a + 0x6d2b79f5) | 0;
    let t = Math.imul(a ^ (a >>> 15), 1 | a);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

const ARTISTS = [
  "Sub Focus",
  "Nu:Tone",
  "Hybrid Minds",
  "Culture Shock",
  "Break",
  "Fred V",
  "Camo & Krooked",
  "Mefjus",
  "Noisia",
  "Philth",
  "Dimension",
  "Wilkinson",
  "Calibre",
  "Etherwood",
  "Lenzman",
  "High Contrast",
  "Logistics",
  "Danny Byrd",
  "Metrik",
  "Rudimental",
];
const TITLES = [
  "Midnight Run",
  "Afterglow",
  "Signal Lost",
  "Cold Water",
  "Gravity Well",
  "Neon Streets",
  "Hold On",
  "Deep End",
  "Low Light",
  "Backdraft",
  "Static",
  "Ghost Notes",
  "Horizon",
  "Burn It Down",
  "Echo Chamber",
  "Slow Motion",
  "Overdrive",
  "Fault Lines",
  "Daybreak",
  "Undertow",
];
const MIXES = [
  "Original Mix",
  "Extended Mix",
  "VIP",
  "Club Mix",
  "Dub",
  "Radio Edit",
];
const KEYS = [
  "1A",
  "2A",
  "3A",
  "4A",
  "5A",
  "6A",
  "7A",
  "8A",
  "9A",
  "10A",
  "11A",
  "12A",
  "1B",
  "5B",
  "8B",
  "11B",
];
const FORMATS: AudioFormat[] = ["mp3", "mp3", "flac", "flac", "wav", "aiff"];
const LABELS = [
  "Hospital",
  "Ram",
  "Viper",
  "Metalheadz",
  "Shogun Audio",
  "Critical",
  "Technique",
];

const VIBE_PATHS = [
  "Dubs",
  "Dubs/Deep",
  "Rollers",
  "Warmup",
  "Peak Time",
  "Peak Time/Big Room",
];

interface Internal {
  tracks: Track[];
  vibes: Vibe[];
  roots: LibraryRoot[];
  waveforms: Map<number, WaveformPeaks>;
  history: SyncRun[];
  /** Crate sizes as last synced to Serato, by vibe id. */
  synced: Map<number, number>;
  /** What the last mock Rekordbox export contained, or null before one. */
  rekordboxExported: string | null;
  /** What the last mock Traktor sync contained, or null before one. */
  traktorSynced: string | null;
  /** What the last mock My Tag sync contained, or null before one. */
  tagsSynced: string | null;
  /** Original Comments recorded before the first tag mirror write. */
  snapshots: Map<number, string | null>;
  auditRunning: boolean;
  cancelAudit: boolean;
  nextVibeId: number;
  nextRootId: number;
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

function fail(code: string, message: string): never {
  const error: CommandError = { code, message };
  throw error;
}

function report(
  verdict: QualityVerdict,
  cutoff: number | null,
  rand: () => number,
): QualityReport {
  const reasons: Partial<Record<QualityVerdict, string>> = {
    pass: "Full-band content up to the Nyquist limit.",
    warning: `Energy falls off sharply near ${Math.round((cutoff ?? 19000) / 100) / 10} kHz; this may be a lossy source.`,
    suspect: `Hard lowpass at ${Math.round((cutoff ?? 16000) / 100) / 10} kHz, typical of an MP3 transcode.`,
    inconclusive: "Too quiet or too short to judge.",
    unchecked: "The file couldn't be decoded.",
  };
  return {
    verdict,
    cutoff_hz: verdict === "unchecked" ? null : cutoff,
    confidence:
      verdict === "unchecked"
        ? null
        : Math.round((0.6 + rand() * 0.39) * 100) / 100,
    reason: reasons[verdict] ?? null,
    analyzer_version: ANALYZER_VERSION,
    analyzed_at: NOW - Math.floor(rand() * 20) * DAY,
    review: "unreviewed",
    reviewed_at: null,
  };
}

function generate(): Internal {
  const rand = seeded(20260101);
  const pick = <T>(list: readonly T[]): T =>
    list[Math.floor(rand() * list.length)] as T;
  const roots: LibraryRoot[] = [
    { id: 1, path: "/Users/dj/Music/Overtone", added_at: NOW - 90 * DAY },
    { id: 2, path: "/Volumes/Gigs/Crates", added_at: NOW - 40 * DAY },
  ];
  const vibes: Vibe[] = VIBE_PATHS.map((path, index) => ({
    id: (index + 1) as VibeId,
    path,
    created_at: NOW - (60 - index) * DAY,
    track_count: 0,
  }));

  const tracks: Track[] = [];
  for (let i = 1; i <= 200; i += 1) {
    const format = pick(FORMATS);
    const lossy = format === "mp3";
    const artist = pick(ARTISTS);
    const title = `${pick(TITLES)} (${pick(MIXES)})`;
    const roll = rand();
    // About 10% are not analyzed yet, 15% flagged, and the rest pass.
    let quality: QualityReport | null;
    if (roll < 0.1) {
      quality = null;
    } else if (roll < 0.17) {
      quality = report("suspect", 15_800 + Math.floor(rand() * 400), rand);
    } else if (roll < 0.25) {
      quality = report("warning", 18_500 + Math.floor(rand() * 1200), rand);
    } else if (roll < 0.27) {
      quality = report("inconclusive", null, rand);
    } else if (roll < 0.28) {
      quality = report("unchecked", null, rand);
    } else {
      quality = report(
        "pass",
        lossy ? 20_000 : 21_900 + Math.floor(rand() * 100),
        rand,
      );
    }
    if (
      quality &&
      (quality.verdict === "warning" || quality.verdict === "suspect") &&
      rand() < 0.3
    ) {
      quality.review = rand() < 0.5 ? "verified_ok" : "replace_wanted";
      quality.reviewed_at = NOW - Math.floor(rand() * 5) * DAY;
    }
    const duration = 150_000 + Math.floor(rand() * 240_000);
    const vibeIds: VibeId[] = [];
    for (const vibe of vibes) {
      if (rand() < (vibe.path.includes("/") ? 0.06 : 0.12))
        vibeIds.push(vibe.id);
    }
    const bpm = 120 + Math.floor(rand() * 56);
    tracks.push({
      id: i as TrackId,
      path: `mock://music/${artist} - ${title.replace(/[()]/g, "")}.${format}`,
      file_size: Math.floor((duration / 1000) * (lossy ? 40_000 : 700_000)),
      modified_at: NOW - Math.floor(rand() * 300) * DAY,
      audio_hash: rand().toString(16).slice(2, 18),
      properties: {
        format,
        sample_rate_hz:
          format === "wav" || format === "aiff"
            ? 44_100
            : pick([44_100, 48_000]),
        bit_depth: lossy ? null : pick([16, 24]),
        channels: 2,
        bitrate_kbps: lossy ? pick([192, 256, 320]) : null,
        duration_ms: duration,
      },
      tags: {
        title,
        artist,
        album: rand() < 0.6 ? `${pick(TITLES)} EP` : null,
        album_artist: null,
        remixer: null,
        label: pick(LABELS),
        genre: pick(["Drum & Bass", "Liquid", "Neurofunk", "Jungle"]),
        year: 2015 + Math.floor(rand() * 11),
        bpm: rand() < 0.97 ? bpm : null,
        key: rand() < 0.95 ? pick(KEYS) : null,
        comment:
          rand() < 0.2
            ? pick([
                "Promo copy",
                "Bought on Beatport",
                "From the Hospital pack",
              ])
            : null,
        grouping: null,
      },
      vibes: vibeIds,
      quality,
      added_at: NOW - Math.floor(rand() * 200) * DAY,
      missing_since: i % 53 === 0 ? NOW - 3 * DAY : null,
    });
  }
  for (const vibe of vibes) {
    vibe.track_count = tracks.filter((t) => t.vibes.includes(vibe.id)).length;
  }
  return {
    tracks,
    vibes,
    roots,
    waveforms: new Map(),
    history: [],
    synced: new Map(),
    rekordboxExported: null,
    traktorSynced: null,
    tagsSynced: null,
    snapshots: new Map(),
    auditRunning: false,
    cancelAudit: false,
    nextVibeId: vibes.length + 1,
    nextRootId: 3,
  };
}

function waveformFor(id: number): WaveformPeaks {
  const rand = seeded(id * 7919);
  const buckets: [number, number][] = [];
  const buildAt = 0.08 + rand() * 0.1;
  for (let i = 0; i < 1200; i += 1) {
    const t = i / 1200;
    const section =
      t < buildAt ? 0.45 : Math.sin(t * Math.PI * 6) > 0.6 ? 0.55 : 0.95;
    const fade = Math.min(1, t * 20, (1 - t) * 20);
    const amp = Math.min(1, section * fade * (0.8 + rand() * 0.2));
    buckets.push([-amp * (0.85 + rand() * 0.15), amp]);
  }
  return { buckets };
}

function slug(path: string): string {
  return path.toLowerCase().replace(/\s+/g, "-");
}

function hashtags(vibes: Vibe[], ids: VibeId[]): string {
  return vibes
    .filter((v) => ids.includes(v.id))
    .map((v) => `#${slug(v.path)}`)
    .sort()
    .join(" ");
}

/** The Comment a track gets: its text with all Overtone hashtags replaced. */
function mirroredComment(state: Internal, track: Track): string | null {
  const known = new Set(state.vibes.map((v) => `#${slug(v.path)}`));
  const kept = (track.tags.comment ?? "")
    .split(/\s+/)
    .filter((word) => word && !known.has(word))
    .join(" ");
  const tags = hashtags(state.vibes, track.vibes);
  const next = [kept, tags].filter(Boolean).join(" ");
  return next === "" ? null : next;
}

function countVerdicts(verdicts: QualityVerdict[]): VerdictCounts {
  const counts: VerdictCounts = {
    pass: 0,
    warning: 0,
    suspect: 0,
    inconclusive: 0,
    unchecked: 0,
  };
  for (const verdict of verdicts) counts[verdict] += 1;
  return counts;
}

/** Sends one message on a progress channel passed as a command argument. */
function send<T>(channel: unknown, message: T): void {
  const target = channel as { onmessage?: (message: T) => void } | null;
  target?.onmessage?.(message);
}

type Args = Record<string, unknown>;

/**
 * Builds the command handler with its own fresh data. `installMockBackend`
 * wires one into `mockIPC`; tests can call it directly.
 */
export function createMockBackend(): (
  cmd: string,
  args?: Args,
) => Promise<unknown> {
  const state = generate();
  const find = (id: unknown): Track => {
    const track = state.tracks.find((t) => t.id === id);
    return track ?? fail("not_found", `no track with id ${String(id)}`);
  };
  const vibeById = (id: unknown): Vibe =>
    state.vibes.find((v) => v.id === id) ??
    fail("not_found", `no vibe with id ${String(id)}`);
  const refreshCounts = () => {
    for (const vibe of state.vibes) {
      vibe.track_count = state.tracks.filter((t) =>
        t.vibes.includes(vibe.id),
      ).length;
    }
  };
  const summary = (): AppStatus => ({
    version: "0.1.0-mock",
    library_path: "/mock/library.sqlite",
    library: {
      schema_version: 1,
      track_count: state.tracks.length,
      missing_count: state.tracks.filter((t) => t.missing_since !== null)
        .length,
      vibe_count: state.vibes.length,
      needs_review_count: state.tracks.filter(
        (t) =>
          (t.quality?.verdict === "warning" ||
            t.quality?.verdict === "suspect") &&
          t.quality.review === "unreviewed",
      ).length,
    },
  });
  const checkPath = (path: unknown): string => {
    const text = typeof path === "string" ? path.trim() : "";
    if (
      text === "" ||
      /%%|#|:|\\/.test(text) ||
      text.split("/").some((s) => s.trim() === "")
    ) {
      return fail("invalid_vibe", `"${text}" isn't a valid vibe name`);
    }
    return text
      .split("/")
      .map((s) => s.trim())
      .join("/");
  };
  const rekordboxSignature = () =>
    JSON.stringify(
      state.vibes.map((v) => [
        v.path,
        state.tracks
          .filter((t) => t.vibes.includes(v.id) && t.missing_since === null)
          .map((t) => t.id),
      ]),
    );
  const planRekordbox = (path: unknown): RekordboxExportPlan => {
    const playlists = state.vibes
      .map((v) => ({
        path: v.path.split("/"),
        track_count: state.tracks.filter(
          (t) => t.vibes.includes(v.id) && t.missing_since === null,
        ).length,
      }))
      .filter((p) => p.track_count > 0);
    const tracks = new Set(
      state.tracks
        .filter((t) => t.vibes.length > 0 && t.missing_since === null)
        .map((t) => t.id),
    );
    const exists = state.rekordboxExported !== null;
    return {
      path: typeof path === "string" ? path : "/mock/data/rekordbox.xml",
      track_count: tracks.size,
      playlist_count: playlists.length,
      playlists,
      changed: state.rekordboxExported !== rekordboxSignature(),
      exists,
    };
  };
  const planTags = (db: unknown): RekordboxTagPlan => {
    const fresh = state.tagsSynced === null;
    const changed = state.tagsSynced !== rekordboxSignature();
    const tags = state.vibes.map((v) => {
      const count = state.tracks.filter(
        (t) => t.vibes.includes(v.id) && t.missing_since === null,
      ).length;
      return {
        name: v.path,
        action: (fresh
          ? "create"
          : changed
            ? "update"
            : "unchanged") as RekordboxTagPlan["tags"][number]["action"],
        track_count: count,
        added: changed ? count : 0,
        removed: 0,
      };
    });
    return {
      db_path:
        typeof db === "string"
          ? db
          : "/mock/Library/Pioneer/rekordbox/master.db",
      create_category: fresh,
      tags,
      added: tags.reduce((n, t) => n + t.added, 0),
      removed: 0,
      skipped_tracks: [],
      changed,
      rekordbox_running: false,
      agent_running: false,
    };
  };
  const planTraktor = (collection: unknown): TraktorSyncPlan => {
    const base = planRekordbox(null);
    // Pretend every tenth tagged track hasn't been imported into Traktor.
    const skipped = state.tracks
      .filter(
        (t) =>
          t.vibes.length > 0 && t.missing_since === null && t.id % 10 === 0,
      )
      .map((t) => ({
        track_id: t.id,
        path: t.path,
        reason: "not in Traktor's collection",
      }));
    return {
      collection:
        typeof collection === "string"
          ? collection
          : "/mock/Documents/Native Instruments/Traktor 4.1.0/collection.nml",
      traktor_version: "20",
      playlists: base.playlists,
      track_count: base.track_count - skipped.length,
      skipped_tracks: skipped,
      changed: state.traktorSynced !== rekordboxSignature(),
      has_existing_folder: state.traktorSynced !== null,
    };
  };
  const recordSync = (app: DjApp, target: string, summaryText: string) => {
    state.history.unshift({
      id: state.history.length + 1,
      app,
      target,
      dry_run: false,
      status: "succeeded",
      started_at: Date.now() - 1500,
      finished_at: Date.now(),
      backup_path: `/mock/backups/${app}/${Date.now()}`,
      summary: summaryText,
    });
  };
  const planSerato = (dir: unknown): SeratoSyncPlan[] => {
    const files: SeratoFilePlan[] = state.vibes.map((vibe) => {
      const tracks = state.tracks.filter(
        (t) => t.vibes.includes(vibe.id) && t.missing_since === null,
      );
      const filename = `Overtone%%${vibe.path.replace(/\//g, "%%")}.crate`;
      const before = state.synced.get(vibe.id);
      if (before === undefined) {
        return {
          filename,
          action: { action: "create", tracks: tracks.length },
          track_count: tracks.length,
        };
      }
      if (before === tracks.length) {
        return {
          filename,
          action: { action: "unchanged" },
          track_count: tracks.length,
        };
      }
      return {
        filename,
        action: {
          action: "update",
          added: Math.max(0, tracks.length - before),
          removed: Math.max(0, before - tracks.length),
        },
        track_count: tracks.length,
      };
    });
    return [
      {
        serato_dir: typeof dir === "string" ? dir : "/Users/dj/Music/_Serato_",
        root_crate: "Overtone",
        files,
        neworder: {
          exists: true,
          added: state.synced.size === 0 ? ["Overtone"] : [],
          removed: [],
        },
        skipped_tracks: [],
        warnings: [],
      },
    ];
  };

  const handlers: Record<string, (args: Args) => unknown> = {
    app_status: summary,
    list_tracks: () => state.tracks,
    list_vibes: () => state.vibes,
    list_roots: () => state.roots,
    add_root: ({ path }) => {
      const text = String(path);
      if (!text.startsWith("/"))
        fail(
          "invalid_root",
          `can't add ${text} as a library folder: not found`,
        );
      const root: LibraryRoot = {
        id: state.nextRootId,
        path: text.replace(/\/$/, ""),
        added_at: Date.now(),
      };
      state.nextRootId += 1;
      state.roots.push(root);
      return root;
    },
    remove_root: ({ id }) => {
      state.roots = state.roots.filter((r) => r.id !== id);
      return null;
    },
    set_review: ({ trackId, review }) => {
      const track = find(trackId);
      if (track.quality) {
        track.quality = {
          ...track.quality,
          review: review as ReviewStatus,
          reviewed_at: review === "unreviewed" ? null : Date.now(),
        };
      }
      return track;
    },
    track_waveform: ({ trackId }) => {
      const track = find(trackId);
      if (track.quality === null || track.quality.verdict === "unchecked")
        return null;
      let peaks = state.waveforms.get(track.id);
      if (!peaks) {
        peaks = waveformFor(track.id);
        state.waveforms.set(track.id, peaks);
      }
      return peaks;
    },
    render_spectrogram: ({ trackId, log }): SpectrogramImage => {
      const track = find(trackId);
      track.quality ??= report("pass", 21_900, seeded(track.id));
      const axis = log === true ? "log" : "lin";
      return {
        path: `mock://spectrogram/${track.id}-${axis}?cutoff=${Math.round(track.quality.cutoff_hz ?? 22_050)}`,
        report: track.quality,
        log_frequency: log === true,
      };
    },
    create_vibe: ({ path }) => {
      const text = checkPath(path);
      const existing = state.vibes.find((v) => v.path === text);
      if (existing) return existing;
      const vibe: Vibe = {
        id: state.nextVibeId as VibeId,
        path: text,
        created_at: Date.now(),
        track_count: 0,
      };
      state.nextVibeId += 1;
      state.vibes.push(vibe);
      return vibe;
    },
    rename_vibe: ({ vibeId, newPath }) => {
      const vibe = vibeById(vibeId);
      const next = checkPath(newPath);
      if (state.vibes.some((v) => v.id !== vibe.id && v.path === next)) {
        fail("vibe_exists", `vibe ${next} already exists`);
      }
      const old = vibe.path;
      for (const v of state.vibes) {
        if (v.path === old || v.path.startsWith(`${old}/`))
          v.path = next + v.path.slice(old.length);
      }
      return vibe;
    },
    delete_vibe: ({ vibeId }) => {
      const vibe = vibeById(vibeId);
      const doomed = new Set(
        state.vibes
          .filter(
            (v) => v.path === vibe.path || v.path.startsWith(`${vibe.path}/`),
          )
          .map((v) => v.id),
      );
      state.vibes = state.vibes.filter((v) => !doomed.has(v.id));
      for (const track of state.tracks)
        track.vibes = track.vibes.filter((id) => !doomed.has(id));
      return null;
    },
    assign_vibe: ({ trackIds, vibeId }) => {
      const vibe = vibeById(vibeId);
      for (const id of trackIds as TrackId[]) {
        const track = find(id);
        if (!track.vibes.includes(vibe.id))
          track.vibes = [...track.vibes, vibe.id];
      }
      refreshCounts();
      return null;
    },
    unassign_vibe: ({ trackIds, vibeId }) => {
      const vibe = vibeById(vibeId);
      for (const id of trackIds as TrackId[]) {
        const track = find(id);
        track.vibes = track.vibes.filter((v) => v !== vibe.id);
      }
      refreshCounts();
      return null;
    },
    dj_apps_running: (): RunningApps => ({
      serato: false,
      rekordbox: false,
      traktor: false,
    }),
    plan_serato_sync: ({ seratoDir }) => planSerato(seratoDir),
    apply_serato_sync: ({ seratoDir }): SeratoSyncReport => {
      const plans = planSerato(seratoDir);
      const folders = plans.map((plan) => {
        const names = (kind: SeratoFilePlan["action"]["action"]) =>
          plan.files
            .filter((f) => f.action.action === kind)
            .map((f) => f.filename);
        return {
          serato_dir: plan.serato_dir,
          report: {
            backup_dir: `/mock/backups/serato/${Date.now()}`,
            created: names("create"),
            updated: names("update"),
            deleted: names("delete"),
            unchanged: names("unchanged").length,
            neworder_written: true,
          },
        };
      });
      for (const vibe of state.vibes) {
        state.synced.set(
          vibe.id,
          state.tracks.filter(
            (t) => t.vibes.includes(vibe.id) && t.missing_since === null,
          ).length,
        );
      }
      for (const plan of plans)
        recordSync("serato", plan.serato_dir, `${plan.files.length} crates`);
      return { folders };
    },
    plan_rekordbox_export: ({ path }) => planRekordbox(path),
    export_rekordbox_xml: ({ path }): RekordboxExportReport => {
      const plan = planRekordbox(path);
      const report: RekordboxExportReport = {
        path: plan.path,
        track_count: plan.track_count,
        playlist_count: plan.playlist_count,
        changed: plan.changed,
        backup_path:
          plan.changed && plan.exists
            ? `/mock/backups/rekordbox/${Date.now()}/rekordbox.xml`
            : null,
        rekordbox_running: false,
      };
      state.rekordboxExported = rekordboxSignature();
      recordSync("rekordbox", plan.path, `${plan.playlist_count} playlists`);
      return report;
    },
    plan_rekordbox_tags: ({ db }) => planTags(db),
    apply_rekordbox_tags: ({ db }): RekordboxTagReport => {
      const plan = planTags(db);
      state.tagsSynced = rekordboxSignature();
      if (plan.changed)
        recordSync("rekordbox", plan.db_path, `${plan.tags.length} My Tags`);
      return {
        db_path: plan.db_path,
        changed: plan.changed,
        created_category: plan.create_category,
        created_tags: plan.tags
          .filter((t) => t.action === "create")
          .map((t) => t.name),
        deleted_tags: [],
        added: plan.added,
        removed: plan.removed,
        skipped_tracks: plan.skipped_tracks,
        backup_dir: plan.changed
          ? `/mock/backups/rekordbox-db/${Date.now()}`
          : null,
      };
    },
    plan_traktor_sync: ({ collection }) => planTraktor(collection),
    apply_traktor_sync: ({ collection }): TraktorSyncReport => {
      const plan = planTraktor(collection);
      state.traktorSynced = rekordboxSignature();
      if (plan.changed)
        recordSync(
          "traktor",
          plan.collection,
          `${plan.playlists.length} playlists`,
        );
      return {
        collection: plan.collection,
        playlist_count: plan.playlists.length,
        track_count: plan.track_count,
        skipped_tracks: plan.skipped_tracks,
        changed: plan.changed,
        backup_path: plan.changed
          ? `/mock/backups/traktor/${Date.now()}/collection.nml`
          : null,
      };
    },
    plan_tag_mirror: ({ vibe }): CommentChange[] => {
      const scope =
        vibe === null ? null : state.vibes.find((v) => v.path === vibe);
      if (vibe !== null && !scope)
        fail("not_found", `no vibe named ${String(vibe)}`);
      return state.tracks
        .filter(
          (t) =>
            t.missing_since === null && (!scope || t.vibes.includes(scope.id)),
        )
        .map((t) => ({
          track_id: t.id,
          path: t.path,
          before: t.tags.comment,
          after: mirroredComment(state, t),
        }))
        .filter((change) => change.before !== change.after);
    },
    apply_tag_mirror: ({ vibe }): MirrorReport => {
      const changes = handlers.plan_tag_mirror?.({ vibe }) as CommentChange[];
      for (const change of changes) {
        const track = find(change.track_id);
        if (!state.snapshots.has(track.id))
          state.snapshots.set(track.id, track.tags.comment);
        track.tags = { ...track.tags, comment: change.after };
      }
      if (changes.length > 0)
        recordSync("serato", "tag mirror", `${changes.length} files`);
      return {
        written: changes.map((c) => c.track_id),
        unchanged: [],
        failed: [],
      };
    },
    restore_comments: (): MirrorReport => {
      const written: TrackId[] = [];
      for (const [id, comment] of state.snapshots) {
        const track = find(id);
        track.tags = { ...track.tags, comment };
        written.push(track.id);
      }
      state.snapshots.clear();
      return { written, unchanged: [], failed: [] };
    },
    sync_history: ({ limit }) => state.history.slice(0, Number(limit)),
  };

  const scan = async (args: Args): Promise<ScanReport> => {
    const total = state.tracks.length;
    send<ScanProgress>(args.onProgress, { phase: "walking", found: total });
    send<ScanProgress>(args.onProgress, {
      phase: "processing",
      done: total,
      total,
    });
    send<ScanProgress>(args.onProgress, {
      phase: "saving",
      done: total,
      total,
    });
    await sleep(20);
    return {
      added: 0,
      updated: args.refresh === true ? total : 0,
      unchanged: args.refresh === true ? 0 : total,
      missing: state.tracks.filter((t) => t.missing_since !== null).length,
      failed: [],
      unavailable_roots: [],
      elapsed_ms: 420,
    };
  };

  const audit = async (args: Args): Promise<AuditReport> => {
    if (state.auditRunning) fail("busy", "an audit is already running");
    state.auditRunning = true;
    state.cancelAudit = false;
    try {
      const ids =
        (args.trackIds as TrackId[] | null) ??
        state.tracks.filter((t) => t.quality === null).map((t) => t.id);
      const rand = seeded(ids.length + 17);
      const verdicts: QualityVerdict[] = [];
      const flagged: TrackId[] = [];
      for (const [index, id] of ids.entries()) {
        if (state.cancelAudit) break;
        await sleep(8);
        const track = find(id);
        const roll = rand();
        const verdict: QualityVerdict =
          roll < 0.1 ? "suspect" : roll < 0.2 ? "warning" : "pass";
        const cutoff =
          verdict === "suspect"
            ? 16_000
            : verdict === "warning"
              ? 19_000
              : 22_000;
        track.quality = {
          ...report(verdict, cutoff, rand),
          review: track.quality?.review ?? "unreviewed",
        };
        verdicts.push(verdict);
        if (verdict !== "pass") flagged.push(track.id);
        send<AuditProgress>(args.onProgress, {
          done: index + 1,
          total: ids.length,
          track_id: track.id,
          verdict,
        });
      }
      const cancelled = verdicts.length < ids.length;
      return {
        analyzed: verdicts.length,
        skipped: ids.length - verdicts.length,
        cancelled,
        verdicts: countVerdicts(verdicts),
        flagged,
        analyzer_version: ANALYZER_VERSION,
        elapsed_ms: verdicts.length * 8,
        decoded_with_ffmpeg: 0,
        ffmpeg_available: true,
      };
    } finally {
      state.auditRunning = false;
    }
  };

  return async (cmd, args = {}) => {
    switch (cmd) {
      case "scan_library":
        return scan(args);
      case "audit_tracks":
        return audit(args);
      case "cancel_audit":
        state.cancelAudit = true;
        return null;
      default: {
        const handler = handlers[cmd];
        if (!handler) fail("internal", `mock backend has no command ${cmd}`);
        return structuredClone(handler(args));
      }
    }
  };
}

export function installMockBackend(): void {
  const handler = createMockBackend();
  mockIPC((cmd, args) => handler(cmd, args as Args | undefined));
}
