# Manual verification checklist

Checks that automated tests can't cover: behavior inside real DJ apps, listening by ear, and judgment calls. Work through them in one pass before a release. Add new items here as features land, and tick them off (`- [x]`) with a note of what you saw.

Use copies, never your real library. The `local/` testing ground (see [AGENTS.md](../AGENTS.md#audio-files-and-dj-database-safety)) holds APFS clones of the DnB, Garage, and House `My Favourite` folders and copies of your Serato data. Debug builds refuse to write under `~/Music` and the DJ apps' data folders, so run write tests against `local/` or a scratch copy.

Commands below assume a scratch library so your default one stays clean:

```bash
alias ot='cargo run -q --release -p overtone-cli -- --library /tmp/overtone-verify.sqlite'
```

## Setup

- [ ] Scan the testing ground: `ot scan local/music`. Expect about 1,151 tracks and one tag read failure (`Tokyo Drift (Keenan Mathias Remix).wav`, an invalid WAV chunk size).
- [ ] Run the audit: `ot audit`. Expect roughly 1,092 pass, 39 warning, 15 suspect, 5 inconclusive.

## Quality analysis (listen and look)

Render a spectrogram for any track with `ot spectrogram <id or path> --out <file>.png`, and list flagged tracks with `ot ls --needs-review`. Record your verdict with `ot review <track> ok|replace`.

- [ ] The 13 "320 kbps" MP3s flagged Suspect look like 128 kbps sources (flat ceiling at 15.4 to 16.8 kHz). Listen for 128k artifacts (swirly hats, smeared cymbals) and look for better copies:
  - Circadian "Energy In Motion"
  - Junk Mail "Lock Stock"
  - Dimension `UK (Bou Remix)[DUB]`
  - Traumatize & Rodeo "Badge Flasher (VIP)"
  - Konflict "Messiah"
  - La Roux "In For The Kill (High Contrast Remix)"
  - Olivia Rodrigo "Deja Vu (Jakar Bootleg)"
  - Oppidan & Cutty Ranks "Armed & Dangerous"
  - Peggy Gou "It Goes Like (Nanana) (Sammy Virji Edit)"
  - Badflite & RYVM "Where the Sun Goes Down"
  - "01. April 14th 2020"
  - Romy & Fred Again "Strong"
  - A$AP Rocky "Lord Pretty Flacko Jodye 2 (Hntr Remix)"
- [ ] The two lossless files flagged Suspect (hard 15.8 kHz ceiling) are MP3s wrapped as lossless. Did they come from free or promo downloads?
  - "Adele – Rolling In The Deep (Dimension DNB Remix).flac"
  - "A$AP Ferg – Work (Proppa x Rich DietZ… Tech House Edit).wav"
- [ ] Lossless files flagged Warning with a steep wall at about 20 to 20.8 kHz (a 320 kbps transcode looks the same as some genuine masters). For store-bought files, mark them Verified OK. Strongest cases: Danny Byrd "Red Mist VIP" and "Face to Face Again (Seba remix)". Others include Calibre "Even If", RUFUS "Innerbloom", Koven "The Outlines", Y U QT "Hyph Mngo Dub".
- [ ] MP3 Warnings with a 15 to 18 kHz edge but some content above it (Eric Prydz "Generate", Boys Noize, Odd Mob, Oppidan "You & I", Adam F): check where they came from; they may be fine.
- [ ] Spot-check a few Pass tracks' spectrograms against Spek or a similar tool so the rendering and dB scale look right.
- [ ] Integrity check: run `ot audit --integrity`, then `ot ls --damaged`. On `local/` expect about 16 tracks. Listen to the 14 MP3s whose `ot show` lists `bits_left` errors and note whether any audibly glitch; if none do, those warnings should become edge-only or ignored. Listen to the one MP3 with corrupt frames and confirm the damage is audible.
- [ ] Re-run `ot audit --all` after reviewing: your Verified OK and replace decisions are kept.

## Metadata lookup

Use a scratch library (`OVERTONE_LIBRARY=/tmp/lookup.sqlite`) over a few clones from `local/music`.

- [ ] `ot meta lookup <tracks>` with MusicBrainz only: for well-tagged tracks, the suggested album, label, and release date are the original release, not a compilation or DJ mix. Note any wrong matches.
- [ ] Add an AcoustID key (`ot meta config --acoustid-key`), then look up a track with deliberately wrong tags: AcoustID still identifies it.
- [ ] Add a Discogs token and look up a few electronic releases: labels, catalog numbers, and `store_genre` styles are right.
- [ ] A wrong key or token reports that the provider refused it and the run carries on with the others.
- [ ] Accept and reject a few suggestions; `ot meta show` shows accepted values as "lookup: <provider>", and rejected ones don't come back on the next lookup.

## Tag writing (Comment hashtags)

Do this on clones, never originals: `cp -c -R local/music /tmp/tag-test`, then `ot scan /tmp/tag-test`, add vibes with `ot vibe add <vibe> <tracks…>`, preview with `ot tag mirror`, and write with `ot tag mirror --write`. Include at least one ID3v2.3 MP3, one ID3v2.4 MP3, a FLAC, a WAV, and an AIFF, each previously analyzed in the DJ app (so it carries cues and beatgrids).

- [ ] Serato DJ: cues, loops, beatgrid, track color, and the waveform overview are intact after the write. The Comment shows the hashtags. A smart crate "Comment contains `#vibe`" matches the right tracks.
- [ ] Rekordbox: after Reload Tag, hot cues, memory cues, and the beatgrid are intact, and the Comment shows. Note whether the changed file size or modification time alone makes Rekordbox reanalyze the track.
- [ ] Traktor: after Import Metadata, cues and the beatgrid are intact, the Comment shows, and it doesn't reanalyze.
- [ ] Existing Comment text survives: a file with Mixed In Key text (for example `8A - Energy 6`) or a Rekordbox `/* tag */` block keeps it, with hashtags appended.
- [ ] Files the old writer skipped now write: pick an MP3 with an ID3v2.3 tag holding `TSOP` or duplicate `TXXX` frames, a FLAC with lowercase field names, and a WAV with no tags at all (Overtone appends an `ID3` chunk at the end). After the write, each DJ app shows the Comment and every other field as before, and Serato, Rekordbox, and Traktor still see the WAV's cues.
- [ ] `ot tag restore --write` puts the original Comments back, and the DJ apps show the originals after reloading tags.
- [ ] Finder tags and the creation date on written files are unchanged (Get Info in Finder).

## Serato crate sync

Test on a throwaway Serato library: a spare macOS user account, or temporarily swap in a copy of `~/Music/_Serato_` and `~/Library/Application Support/Serato` with Serato closed, and restore the originals afterwards. Serato 4 keeps its library in `master.sqlite` and tracks each `.crate` file's size and MD5, so its behavior with externally written crates is the main unknown.

- [ ] With Serato closed, `ot sync serato --dry-run` shows the expected crates under `Overtone`, and `ot sync serato` writes them.
- [ ] On launch, Serato DJ 4 shows the `Overtone` crate with nested vibe crates and the right tracks, in a sensible position in the crate list.
- [ ] The crates survive a second launch: Serato doesn't remove them or rewrite them from its own database.
- [ ] Tracks that weren't in the Serato library before show their metadata correctly.
- [ ] Your own crates outside `Overtone` are unchanged.
- [ ] Renaming a vibe (`ot vibe rename`), syncing again, and relaunching moves the crate; deleting a vibe removes its crate. Backups of replaced crate files appear under `~/Library/Application Support/com.jacksonferguson.overtone/backups/serato/`.
- [ ] `ot sync serato` refuses while Serato DJ is running.
- [ ] Tracks on an external drive sync into that drive's `/Volumes/<Drive>/_Serato_` folder and appear in Serato (no external-drive sample has been tested yet).
- [ ] Serato DJ 3.x, if you still use it, also picks up the crates.

## Rekordbox XML export

Use a throwaway Rekordbox library (a spare macOS user account, or back up Rekordbox's data folder first). Importing only reads the XML and adds to the Rekordbox collection, but it does add tracks. Export from the scratch library with `ot sync rekordbox --out /tmp/overtone-verify/rekordbox.xml`.

- [ ] `ot sync rekordbox --dry-run --out <file>` lists the expected playlists and writes nothing, and the real run writes the file. A second run reports it is already up to date.
- [ ] In Rekordbox 6 or 7, Preferences > Advanced > Database > rekordbox xml > Imported Library points at the file, and the `rekordbox xml` view in the sidebar shows an `Overtone` folder with the vibe tree. A vibe with child vibes shows as a folder starting with a playlist of its own tracks.
- [ ] Importing a playlist adds its tracks with the right title, artist, BPM, key, and duration, and the tracks play (the `Location` paths resolve).
- [ ] Tracks with spaces, `&`, `#`, `%`, parentheses, and accented characters in their file names import and play. Check one name stored in NFD (the form macOS file names usually use) and one in NFC.
- [ ] Renaming a vibe and exporting again shows the new name after Rekordbox reloads the file. Playlists you imported earlier stay in your collection until you delete them yourself.
- [ ] Your own playlists outside the imported `Overtone` folder are unchanged.
- [ ] The export succeeds while Rekordbox is running and the CLI says to re-import. Confirm the process check detects your install: `ot sync rekordbox --json` reports `"rekordbox_running": true` with Rekordbox open and `false` with it closed (the process name and bundle layout are unverified).
- [ ] Replacing an existing file leaves a copy under `~/Library/Application Support/com.jacksonferguson.overtone/backups/rekordbox/`.

## Desktop app commands

Run the desktop app against a copy of your library under `local/` (see AGENTS.md), not real data.

- [ ] Start the app with a scratch data folder so it doesn't use your default library: `OVERTONE_DATA_DIR=/tmp/overtone-app pnpm --dir app tauri dev`. Add `local/music` in Settings, scan, and audit; the library fills in without a restart.
- [ ] Playback works for each format WKWebView has to handle: MP3, AAC and ALAC `.m4a`, WAV, AIFF, and FLAC (FLAC in WKWebView is the least certain). Seeking to the middle of a long FLAC or WAV starts within a moment.
- [ ] After adding a folder in the app, audio from that folder plays without restarting (the asset-protocol scope widens in `add_root`).
- [ ] Audio outside library folders can't be loaded by the webview.
- [ ] A removed folder's audio stays playable until the app restarts (Tauri can't narrow the scope again). Confirm that's acceptable.
- [ ] A full audit can be cancelled from the UI; finished tracks keep their reports and a second audit starts afterwards.
- [ ] Spectrogram PNGs appear under `~/Library/Caches/com.jacksonferguson.overtone/spectrograms/` and are replaced when the audio file changes.

## Desktop app interface

Check at the default window size (1280x860) and the minimum (840x560). See [Using the desktop app](../docs/desktop-app.md) for the shortcuts.

- [ ] Scrolling the Library table with about 20,000 tracks stays smooth, and sorting or typing in the search field doesn't stall noticeably.
- [ ] At 840x560 the table drops the Time, Key, and Format columns without overlapping text, and the inspector floats over the table instead of squeezing it.
- [ ] The waveform is crisp on a Retina display, the played part is tinted, and click or drag seeks to the right spot by ear.
- [ ] `Space`, `J`, `K`, arrow keys, and `1` to `9` control playback, and none of them fire while typing in the search field or the quick-tag palette.
- [ ] `Space` on a focused button toggles playback without also clicking the button.
- [ ] The spectrogram's frequency labels line up with the image, and the dashed cutoff line sits where the energy falls off, in both linear and log scales.
- [ ] In Needs review, `O` and `R` advance to the next flagged track and playback follows. Judge whether that flow is fast enough for a real triage session.
- [ ] Dragging selected rows onto a vibe in the sidebar tags them.
- [ ] The Sync view's Serato Apply, Rekordbox Export, Rekordbox My Tags Apply, Traktor Apply, and Comment tag Apply each match the CLI's result for the same library (run them on the throwaway setups described in the sections above).
- [ ] Focus rings are visible on every control, and text contrast is comfortable in a dark room.

## Traktor collection sync

Use a throwaway Traktor setup: a spare macOS user account, or back up `~/Documents/Native Instruments/Traktor <version>/` first. Quit Traktor before each sync. Debug builds refuse to write under `~/Documents/Native Instruments`, so use a release build or a copy of the folder for this pass.

- [ ] `ot sync traktor --dry-run` finds the collection on its own (or lists several installed versions and asks for `--collection`), lists the expected playlists and the tracks Traktor doesn't have, and writes nothing.
- [ ] With Traktor Pro 3 and Traktor Pro 4 open, `ot sync traktor` refuses with `dj_app_running`. The process names are unverified: confirm each version is detected, and that it is no longer detected after quitting.
- [ ] After a sync, Traktor Pro 3 (NML version 19) and Traktor Pro 4 (version 20) open the collection without warnings and show an `Overtone` folder at the top level with the vibe tree; a vibe with child vibes shows as a folder starting with a playlist of its own tracks.
- [ ] Playlist tracks load and play, including file names with `&`, accented characters (one stored in NFD and one in NFC), a `:` in a folder or file name, and a stem file (`.stem.mp4`, written as `PRIMARYKEY TYPE="STEM"`).
- [ ] A track on an external drive (`/Volumes/<Drive>/...`) that is in Traktor's collection ends up in the playlist. The volume mapping is unverified.
- [ ] Your own playlists, smartlists, folders (including a folder named `Overtone` inside another folder), cue points, beatgrids, and track ratings are unchanged after the sync and after Traktor quits and rewrites the file.
- [ ] Syncing again with no vibe changes reports "Already up to date" and doesn't touch the file. Changing a vibe and syncing again updates the folder in place, and Traktor keeps it in the same spot.
- [ ] After Traktor quits and rewrites `collection.nml`, a dry run reports "Up to date" (Traktor kept Overtone's layout and UUIDs). If it reports a change, record how Traktor rewrote the folder.
- [ ] Each changed sync leaves a copy under `~/Library/Application Support/com.jacksonferguson.overtone/backups/traktor/`, and restoring that copy restores the old collection.

## Rekordbox My Tags (master.db)

This writes Rekordbox's live database, so do it on a throwaway setup: a spare macOS user account with its own Rekordbox library, or first copy all of `~/Library/Pioneer/rekordbox/` somewhere safe yourself. Debug builds refuse to write under `~/Library/Pioneer` (protected roots), so use a release build (`cargo build --release -p overtone-cli`) for the real-path checks. Try `--db` against a copy first: `cp -c ~/Library/Pioneer/rekordbox/master.db* /tmp/overtone-rb/` with Rekordbox closed, then `ot sync rekordbox-tags --db /tmp/overtone-rb/master.db`.

- [ ] With Rekordbox and `rekordboxAgent` quit, `ot sync rekordbox-tags --dry-run` lists the vibes as tags to create, and `ot sync rekordbox-tags` reports a backup folder under `backups/rekordbox-db/` holding `master.db`, `master.db-wal` and `master.db-shm` (when present), and `masterPlaylists6.xml`.
- [ ] Rekordbox opens normally afterwards: no "database is damaged" prompt, no forced reimport, and the collection, playlists, hot cues, beatgrids, and history look the same.
- [ ] The My Tag panel shows a fifth column named `Overtone` after Genre, Components, Situation, and the fourth column, with one tag per vibe named like `Dubs/Deep`. Check whether Rekordbox shows more than four My Tag columns at all, and where it puts the `Overtone` one (the category's `Seq` is 5).
- [ ] Tracks carry the right Overtone tags in the My Tag panel, in the track list's My Tag column, and in the Comment's `/* tag */` block if you have "Add My Tag to Comments" turned on. Note whether Rekordbox rewrites that Comment block for tagged tracks without being asked.
- [ ] Your own My Tags and their assignments in Genre, Components, Situation, and the fourth column are unchanged.
- [ ] Renaming a vibe and syncing again (with Rekordbox closed) replaces the old tag with the new one in Rekordbox, and the old tag's tracks lose it.
- [ ] Edit My Tags in Rekordbox (add a tag of your own in Genre, tag a track), quit, sync again: your edits survive. This proves changes Rekordbox left in `master.db-wal` are carried over.
- [ ] With Rekordbox open, the sync refuses with a message naming Rekordbox. After quitting Rekordbox, check whether `rekordboxAgent` keeps running (menu bar icon); if it does, the sync refuses naming `rekordboxAgent`. Confirm both process names match your install (`ps -axo comm | grep -i rekordbox`).
- [ ] Filter by an Overtone My Tag in Rekordbox's My Tag filter and in an intelligent playlist rule: the right tracks appear.
- [ ] If you use Rekordbox cloud library sync or a Rekordbox account, note whether the new tags sync to another device or get reverted. Overtone stamps each new row's `rb_local_usn` from `localUpdateCount` the way rbox does, leaves `usn` empty, and sets `rb_local_synced` to 0; whether Rekordbox's cloud sync picks that up is unverified.
- [ ] Export to a USB stick from Rekordbox after a sync and check that CDJs or XDJs show the Overtone My Tags (if your players support My Tag browsing).
- [ ] Restoring works: quit Rekordbox, delete `master.db-wal` and `master.db-shm`, copy the backup's files back into `~/Library/Pioneer/rekordbox/`, and Rekordbox opens with the pre-sync state.
