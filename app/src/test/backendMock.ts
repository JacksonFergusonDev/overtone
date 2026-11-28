/**
 * Stand-in for `src/lib/backend.ts`. Use it with
 * `vi.mock("../lib/backend", () => import("../test/backendMock"))`
 * (path relative to the test file), then configure `backend.*` per test.
 */
import { vi } from "vitest";
import type { Backend } from "../lib/backend";
import { STATUS, TRACKS, VIBES } from "./fixtures";

type Mocked<T> = { [K in keyof T]: T[K] & ReturnType<typeof vi.fn> };

function createBackend(): Mocked<Backend> {
  return {
    appStatus: vi.fn(async () => STATUS),
    listTracks: vi.fn(async () => TRACKS),
    listVibes: vi.fn(async () => VIBES),
    listRoots: vi.fn(async () => []),
    addRoot: vi.fn(),
    removeRoot: vi.fn(async () => null),
    scanLibrary: vi.fn(),
    auditTracks: vi.fn(),
    cancelAudit: vi.fn(async () => null),
    setReview: vi.fn(),
    trackWaveform: vi.fn(async () => null),
    renderSpectrogram: vi.fn(() => new Promise(() => {})),
    createVibe: vi.fn(),
    renameVibe: vi.fn(),
    deleteVibe: vi.fn(async () => null),
    assignVibe: vi.fn(async () => null),
    unassignVibe: vi.fn(async () => null),
    djAppsRunning: vi.fn(async () => ({
      serato: false,
      rekordbox: false,
      traktor: false,
    })),
    planSeratoSync: vi.fn(async () => []),
    applySeratoSync: vi.fn(),
    planRekordboxExport: vi.fn(),
    exportRekordboxXml: vi.fn(),
    planRekordboxTags: vi.fn(),
    applyRekordboxTags: vi.fn(),
    planTraktorSync: vi.fn(),
    applyTraktorSync: vi.fn(),
    planTagMirror: vi.fn(async () => []),
    applyTagMirror: vi.fn(),
    restoreComments: vi.fn(),
    syncHistory: vi.fn(async () => []),
    trackDetails: vi.fn(async (trackId: number) => ({
      track_id: trackId,
      fields: [],
      suggestions: [],
      loudness: null,
      integrity: null,
      conversion: null,
      unwritten: [],
      restorable: false,
    })),
    setTrackField: vi.fn(),
    resetTrackField: vi.fn(),
    listSuggestions: vi.fn(async () => []),
    acceptSuggestion: vi.fn(),
    rejectSuggestion: vi.fn(async () => null),
    lookupSettings: vi.fn(async () => ({
      acoustid_key_set: false,
      discogs_token_set: false,
      contact: null,
      providers: ["musicbrainz"],
    })),
    setLookupSettings: vi.fn(),
    lookupTracks: vi.fn(),
    checkIntegrity: vi.fn(),
    exportTracks: vi.fn(),
    planConversion: vi.fn(),
    convertTracks: vi.fn(),
    discardConversion: vi.fn(async () => true),
    planTagWrites: vi.fn(async () => []),
    writeTags: vi.fn(),
    restoreOriginalTags: vi.fn(),
  } as unknown as Mocked<Backend>;
}

export const backend = createBackend();

/** Restores every mock to the default fixtures. Call in `beforeEach`. */
export function resetBackend(): void {
  Object.assign(backend, createBackend());
}

export function errorMessage(error: unknown): string {
  if (typeof error === "object" && error !== null && "message" in error) {
    return String((error as { message: unknown }).message);
  }
  return String(error);
}

export const audioSrc = (path: string): string => `asset://${path}`;
export const imageSrc = (path: string): string => `asset://${path}`;
