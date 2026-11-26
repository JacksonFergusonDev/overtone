// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import App from "./App";

(
  globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }
).IS_REACT_ACT_ENVIRONMENT = true;

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
  isTauri: vi.fn(() => false),
}));

let container: HTMLDivElement | null = null;
let root: Root | null = null;

afterEach(async () => {
  if (root) {
    await act(async () => {
      root?.unmount();
    });
    root = null;
  }
  container?.remove();
  container = null;
  vi.clearAllMocks();
});

describe("App", () => {
  it("renders status returned by the Tauri backend", async () => {
    vi.mocked(invoke).mockResolvedValueOnce({
      version: "0.1.0",
      library: { schema_version: 1, track_count: 12, vibe_count: 4 },
    });

    container = document.createElement("div");
    document.body.appendChild(container);

    await act(async () => {
      root = createRoot(container!);
      root.render(<App />);
    });

    expect(container.textContent).toContain("Overtone");
    expect(container.textContent).toContain("0.1.0");
    expect(container.textContent).toContain("12");
    expect(container.textContent).toContain("4");
  });
});
