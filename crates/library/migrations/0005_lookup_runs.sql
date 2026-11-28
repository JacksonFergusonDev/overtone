-- When each track was last looked up with each provider, so a lookup run
-- skips tracks it already searched instead of querying them again.

CREATE TABLE lookup_runs (
    track_id INTEGER NOT NULL REFERENCES tracks (id) ON DELETE CASCADE,
    provider TEXT NOT NULL,
    looked_up_at INTEGER NOT NULL,
    matched INTEGER NOT NULL CHECK (matched IN (0, 1)),
    PRIMARY KEY (track_id, provider)
) STRICT;
