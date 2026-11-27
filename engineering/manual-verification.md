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
- [ ] Re-run `ot audit --all` after reviewing: your Verified OK and replace decisions are kept.

## Tag writing (Comment hashtags)

Do this on clones, never originals: `cp -c -R local/music /tmp/tag-test`, then `ot scan /tmp/tag-test`, add vibes with `ot vibe add <vibe> <tracks…>`, preview with `ot tag mirror`, and write with `ot tag mirror --write`. Include at least one ID3v2.3 MP3, one ID3v2.4 MP3, a FLAC, a WAV, and an AIFF, each previously analyzed in the DJ app (so it carries cues and beatgrids).

- [ ] Serato DJ: cues, loops, beatgrid, track color, and the waveform overview are intact after the write. The Comment shows the hashtags. A smart crate "Comment contains `#vibe`" matches the right tracks.
- [ ] Rekordbox: after Reload Tag, hot cues, memory cues, and the beatgrid are intact, and the Comment shows. Note whether the changed file size or modification time alone makes Rekordbox reanalyze the track.
- [ ] Traktor: after Import Metadata, cues and the beatgrid are intact, the Comment shows, and it doesn't reanalyze.
- [ ] Existing Comment text survives: a file with Mixed In Key text (for example `8A - Energy 6`) or a Rekordbox `/* tag */` block keeps it, with hashtags appended.
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

## Desktop app commands

Run the desktop app against a copy of your library under `local/` (see AGENTS.md), not real data.

- [ ] After adding a folder in the app, audio from that folder plays without restarting (the asset-protocol scope widens in `add_root`).
- [ ] Audio outside library folders can't be loaded by the webview.
- [ ] A removed folder's audio stays playable until the app restarts (Tauri can't narrow the scope again). Confirm that's acceptable.
- [ ] A full audit can be cancelled from the UI; finished tracks keep their reports and a second audit starts afterwards.
- [ ] Spectrogram PNGs appear under `~/Library/Caches/com.jacksonferguson.overtone/spectrograms/` and are replaced when the audio file changes.
