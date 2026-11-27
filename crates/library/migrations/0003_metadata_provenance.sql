-- Metadata provenance: values the DJ entered or accepted, kept apart from
-- what the file holds, and lookup suggestions waiting for review.
--
-- `tracks` keeps the file's values exactly as scanned. A row in
-- `track_field_overrides` replaces one field's value; without one the
-- field's source is the file. Field names are validated in Rust
-- (`MetadataField`), so adding a field needs no migration.

CREATE TABLE metadata_suggestions (
    id INTEGER PRIMARY KEY,
    track_id INTEGER NOT NULL REFERENCES tracks (id) ON DELETE CASCADE,
    field TEXT NOT NULL,
    value TEXT NOT NULL,
    provider TEXT NOT NULL,
    provider_ref TEXT,
    confidence REAL,
    fetched_at INTEGER NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending', 'accepted', 'rejected')),
    decided_at INTEGER,
    -- A value the DJ rejected is never suggested again by the same provider.
    UNIQUE (track_id, field, provider, value)
) STRICT;

CREATE INDEX metadata_suggestions_pending
    ON metadata_suggestions (track_id) WHERE status = 'pending';

CREATE TABLE track_field_overrides (
    track_id INTEGER NOT NULL REFERENCES tracks (id) ON DELETE CASCADE,
    field TEXT NOT NULL,
    -- NULL: the DJ cleared the field.
    value TEXT,
    source TEXT NOT NULL CHECK (source IN ('user', 'lookup')),
    provider TEXT,
    provider_ref TEXT,
    suggestion_id INTEGER REFERENCES metadata_suggestions (id) ON DELETE SET NULL,
    set_at INTEGER NOT NULL,
    PRIMARY KEY (track_id, field),
    CHECK ((source = 'lookup') = (provider IS NOT NULL))
) STRICT;
