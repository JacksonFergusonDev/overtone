import { invoke } from "@tauri-apps/api/core";

export interface LibrarySummary {
  schema_version: number;
  track_count: number;
  vibe_count: number;
}

export interface AppStatus {
  version: string;
  library: LibrarySummary;
}

export async function getAppStatus(): Promise<AppStatus> {
  return invoke<AppStatus>("app_status");
}
