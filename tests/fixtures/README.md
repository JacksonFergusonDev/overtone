# Test fixtures

Automated tests must run against synthetic or minimal fixtures in this directory and temporary directories (`tempfile::TempDir`). Never read from or write to real user music libraries, `_Serato_`, Rekordbox `master.db`, or Traktor `collection.nml` during tests.

Put any private, real-world audio or database samples used only for manual local checks under `tests/fixtures/private/`, which is ignored by Git.

## Tags

`tags/geob_serato.id3` is a tag-only ID3v2.4 file with the GEOB frames Serato writes (markers, beatgrid, overview, autotags, each twice in Latin-1 and UTF-16). It comes from the `testdata` folder of the [id3 crate](https://codeberg.org/polyfloyd/rust-id3) and is MIT-licensed; see `tags/LICENSE-id3`.
