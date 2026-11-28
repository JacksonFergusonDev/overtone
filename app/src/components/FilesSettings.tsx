import { useCallback, useEffect, useState } from "react";
import { backend, errorMessage } from "../lib/backend";
import type { LookupSettings } from "../lib/types";
import { confirmAction } from "../store/confirm";
import {
  backgroundLabel,
  cancelBackground,
  runConvert,
  runIntegrity,
  runLookup,
} from "../store/details";
import { useJobs } from "../store/jobs";
import { notify, notifyError } from "../store/toasts";
import { useUi } from "../store/ui";
import { ProgressBar } from "./SettingsView";

/** Progress and cancel for the running lookup, check, export, or conversion. */
function BackgroundProgress() {
  const job = useJobs((s) => s.background);
  if (!job.running) return null;
  return (
    <div className="card-actions">
      <ProgressBar
        value={job.total > 0 ? job.done / job.total : null}
        label={backgroundLabel() ?? "Working…"}
      />
      <button
        type="button"
        className="btn"
        disabled={job.cancelling}
        onClick={() => void cancelBackground()}
      >
        {job.cancelling ? "Cancelling…" : "Cancel"}
      </button>
    </div>
  );
}

export function LookupCard() {
  const visible = useUi((s) => s.view === "settings");
  const busy = useJobs((s) => s.background.running || s.audit.running);
  const [settings, setSettings] = useState<LookupSettings | null>(null);
  const [acoustid, setAcoustid] = useState("");
  const [discogs, setDiscogs] = useState("");
  const [contact, setContact] = useState("");

  const refresh = useCallback(() => {
    backend
      .lookupSettings()
      .then((found) => {
        setSettings(found);
        setContact(found.contact ?? "");
      })
      .catch((error: unknown) =>
        notifyError(`Couldn't read lookup settings: ${errorMessage(error)}`),
      );
  }, []);

  useEffect(() => {
    if (visible) refresh();
  }, [visible, refresh]);

  const save = async () => {
    try {
      const saved = await backend.setLookupSettings(
        acoustid === "" ? null : acoustid,
        discogs === "" ? null : discogs,
        contact,
      );
      setSettings(saved);
      setAcoustid("");
      setDiscogs("");
      notify(`Lookups will use ${saved.providers.join(", ")}`);
    } catch (error) {
      notifyError(`Couldn't save: ${errorMessage(error)}`);
    }
  };

  return (
    <section className="card" aria-labelledby="settings-lookup">
      <header className="card-header">
        <div>
          <h2 id="settings-lookup">Metadata lookup</h2>
          <p className="dim">
            Searches online databases for labels, release dates, catalog
            numbers, and more. Results wait as suggestions in the inspector;
            nothing changes until you accept one.
          </p>
        </div>
        <div className="card-actions">
          <button
            type="button"
            className="btn btn-primary"
            disabled={busy}
            onClick={() => void runLookup(null)}
          >
            Look up new tracks
          </button>
        </div>
      </header>
      <form
        className="settings-form"
        onSubmit={(event) => {
          event.preventDefault();
          void save();
        }}
      >
        <label>
          AcoustID key{" "}
          <span className="dim small">
            {settings?.acoustid_key_set
              ? "(set)"
              : "(identifies tracks by sound; free at acoustid.org)"}
          </span>
          <input
            type="password"
            value={acoustid}
            placeholder={
              settings?.acoustid_key_set ? "Leave blank to keep" : ""
            }
            onChange={(event) => setAcoustid(event.target.value)}
          />
        </label>
        <label>
          Discogs token{" "}
          <span className="dim small">
            {settings?.discogs_token_set
              ? "(set)"
              : "(labels and styles; discogs.com/settings/developers)"}
          </span>
          <input
            type="password"
            value={discogs}
            placeholder={
              settings?.discogs_token_set ? "Leave blank to keep" : ""
            }
            onChange={(event) => setDiscogs(event.target.value)}
          />
        </label>
        <label>
          Contact for MusicBrainz{" "}
          <span className="dim small">
            (optional; replaces the Overtone project's)
          </span>
          <input
            value={contact}
            onChange={(event) => setContact(event.target.value)}
          />
        </label>
        <button type="submit" className="btn">
          Save
        </button>
      </form>
      {settings && (
        <p className="dim small">Using: {settings.providers.join(", ")}</p>
      )}
    </section>
  );
}

export function FilesCard() {
  const busy = useJobs((s) => s.background.running || s.audit.running);

  const convertAll = async () => {
    let plan;
    try {
      plan = await backend.planConversion(null);
    } catch (error) {
      notifyError(errorMessage(error));
      return;
    }
    if (plan.planned.length === 0) {
      notify("No WAV or AIFF tracks to convert.");
      return;
    }
    const dropped = new Set(plan.planned.flatMap((p) => p.dropped));
    const lines = [
      `${plan.planned.length} track(s) get a FLAC next to the original, checked sample for sample, with tags, artwork, and Serato cues carried over. Originals aren't touched and stay the tracks your DJ software uses until relocation.`,
    ];
    if (dropped.size)
      lines.push(`Not carried over: ${[...dropped].join(", ")}.`);
    if (plan.skipped.length)
      lines.push(`${plan.skipped.length} track(s) will be skipped.`);
    const ok = await confirmAction({
      title: "Convert WAV and AIFF to FLAC?",
      body: lines.join("\n\n"),
      confirmLabel: "Convert",
    });
    if (ok) await runConvert(null);
  };

  return (
    <section className="card" aria-labelledby="settings-files">
      <header className="card-header">
        <div>
          <h2 id="settings-files">Files</h2>
          <p className="dim">
            Integrity checks decode every file in full and list errors.
            Conversion writes FLACs beside your WAV and AIFF files. Both need
            ffmpeg.
          </p>
        </div>
        <div className="card-actions">
          <button
            type="button"
            className="btn"
            disabled={busy}
            onClick={() => void runIntegrity(null)}
          >
            Check files
          </button>
          <button
            type="button"
            className="btn"
            disabled={busy}
            onClick={() => void convertAll()}
          >
            Convert to FLAC…
          </button>
        </div>
      </header>
      <BackgroundProgress />
    </section>
  );
}
