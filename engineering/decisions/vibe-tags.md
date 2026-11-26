# Vibe tags in audio files and DJ software

Decided November 26, 2026. This records how Overtone gets your vibe groupings into Serato, Rekordbox, and Traktor, and which embedded tag field mirrors them.

## Decision

Native DJ structures are the primary sync path. Embedded tags are an opt-in, portable fallback.

1. **Primary:** each vibe becomes a real crate or playlist in each DJ app: Serato `.crate` files (nested vibes become subcrates), Rekordbox playlists in `rekordbox.xml` (Phase 3) plus My Tags in a dedicated `Overtone` category in `master.db` (Phase 5), and Traktor playlists in `collection.nml` (Phase 5).
2. **Fallback:** when you turn it on, Overtone mirrors vibes as hashtags into the **Comment** field, for example `#dubs/deep #warmup`. It is off by default.
3. **Grouping is read, never written.** On scan, Overtone can offer to import hashtags it finds in Comment or Grouping as vibes.

## Why Comment and not Grouping

| | Comment | Grouping |
| --- | --- | --- |
| Serato smart crates | "contains" rules | "contains" rules |
| Rekordbox intelligent playlists | "contains" rules | Not in the documented criteria; `rbox-masterdb` 0.3.1 models it as equality only |
| Traktor smartlists | "contains" rules | Not read or displayed |
| CDJ display | Shown | Not shown |

Comment is the only field all three apps read and can filter on. Grouping also has an MP3 ambiguity: since iTunes 12.5, Apple writes Grouping to the `GRP1` frame and reuses `TIT1` for Work, while most other software still reads `TIT1`.

## Why tags are not the primary path

Rekordbox and Traktor copy track metadata into their own databases at import and do not pick up later file tag edits until you reload tags. A tag-only approach would leave already-imported tracks stale. Native crates and playlists update reliably.

## Comment mirroring rules

- **Overtone owns only its tokens.** It adds or removes `#slug` tokens for vibes in its own database. Every other character in the Comment stays untouched, including Mixed In Key key and energy text, Rekordbox's appended `/* tag / tag */` My Tag block, emoji ratings, and hashtags Overtone does not know about.
- **Idempotent.** Writing the same vibe set twice produces identical bytes.
- **Hierarchy:** a nested vibe `Dubs/Deep` is written as `#dubs/deep`. A Serato or Traktor "contains `#dubs`" rule then matches the parent and its children.
- **Prefix collisions:** "contains `#dub`" also matches `#dubtechno`. Overtone warns when one vibe slug is a prefix of another unrelated slug.
- **Preview and undo:** batch tag writes show a preview first, and Overtone stores each file's original Comment value in its database so you can revert.
- **Fields per format:**
  - MP3, AIFF, WAV: ID3v2 `COMM` frame with language `eng` and an empty description. Other `COMM` frames (such as iTunes `iTunNORM`) are preserved.
  - FLAC: Vorbis `COMMENT`.
  - M4A: `©cmt`.
  - All formats: vendor frames (`GEOB` for Serato, `PRIV` `TRAKTOR4` for Traktor) are preserved byte for byte, with tests.
- **Traktor consistency:** the Phase 5 `collection.nml` patcher also updates each patched entry's `INFO/@COMMENT` so Traktor matches the file.

## Consequence for duplicate detection

Because tag writes change file bytes, duplicate detection hashes audio content only, never the whole file.

## Verification still needed

Serato and Rekordbox do not document which file frame backs each field, and several sources are forum posts. Before enabling mirroring for real libraries, tag test files and confirm in each app that the Comment value appears and smart rules match.

## Sources

- [Serato: Library Smart Crates](https://support.serato.com/hc/en-us/articles/360001447715-Library-Smart-Crates)
- [Serato forum: Grouping codes for smart crates](https://serato.com/forum/discussion/243558)
- [Rekordbox 7.0.0 manual](https://cdn.rekordbox.com/files/20240509141437/rekordbox7.0.0_manual_EN.pdf)
- [Pioneer forum: My Tag in Comments format](https://forums.pioneerdj.com/hc/fr/community/posts/900001887546/comments/900001391866)
- [Deejayplaza: Rekordbox intelligent playlists](https://www.deejayplaza.com/en/articles/rekordbox-intelligent-playlist)
- [NI community: Grouping not shown in Traktor](https://community.native-instruments.com/discussion/comment/42435/)
- [Mp3tag: Traktor Pro 4 tag compatibility](https://community.mp3tag.de/t/native-instrument-traktor-pro-4-v4-1-1-23-tag-compatibility-with-mp3tag/67494)
- [Digital DJ Tips: Traktor smartlists](https://www.digitaldjtips.com/2020/08/smart-playlists-smartlists-traktor-pro/)
- [Mixxx bug: iTunes `GRP1` Grouping](https://bugs.launchpad.net/bugs/1773683)
- [Lexicon: tags](https://rekordcloud.firebaseapp.com/blog/lexicon-tags-tags-tags)
- `rbox-masterdb` 0.3.1 source: `src/models/djmd_my_tag.rs` (My Tag insert models) and `src/smart_list.rs` (Grouping equality-only filter)
