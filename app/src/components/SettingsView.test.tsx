import { screen, waitFor, within } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ScanProgress, ScanReport } from "../lib/types";
import { backend, resetBackend } from "../test/backendMock";
import { renderApp, resetStores } from "../test/render";

vi.mock("../lib/backend", () => import("../test/backendMock"));
vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: vi.fn(async () => "/Volumes/Music"),
}));

beforeEach(() => {
  resetBackend();
  resetStores();
});

describe("Settings view", () => {
  it("streams scan progress, then reloads the tracks", async () => {
    let report: (value: ScanReport) => void = () => {};
    let progress: (value: ScanProgress) => void = () => {};
    backend.scanLibrary.mockImplementation(
      (_refresh: boolean, onProgress: typeof progress) => {
        progress = onProgress;
        return new Promise<ScanReport>((resolve) => {
          report = resolve;
        });
      },
    );
    const { user } = await renderApp();
    await user.click(screen.getByRole("button", { name: "Settings" }));
    const scan = screen.getByRole("region", { name: "Scan" });
    await user.click(
      within(scan).getByRole("checkbox", { name: "Refresh all tags" }),
    );
    await user.click(within(scan).getByRole("button", { name: "Scan" }));
    expect(backend.scanLibrary).toHaveBeenCalledWith(
      true,
      expect.any(Function),
    );

    progress({ phase: "processing", done: 50, total: 100 });
    const bar = await within(scan).findByRole("progressbar", {
      name: "Reading tags 50 of 100",
    });
    expect(bar).toHaveAttribute("aria-valuenow", "45");

    report({
      added: 3,
      updated: 1,
      unchanged: 96,
      missing: 0,
      failed: [],
      unavailable_roots: [],
      elapsed_ms: 1200,
    });
    expect(
      await within(scan).findByText(/3 added, 1 updated/),
    ).toBeInTheDocument();
    await waitFor(() => expect(backend.listTracks).toHaveBeenCalledTimes(2));
  });

  it("adds a library folder from the folder picker", async () => {
    backend.addRoot.mockResolvedValue({
      id: 1,
      path: "/Volumes/Music",
      added_at: 0,
    });
    const { user } = await renderApp();
    await user.click(screen.getByRole("button", { name: "Settings" }));
    backend.listRoots.mockResolvedValue([
      { id: 1, path: "/Volumes/Music", added_at: 0 },
    ]);
    await user.click(screen.getByRole("button", { name: "Add folder" }));
    await waitFor(() =>
      expect(backend.addRoot).toHaveBeenCalledWith("/Volumes/Music"),
    );
    expect(
      await screen.findByRole("button", { name: "Remove /Volumes/Music" }),
    ).toBeInTheDocument();
  });
});

describe("Metadata lookup settings", () => {
  it("saves credentials without showing them back", async () => {
    backend.setLookupSettings.mockResolvedValue({
      acoustid_key_set: true,
      discogs_token_set: false,
      contact: null,
      providers: ["acoustid", "musicbrainz"],
    });
    const { user } = await renderApp();
    await user.click(screen.getByRole("button", { name: "Settings" }));
    const card = screen.getByRole("region", { name: "Metadata lookup" });
    await user.type(within(card).getByLabelText(/AcoustID key/), "secret");
    await user.click(within(card).getByRole("button", { name: "Save" }));
    expect(backend.setLookupSettings).toHaveBeenCalledWith("secret", null, "");
    await waitFor(() =>
      expect(
        within(card).getByText("Using: acoustid, musicbrainz"),
      ).toBeInTheDocument(),
    );
    expect(within(card).getByLabelText(/AcoustID key/)).toHaveValue("");
  });
});
