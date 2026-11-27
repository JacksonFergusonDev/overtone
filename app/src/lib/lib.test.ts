import { describe, expect, it } from "vitest";
import { quality, track, vibe } from "../test/fixtures";
import { EMPTY_FILTER, filterTracks, needsReview } from "./filter";
import { formatAudio, formatBpm, formatDuration } from "./format";
import { frequencyFraction, frequencyTicks } from "./frequency";
import { fuzzyRank, fuzzyScore } from "./fuzzy";
import {
  EMPTY_SELECTION,
  selectedInOrder,
  selectionReducer,
} from "./selection";
import { keyRank, nextSort, sortTracks } from "./sort";
import type { Track } from "./types";
import {
  parseVibePath,
  vibeSlug,
  vibeTree,
  vibeWithDescendants,
} from "./vibes";
import {
  canvasSize,
  clampTime,
  digitFraction,
  resamplePeaks,
  timeToX,
  xToTime,
} from "./waveform";

const mapOf = (tracks: Track[]) => new Map(tracks.map((t) => [t.id, t]));

describe("sortTracks", () => {
  const tracks = [
    track(1, { title: "beta", artist: "B", bpm: 128 }),
    track(2, { title: "Alpha", artist: null, bpm: null }),
    track(3, { title: "alpha 10", artist: "a", bpm: 90 }),
    track(4, { title: "Alpha 9", artist: "C", bpm: 174 }),
  ];
  const map = mapOf(tracks);
  const ids = tracks.map((t) => t.id);

  it("sorts text naturally and case-insensitively", () => {
    expect(sortTracks(ids, map, { key: "title", dir: "asc" })).toEqual([
      2, 4, 3, 1,
    ]);
    expect(sortTracks(ids, map, { key: "title", dir: "desc" })).toEqual([
      1, 3, 4, 2,
    ]);
  });

  it("keeps empty values last in both directions", () => {
    expect(sortTracks(ids, map, { key: "bpm", dir: "asc" })).toEqual([
      3, 1, 4, 2,
    ]);
    expect(sortTracks(ids, map, { key: "bpm", dir: "desc" })).toEqual([
      4, 1, 3, 2,
    ]);
    expect(sortTracks(ids, map, { key: "artist", dir: "desc" }).at(-1)).toBe(2);
  });

  it("puts the worst verdicts first for quality", () => {
    const q = [
      track(1, { quality: quality({ verdict: "pass" }) }),
      track(2, { quality: quality({ verdict: "suspect" }) }),
      track(3, { quality: null }),
      track(4, { quality: quality({ verdict: "warning" }) }),
    ];
    expect(
      sortTracks([1, 2, 3, 4], mapOf(q), { key: "quality", dir: "asc" }),
    ).toEqual([2, 4, 3, 1]);
  });

  it("orders Camelot keys around the wheel", () => {
    expect(keyRank("8A")).toBeLessThan(keyRank("8B") as number);
    expect(keyRank("2A")).toBeLessThan(keyRank("11A") as number);
    expect(keyRank(null)).toBeNull();
  });

  it("flips direction when the same header is clicked", () => {
    expect(nextSort({ key: "title", dir: "asc" }, "title")).toEqual({
      key: "title",
      dir: "desc",
    });
    expect(nextSort({ key: "title", dir: "desc" }, "bpm")).toEqual({
      key: "bpm",
      dir: "asc",
    });
  });
});

describe("filterTracks", () => {
  const vibes = [
    vibe(1, "Dubs"),
    vibe(2, "Dubs/Deep"),
    vibe(3, "Dubstep"),
    vibe(4, "Warmup"),
  ];
  const tracks = [
    track(1, { title: "Rain", artist: "Kode", vibes: [1] }),
    track(2, { title: "Fog", comment: "late night roller", vibes: [2] }),
    track(3, { title: "Wobble", vibes: [3] }),
    track(4, { title: "Dawn", album: "Sunrise", missing: true, vibes: [4] }),
    track(5, { title: "Grit", quality: quality({ verdict: "suspect" }) }),
    track(6, {
      title: "Hiss",
      quality: quality({ verdict: "warning", review: "verified_ok" }),
    }),
  ];
  const map = mapOf(tracks);
  const ids = tracks.map((t) => t.id);
  const run = (patch: Partial<typeof EMPTY_FILTER>) =>
    filterTracks(ids, map, { ...EMPTY_FILTER, ...patch });

  it("matches every word across title, artist, album, and comment", () => {
    expect(run({ query: "kode" })).toEqual([1]);
    expect(run({ query: "night ROLLER" })).toEqual([2]);
    expect(run({ query: "sunrise" })).toEqual([4]);
    expect(run({ query: "rain fog" })).toEqual([]);
  });

  it("includes descendant vibes but not slug look-alikes", () => {
    const ids1 = vibeWithDescendants(vibes, 1);
    expect([...ids1].sort()).toEqual([1, 2]);
    expect(run({ vibeIds: ids1 })).toEqual([1, 2]);
  });

  it("filters by verdict, missing, and needs review", () => {
    expect(run({ verdicts: ["suspect", "warning"] })).toEqual([5, 6]);
    expect(run({ verdicts: ["unchecked"] })).toEqual([1, 2, 3, 4]);
    expect(run({ missing: "missing" })).toEqual([4]);
    expect(run({ needsReview: true })).toEqual([5]);
    expect(needsReview(tracks[5])).toBe(false);
  });
});

