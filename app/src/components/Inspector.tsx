import { X } from "lucide-react";
import { useEffect, useState } from "react";
import { useHotkeys } from "react-hotkeys-hook";
import { errorMessage, imageSrc } from "../lib/backend";
import {
  formatAudio,
  formatDateTime,
  formatHz,
  formatPercent,
  REVIEW_LABEL,
  trackTitle,
} from "../lib/format";
import { frequencyFraction, frequencyTicks, nyquist } from "../lib/frequency";
import type { ReviewStatus, Track } from "../lib/types";
import { reviewTrack } from "../store/actions";
import { runAudit, useJobs } from "../store/jobs";
import { loadSpectrogram, spectrogramKey, useLibrary } from "../store/library";
import {
  activeTable,
  setInspectorOpen,
  useUi,
  type TableKey,
} from "../store/ui";
import { VerdictBadge } from "./Badges";

function FrequencyAxis({
  sampleRate,
  log,
}: {
  sampleRate: number;
  log: boolean;
}) {
  return (
    <div className="freq-axis" aria-hidden="true">
      {frequencyTicks(sampleRate, log).map((tick) => (
        <span
          key={tick.hz}
          className="freq-tick"
          style={{ bottom: `${tick.fraction * 100}%` }}
        >
          {tick.label}
        </span>
      ))}
      <span className="freq-unit">kHz</span>
    </div>
  );
}

function Spectrogram({ track, log }: { track: Track; log: boolean }) {
  const key = spectrogramKey(track.id, log);
  const image = useLibrary((s) => s.spectrograms.get(key));
  const [failed, setFailed] = useState<{ key: string; message: string } | null>(
    null,
  );

  useEffect(() => {
    if (image) return;
    let active = true;
    loadSpectrogram(track.id, log).catch((error: unknown) => {
      if (active) setFailed({ key, message: errorMessage(error) });
    });
    return () => {
      active = false;
    };
  }, [image, track.id, log, key]);

  const sampleRate = track.properties.sample_rate_hz ?? 44_100;
  const cutoff = track.quality?.cutoff_hz ?? null;
  const error = failed?.key === key ? failed.message : null;

  return (
    <figure className="spectrogram">
      <FrequencyAxis sampleRate={sampleRate} log={log} />
      <div className="spectrogram-frame">
        {image ? (
          <img
            src={imageSrc(image.path)}
            alt={`${log ? "Logarithmic" : "Linear"} spectrogram of ${trackTitle(track)}, up to ${formatHz(nyquist(sampleRate))}`}
          />
        ) : (
          <div className="spectrogram-placeholder">
            {error ? (
              <span className="error-text">{error}</span>
            ) : (
              "Rendering spectrogram…"
            )}
          </div>
        )}
        {cutoff !== null && (
          <div
            className="cutoff-line"
            style={{
              bottom: `${frequencyFraction(cutoff, sampleRate, log) * 100}%`,
            }}
          >
            <span className="cutoff-label">Cutoff {formatHz(cutoff)}</span>
          </div>
        )}
      </div>
    </figure>
  );
}

const REVIEW_ACTIONS: { review: ReviewStatus; label: string; key: string }[] = [
  { review: "verified_ok", label: "Verify OK", key: "O" },
  { review: "replace_wanted", label: "Replace wanted", key: "R" },
  { review: "unreviewed", label: "Clear", key: "U" },
];

