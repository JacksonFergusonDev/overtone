import { screen, waitFor, within } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { SERATO_PLAN } from "../test/fixtures";
import { backend, resetBackend } from "../test/backendMock";
import { renderApp, resetStores } from "../test/render";

vi.mock("../lib/backend", () => import("../test/backendMock"));

beforeEach(() => {
  resetBackend();
  resetStores();
});

const card = (name: string) => screen.getByRole("region", { name });

async function openSync(
  running = { serato: false, rekordbox: false, traktor: false },
) {
  backend.djAppsRunning.mockResolvedValue(running);
  backend.planSeratoSync.mockResolvedValue([SERATO_PLAN]);
  const app = await renderApp();
  await app.user.click(screen.getByRole("button", { name: "Sync" }));
  await screen.findByText(
    `Serato DJ: ${running.serato ? "Running" : "Closed"}`,
  );
  return app;
}

describe("Sync view", () => {
  it("previews per-folder crate changes", async () => {
    const { user } = await openSync();
    const serato = card("Serato");
    const apply = within(serato).getByRole("button", { name: "Apply" });
    expect(apply).toBeDisabled();
    await user.click(within(serato).getByRole("button", { name: "Preview" }));
    expect(
      await within(serato).findByText("/Volumes/Music/_Serato_"),
    ).toBeInTheDocument();
    expect(
      within(serato).getByText("Create with 2 tracks"),
    ).toBeInTheDocument();
    expect(within(serato).getByText("Add 1, remove 2")).toBeInTheDocument();
    expect(within(serato).getByText("Delete (3 tracks)")).toBeInTheDocument();
    expect(within(serato).getByText(/1 added, 0 removed/)).toBeInTheDocument();
    expect(within(serato).getByText(/1 tracks skipped/)).toBeInTheDocument();
    expect(
      within(serato).getByText("#dub is a prefix of #dubtechno"),
    ).toBeInTheDocument();
    expect(apply).toBeEnabled();
  });

  it("asks before applying and shows the report", async () => {
    backend.applySeratoSync.mockResolvedValue({
      folders: [
        {
          serato_dir: "/Volumes/Music/_Serato_",
          report: {
            backup_dir: "/Volumes/Music/_Serato_/Overtone backups/1",
            created: ["a"],
            updated: ["b"],
            deleted: ["c"],
            unchanged: 0,
            neworder_written: true,
          },
        },
      ],
    });
    const { user } = await openSync();
    const serato = card("Serato");
    await user.click(within(serato).getByRole("button", { name: "Preview" }));
    await within(serato).findByText("Create with 2 tracks");
    await user.click(within(serato).getByRole("button", { name: "Apply" }));
    const dialog = screen.getByRole("alertdialog", {
      name: "Write Serato crates?",
    });
    await user.click(
      within(dialog).getByRole("button", { name: "Apply to Serato" }),
    );
    await waitFor(() =>
      expect(backend.applySeratoSync).toHaveBeenCalledWith(null),
    );
    expect(
      await within(serato).findByText(/1 created, 1 updated, 1 deleted/),
    ).toBeInTheDocument();
  });

  it("keeps Apply disabled while Serato is running", async () => {
    const { user } = await openSync({
      serato: true,
      rekordbox: false,
      traktor: false,
    });
    const serato = card("Serato");
    expect(
      within(serato).getByText(/Serato DJ is running/),
    ).toBeInTheDocument();
    await user.click(within(serato).getByRole("button", { name: "Preview" }));
    await within(serato).findByText("Create with 2 tracks");
    expect(
      within(serato).getByRole("button", { name: "Apply" }),
    ).toBeDisabled();
  });

  it("previews Comment changes before and after", async () => {
    backend.planTagMirror.mockResolvedValue([
      {
        track_id: 1,
        path: "/music/a.flac",
        before: "8A Energy 6",
        after: "8A Energy 6 #dubs",
      },
    ]);
    const { user } = await openSync();
    const tags = card("Comment tags");
    expect(
      within(tags).getByText(/writes to your audio files' Comment tags/),
    ).toBeInTheDocument();
    await user.click(within(tags).getByRole("button", { name: "Preview" }));
    expect(
      await within(tags).findByText("8A Energy 6 #dubs"),
    ).toBeInTheDocument();
    expect(within(tags).getByText("8A Energy 6")).toBeInTheDocument();
  });

  it("previews and exports the Rekordbox XML while Rekordbox runs", async () => {
    backend.planRekordboxExport.mockResolvedValue({
      path: "/data/rekordbox.xml",
      track_count: 3,
      playlist_count: 2,
      playlists: [
        { path: ["Dubs", "Dubs"], track_count: 1 },
        { path: ["Dubs", "Deep"], track_count: 2 },
      ],
      changed: true,
      exists: false,
    });
    backend.exportRekordboxXml.mockResolvedValue({
      path: "/data/rekordbox.xml",
      track_count: 3,
      playlist_count: 2,
      changed: true,
      backup_path: null,
      rekordbox_running: true,
    });
    const { user } = await openSync({
      serato: false,
      rekordbox: true,
      traktor: false,
    });
    const rekordbox = card("Rekordbox");
    await user.click(
      within(rekordbox).getByRole("button", { name: "Preview" }),
    );
    expect(
      await within(rekordbox).findByText("Dubs / Deep"),
    ).toBeInTheDocument();
    await user.click(within(rekordbox).getByRole("button", { name: "Export" }));
    const dialog = screen.getByRole("alertdialog", {
      name: "Write rekordbox.xml?",
    });
    await user.click(within(dialog).getByRole("button", { name: "Export" }));
    expect(
      await within(rekordbox).findByText(/running: import the file again/),
    ).toBeInTheDocument();
    expect(backend.exportRekordboxXml).toHaveBeenCalledWith(null);
  });
});