describe("selectionReducer", () => {
  const order = [10, 20, 30, 40, 50];

  it("selects, toggles, and selects ranges from the anchor", () => {
    let s = selectionReducer(EMPTY_SELECTION, { type: "select", id: 20 });
    s = selectionReducer(s, { type: "range", id: 40, order });
    expect([...s.selected]).toEqual([20, 30, 40]);
    expect(s.cursor).toBe(40);
    s = selectionReducer(s, { type: "range", id: 10, order });
    expect([...s.selected]).toEqual([10, 20]);
    s = selectionReducer(s, { type: "toggle", id: 50 });
    expect(selectedInOrder(s, order)).toEqual([10, 20, 50]);
    s = selectionReducer(s, { type: "toggle", id: 10 });
    expect(selectedInOrder(s, order)).toEqual([20, 50]);
  });

  it("moves the cursor and extends with shift", () => {
    let s = selectionReducer(EMPTY_SELECTION, {
      type: "move",
      delta: 1,
      order,
      extend: false,
    });
    expect(s.cursor).toBe(10);
    s = selectionReducer(s, { type: "move", delta: 1, order, extend: true });
    s = selectionReducer(s, { type: "move", delta: 1, order, extend: true });
    expect(selectedInOrder(s, order)).toEqual([10, 20, 30]);
    s = selectionReducer(s, { type: "move", delta: -1, order, extend: true });
    expect(selectedInOrder(s, order)).toEqual([10, 20]);
    s = selectionReducer(s, { type: "move", delta: 100, order, extend: false });
    expect(s.cursor).toBe(50);
    expect([...s.selected]).toEqual([50]);
  });

  it("starts from the end when moving up with no cursor", () => {
    const s = selectionReducer(EMPTY_SELECTION, {
      type: "move",
      delta: -1,
      order,
      extend: false,
    });
    expect(s.cursor).toBe(50);
  });

  it("selects all and clears", () => {
    let s = selectionReducer(EMPTY_SELECTION, { type: "all", order });
    expect(s.selected.size).toBe(5);
    s = selectionReducer(s, { type: "clear" });
    expect(s).toBe(EMPTY_SELECTION);
  });
});

describe("waveform math", () => {
  it("converts between time and x and clamps", () => {
    expect(timeToX(30, 120, 600)).toBe(150);
    expect(xToTime(150, 600, 120)).toBe(30);
    expect(timeToX(500, 120, 600)).toBe(600);
    expect(xToTime(-10, 600, 120)).toBe(0);
    expect(timeToX(10, 0, 600)).toBe(0);
    expect(xToTime(10, 0, 120)).toBe(0);
    expect(clampTime(-5, 100)).toBe(0);
    expect(clampTime(105, 100)).toBe(100);
  });

  it("sizes the backing store for HiDPI", () => {
    expect(canvasSize(300, 40, 2)).toEqual({ width: 600, height: 80 });
    expect(canvasSize(300.4, 40, 0)).toEqual({ width: 300, height: 40 });
  });

  it("resamples peaks keeping the extremes", () => {
    const buckets: [number, number][] = [
      [-0.1, 0.2],
      [-0.5, 0.1],
      [-0.2, 0.9],
      [-0.3, 0.3],
    ];
    expect(resamplePeaks(buckets, 2)).toEqual([
      [-0.5, 0.2],
      [-0.3, 0.9],
    ]);
    expect(resamplePeaks(buckets, 8)).toHaveLength(8);
    expect(resamplePeaks(buckets, 8)[7]).toEqual([-0.3, 0.3]);
    expect(resamplePeaks([], 8)).toEqual([]);
  });

  it("maps number keys to fractions", () => {
    expect(digitFraction("1")).toBe(0.1);
    expect(digitFraction("9")).toBe(0.9);
    expect(digitFraction("0")).toBeNull();
  });
});

