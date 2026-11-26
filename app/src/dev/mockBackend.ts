import { mockIPC } from "@tauri-apps/api/mocks";
import type { AppStatus } from "../api";

const MOCK_STATUS: AppStatus = {
  version: "0.1.0-mock",
  library: {
    schema_version: 1,
    track_count: 0,
    vibe_count: 0,
  },
};

export function installMockBackend(): void {
  mockIPC((cmd) => {
    if (cmd === "app_status") {
      return MOCK_STATUS;
    }
    return null;
  });
}
