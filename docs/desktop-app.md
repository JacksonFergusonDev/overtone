# Using the desktop app

The Overtone desktop app is where you listen to tracks, check flagged files, tag vibes, and sync crates to your DJ software. You can do almost everything from the keyboard; press `?` at any time to see every shortcut.

## Set up your library

1. Open **Settings** in the sidebar.
2. Under **Library folders**, choose **Add folder** and pick the folder that holds your music. Overtone indexes files where they are and never moves them.
3. Choose **Scan**. Turn on **Refresh all tags** if you edited tags in another app and want Overtone to read them again.
4. Under **Quality audit**, choose **Audit pending tracks** to analyze every track that hasn't been checked yet. **Re-audit all** analyzes everything again. You can cancel an audit; tracks already analyzed keep their results.

Scans and audits run in the background. Their progress shows in the status bar at the bottom of the window, and the track table updates when they finish.

## Browse and play

The **Library** view lists every track. Click a column header to sort by it, and click it again to reverse the order. Use the search field (`/` or `⌘ F`) to find tracks by title, artist, album, or comment, and the chips next to it to show only certain verdicts or missing files.

Click a vibe in the sidebar to show only its tracks, including tracks in vibes nested below it. Choose **Library** to show everything again.

Double-click a track or press `Enter` to play it. Tracks whose files are missing (for example on an unplugged drive) are struck through and can't be played; reconnect the drive and scan again.

The player bar shows the waveform of the playing track. Click or drag on it to seek. The part you've already heard is tinted.

## Tag vibes

1. Select one or more tracks. Click, shift-click to select a range, or `⌘`-click to add single tracks. With the keyboard, use the arrow keys and hold `Shift` to extend the selection.
2. Press `T` (or `⌘ K`) to open the quick-tag palette.
3. Type part of a vibe's name. A check mark means every selected track has the vibe; a dash means some do.
4. Press `Enter` to toggle the highlighted vibe: Overtone adds it to every selected track, or removes it if they all have it already. The palette stays open so you can tag several vibes in a row.
5. If nothing matches, choose **Create vibe** to make a new one. Use `/` to nest vibes, for example `Dubs/Deep`. Vibe names can't contain `#`, `:`, `\`, or `%%`.
6. Press `Esc` to close the palette.

Your nine most recent vibes are listed first when the search is empty; press `⌥ 1` to `⌥ 9` to toggle one directly. You can also drag selected tracks onto a vibe in the sidebar.

To rename or delete a vibe, hover over it in the sidebar and use the pencil or trash button. Deleting a vibe also deletes the vibes nested below it. Your tracks stay in the library.

## Review flagged tracks

The audit flags tracks whose spectrum suggests a lossy source, such as an MP3 re-encoded as a high-bitrate file. Flags are only suggestions: listen and look at the spectrogram before you replace a file. Overtone never deletes or moves files based on a verdict.

1. Open **Needs review** in the sidebar. It lists flagged tracks you haven't reviewed yet, worst first, and opens the quality inspector.
2. The inspector shows the verdict, the reason, the estimated cutoff frequency, and a spectrogram with a dashed line at the cutoff. Press `L` to switch the spectrogram between a linear and a logarithmic frequency scale.
3. Press `O` for **Verify OK** if the track sounds fine, or `R` for **Replace wanted** if you want to find a better copy. Press `U` to clear your decision.
4. The next flagged track is selected automatically, and playback follows if you were listening.

In the Library view, press `I` to show or hide the inspector for the focused track.

## Check metadata, sound, and files

The inspector (`I`) also shows, below the quality section, everything Overtone knows about the focused track.

- **Sound:** loudness (LUFS), true peak, loudness range, clipping, and silence before the music, measured by the audit. **Check the file** decodes the whole track with ffmpeg and reports decode errors. Errors only in junk at the very start or end are called out separately, because the music itself is fine.
- **Metadata:** each field shows where its value came from: **file**, **you**, or the lookup service you accepted it from. Choose **Edit** to set your own value, which rescans never replace, and **Revert** to go back to the file's value. **Show all fields** includes empty ones. A dot marks values the audio file doesn't hold yet; **Write to file…** shows the changes and writes them after you confirm, and **Restore the file's original values** undoes that. See [Write your values into your files](metadata.md#write-your-values-into-your-files).
- **Suggestions:** **Look this track up** searches MusicBrainz, and AcoustID and Discogs once you add keys in Settings. Each result waits here with its source and match strength. Choose **Accept** (or **Replace** when it would change a value you already have) or **Reject**. Rejected values never come back. Lookups never touch Comment, Grouping, or your Genre.
- **Files:** pick a format and choose **Export a copy…** to write a copy into a folder you pick, with your tags and artwork ([Export copies](export.md)). For WAV and AIFF tracks, **Convert to FLAC…** shows what carries over and writes a verified FLAC beside the original ([Convert to FLAC](convert.md)). The original stays the track your DJ software uses; **Discard the FLAC** deletes the new file.

