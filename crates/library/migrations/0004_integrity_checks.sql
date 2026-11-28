-- One ffmpeg integrity check per track: a full decode listing every error
-- it found. Kept beside the spectral verdict, never instead of it.

CREATE TABLE integrity_checks (
    track_id INTEGER PRIMARY KEY REFERENCES tracks (id) ON DELETE CASCADE,
    clean INTEGER NOT NULL CHECK (clean IN (0, 1)),
    -- 1 when every error is at the very start or end of the file.
    edge_only INTEGER NOT NULL CHECK (edge_only IN (0, 1)),
    -- JSON array of error lines, at most IntegrityCheck::MAX_ERRORS.
    errors TEXT NOT NULL,
    error_count INTEGER NOT NULL,
    file_modified_at INTEGER NOT NULL,
    ffmpeg_version TEXT NOT NULL,
    checked_at INTEGER NOT NULL
) STRICT;

CREATE INDEX integrity_checks_damaged ON integrity_checks (track_id)
    WHERE clean = 0 AND edge_only = 0;
