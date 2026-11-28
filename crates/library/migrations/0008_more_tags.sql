-- Tags the scanner reads since reader version 2, so field writes can
-- cover them and lookups can tell when they would replace a value. Rows
-- stay NULL until the next scan rereads the file.
ALTER TABLE tracks ADD COLUMN catalog_number TEXT;
ALTER TABLE tracks ADD COLUMN isrc TEXT;
ALTER TABLE tracks ADD COLUMN release_date TEXT;
ALTER TABLE tracks ADD COLUMN track_number TEXT;
ALTER TABLE tracks ADD COLUMN disc_number TEXT;