In **Settings**, **Metadata lookup** holds your AcoustID key, Discogs token, and optional MusicBrainz contact, and **Look up new tracks** looks up every track that hasn't been looked up yet. **Files** has **Check files** to check every track for decode errors, **Convert to FLAC…** for every WAV and AIFF track, and **Write metadata…** to write every track's values you set or accepted into its file. Checks and conversions need ffmpeg. You can cancel any of them; finished tracks keep their results.

## Sync to your DJ software

Open **Sync** in the sidebar. The top of the view shows whether Serato, Rekordbox, and Traktor are running.

- **Serato:** choose **Preview** to see which crate files Overtone would create, update, or delete in each `_Serato_` folder, and any tracks skipped because they're on another drive. Quit Serato, then choose **Apply**. Overtone backs up each folder before writing.
- **Rekordbox:** choose **Preview** to see the playlists Overtone would write to `rekordbox.xml`, then **Export**. You can export while Rekordbox is running. In Rekordbox, open Preferences > Advanced > Database and set **rekordbox xml > Imported Library** to the exported file, then import the Overtone playlists from the **rekordbox xml** section of the sidebar. Import again after each export.
- **Rekordbox My Tags:** choose **Preview** to see which My Tags Overtone would create, update, or remove in the `Overtone` category of Rekordbox's database (preview works while Rekordbox is open). Quit Rekordbox, including **rekordboxAgent** in the menu bar, then choose **Apply**. Overtone backs up the database, edits and checks a copy, then replaces it. Tracks Rekordbox hasn't imported are listed and left out.
- **Traktor:** quit Traktor, then choose **Preview** to see the playlists Overtone would write to the `Overtone` folder in Traktor's collection, and any tagged tracks that aren't in Traktor yet (add them in Traktor, then sync again). Choose **Apply** to write them. Nothing else in the collection changes, and the file is backed up first.
- **Comment tags:** optionally mirror vibes as hashtags, such as `#dubs/deep`, into each file's Comment tag for smart crates and CDJ screens. This writes to your audio files. **Preview** shows each Comment before and after. Only Overtone's own hashtags change. **Restore originals** writes back the Comment each file had before Overtone first changed it.

The **History** list shows recent syncs and where their backups are.

## Keyboard shortcuts

Shortcuts don't fire while you're typing in a text field.

### Playback

| Keys | Action |
| --- | --- |
| `Space` | Play or pause |
| `J` | Next track in the table |
| `K` | Previous track in the table |
| `←` / `→` | Seek back or forward 5 seconds |
| `Shift ←` / `Shift →` | Seek back or forward 30 seconds |
| `1` to `9` | Jump to 10% to 90% of the track |

### Track table

These work when the table has focus (click a row, or press `Tab` to reach it).

| Keys | Action |
| --- | --- |
| `↑` / `↓` | Move the cursor |
| `Shift ↑` / `Shift ↓` | Extend the selection |
| `Home` / `End` | First or last track |
| `Page Up` / `Page Down` | Move by a page |
| `Enter` | Play the focused track |
| `⌘ A` | Select every track shown |
| `Esc` | Clear the selection |
| `/` or `⌘ F` | Search the table |

### Tagging

| Keys | Action |
| --- | --- |
| `T` or `⌘ K` | Open the quick-tag palette for the selection |
| `↑` / `↓`, `Enter` | Pick and toggle a vibe in the palette |
| `⌥ 1` to `⌥ 9` | Toggle a recent vibe in the palette |
| `Esc` | Close the palette |

### Quality review

| Keys | Action |
| --- | --- |
| `I` | Show or hide the quality inspector |
| `O` | Mark Verify OK |
| `R` | Mark Replace wanted |
| `U` | Clear the review |
| `L` | Switch the spectrogram between linear and log |

### Views

| Keys | Action |
| --- | --- |
| `⌘ 1` | Library |
| `⌘ 2` | Needs review |
| `⌘ 3` | Sync |
| `⌘ 4` | Settings |
| `?` | Show all shortcuts |