/** Quality details and review actions for the focused row of the active table. */
export function Inspector() {
  const tableKey = useUi((s) => activeTable(s));
  const cursor = useUi((s) =>
    tableKey ? s.tables[tableKey].selection.cursor : null,
  );
  const track = useLibrary((s) =>
    cursor === null ? undefined : s.tracks.get(cursor),
  );
  const auditing = useJobs((s) => s.audit.running);
  const [log, setLog] = useState(false);

  const act = (review: ReviewStatus) => {
    if (track && tableKey)
      void reviewTrack(tableKey as TableKey, track.id, review);
  };
  const analyzed = track?.quality && track.quality.verdict !== "unchecked";

  useHotkeys("o", () => act("verified_ok"), {
    scopes: "inspector",
    enabled: Boolean(analyzed),
  });
  useHotkeys("r", () => act("replace_wanted"), {
    scopes: "inspector",
    enabled: Boolean(analyzed),
  });
  useHotkeys("u", () => act("unreviewed"), {
    scopes: "inspector",
    enabled: Boolean(analyzed),
  });
  useHotkeys("l", () => setLog((value) => !value), { scopes: "inspector" });

  return (
    <aside className="inspector" aria-label="Quality inspector">
      <header className="inspector-header">
        <h2>Quality</h2>
        <button
          type="button"
          className="icon-button"
          aria-label="Close inspector (I)"
          onClick={() => setInspectorOpen(false)}
        >
          <X size={16} aria-hidden="true" />
        </button>
      </header>
      {!track ? (
        <p className="empty">Select a track to inspect its quality.</p>
      ) : (
        <div className="inspector-body">
          <h3 className="inspector-title truncate" title={trackTitle(track)}>
            {trackTitle(track)}
          </h3>
          <p className="dim truncate" title={track.path}>
            {track.tags.artist ?? "Unknown artist"} ·{" "}
            {formatAudio(track.properties).label}{" "}
            {formatAudio(track.properties).detail}
          </p>
          {track.missing_since !== null && (
            <p className="banner banner-warn">
              The file is missing, so it can't be played or analyzed.
            </p>
          )}
          {!analyzed ? (
            <div className="inspector-unchecked">
              <p>Not analyzed yet.</p>
              <button
                type="button"
                className="btn"
                disabled={auditing || track.missing_since !== null}
                onClick={() => void runAudit([track.id])}
              >
                Analyze this track
              </button>
            </div>
          ) : (
            <>
              <dl className="facts">
                <div>
                  <dt>Verdict</dt>
                  <dd>
                    <VerdictBadge quality={track.quality} />
                  </dd>
                </div>
                <div>
                  <dt>Review</dt>
                  <dd>{REVIEW_LABEL[track.quality?.review ?? "unreviewed"]}</dd>
                </div>
                <div>
                  <dt>Cutoff</dt>
                  <dd className="num">
                    {formatHz(track.quality?.cutoff_hz) || "None found"}
                  </dd>
                </div>
                <div>
                  <dt>Confidence</dt>
                  <dd className="num">
                    {formatPercent(track.quality?.confidence) || "Unknown"}
                  </dd>
                </div>
              </dl>
              {track.quality?.reason && (
                <p className="reason">{track.quality.reason}</p>
              )}
              <div className="review-actions" role="group" aria-label="Review">
                {REVIEW_ACTIONS.map((action) => (
                  <button
                    key={action.review}
                    type="button"
                    className={`btn review-${action.review}`}
                    aria-pressed={
                      action.review !== "unreviewed" &&
                      track.quality?.review === action.review
                    }
                    aria-keyshortcuts={action.key}
                    onClick={() => act(action.review)}
                  >
                    {action.label} <kbd>{action.key}</kbd>
                  </button>
                ))}
              </div>
              <div
                className="spectrogram-toolbar"
                role="group"
                aria-label="Frequency scale"
              >
                <button
                  type="button"
                  className="chip"
                  aria-pressed={!log}
                  onClick={() => setLog(false)}
                >
                  Linear
                </button>
                <button
                  type="button"
                  className="chip"
                  aria-pressed={log}
                  onClick={() => setLog(true)}
                >
                  Log
                </button>
                <span className="dim">
                  Analyzed {formatDateTime(track.quality?.analyzed_at)}
                </span>
              </div>
              <Spectrogram track={track} log={log} />
              <p className="dim small">
                Verdicts only flag tracks. Listen and compare the spectrogram
                before you replace a file; Overtone never deletes or moves it.
              </p>
            </>
          )}
        </div>
      )}
    </aside>
  );
}
