import { render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import App from "./App";
import type { AppStatus } from "./api";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
  isTauri: vi.fn(() => false),
}));

afterEach(() => {
  vi.clearAllMocks();
});

const STATUS: AppStatus = {
  version: "0.1.0",
  library_path: "/tmp/library.sqlite",
  library: {
    schema_version: 1,
    track_count: 12,
    missing_count: 0,
    vibe_count: 4,
    needs_review_count: 3,
  },
};

describe("App", () => {
  it("renders status returned by the backend", async () => {
    vi.mocked(invoke).mockResolvedValueOnce(STATUS);
    render(<App />);
    expect(await screen.findByText("12")).toBeInTheDocument();
    expect(screen.getByText("0.1.0")).toBeInTheDocument();
    expect(screen.getByText("Needs review").nextSibling).toHaveTextContent("3");
    expect(invoke).toHaveBeenCalledWith("app_status");
  });

  it("shows the backend's error message", async () => {
    vi.mocked(invoke).mockRejectedValueOnce({
      code: "schema_too_new",
      message: "this library was created by a newer version of Overtone",
    });
    render(<App />);
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "created by a newer version",
    );
  });
});
