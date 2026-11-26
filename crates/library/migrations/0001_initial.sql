-- Overtone library schema, version 1.
--
-- Paths are stored exactly as the filesystem reports them and compared
-- literally (never with LIKE, whose `_` and `%` wildcards match real
-- filename characters). Timestamps are Unix milliseconds, UTC.

CREATE TABLE library_roots (
    id INTEGER PRIMARY KEY,
    path TEXT NOT NULL UNIQUE,
    added_at INTEGER NOT NULL
) STRICT;

CREATE TABLE tracks (
    id INTEGER PRIMARY KEY,
    path TEXT NOT NULL UNIQUE,
    file_size INTEGER NOT NULL,
    modified_at INTEGER NOT NULL,
    audio_hash TEXT,
    format TEXT CHECK (format IN ('flac', 'wav', 'aiff', 'alac', 'mp3', 'aac', 'other')),
    sample_rate_hz INTEGER,
    bit_depth INTEGER,
    channels INTEGER,
    bitrate_kbps INTEGER,
    duration_ms INTEGER,
    title TEXT,
    artist TEXT,
    album TEXT,
    album_artist TEXT,
    remixer TEXT,
    label TEXT,
    genre TEXT,
    year INTEGER,
    bpm REAL,
    musical_key TEXT,
    comment TEXT,
    grouping TEXT,
    added_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    missing_since INTEGER
) STRICT;

CREATE INDEX tracks_audio_hash ON tracks (audio_hash) WHERE audio_hash IS NOT NULL;

-- Nested vibes are stored by full path ("Dubs/Deep"). `slug` is the
-- normalized identity ("dubs/deep") shared with Comment hashtags, so two
-- vibes that would produce the same hashtag can't coexist.
CREATE TABLE vibes (
    id INTEGER PRIMARY KEY,
    path TEXT NOT NULL,
    slug TEXT NOT NULL UNIQUE,
    created_at INTEGER NOT NULL
) STRICT;

CREATE TABLE track_vibes (
    track_id INTEGER NOT NULL REFERENCES tracks (id) ON DELETE CASCADE,
    vibe_id INTEGER NOT NULL REFERENCES vibes (id) ON DELETE CASCADE,
    added_at INTEGER NOT NULL,
    PRIMARY KEY (track_id, vibe_id)
) STRICT, WITHOUT ROWID;

CREATE INDEX track_vibes_vibe ON track_vibes (vibe_id);

-- One current report per track. The DJ's review is kept beside the
-- automated verdict so re-analysis never erases a human decision.
CREATE TABLE quality_reports (
    track_id INTEGER PRIMARY KEY REFERENCES tracks (id) ON DELETE CASCADE,
    verdict TEXT NOT NULL CHECK (verdict IN ('unchecked', 'pass', 'warning', 'suspect')),
    cutoff_hz REAL,
    confidence REAL,
    reason TEXT,
    analyzer_version INTEGER NOT NULL,
    analyzed_at INTEGER NOT NULL,
    review TEXT NOT NULL DEFAULT 'unreviewed'
        CHECK (review IN ('unreviewed', 'verified_ok', 'replace_wanted')),
    reviewed_at INTEGER
) STRICT;

-- The value a tag field had before Overtone first wrote it, so tag
-- mirroring can be reverted. Only the first original is kept per field.
CREATE TABLE tag_snapshots (
    track_id INTEGER NOT NULL REFERENCES tracks (id) ON DELETE CASCADE,
    field TEXT NOT NULL,
    original_value TEXT,
    captured_at INTEGER NOT NULL,
    PRIMARY KEY (track_id, field)
) STRICT, WITHOUT ROWID;

CREATE TABLE sync_runs (
    id INTEGER PRIMARY KEY,
    app TEXT NOT NULL CHECK (app IN ('serato', 'rekordbox', 'traktor')),
    target TEXT NOT NULL,
    dry_run INTEGER NOT NULL CHECK (dry_run IN (0, 1)),
    status TEXT NOT NULL CHECK (status IN ('running', 'succeeded', 'failed')),
    started_at INTEGER NOT NULL,
    finished_at INTEGER,
    backup_path TEXT,
    summary TEXT
) STRICT;

CREATE TABLE settings (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
) STRICT;
