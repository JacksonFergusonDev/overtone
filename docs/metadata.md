# Your metadata and where it came from

Overtone keeps track of where every metadata value came from, so values you typed never get mixed up with values it fetched. Each field of each track has one of three sources:

- **file:** the value in the audio file when Overtone scanned it.
- **you:** a value you set in Overtone. Your values win over the file and over every lookup, and rescans never replace them.
- **lookup:** a value from an online database (such as MusicBrainz) that you accepted.

## See where values came from

```bash
overtone-cli meta show <track>
```

`<track>` is a track id (from `overtone-cli ls`) or a file path. The table lists each field's value, its source, and, when your value replaces the file's, what the file has.

## Set your own values

```bash
overtone-cli meta set <track> label "Hospital Records"
```

Use `--clear` instead of a value to blank a field on purpose. `overtone-cli meta reset <track> label` drops your value so the file's shows again.

You can set title, artist, album, album_artist, remixer, label, genre, year, bpm, key, grouping, catalog_number, isrc, release_date, track_number, disc_number, and store_genre. Comment changes only through vibe mirroring (`overtone-cli tag mirror`).

Values you set live in Overtone's library until you write them into your files (see below). Exports already use them.

## Write your values into your files

Your values and accepted lookup values stay in Overtone until you copy them into the audio files, so your DJ software sees them too:

```bash
overtone-cli meta apply
overtone-cli meta apply --write
```

The first command previews each file's changes; `--write` writes them. Name tracks or add `--vibe <vibe>` to write only some. Overtone writes title, artist, album, album artist, remixer, label, genre, year, BPM, key, grouping, catalog number, ISRC, release date, and track and disc numbers into MP3, WAV, AIFF, and FLAC files. Store genre stays in Overtone, and M4A files aren't written yet. A release date must be `YYYY`, `YYYY-MM`, or `YYYY-MM-DD`.

Each value goes into the tag your DJ software already reads it from. For example, a FLAC whose label is in `ORGANIZATION` keeps it there, and an MP3 with an iTunes `GRP1` grouping keeps using `GRP1`. Every write is checked before it replaces the file: Serato cues and beatgrids, artwork, every other tag, and the audio itself stay exactly as they were, and anything Overtone can't check is refused and left untouched, with the reason.

Your values stay yours after they're written, and Overtone remembers what each file held before its first write:

```bash
overtone-cli meta restore --write
```

This puts the files' original values back. Your values stay in Overtone, so `meta apply` can write them again later.

## Look tracks up online

```bash
overtone-cli meta lookup
```

Overtone searches MusicBrainz for each track by title, artist, and length, and adds what the best match proposes (album, release date, label, catalog number, ISRC, track number) as suggestions. Without track ids it looks up tracks it hasn't searched yet; name tracks to look up just those, and `--provider musicbrainz` (or `acoustid`, `discogs`) to use one source.

Two more sources work once you add credentials:

- **AcoustID** identifies a track from its sound, even with wrong or missing tags. Get a free application key at acoustid.org/new-application, then run `overtone-cli meta config --acoustid-key <key>`.
- **Discogs** is the best source for electronic labels, catalog numbers, and styles. Create a personal token at discogs.com/settings/developers, then run `overtone-cli meta config --discogs-token <token>`.

MusicBrainz is sometimes busy; Overtone waits and retries, so a lookup can take a few seconds per track. MusicBrainz asks apps to say who they are, so Overtone's requests name the Overtone project and its maintainer's contact. `overtone-cli meta config --contact <email or URL>` sends yours instead. Lookups send only titles, artists, lengths, and (for AcoustID) an audio fingerprint, never your files.

## Review lookup suggestions

Lookups never change a track by themselves. What they find waits as a suggestion until you decide:

```bash
overtone-cli meta suggestions
overtone-cli meta accept <id>
overtone-cli meta reject <id>
```

- Lookups never suggest Comment or Grouping, which hold your own curation.
- A store's genre arrives as **store_genre**, kept apart from your Genre.
- A suggestion that would replace a value already there (yours or the file's) is marked "would replace", and accepting it needs `--replace`.
- Accepting one suggestion rejects the field's other pending ones. A rejected value is never suggested again by the same source.

Add `--all` to `meta suggestions` to include ones you already decided on. Every `meta` command supports `--json`.
