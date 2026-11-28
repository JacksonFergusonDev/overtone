import type {
  Conversion,
  ConvertReport,
  ExportReport,
  ExportSettings,
  FieldSource,
  FieldValue,
  IntegrityCheck,
  IntegrityRunReport,
  LookupReport,
  LookupSettings,
  LoudnessReport,
  MetadataField,
  MirrorReport,
  MetadataSuggestion,
  Provider,
  TagWritePlan,
  Track,
  TrackDetails,
  TrackId,
} from "../api";

/**
 * The mock backend's metadata, lookup, integrity, export, and conversion
 * commands. Split from `mockBackend.ts`, which wires these in.
 */

export const METADATA_FIELDS: MetadataField[] = [
  "title",
  "artist",
  "album",
  "album_artist",
  "remixer",
  "label",
  "genre",
  "year",
  "bpm",
  "key",
  "comment",
  "grouping",
  "catalog_number",
  "isrc",
  "release_date",
  "track_number",
  "disc_number",
  "store_genre",
];

const PERSONAL: MetadataField[] = ["comment", "grouping"];

/** Fields the real backend can write into files. */
const WRITABLE: MetadataField[] = [
  "title",
  "artist",
  "album",
  "album_artist",
  "remixer",
  "label",
  "genre",
  "year",
  "bpm",
  "key",
  "grouping",
];

function setFileValue(
  track: Track,
  field: MetadataField,
  value: string | null,
) {
  const t = track.tags;
  switch (field) {
    case "year":
      t.year = value === null ? null : Number.parseInt(value, 10);
      break;
    case "bpm":
      t.bpm = value === null ? null : Number.parseFloat(value);
      break;
    case "title":
    case "artist":
    case "album":
    case "album_artist":
    case "remixer":
    case "label":
    case "genre":
    case "key":
    case "grouping":
      t[field] = value;
      break;
    default:
      break;
  }
}

interface Override {
  value: string | null;
  source: FieldSource;
  provider: string | null;
  set_at: number;
}

export interface MockJobs {
  running: () => boolean;
  start: () => void;
  stop: () => void;
  cancelled: () => boolean;
}

export interface MockContext {
  tracks: () => Track[];
  find: (id: unknown) => Track;
  fail: (code: string, message: string) => never;
  sleep: (ms: number) => Promise<void>;
  send: (channel: unknown, message: unknown) => void;
  jobs: MockJobs;
  now: () => number;
}

type Args = Record<string, unknown>;

function fileValue(track: Track, field: MetadataField): string | null {
  const t = track.tags;
  switch (field) {
    case "title":
      return t.title;
    case "artist":
      return t.artist;
    case "album":
      return t.album;
    case "album_artist":
      return t.album_artist;
    case "remixer":
      return t.remixer;
    case "label":
      return t.label;
    case "genre":
      return t.genre;
    case "year":
      return t.year === null ? null : String(t.year);
    case "bpm":
      return t.bpm === null ? null : String(Number(t.bpm.toFixed(2)));
    case "key":
      return t.key;
    case "comment":
      return t.comment;
    case "grouping":
      return t.grouping;
    default:
      return null;
  }
}

