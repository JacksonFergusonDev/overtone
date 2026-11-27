import { X } from "lucide-react";
import { useEffect, useMemo, useRef } from "react";
import { SHORTCUT_GROUPS } from "../lib/shortcuts";
import { scanFraction, scanLabel, useJobs } from "../store/jobs";
import { settleConfirm, useConfirm } from "../store/confirm";
import { useLibrary } from "../store/library";
import { dismissToast, useToasts } from "../store/toasts";
import { closeModal } from "../store/ui";
import { ProgressBar } from "./SettingsView";

/** Errors are announced as alerts; other messages politely. */
export function Toasts() {
  const toasts = useToasts((s) => s.toasts);
  return (
    <div className="toasts">
      {toasts.map((toast) => (
        <div
          key={toast.id}
          className={`toast toast-${toast.kind}`}
          role={toast.kind === "error" ? "alert" : "status"}
        >
          <span>{toast.message}</span>
          <button
            type="button"
            className="icon-button small"
            aria-label="Dismiss"
            onClick={() => dismissToast(toast.id)}
          >
            <X size={12} aria-hidden="true" />
          </button>
        </div>
      ))}
    </div>
  );
}

/** Library counts, plus progress for a running scan or audit. */
export function StatusBar() {
  const total = useLibrary((s) => s.order.length);
  const tracks = useLibrary((s) => s.tracks);
  const missing = useMemo(() => {
    let count = 0;
    for (const track of tracks.values())
      if (track.missing_since !== null) count += 1;
    return count;
  }, [tracks]);
  const scan = useJobs((s) => s.scan);
  const audit = useJobs((s) => s.audit);
  return (
    <div className="status-bar">
      <span className="num">
        {total.toLocaleString()} tracks
        {missing > 0 && ` · ${missing.toLocaleString()} missing`}
      </span>
      <div className="status-jobs" role="status" aria-label="Background jobs">
        {scan.running && (
          <ProgressBar
            value={scanFraction(scan.progress)}
            label={scanLabel(scan.progress)}
          />
        )}
        {audit.running && (
          <ProgressBar
            value={
              audit.progress && audit.progress.total > 0
                ? audit.progress.done / audit.progress.total
                : null
            }
            label={
              audit.progress
                ? `Auditing ${audit.progress.done.toLocaleString()} of ${audit.progress.total.toLocaleString()}`
                : "Starting audit…"
            }
          />
        )}
      </div>
      <span className="dim">Press ? for shortcuts</span>
    </div>
  );
}

export function ConfirmDialog() {
  const request = useConfirm((s) => s.request);
  const confirmRef = useRef<HTMLButtonElement | null>(null);
  useEffect(() => {
    if (!request) return;
    const previous = document.activeElement as HTMLElement | null;
    confirmRef.current?.focus();
    return () => previous?.focus?.();
  }, [request]);
  if (!request) return null;
  return (
    <div
      className="overlay"
      onMouseDown={(event) =>
        event.target === event.currentTarget && settleConfirm(false)
      }
    >
      <div
        className="dialog"
        role="alertdialog"
        aria-modal="true"
        aria-labelledby="confirm-title"
        aria-describedby="confirm-body"
        onKeyDown={(event) => {
          if (event.key === "Escape") {
            event.preventDefault();
            settleConfirm(false);
          }
        }}
      >
        <h2 id="confirm-title">{request.title}</h2>
        <p id="confirm-body" className="pre-line">
          {request.body}
        </p>
        <div className="dialog-actions">
          <button
            type="button"
            className="btn"
            onClick={() => settleConfirm(false)}
          >
            Cancel
          </button>
          <button
            ref={confirmRef}
            type="button"
            className={`btn ${request.danger ? "btn-danger" : "btn-primary"}`}
            onClick={() => settleConfirm(true)}
          >
            {request.confirmLabel}
          </button>
        </div>
      </div>
    </div>
  );
}

export function ShortcutsOverlay() {
  const closeRef = useRef<HTMLButtonElement | null>(null);
  useEffect(() => {
    const previous = document.activeElement as HTMLElement | null;
    closeRef.current?.focus();
    return () => previous?.focus?.();
  }, []);
  return (
    <div
      className="overlay"
      onMouseDown={(event) =>
        event.target === event.currentTarget && closeModal()
      }
    >
      <div
        className="dialog shortcuts"
        role="dialog"
        aria-modal="true"
        aria-labelledby="shortcuts-title"
        onKeyDown={(event) => {
          if (event.key === "Escape" || event.key === "?") {
            event.preventDefault();
            closeModal();
          }
        }}
      >
        <header className="card-header">
          <h2 id="shortcuts-title">Keyboard shortcuts</h2>
          <button
            ref={closeRef}
            type="button"
            className="icon-button"
            aria-label="Close"
            onClick={closeModal}
          >
            <X size={16} aria-hidden="true" />
          </button>
        </header>
        <div className="shortcut-groups">
          {SHORTCUT_GROUPS.map((group) => (
            <section key={group.title}>
              <h3>{group.title}</h3>
              <dl>
                {group.shortcuts.map((shortcut) => (
                  <div key={shortcut.action} className="shortcut">
                    <dt>
                      {shortcut.keys.map((key) => (
                        <kbd key={key}>{key}</kbd>
                      ))}
                    </dt>
                    <dd>{shortcut.action}</dd>
                  </div>
                ))}
              </dl>
            </section>
          ))}
        </div>
      </div>
    </div>
  );
}
