import { render, screen, within } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import App from "./App";
import { backend, resetBackend } from "./test/backendMock";
import { renderApp, resetStores } from "./test/render";

vi.mock("./lib/backend", () => import("./test/backendMock"));

beforeEach(() => {
  resetBackend();
  resetStores();
});

describe("App", () => {
  it("loads status, tracks, and vibes once, in parallel", async () => {
    await renderApp();
    expect(backend.appStatus).toHaveBeenCalledTimes(1);
    expect(backend.listTracks).toHaveBeenCalledTimes(1);
    expect(backend.listVibes).toHaveBeenCalledTimes(1);
    const nav = screen.getByRole("navigation", { name: "Main" });
    // Five fixture tracks; two flagged and unreviewed.
    expect(
      within(nav).getByRole("button", { name: /^Library\s*5$/ }),
    ).toBeInTheDocument();
    expect(
      within(nav).getByRole("button", { name: /^Needs review\s*2$/ }),
    ).toBeInTheDocument();
    expect(
      within(nav).getByRole("button", { name: /^Deep/ }),
    ).toBeInTheDocument();
    expect(screen.getByRole("row", { name: /Alpha/ })).toBeInTheDocument();
  });

  it("shows the library path and version in Settings", async () => {
    const { user } = await renderApp();
    await user.click(screen.getByRole("button", { name: "Settings" }));
    expect(await screen.findByText("0.1.0")).toBeInTheDocument();
    expect(screen.getByText("/tmp/library.sqlite")).toBeInTheDocument();
  });

  it("shows a single full-window error when the library can't open", async () => {
    backend.appStatus.mockRejectedValueOnce({
      code: "schema_too_new",
      message: "this library was created by a newer version of Overtone",
    });
    render(<App />);
    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent("Couldn't open your library:");
    expect(alert).toHaveTextContent("created by a newer version");
    expect(screen.queryByRole("navigation")).not.toBeInTheDocument();
  });
});
