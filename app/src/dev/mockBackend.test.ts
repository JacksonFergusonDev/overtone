import { describe, expect, it } from "vitest";
import { createMockBackend } from "./mockBackend";
import type { AuditProgress, Track } from "../api";

describe("mock backend", () => {
  it("serves a varied library and applies vibe mutations", async () => {
    const call = createMockBackend();
    const tracks = (await call("list_tracks")) as Track[];
    expect(tracks.length).toBeGreaterThanOrEqual(200);
    expect(
      new Set(tracks.map((t) => t.properties.format)).size,
    ).toBeGreaterThan(2);
    const vibe = (await call("create_vibe", { path: "Dubs/New" })) as {
      id: number;
    };
    await call("assign_vibe", { trackIds: [tracks[0]?.id], vibeId: vibe.id });
    const after = (await call("list_tracks")) as Track[];
    expect(after[0]?.vibes).toContain(vibe.id);
  });

  it("streams audit progress, rejects a second concurrent audit, and can cancel", async () => {
    const call = createMockBackend();
    const seen: AuditProgress[] = [];
    const onProgress = { onmessage: (p: AuditProgress) => seen.push(p) };
    const first = call("audit_tracks", { trackIds: null, onProgress });
    await expect(
      call("audit_tracks", { trackIds: null, onProgress }),
    ).rejects.toMatchObject({
      code: "busy",
    });
    await call("cancel_audit");
    const report = (await first) as { cancelled: boolean };
    expect(report.cancelled).toBe(true);
    expect(seen.length).toBeLessThan(40);
  });
});
