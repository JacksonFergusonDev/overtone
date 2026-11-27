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

Values you set live in Overtone's library for now. Writing them into your audio files and DJ software exports comes in a later release.

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
