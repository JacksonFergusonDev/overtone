import { useEffect, useState } from "react";
import { commands, errorMessage, type AppStatus } from "./api";
import "./styles.css";

export default function App() {
  const [status, setStatus] = useState<AppStatus | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let active = true;
    commands
      .appStatus()
      .then((data) => {
        if (active) setStatus(data);
      })
      .catch((err: unknown) => {
        if (active) setError(errorMessage(err));
      });
    return () => {
      active = false;
    };
  }, []);

  return (
    <main className="shell">
      <header className="header">
        <h1>Overtone</h1>
        <p className="subtitle">
          DJ library hub for spectral quality checks, overlapping vibes, and
          Serato, Rekordbox, and Traktor sync.
        </p>
      </header>
      <section className="panel" aria-live="polite">
        {error ? (
          <p className="error" role="alert">
            Couldn't open your library: {error}
          </p>
        ) : status ? (
          <dl className="stats">
            <div>
              <dt>Version</dt>
              <dd>{status.version}</dd>
            </div>
            <div>
              <dt>Tracks</dt>
              <dd>{status.library.track_count}</dd>
            </div>
            <div>
              <dt>Vibes</dt>
              <dd>{status.library.vibe_count}</dd>
            </div>
            <div>
              <dt>Needs review</dt>
              <dd>{status.library.needs_review_count}</dd>
            </div>
          </dl>
        ) : (
          <p className="muted">Opening your library…</p>
        )}
      </section>
    </main>
  );
}
