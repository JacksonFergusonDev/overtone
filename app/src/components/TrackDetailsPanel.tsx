import { open } from "@tauri-apps/plugin-dialog";
import { useEffect, useState } from "react";
import { errorMessage } from "../lib/backend";
import type {
  ExportFormat,
  FieldValue,
  MetadataField,
  MetadataSuggestion,
  Track,
  TrackDetails,
} from "../lib/types";
import { confirmAction } from "../store/confirm";
import {
  decideSuggestion,
  loadDetails,
  resetField,
  runConvert,
  runExport,
  runIntegrity,
  runLookup,
  runRestoreTags,
  setField,
  useDetails,
  writeTagsWithConfirm,
} from "../store/details";
import { useJobs } from "../store/jobs";
import { notifyError } from "../store/toasts";
import { backend } from "../lib/backend";

const FIELD_LABEL: Record<MetadataField, string> = {
  title: "Title",
  artist: "Artist",
  album: "Album",
  album_artist: "Album artist",
  remixer: "Remixer",
  label: "Label",
  genre: "Genre",
  year: "Year",
  bpm: "BPM",
  key: "Key",
  comment: "Comment",
  grouping: "Grouping",
  catalog_number: "Catalog no.",
  isrc: "ISRC",
  release_date: "Release date",
  track_number: "Track no.",
  disc_number: "Disc no.",
  store_genre: "Store genre",
};

const EXPORT_FORMATS: { format: ExportFormat | "cdj"; label: string }[] = [
  { format: "cdj", label: "CDJ (AIFF 16-bit, 44.1 kHz)" },
  { format: "aac", label: "AAC (.m4a)" },
  { format: "mp3", label: "MP3 320" },
  { format: "flac", label: "FLAC" },
  { format: "aiff", label: "AIFF" },
  { format: "wav", label: "WAV" },
];

function sourceLabel(value: FieldValue): string {
  switch (value.source) {
    case "file":
      return "file";
    case "user":
      return "you";
    case "lookup":
      return `lookup: ${value.provider ?? "?"}`;
  }
}

function db(value: number | null, unit: string): string {
  return value === null ? "—" : `${value.toFixed(1)} ${unit}`;
}

function FieldRow({
  trackId,
  value,
  unwritten,
}: {
  trackId: number;
  value: FieldValue;
  unwritten: boolean;
}) {
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(value.value ?? "");
  const editable = value.field !== "comment";
  const save = async () => {
    setEditing(false);
    await setField(trackId, value.field, draft.trim() === "" ? null : draft);
  };
  return (
    <div className="field-row">
      <dt>{FIELD_LABEL[value.field]}</dt>
      <dd>
        {editing ? (
          <form
            className="field-edit"
            onSubmit={(event) => {
              event.preventDefault();
              void save();
            }}
          >
            <input
              aria-label={FIELD_LABEL[value.field]}
              value={draft}
              autoFocus
              onFocus={(event) => event.currentTarget.select()}
              onChange={(event) => setDraft(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === "Escape") setEditing(false);
              }}
            />
            <button type="submit" className="btn btn-small">
              Save
            </button>
          </form>
        ) : (
          <>
            <span className="field-value truncate" title={value.value ?? ""}>
              {value.value ?? <span className="dim">blank</span>}
            </span>
            <span className={`source-pill source-${value.source}`}>
              {sourceLabel(value)}
            </span>
            {unwritten && (
              <span
                className="unwritten-dot"
                title="Not in the file yet"
                aria-label="Not in the file yet"
              />
            )}
            {editable && (
              <button
                type="button"
                className="link-button"
                onClick={() => {
                  setDraft(value.value ?? "");
                  setEditing(true);
                }}
              >
                Edit
              </button>
            )}
            {value.source !== "file" && (
              <button
                type="button"
                className="link-button"
                title={`The file has: ${value.file_value ?? "nothing"}`}
                onClick={() => void resetField(trackId, value.field)}
              >
                Revert
              </button>
            )}
          </>
        )}
      </dd>
    </div>
  );
}

