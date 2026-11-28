# Metadata lookup

Decided November 27, 2026. Phase 6 step 5 ([plan](../plans/metadata-and-conversion.md#step-5-metadata-lookup)). Implemented in `crates/metadata` (`overtone-metadata`), `overtone_audio::analysis::fingerprint`, and `overtone_core::lookup`.

## Decision

- Lookups only add pending suggestions through the provenance model ([step 2](../plans/metadata-and-conversion.md#step-2-per-field-metadata-provenance)). They fill factual fields (album, release date, year, label, catalog number, ISRC, track number, and the store's genre as `store_genre`) and never Comment, Grouping, or Genre.
- Providers, in lookup order: **AcoustID** (when an application key is set) identifies the recording from a Chromaprint fingerprint, then **MusicBrainz** (always available, no key) supplies the data; when AcoustID found the recording, the MusicBrainz text search is skipped. **Discogs** (when a personal token is set) adds labels, catalog numbers, and styles, which it covers best for electronic music.
- Matching is ported from OneTagger's `MatchingUtils` (GPL-3.0, attributed in `matching.rs`): titles are cleaned step by step (lowercase, articles, "original mix", DJ-pool attributes, "feat."), an exact match after any step scores 1.0, otherwise normalized Levenshtein at strictness 0.7 or better; an artist must match; durations must be within 30 s. Two additions: equal scores go to the closest duration (gaps within 3 s count as equal, so the DJ's exact version wins over a radio edit) and then to the most original release, and Discogs results are capped at confidence 0.8 because Discogs returns releases, not tracks.
- MusicBrainz picks the release to describe a match from the full recording (search results list only some releases): official singles, EPs, and albums first, then oldest. Compilations and DJ mixes never supply album, date, or track number, because those describe the collection.
- Each lookup is recorded per track and provider (`lookup_runs`), so a run without arguments only searches tracks a chosen provider hasn't searched. Failed requests aren't recorded, so they are retried next time. A provider that refuses its credentials (401, 403, or AcoustID's "invalid API key") is turned off for the rest of the run.

## Crates

| Need | Chosen | Why | Rejected |
|---|---|---|---|
| HTTP | `ureq` 3.5 (rustls, gzip, json) | Blocking, small, no async runtime; lookups are sequential because every provider rate limits | `reqwest` (pulls in tokio for no gain), OneTagger's `onetagger-platforms` (unpublished, scrapers, tokio, rspotify) |
| Fingerprint | `rusty-chromaprint` 0.3.0 (MIT) | Pure Rust Chromaprint; `preset_test2` is algorithm 2, fpcalc's default (checked: `id() == 1`) | Shelling out to `fpcalc` (another binary to bundle) |
| String similarity | `strsim` 0.11 (MIT) | What OneTagger's matching uses | |
| Title cleaning | `regex` 1.12 | OneTagger's patterns | |

Accent folding uses Unicode decomposition (`unicode-normalization`, already in the workspace) instead of OneTagger's `unidecode`. AcoustID's URL-safe base64 is a 20-line encoder in `acoustid.rs`.

## Rate limits and the User-Agent

`UreqHttp` keeps at least 1.1 s between requests to MusicBrainz and Discogs and 0.35 s to AcoustID, and backs off (2, 4, 8 s) on 429 and 503 answers. A MusicBrainz lookup costs three requests (search, full recording, release labels), so about 3.5 s per track.

The User-Agent is `Overtone/<version>`, plus a contact only when the DJ sets one (`overtone-cli meta config --contact`). MusicBrainz asks clients for a contact and was answering many anonymous requests with 503 "busy" during development; with backoff, lookups still completed. Whether the shipped app should send a default contact (such as the project's URL) is open.

## Verification

Unit tests run every provider against recorded responses: MusicBrainz search, recording, and release responses captured live for a well-known track (`tests/fixtures/`), and Discogs and AcoustID bodies written from their documented formats, because both need credentials. A live run on a synthetic file tagged "Daft Punk - One More Time" (320 s) matched through MusicBrainz. Live Discogs and AcoustID runs need a token and a key and are on the [manual verification checklist](../manual-verification.md). Fingerprints are tested to match across FLAC, 24-bit WAV, and Opus (through ffmpeg) of the same audio and to differ for different audio.

## Known limits

- MusicBrainz data quality varies. A popular track has dozens of recordings, and the closest-duration one can have only compilation releases, in which case the match yields no album suggestion. Trying the next candidate when the best yields nothing is a possible improvement.
- Discogs search has no durations, so its matches rest on artist and title alone.
