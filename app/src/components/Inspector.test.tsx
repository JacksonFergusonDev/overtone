import { screen, waitFor, within } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { SpectrogramImage } from "../lib/types";
import { useUi } from "../store/ui";
import { TRACKS, quality } from "../test/fixtures";
import { backend, resetBackend } from "../test/backendMock";
import { renderApp, resetStores } from "../test/render";

vi.mock("../lib/backend", () => import("../test/backendMock"));

beforeEach(() => {
  resetBackend();
  resetStores();
});

const inspector = () =>
  screen.getByRole("complementary", { name: "Quality inspector" });

describe("Quality inspector", () => {
  it("opens on the review queue and advances after a review", async () => {
    backend.setReview.mockImplementation(async (id: number, review) => {
      const track = TRACKS.find((t) => t.id === id);
      if (!track?.quality) throw new Error("no report");
      return { ...track, quality: { ...track.quality, review } };
    });
    const { user } = await renderApp();
    await user.click(screen.getByRole("button", { name: /^Needs review/ }));
    const queue = screen.getByRole("grid", { name: "Needs review" });
    // Worst first: Bravo (suspect), then Charlie (warning).
    expect(within(queue).getAllByRole("row")).toHaveLength(3);
    expect(
      within(inspector()).getByRole("heading", { name: "Bravo" }),
    ).toBeInTheDocument();
    expect(
      within(inspector()).getByText("Cutoff at 16 kHz"),
    ).toBeInTheDocument();

    await user.keyboard("o");
    expect(backend.setReview).toHaveBeenCalledWith(2, "verified_ok");
    await waitFor(() =>
      expect(
        within(inspector()).getByRole("heading", { name: "Charlie" }),
      ).toBeInTheDocument(),
    );
    expect(within(queue).getAllByRole("row")).toHaveLength(2);

    await user.click(
      within(inspector()).getByRole("button", { name: /Replace wanted/ }),
    );
    expect(backend.setReview).toHaveBeenLastCalledWith(3, "replace_wanted");
    expect(await screen.findByText(/Nothing needs review/)).toBeInTheDocument();
  });

  it("shows the spectrogram with a frequency axis and a cutoff marker", async () => {
    const image: SpectrogramImage = {
      path: "/cache/2-lin.png",
      report: quality({ verdict: "suspect", cutoff_hz: 16_000 }),
      log_frequency: false,
    };
    backend.renderSpectrogram.mockResolvedValue(image);
    const { user } = await renderApp();
    await user.click(screen.getByRole("row", { name: /Bravo/ }));
    await user.keyboard("i");
    const img = await within(inspector()).findByRole("img", {
      name: /Linear spectrogram of Bravo/,
    });
    expect(img).toHaveAttribute("src", "asset:///cache/2-lin.png");
    expect(within(inspector()).getByText("Cutoff 16 kHz")).toBeInTheDocument();
    expect(within(inspector()).getByText("20")).toBeInTheDocument();
    expect(backend.renderSpectrogram).toHaveBeenCalledWith(2, false);

    await user.click(within(inspector()).getByRole("button", { name: "Log" }));
    expect(backend.renderSpectrogram).toHaveBeenCalledWith(2, true);
    expect(within(inspector()).getByText("0.05")).toBeInTheDocument();
    expect(useUi.getState().inspectorOpen).toBe(true);
  });

  it("offers to analyze a track that has no report", async () => {
    backend.auditTracks.mockResolvedValue({
      analyzed: 1,
      verdicts: {
        pass: 1,
        warning: 0,
        suspect: 0,
        inconclusive: 0,
        unchecked: 0,
      },
      flagged: [],
      analyzer_version: 1,
      elapsed_ms: 10,
    });
    const { user } = await renderApp();
    await user.click(screen.getByRole("row", { name: /Delta/ }));
    await user.keyboard("i");
    expect(
      within(inspector()).getByText(/file is missing/),
    ).toBeInTheDocument();
    expect(
      within(inspector()).getByRole("button", { name: "Analyze this track" }),
    ).toBeDisabled();
  });
});
