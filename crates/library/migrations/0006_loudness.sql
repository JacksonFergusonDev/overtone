-- Loudness and level measurements, one row per analyzed track, written by
-- the same analysis pass as the quality report.

CREATE TABLE loudness_reports (
    track_id INTEGER PRIMARY KEY REFERENCES tracks (id) ON DELETE CASCADE,
    integrated_lufs REAL,
    loudness_range_lu REAL,
    true_peak_dbtp REAL,
    sample_peak_dbfs REAL,
    clipped_runs INTEGER NOT NULL,
    leading_silence_ms INTEGER NOT NULL
) STRICT;
