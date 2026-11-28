import { screen, waitFor, within } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type {
  FieldValue,
  MetadataSuggestion,
  TrackDetails,
} from "../lib/types";
import { backend, resetBackend } from "../test/backendMock";
import { renderApp, resetStores } from "../test/render";

vi.mock("../lib/backend", () => import("../test/backendMock"));
vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: vi.fn(async () => "/Volumes/USB"),
}));

beforeEach(() => {
  resetBackend();
  resetStores();
});

const field = (overrides: Partial<FieldValue>): FieldValue => ({
  field: "label",
  value: null,
  source: "file",
  provider: null,
  provider_ref: null,
  set_at: null,
  file_value: null,
  ...overrides,
});

const suggestion = (
  overrides: Partial<MetadataSuggestion>,
): MetadataSuggestion => ({
  id: 7,
  track_id: 2,
  field: "label",
  value: "Hospital Records",
  provider: "musicbrainz",
  provider_ref: null,
  confidence: 0.9,
  fetched_at: 0,
  status: "pending",
  decided_at: null,
  current: field({}),
  ...overrides,
});

function details(trackId: number, extra: Partial<TrackDetails>): TrackDetails {
  return {
    track_id: trackId,
    fields: [field({ field: "title", value: "Bravo" })],
    suggestions: [],
    loudness: null,
    integrity: null,
    conversion: null,
    unwritten: [],
    restorable: false,
    ...extra,
  };
}

const inspector = () =>
  screen.getByRole("complementary", { name: "Inspector" });

async function openFirstReviewTrack() {
  const app = await renderApp();
  await app.user.click(screen.getByRole("button", { name: /^Needs review/ }));
  return app;
}

describe("Track details", () => {
  it("shows each value's source and lets the DJ accept a suggestion", async () => {
    backend.trackDetails.mockImplementation(async (id: number) =>
      details(id, {
        fields: [
          field({ field: "title", value: "Bravo" }),
          field({
            field: "album",
            value: "Mine",
            source: "user",
            file_value: "Theirs",
          }),
        ],
        suggestions: [suggestion({ track_id: id })],
      }),
    );
    backend.acceptSuggestion.mockResolvedValue(
      field({
        value: "Hospital Records",
        source: "lookup",
        provider: "musicbrainz",
      }),
    );
    const { user } = await openFirstReviewTrack();
    const metadata = await within(inspector()).findByRole("region", {
      name: "Metadata",
    });
    expect(within(metadata).getByText("you")).toBeInTheDocument();
    expect(
      within(metadata).getByRole("button", { name: "Revert" }),
    ).toBeInTheDocument();

    const suggestions = within(inspector()).getByRole("region", {
      name: "Suggestions",
    });
    expect(
      within(suggestions).getByText(/Hospital Records/),
    ).toBeInTheDocument();
    await user.click(
      within(suggestions).getByRole("button", { name: "Accept" }),
    );
    expect(backend.acceptSuggestion).toHaveBeenCalledWith(7, false);
  });

  it("asks to replace when a suggestion would overwrite a value", async () => {
    backend.trackDetails.mockImplementation(async (id: number) =>
      details(id, {
        suggestions: [
          suggestion({
            track_id: id,
            current: field({ value: "Mine", source: "user" }),
          }),
        ],
      }),
    );
    backend.acceptSuggestion.mockResolvedValue(field({}));
    const { user } = await openFirstReviewTrack();
    const suggestions = await within(inspector()).findByRole("region", {
      name: "Suggestions",
    });
    expect(
      await within(suggestions).findByText(/Would replace Mine/),
    ).toBeInTheDocument();
    await user.click(
      within(suggestions).getByRole("button", { name: "Replace" }),
    );
    expect(backend.acceptSuggestion).toHaveBeenCalledWith(7, true);
  });

  it("edits a field as the DJ's own value", async () => {
    backend.trackDetails.mockImplementation(async (id: number) =>
      details(id, { fields: [field({ field: "label", value: "Old" })] }),
    );
    backend.setTrackField.mockResolvedValue(
      field({ value: "New", source: "user" }),
    );
    const { user } = await openFirstReviewTrack();
    const metadata = await within(inspector()).findByRole("region", {
      name: "Metadata",
    });
    await user.click(
      await within(metadata).findByRole("button", { name: "Edit" }),
    );
    const input = within(metadata).getByRole("textbox", { name: "Label" });
    await user.clear(input);
    await user.type(input, "New{Enter}");
    expect(backend.setTrackField).toHaveBeenCalledWith(2, "label", "New");
  });

  it("shows loudness and flags decode errors in the music", async () => {
    backend.trackDetails.mockImplementation(async (id: number) =>
      details(id, {
        loudness: {
          integrated_lufs: -7.4,
          loudness_range_lu: 4,
          true_peak_dbtp: 0.6,
          sample_peak_dbfs: -0.1,
          clipped_runs: 12,
          leading_silence_ms: 1800,
        },
        integrity: {
          track_id: id,
          clean: false,
          edge_only: false,
          errors: ["[flac] invalid sync code"],
          error_count: 4,
          file_modified_at: 0,
          ffmpeg_version: "9.1",
          checked_at: 0,
        },
      }),
    );
    await openFirstReviewTrack();
    const sound = await within(inspector()).findByRole("region", {
      name: "Sound",
    });
    await waitFor(() =>
      expect(within(sound).getByText("-7.4 LUFS")).toBeInTheDocument(),
    );
    expect(within(sound).getByText("12 runs")).toBeInTheDocument();
    expect(
      within(sound).getByText(/1.8 s before the music/),
    ).toBeInTheDocument();
    expect(
      within(sound).getByText(/4 decode error\(s\) in the music/),
    ).toBeInTheDocument();
  });
});

