# Test fixtures

Automated tests must run against synthetic or minimal fixtures in this directory and temporary directories (`tempfile::TempDir`). Never read from or write to real user music libraries, `_Serato_`, Rekordbox `master.db`, or Traktor `collection.nml` during tests.

Put any private, real-world audio or database samples used only for manual local checks under `tests/fixtures/private/`, which is ignored by Git.