export function createMetadataMock(ctx: MockContext) {
  const overrides = new Map<number, Map<MetadataField, Override>>();
  const suggestions: MetadataSuggestion[] = [];
  const integrity = new Map<number, IntegrityCheck>();
  const conversions = new Map<number, Conversion>();
  const looked = new Set<number>();
  const settings = {
    acoustid: false,
    discogs: false,
    contact: null as string | null,
  };
  let nextSuggestion = 1;
  let nextConversion = 1;

  const fieldValue = (track: Track, field: MetadataField): FieldValue => {
    const found = overrides.get(track.id)?.get(field);
    if (found) {
      return {
        field,
        value: found.value,
        source: found.source,
        provider: found.provider,
        provider_ref: null,
        set_at: found.set_at,
        file_value: fileValue(track, field),
      };
    }
    return {
      field,
      value: fileValue(track, field),
      source: "file",
      provider: null,
      provider_ref: null,
      set_at: null,
      file_value: null,
    };
  };

  const originals = new Map<number, Map<MetadataField, string | null>>();

  const tagPlan = (track: Track): TagWritePlan | null => {
    const changes = WRITABLE.flatMap((field) => {
      const found = overrides.get(track.id)?.get(field);
      const before = fileValue(track, field);
      if (!found || found.value === before) return [];
      return [{ field, before, after: found.value }];
    });
    return changes.length
      ? { track_id: track.id, path: track.path, changes }
      : null;
  };

  const tagPlans = (ids: TrackId[] | null): TagWritePlan[] =>
    (ids ?? [...overrides.keys()])
      .map((id) => ctx.find(id))
      .filter((t) => t.missing_since === null)
      .map(tagPlan)
      .filter((p): p is TagWritePlan => p !== null);

  const suggest = (
    track: Track,
    field: MetadataField,
    value: string,
    provider: string,
  ) => {
    const current = fieldValue(track, field);
    if (current.value === value) return 0;
    if (
      suggestions.some(
        (s) =>
          s.track_id === track.id &&
          s.field === field &&
          s.provider === provider &&
          s.value === value,
      )
    )
      return 0;
    suggestions.push({
      id: nextSuggestion++,
      track_id: track.id,
      field,
      value,
      provider,
      provider_ref: `${provider}/${track.id}`,
      confidence: 0.92,
      fetched_at: ctx.now(),
      status: "pending",
      decided_at: null,
      current,
    });
    return 1;
  };

  // A few tracks start with suggestions so the inspector has some to show.
  for (const track of ctx.tracks()) {
    if (track.id % 9 === 0) {
      suggest(track, "label", "Hospital Records", "musicbrainz");
      suggest(track, "catalog_number", `NHS${track.id}`, "discogs");
      suggest(track, "store_genre", "Drum & Bass, Liquid Funk", "discogs");
    }
  }

  const loudness = (track: Track): LoudnessReport | null => {
    if (!track.quality || track.quality.verdict === "unchecked") return null;
    const r = (track.id * 7919) % 100;
    return {
      integrated_lufs: -6 - (r % 9) - (r % 10) / 10,
      loudness_range_lu: 3 + (r % 5),
      true_peak_dbtp: 1.2 - (r % 20) / 10,
      sample_peak_dbfs: -(r % 10) / 10,
      clipped_runs: r % 3 === 0 ? r : 0,
      leading_silence_ms: r % 11 === 0 ? 1500 : 20,
    };
  };

  const pending = (trackId: number) =>
    suggestions
      .map((s) => ({
        ...s,
        current: fieldValue(ctx.find(s.track_id), s.field),
      }))
      .filter(
        (s) =>
          s.status === "pending" && (trackId < 0 || s.track_id === trackId),
      );

  const lookupSettings = (): LookupSettings => {
    const providers: Provider[] = [];
    if (settings.acoustid) providers.push("acoustid");
    providers.push("musicbrainz");
    if (settings.discogs) providers.push("discogs");
    return {
      acoustid_key_set: settings.acoustid,
      discogs_token_set: settings.discogs,
      contact: settings.contact,
      providers,
    };
  };

  const convertible = (track: Track) =>
    track.missing_since === null &&
    (track.properties.format === "wav" || track.properties.format === "aiff");

  const planConversion = (ids: TrackId[] | null): ConvertReport => {
    const chosen = (
      ids ??
      ctx
        .tracks()
        .filter(convertible)
        .map((t) => t.id)
    ).map(ctx.find);
    const report: ConvertReport = {
      planned: [],
      converted: [],
      skipped: [],
      failures: [],
      written: false,
      cancelled: false,
    };
    for (const track of chosen) {
      if (!convertible(track)) {
        report.skipped.push({
          track_id: track.id,
          reason: "only WAV and AIFF files are converted to FLAC",
        });
      } else if (conversions.has(track.id)) {
        report.skipped.push({
          track_id: track.id,
          reason: "already converted; waiting for relocation",
        });
      } else {
        report.planned.push({
          track_id: track.id,
          source: track.path,
          output: track.path.replace(/\.[^.]+$/, ".flac"),
          serato: ["Serato Markers2", "Serato BeatGrid", "Serato Overview"],
          dropped: track.id % 3 === 0 ? ["RVAD"] : [],
        });
      }
    }
    return report;
  };

  const sync: Record<string, (args: Args) => unknown> = {
    track_details: (args): TrackDetails => {
      const track = ctx.find(args.trackId);
      return {
        track_id: track.id,
        fields: METADATA_FIELDS.map((f) => fieldValue(track, f)),
        suggestions: pending(track.id),
        loudness: loudness(track),
        integrity: integrity.get(track.id) ?? null,
        conversion: conversions.get(track.id) ?? null,
        unwritten: tagPlan(track)?.changes.map((c) => c.field) ?? [],
        restorable: originals.has(track.id),
      };
    },
    plan_tag_writes: (args) =>
      tagPlans((args.trackIds as TrackId[] | null) ?? null),
    set_track_field: (args) => {
      const track = ctx.find(args.trackId);
      const field = args.field as MetadataField;
      if (field === "comment")
        ctx.fail(
          "field_not_allowed",
          "comment changes only through vibe mirroring",
        );
      const raw = typeof args.value === "string" ? args.value.trim() : "";
      const map = overrides.get(track.id) ?? new Map<MetadataField, Override>();
      map.set(field, {
        value: raw === "" ? null : raw,
        source: "user",
        provider: null,
        set_at: ctx.now(),
      });
      overrides.set(track.id, map);
      return fieldValue(track, field);
    },
    reset_track_field: (args) => {
      const track = ctx.find(args.trackId);
      overrides.get(track.id)?.delete(args.field as MetadataField);
      return fieldValue(track, args.field as MetadataField);
    },
    list_suggestions: () => pending(-1),
    accept_suggestion: (args) => {
      const s =
        suggestions.find((x) => x.id === args.suggestionId) ??
        ctx.fail("not_found", `no suggestion ${String(args.suggestionId)}`);
      if (s.status !== "pending")
        ctx.fail(
          "already_decided",
          `suggestion ${s.id} was already ${s.status}`,
        );
      const track = ctx.find(s.track_id);
      const current = fieldValue(track, s.field);
      if (current.value && current.value !== s.value && args.replace !== true)
        ctx.fail(
          "would_replace",
          `accepting would replace the current ${s.field} "${current.value}"; choose to replace it explicitly`,
        );
      const map = overrides.get(track.id) ?? new Map<MetadataField, Override>();
      map.set(s.field, {
        value: s.value,
        source: "lookup",
        provider: s.provider,
        set_at: ctx.now(),
      });
      overrides.set(track.id, map);
      for (const other of suggestions) {
        if (
          other.track_id === s.track_id &&
          other.field === s.field &&
          other.status === "pending"
        )
          other.status = "rejected";
      }
      s.status = "accepted";
      return fieldValue(track, s.field);
    },
    reject_suggestion: (args) => {
      const s =
        suggestions.find((x) => x.id === args.suggestionId) ??
        ctx.fail("not_found", `no suggestion ${String(args.suggestionId)}`);
      s.status = "rejected";
      return null;
    },
    lookup_settings: lookupSettings,
    set_lookup_settings: (args) => {
      if (typeof args.acoustidKey === "string")
        settings.acoustid = args.acoustidKey.trim() !== "";
      if (typeof args.discogsToken === "string")
        settings.discogs = args.discogsToken.trim() !== "";
      if (typeof args.contact === "string")
        settings.contact =
          args.contact.trim() === "" ? null : args.contact.trim();
      return lookupSettings();
    },
    plan_conversion: (args) =>
      planConversion((args.trackIds as TrackId[] | null) ?? null),
    discard_conversion: (args) => conversions.delete(ctx.find(args.trackId).id),
  };

  const job = async <T>(work: () => Promise<T>): Promise<T> => {
    if (ctx.jobs.running())
      ctx.fail("busy", "another background job is running");
    ctx.jobs.start();
    try {
      return await work();
    } finally {
      ctx.jobs.stop();
    }
  };

  const async: Record<string, (args: Args) => Promise<unknown>> = {
    write_tags: (args) =>
      job(async (): Promise<MirrorReport> => {
        const plans = tagPlans((args.trackIds as TrackId[] | null) ?? null);
        const report: MirrorReport = { written: [], unchanged: [], failed: [] };
        for (const [index, plan] of plans.entries()) {
          if (ctx.jobs.cancelled()) break;
          await ctx.sleep(5);
          const track = ctx.find(plan.track_id);
          const saved = originals.get(track.id) ?? new Map();
          for (const change of plan.changes) {
            if (!saved.has(change.field))
              saved.set(change.field, change.before);
            setFileValue(track, change.field, change.after);
          }
          originals.set(track.id, saved);
          report.written.push(track.id);
          ctx.send(args.onProgress, {
            done: index + 1,
            total: plans.length,
            track_id: track.id,
          });
        }
        return report;
      }),
    restore_original_tags: (args) =>
      job(async (): Promise<MirrorReport> => {
        const ids = (args.trackIds as TrackId[] | null) ?? [
          ...originals.keys(),
        ];
        const report: MirrorReport = { written: [], unchanged: [], failed: [] };
        for (const id of ids) {
          const saved = originals.get(id);
          if (!saved) {
            report.unchanged.push(id);
            continue;
          }
          const track = ctx.find(id);
          for (const [field, value] of saved) setFileValue(track, field, value);
          originals.delete(id);
          report.written.push(id);
        }
        return report;
      }),
    lookup_tracks: (args) =>
      job(async (): Promise<LookupReport> => {
        const ids =
          (args.trackIds as TrackId[] | null) ??
          ctx
            .tracks()
            .filter((t) => !looked.has(t.id))
            .map((t) => t.id);
        const report: LookupReport = {
          looked_up: 0,
          matched: 0,
          suggestions_added: 0,
          providers: lookupSettings().providers,
          disabled: [],
          failures: [],
          cancelled: false,
        };
        for (const id of ids) {
          if (ctx.jobs.cancelled()) {
            report.cancelled = true;
            break;
          }
          await ctx.sleep(4);
          const track = ctx.find(id);
          looked.add(id);
          report.looked_up += 1;
          if (id % 2 === 0) {
            report.matched += 1;
            report.suggestions_added +=
              suggest(track, "release_date", "2021-06-04", "musicbrainz") +
              suggest(track, "label", "Critical Music", "musicbrainz");
          }
          ctx.send(args.onProgress, {
            done: report.looked_up,
            total: ids.length,
            track_id: id,
            suggestions: 0,
          });
        }
        return report;
      }),
    check_integrity: (args) =>
      job(async (): Promise<IntegrityRunReport> => {
        const ids =
          (args.trackIds as TrackId[] | null) ??
          ctx
            .tracks()
            .filter((t) => t.missing_since === null && !integrity.has(t.id))
            .map((t) => t.id);
        const report: IntegrityRunReport = {
          checked: 0,
          skipped: 0,
          cancelled: false,
          damaged: [],
          edge_only: 0,
          ffmpeg_version: "9.1-mock",
          elapsed_ms: ids.length * 5,
        };
        for (const id of ids) {
          if (ctx.jobs.cancelled()) {
            report.cancelled = true;
            break;
          }
          await ctx.sleep(2);
          const damaged = id % 25 === 0;
          const edge = !damaged && id % 13 === 0;
          integrity.set(id, {
            track_id: id,
            clean: !damaged && !edge,
            edge_only: edge,
            errors: damaged || edge ? ["[mp3float] Header missing"] : [],
            error_count: damaged ? 42 : edge ? 4 : 0,
            file_modified_at: 0,
            ffmpeg_version: "9.1-mock",
            checked_at: ctx.now(),
          });
          report.checked += 1;
          if (damaged) report.damaged.push(id);
          if (edge) report.edge_only += 1;
          ctx.send(args.onProgress, {
            done: report.checked,
            total: ids.length,
            track_id: id,
            clean: !damaged,
          });
        }
        report.skipped = ids.length - report.checked;
        return report;
      }),
    export_tracks: (args) =>
      job(async (): Promise<ExportReport> => {
        const ids = args.trackIds as TrackId[];
        const settings = args.settings as ExportSettings;
        const ext = {
          aac: "m4a",
          mp3: "mp3",
          flac: "flac",
          aiff: "aiff",
          wav: "wav",
        }[settings.format];
        const report: ExportReport = {
          exported: [],
          failures: [],
          cancelled: false,
        };
        for (const [index, id] of ids.entries()) {
          await ctx.sleep(4);
          const track = ctx.find(id);
          const name =
            track.path
              .split("/")
              .pop()
              ?.replace(/\.[^.]+$/, "") ?? "track";
          report.exported.push({
            track_id: id,
            path: `${String(args.folder)}/${name}.${ext}`,
          });
          ctx.send(args.onProgress, {
            done: index + 1,
            total: ids.length,
            track_id: id,
          });
        }
        return report;
      }),
    convert_tracks: (args) =>
      job(async (): Promise<ConvertReport> => {
        const report = planConversion(
          (args.trackIds as TrackId[] | null) ?? null,
        );
        report.written = true;
        for (const [index, plan] of report.planned.entries()) {
          if (ctx.jobs.cancelled()) {
            report.cancelled = true;
            break;
          }
          await ctx.sleep(6);
          conversions.set(plan.track_id, {
            id: nextConversion++,
            track_id: plan.track_id,
            output_path: plan.output,
            format: "flac",
            status: "pending_relocation",
            dropped: plan.dropped,
            serato: plan.serato,
            frames: 13_230_000,
            created_at: ctx.now(),
          });
          report.converted.push(plan.track_id);
          ctx.send(args.onProgress, {
            done: index + 1,
            total: report.planned.length,
            track_id: plan.track_id,
          });
        }
        return report;
      }),
  };

  return { sync, async, personal: PERSONAL };
}