function SuggestionRow({ suggestion }: { suggestion: MetadataSuggestion }) {
  const current = suggestion.current.value;
  const replaces =
    current !== null && current.trim() !== "" && current !== suggestion.value;
  return (
    <li className="suggestion">
      <div>
        <strong>{FIELD_LABEL[suggestion.field]}</strong>: {suggestion.value}
        <span className="dim small">
          {" "}
          from {suggestion.provider}
          {suggestion.confidence !== null &&
            `, ${Math.round(suggestion.confidence * 100)}% match`}
        </span>
      </div>
      {replaces && (
        <div className="dim small">
          Would replace {current} ({sourceLabel(suggestion.current)})
        </div>
      )}
      <div className="suggestion-actions">
        <button
          type="button"
          className="btn btn-small"
          onClick={() =>
            void decideSuggestion(suggestion, replaces ? "replace" : "accept")
          }
        >
          {replaces ? "Replace" : "Accept"}
        </button>
        <button
          type="button"
          className="btn btn-small"
          onClick={() => void decideSuggestion(suggestion, "reject")}
        >
          Reject
        </button>
      </div>
    </li>
  );
}

function FilesSection({
  track,
  details,
}: {
  track: Track;
  details: TrackDetails | undefined;
}) {
  const busy = useJobs((s) => s.background.running || s.audit.running);
  const [format, setFormat] = useState<ExportFormat | "cdj">("cdj");
  const missing = track.missing_since !== null;
  const convertible =
    !missing &&
    (track.properties.format === "wav" || track.properties.format === "aiff");
  const conversion = details?.conversion ?? null;

  const exportCopy = async () => {
    let folder: string | null;
    try {
      const picked = await open({
        directory: true,
        multiple: false,
        title: "Export a copy to…",
      });
      folder = typeof picked === "string" ? picked : null;
    } catch (error) {
      notifyError(`Couldn't open the folder picker: ${errorMessage(error)}`);
      return;
    }
    if (!folder) return;
    const settings =
      format === "cdj"
        ? {
            format: "aiff" as const,
            bitrate_kbps: null,
            sample_rate_hz: 44_100,
            bit_depth: 16,
          }
        : { format, bitrate_kbps: null, sample_rate_hz: null, bit_depth: null };
    await runExport([track.id], folder, settings);
  };

  const convert = async () => {
    let plan;
    try {
      plan = await backend.planConversion([track.id]);
    } catch (error) {
      notifyError(errorMessage(error));
      return;
    }
    const item = plan.planned[0];
    if (!item) {
      notifyError(plan.skipped[0]?.reason ?? "This track can't be converted.");
      return;
    }
    const lines = [
      `Writes ${item.output.split("/").pop() ?? "a FLAC"} next to the original, checked sample for sample. The original isn't touched and stays the track your DJ software uses until relocation.`,
    ];
    if (item.serato.length)
      lines.push(`Serato data carried over: ${item.serato.join(", ")}.`);
    if (item.dropped.length)
      lines.push(`Not carried over: ${item.dropped.join(", ")}.`);
    const ok = await confirmAction({
      title: "Convert to FLAC?",
      body: lines.join("\n\n"),
      confirmLabel: "Convert",
    });
    if (ok) await runConvert([track.id]);
  };

  return (
    <section className="details-section" aria-label="Files">
      <h4>Files</h4>
      <div className="details-actions">
        <select
          aria-label="Export format"
          value={format}
          onChange={(event) =>
            setFormat(event.target.value as ExportFormat | "cdj")
          }
        >
          {EXPORT_FORMATS.map((f) => (
            <option key={f.format} value={f.format}>
              {f.label}
            </option>
          ))}
        </select>
        <button
          type="button"
          className="btn btn-small"
          disabled={busy || missing}
          onClick={() => void exportCopy()}
        >
          Export a copy…
        </button>
      </div>
      {conversion ? (
        <p className="small">
          FLAC written and verified:{" "}
          <span className="mono">
            {conversion.output_path.split("/").pop()}
          </span>
          . It waits for relocation; the original is still the track.{" "}
          <button
            type="button"
            className="link-button"
            onClick={() =>
              void backend
                .discardConversion(track.id)
                .then(() => loadDetails(track.id))
            }
          >
            Discard the FLAC
          </button>
        </p>
      ) : (
        convertible && (
          <button
            type="button"
            className="btn btn-small"
            disabled={busy}
            onClick={() => void convert()}
          >
            Convert to FLAC…
          </button>
        )
      )}
    </section>
  );
}