describe("fuzzy matching", () => {
  it("needs the query as a subsequence", () => {
    expect(fuzzyScore("ddp", "Dubs/Deep")).not.toBeNull();
    expect(fuzzyScore("xyz", "Dubs/Deep")).toBeNull();
  });

  it("ranks prefixes and level starts first", () => {
    const paths = ["Peak Time/Deep", "Deep", "Dubs/Deep", "Ambient"];
    expect(fuzzyRank("deep", paths, (p) => p)).toEqual([
      "Deep",
      "Dubs/Deep",
      "Peak Time/Deep",
    ]);
    expect(fuzzyRank("pt", paths, (p) => p)[0]).toBe("Peak Time/Deep");
    expect(fuzzyRank("", paths, (p) => p)).toEqual(paths);
  });
});

describe("vibe paths", () => {
  it("normalizes whitespace like the backend", () => {
    expect(parseVibePath("  Dubs /  Deep   Cuts ")).toEqual({
      ok: true,
      path: "Dubs/Deep Cuts",
      segments: ["Dubs", "Deep Cuts"],
    });
  });

  it("rejects text that would break sync targets", () => {
    for (const bad of [
      "",
      "  ",
      "Dubs//Deep",
      "/Dubs",
      "a%%b",
      "#tag",
      "a:b",
      "a\\b",
      "a\u0007b",
    ]) {
      expect(parseVibePath(bad).ok, bad).toBe(false);
    }
    expect(parseVibePath("x".repeat(201)).ok).toBe(false);
    expect(parseVibePath("Drum & Bass/90s").ok).toBe(true);
  });

  it("compares by slug", () => {
    expect(vibeSlug("Dubs/Deep  Cuts")).toBe("dubs/deep-cuts");
  });

  it("builds a tree with children after parents", () => {
    const tree = vibeTree([
      vibe(3, "Dubs/Deep"),
      vibe(1, "Dubs-x"),
      vibe(2, "Dubs"),
      vibe(4, "Ambient"),
    ]);
    expect(tree.map((n) => [n.vibe.path, n.depth, n.hasChildren])).toEqual([
      ["Ambient", 0, false],
      ["Dubs", 0, true],
      ["Dubs/Deep", 1, false],
      ["Dubs-x", 0, false],
    ]);
  });
});

describe("frequency axis", () => {
  it("spans 0 to Nyquist on a linear scale", () => {
    const ticks = frequencyTicks(44_100, false);
    expect(ticks[0]).toEqual({ hz: 0, label: "0", fraction: 0 });
    expect(ticks.map((t) => t.label)).toEqual([
      "0",
      "4",
      "8",
      "12",
      "16",
      "20",
    ]);
    expect(ticks.at(-1)?.fraction).toBeCloseTo(20_000 / 22_050);
    expect(frequencyTicks(96_000, false).map((t) => t.label)).toEqual([
      "0",
      "10",
      "20",
      "30",
      "40",
    ]);
  });

  it("spans 20 Hz to Nyquist on a log scale", () => {
    const ticks = frequencyTicks(44_100, true);
    expect(ticks.map((t) => t.label)).toEqual([
      "0.05",
      "0.1",
      "0.2",
      "0.5",
      "1",
      "2",
      "5",
      "10",
      "20",
    ]);
    expect(frequencyFraction(20, 44_100, true)).toBe(0);
    expect(frequencyFraction(22_050, 44_100, true)).toBeCloseTo(1);
    expect(frequencyFraction(16_000, 44_100, false)).toBeCloseTo(
      16_000 / 22_050,
    );
  });
});

describe("formatting", () => {
  it("formats durations, BPM, and audio detail", () => {
    expect(formatDuration(187_000)).toBe("3:07");
    expect(formatDuration(3_723_000)).toBe("1:02:03");
    expect(formatDuration(null)).toBe("");
    expect(formatBpm(128)).toBe("128");
    expect(formatBpm(127.96)).toBe("128.0");
    expect(formatBpm(null)).toBe("");
    expect(
      formatAudio({
        format: "mp3",
        bitrate_kbps: 320,
        sample_rate_hz: 44_100,
        bit_depth: null,
        channels: 2,
        duration_ms: 1,
      }),
    ).toEqual({ label: "MP3", detail: "320k" });
    expect(
      formatAudio({
        format: "flac",
        bitrate_kbps: null,
        sample_rate_hz: 44_100,
        bit_depth: 24,
        channels: 2,
        duration_ms: 1,
      }),
    ).toEqual({ label: "FLAC", detail: "44.1/24" });
  });
});
