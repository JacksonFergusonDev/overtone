import { screen, within } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { usePlayer } from "../store/player";
import { useToasts } from "../store/toasts";
import { backend, resetBackend } from "../test/backendMock";
import { renderApp, resetStores } from "../test/render";

vi.mock("../lib/backend", () => import("../test/backendMock"));

beforeEach(() => {
  resetBackend();
  resetStores();
});

const grid = () => screen.getByRole("grid", { name: "Library" });
const titles = () =>
  within(grid())
    .getAllByRole("row")
    .slice(1)
    .map((row) => within(row).getAllByRole("gridcell")[1].textContent);
const selectedTitles = () =>
  within(grid())
    .getAllByRole("row")
    .filter((row) => row.getAttribute("aria-selected") === "true")
    .map((row) => within(row).getAllByRole("gridcell")[1].textContent);

describe("Library view", () => {
  it("renders every track with its verdict and vibe pills", async () => {
    await renderApp();
    // Sorted by artist (Vee, Wim, Xavi, Yara, Zed).
    expect(titles()).toEqual(["Echo", "Delta", "Charlie", "Bravo", "Alpha"]);
    const bravo = screen.getByRole("row", { name: /Bravo/ });
    expect(within(bravo).getByText("Suspect")).toBeInTheDocument();
    expect(within(bravo).getByText("Dubs/Deep")).toBeInTheDocument();
    expect(
      within(screen.getByRole("row", { name: /Echo/ })).getByText(/OK/),
    ).toBeInTheDocument();
    expect(
      within(screen.getByRole("row", { name: /Delta/ })).getByLabelText(
        "File missing",
      ),
    ).toBeInTheDocument();
  });

  it("sorts when you click a column header", async () => {
    const { user } = await renderApp();
    await user.click(screen.getByRole("button", { name: "Title" }));
    expect(titles()).toEqual(["Alpha", "Bravo", "Charlie", "Delta", "Echo"]);
    expect(screen.getByRole("columnheader", { name: /Title/ })).toHaveAttribute(
      "aria-sort",
      "ascending",
    );
    await user.click(screen.getByRole("button", { name: /Title/ }));
    expect(titles()).toEqual(["Echo", "Delta", "Charlie", "Bravo", "Alpha"]);
  });

  it("moves and extends the selection with the keyboard", async () => {
    const { user } = await renderApp();
    grid().focus();
    await user.keyboard("{ArrowDown}");
    expect(selectedTitles()).toEqual(["Echo"]);
    await user.keyboard("{Shift>}{ArrowDown}{ArrowDown}{/Shift}");
    expect(selectedTitles()).toEqual(["Echo", "Delta", "Charlie"]);
    const charlie = screen.getByRole("row", { name: /Charlie/ });
    expect(grid()).toHaveAttribute("aria-activedescendant", charlie.id);
    await user.keyboard("{ArrowUp}");
    expect(selectedTitles()).toEqual(["Delta"]);
    await user.keyboard("{End}");
    expect(selectedTitles()).toEqual(["Alpha"]);
    await user.keyboard("{Escape}");
    expect(selectedTitles()).toEqual([]);
  });

  it("selects with click, shift-click, and cmd-click", async () => {
    const { user } = await renderApp();
    await user.click(screen.getByRole("row", { name: /Delta/ }));
    await user.keyboard("{Shift>}");
    await user.click(screen.getByRole("row", { name: /Bravo/ }));
    await user.keyboard("{/Shift}");
    expect(selectedTitles()).toEqual(["Delta", "Charlie", "Bravo"]);
    await user.keyboard("{Control>}");
    await user.click(screen.getByRole("row", { name: /Charlie/ }));
    await user.keyboard("{/Control}");
    expect(selectedTitles()).toEqual(["Delta", "Bravo"]);
  });

  it("plays the focused track with Enter and refuses missing files", async () => {
    const { user } = await renderApp();
    await user.dblClick(screen.getByRole("row", { name: /Charlie/ }));
    expect(usePlayer.getState().currentId).toBe(3);
    grid().focus();
    await user.keyboard("{ArrowUp}{Enter}");
    // Delta's file is missing, so playback stays on Charlie.
    expect(usePlayer.getState().currentId).toBe(3);
    expect(useToasts.getState().toasts.at(-1)?.message).toMatch(/missing/);
    await user.keyboard("j");
    expect(usePlayer.getState().currentId).toBe(2);
  });

  it("filters by text and by a sidebar vibe including nested vibes", async () => {
    const { user } = await renderApp();
    await user.type(screen.getByRole("searchbox"), "yara");
    expect(titles()).toEqual(["Bravo"]);
    await user.clear(screen.getByRole("searchbox"));
    await user.click(screen.getByRole("button", { name: /^Dubs/ }));
    expect(screen.getByRole("grid", { name: "Dubs" })).toBeInTheDocument();
    const rows = within(screen.getByRole("grid", { name: "Dubs" }))
      .getAllByRole("row")
      .slice(1);
    expect(
      rows.map((row) => within(row).getAllByRole("gridcell")[1].textContent),
    ).toEqual(["Bravo", "Alpha"]);
    expect(backend.listTracks).toHaveBeenCalledTimes(1);
  });

  it("shows only damaged tracks with the Damaged chip", async () => {
    backend.damagedTracks.mockResolvedValue([3]);
    const { user } = await renderApp();
    const chip = screen.getByRole("button", { name: /^Damaged/ });
    expect(chip).toHaveTextContent("1");
    await user.click(chip);
    expect(chip).toHaveAttribute("aria-pressed", "true");
    expect(titles()).toEqual(["Charlie"]);
    await user.click(chip);
    expect(titles()).toHaveLength(5);
  });

  it("hides the Damaged chip when no check found errors", async () => {
    await renderApp();
    expect(
      screen.queryByRole("button", { name: /^Damaged/ }),
    ).not.toBeInTheDocument();
  });
});