/** Sound, integrity, metadata sources, suggestions, and file actions. */
export function TrackDetailsPanel({ track }: { track: Track }) {
  const details = useDetails((s) => s.details.get(track.id));
  const busy = useJobs((s) => s.background.running || s.audit.running);
  const [showEmpty, setShowEmpty] = useState(false);

  useEffect(() => {
    void loadDetails(track.id);
  }, [track.id, track.modified_at]);

  const fields = (details?.fields ?? []).filter(
    (f) => showEmpty || f.value !== null || f.source !== "file",
  );
  const loudness = details?.loudness ?? null;
  const integrity = details?.integrity ?? null;
  const suggestions = details?.suggestions ?? [];
  const unwritten = details?.unwritten ?? [];

  return (
    <div className="track-details">
      <section className="details-section" aria-label="Sound">
        <h4>Sound</h4>
        {loudness ? (
          <dl className="facts">
            <div>
              <dt>Loudness</dt>
              <dd className="num">{db(loudness.integrated_lufs, "LUFS")}</dd>
            </div>
            <div>
              <dt>True peak</dt>
              <dd className="num">{db(loudness.true_peak_dbtp, "dBTP")}</dd>
            </div>
            <div>
              <dt>Range</dt>
              <dd className="num">{db(loudness.loudness_range_lu, "LU")}</dd>
            </div>
            <div>
              <dt>Clipping</dt>
              <dd className="num">
                {loudness.clipped_runs === 0
                  ? "None"
                  : `${loudness.clipped_runs} runs`}
              </dd>
            </div>
            {loudness.leading_silence_ms >= 500 && (
              <div>
                <dt>Silence</dt>
                <dd className="num">
                  {(loudness.leading_silence_ms / 1000).toFixed(1)} s before the
                  music
                </dd>
              </div>
            )}
          </dl>
        ) : (
          <p className="dim small">Measured when the track is analyzed.</p>
        )}
        <p className="small">
          {integrity === null ? (
            <>
              <span className="dim">Not checked for decode errors. </span>
              <button
                type="button"
                className="link-button"
                disabled={busy || track.missing_since !== null}
                onClick={() => void runIntegrity([track.id])}
              >
                Check the file
              </button>
            </>
          ) : integrity.clean ? (
            "No decode errors."
          ) : integrity.edge_only ? (
            `${integrity.error_count} error(s), all in junk at the very start or end; the music decodes cleanly.`
          ) : (
            <span className="error-text">
              {integrity.error_count} decode error(s) in the music. Listen for
              glitches.
            </span>
          )}
        </p>
      </section>

      <section className="details-section" aria-label="Metadata">
        <h4>
          Metadata{" "}
          <button
            type="button"
            className="link-button"
            onClick={() => setShowEmpty((v) => !v)}
          >
            {showEmpty ? "Hide empty" : "Show all fields"}
          </button>
        </h4>
        <dl className="fields">
          {fields.map((value) => (
            <FieldRow
              key={value.field}
              trackId={track.id}
              value={value}
              unwritten={unwritten.includes(value.field)}
            />
          ))}
        </dl>
        {(unwritten.length > 0 || details?.restorable) && (
          <p className="small details-actions">
            {unwritten.length > 0 && (
              <>
                <span className="dim">
                  {unwritten.length === 1
                    ? "1 value isn't in the file yet."
                    : `${unwritten.length} values aren't in the file yet.`}{" "}
                </span>
                <button
                  type="button"
                  className="btn btn-small"
                  disabled={busy || track.missing_since !== null}
                  onClick={() => void writeTagsWithConfirm([track.id])}
                >
                  Write to file…
                </button>
              </>
            )}
            {details?.restorable && (
              <button
                type="button"
                className="link-button"
                disabled={busy || track.missing_since !== null}
                onClick={() => void runRestoreTags([track.id])}
              >
                Restore the file's original values
              </button>
            )}
          </p>
        )}
      </section>

      <section className="details-section" aria-label="Suggestions">
        <h4>
          Suggestions {suggestions.length > 0 && `(${suggestions.length})`}
        </h4>
        {suggestions.length === 0 ? (
          <p className="dim small">
            None waiting.{" "}
            <button
              type="button"
              className="link-button"
              disabled={busy || track.missing_since !== null}
              onClick={() => void runLookup([track.id])}
            >
              Look this track up
            </button>
          </p>
        ) : (
          <ul className="suggestions">
            {suggestions.map((s) => (
              <SuggestionRow key={s.id} suggestion={s} />
            ))}
          </ul>
        )}
        <p className="dim small">
          Lookups only suggest. Nothing changes until you accept, and they never
          touch Comment, Grouping, or your Genre.
        </p>
      </section>

      <FilesSection track={track} details={details} />
    </div>
  );
}