describe("Track details after a job", () => {
  it("reloads the panel when a lookup finishes", async () => {
    let calls = 0;
    backend.trackDetails.mockImplementation(async (id: number) => {
      calls += 1;
      return details(id, {
        suggestions: calls > 1 ? [suggestion({ track_id: id })] : [],
      });
    });
    backend.lookupTracks.mockResolvedValue({
      looked_up: 1,
      matched: 1,
      suggestions_added: 1,
      providers: ["musicbrainz"],
      disabled: [],
      failures: [],
      cancelled: false,
    });
    const { user } = await openFirstReviewTrack();
    const suggestions = await within(inspector()).findByRole("region", {
      name: "Suggestions",
    });
    await user.click(
      await within(suggestions).findByRole("button", {
        name: "Look this track up",
      }),
    );
    expect(
      await within(suggestions).findByText(/Hospital Records/),
    ).toBeInTheDocument();
  });
});

describe("Writing values into the file", () => {
  it("previews, confirms, and writes, then offers to restore", async () => {
    let written = false;
    backend.trackDetails.mockImplementation(async (id: number) =>
      details(id, {
        fields: [
          field({
            field: "label",
            value: "Hospital",
            source: "user",
            file_value: written ? "Hospital" : "Old",
          }),
        ],
        unwritten: written ? [] : ["label"],
        restorable: written,
      }),
    );
    backend.planTagWrites.mockResolvedValue([
      {
        track_id: 2,
        path: "/m/b.mp3",
        changes: [{ field: "label", before: "Old", after: "Hospital" }],
      },
    ]);
    backend.writeTags.mockImplementation(async () => {
      written = true;
      return { written: [2], unchanged: [], failed: [] };
    });
    const { user } = await openFirstReviewTrack();
    const metadata = await within(inspector()).findByRole("region", {
      name: "Metadata",
    });
    expect(
      await within(metadata).findByLabelText("Not in the file yet"),
    ).toBeInTheDocument();
    await user.click(
      within(metadata).getByRole("button", { name: "Write to file…" }),
    );
    const dialog = await screen.findByRole("alertdialog");
    expect(
      within(dialog).getByText(/Label: Old → Hospital/),
    ).toBeInTheDocument();
    await user.click(within(dialog).getByRole("button", { name: "Write" }));
    await waitFor(() =>
      expect(backend.writeTags).toHaveBeenCalledWith([2], expect.any(Function)),
    );
    expect(
      await within(metadata).findByRole("button", {
        name: "Restore the file's original values",
      }),
    ).toBeInTheDocument();
    expect(
      within(metadata).queryByLabelText("Not in the file yet"),
    ).not.toBeInTheDocument();
  });
});
