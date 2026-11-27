import { screen, waitFor, within } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useLibrary } from "../store/library";
import { useToasts } from "../store/toasts";
import { VIBES, vibe } from "../test/fixtures";
import { backend, resetBackend } from "../test/backendMock";
import { renderApp, resetStores } from "../test/render";

vi.mock("../lib/backend", () => import("../test/backendMock"));

beforeEach(() => {
  resetBackend();
  resetStores();
});

async function selectAlphaAndBravo() {
  const app = await renderApp();
  await app.user.click(screen.getByRole("row", { name: /Bravo/ }));
  await app.user.keyboard("{Shift>}");
  await app.user.click(screen.getByRole("row", { name: /Alpha/ }));
  await app.user.keyboard("{/Shift}");
  return app;
}

const vibesOf = (id: number) => useLibrary.getState().tracks.get(id)?.vibes;

describe("Quick-tag palette", () => {
  it("shows which vibes the selection has and assigns one with Enter", async () => {
    const { user } = await selectAlphaAndBravo();
    await user.keyboard("t");
    const input = screen.getByRole("combobox", {
      name: "Find or create a vibe",
    });
    expect(input).toHaveFocus();
    const dubs = screen.getByRole("option", { name: /^Dubs\s*Some selected/ });
    expect(dubs).toHaveAttribute("aria-checked", "mixed");
    await user.type(input, "warm");
    expect(input).toHaveAttribute(
      "aria-activedescendant",
      screen.getByRole("option", { name: /Warmup/ }).id,
    );
    await user.keyboard("{Enter}");
    expect(backend.assignVibe).toHaveBeenCalledWith([2, 1], 3);
    expect(vibesOf(1)).toContain(3);
    expect(vibesOf(2)).toContain(3);
    expect(
      await screen.findByText("Added “Warmup” to 2 tracks"),
    ).toBeInTheDocument();
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("combobox")).not.toBeInTheDocument();
  });

  it("removes a vibe every selected track already has", async () => {
    const { user } = await renderApp();
    await user.click(screen.getByRole("row", { name: /Alpha/ }));
    await user.keyboard("t");
    await user.keyboard("dubs{Enter}");
    expect(backend.unassignVibe).toHaveBeenCalledWith([1], 1);
    expect(vibesOf(1)).toEqual([]);
  });

  it("creates a vibe when nothing matches exactly", async () => {
    backend.createVibe.mockResolvedValueOnce(vibe(9, "Late Night"));
    backend.listVibes
      .mockResolvedValueOnce(VIBES)
      .mockResolvedValue([...VIBES, vibe(9, "Late Night")]);
    const { user } = await selectAlphaAndBravo();
    await user.keyboard("t");
    await user.keyboard("Late  Night");
    const create = screen.getByRole("option", {
      name: "Create vibe “Late Night”",
    });
    await user.click(create);
    expect(backend.createVibe).toHaveBeenCalledWith("Late Night");
    await waitFor(() =>
      expect(backend.assignVibe).toHaveBeenCalledWith([2, 1], 9),
    );
    const nav = screen.getByRole("navigation", { name: "Main" });
    expect(
      within(nav).getByRole("button", { name: /^Late Night/ }),
    ).toBeInTheDocument();
  });

  it("explains invalid names instead of offering to create them", async () => {
    const { user } = await selectAlphaAndBravo();
    await user.keyboard("t");
    await user.keyboard("dub#techno");
    expect(screen.getByText("Vibe names can't contain #")).toBeInTheDocument();
    expect(
      screen.queryByRole("option", { name: /Create vibe/ }),
    ).not.toBeInTheDocument();
  });

  it("rolls back and shows an error when the backend refuses", async () => {
    backend.assignVibe.mockRejectedValueOnce({
      code: "busy",
      message: "database is locked",
    });
    const { user } = await selectAlphaAndBravo();
    await user.keyboard("t");
    await user.keyboard("peak{Enter}");
    await waitFor(() => expect(vibesOf(1)).toEqual([1]));
    expect(vibesOf(2)).toEqual([2]);
    expect(useToasts.getState().toasts.at(-1)?.message).toMatch(
      /database is locked/,
    );
  });

  it("asks you to select tracks first", async () => {
    const { user } = await renderApp();
    await user.keyboard("t");
    expect(screen.queryByRole("combobox")).not.toBeInTheDocument();
    expect(useToasts.getState().toasts.at(-1)?.message).toMatch(
      /Select one or more tracks/,
    );
  });
});
