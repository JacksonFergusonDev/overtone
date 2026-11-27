-- Overtone library schema, version 2: cached waveform peaks.
--
-- `data` is opaque to the library. overtone-audio's `Waveform::encode`
-- writes it: one version byte, then a signed 8-bit min and max per bucket.
-- `analyzer_version` lets a newer analyzer replace stale peaks.

CREATE TABLE waveforms (
    track_id INTEGER PRIMARY KEY REFERENCES tracks (id) ON DELETE CASCADE,
    analyzer_version INTEGER NOT NULL,
    data BLOB NOT NULL,
    created_at INTEGER NOT NULL
) STRICT;
