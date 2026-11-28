-- Lossless conversions (WAV/AIFF to FLAC). The FLAC is written next to the
-- original, which stays the track until relocation (Phase 6 step 8b)
-- points Overtone and the DJ apps at the FLAC. Scans skip outputs that
-- are waiting, so they don't appear as new tracks.

CREATE TABLE conversions (
    id INTEGER PRIMARY KEY,
    track_id INTEGER NOT NULL REFERENCES tracks (id) ON DELETE CASCADE,
    output_path TEXT NOT NULL UNIQUE,
    format TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('pending_relocation', 'relocated', 'discarded')),
    -- JSON arrays: frames with no FLAC equivalent, and Serato data carried over.
    dropped TEXT NOT NULL,
    serato TEXT NOT NULL,
    frames INTEGER NOT NULL,
    created_at INTEGER NOT NULL
) STRICT;

CREATE UNIQUE INDEX conversions_one_pending
    ON conversions (track_id) WHERE status = 'pending_relocation';
