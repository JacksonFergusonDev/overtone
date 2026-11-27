# DJ software support

Overtone treats your local library database and embedded audio tags as the source of truth, then exports crates and playlists to your DJ software.

| Software | Target format | Sync mode | Status |
| --- | --- | --- | --- |
| Serato DJ Pro | `_Serato_/Subcrates/*.crate` | Direct `.crate` generation under an `Overtone` crate | Available in `overtone-cli sync serato`; not yet confirmed in Serato DJ 4 |
| Rekordbox 6 and 7 | `rekordbox.xml` | XML export for import | Available in `overtone-cli sync rekordbox`; not yet confirmed in Rekordbox |
| Rekordbox 6 and 7 | `master.db` (SQLCipher) | My Tags in an `Overtone` category, written to a verified copy that replaces the database | Available in `overtone-cli sync rekordbox-tags`; not yet confirmed in Rekordbox |
| Traktor Pro 3 and 4 | `collection.nml` | Byte-preserving splice of an `Overtone` playlist folder | Available in `overtone-cli sync traktor`; not yet confirmed in Traktor |

Embedded Comment hashtags (`overtone-cli tag mirror`) work as a fallback for smart crates and playlists in all three apps. See [vibe tags](../engineering/decisions/vibe-tags.md).

## Rekordbox export

`overtone-cli sync rekordbox` writes a `rekordbox.xml` file with every present track that is in a vibe and an `Overtone` playlist folder holding your vibe tree. It never touches Rekordbox's own database, and it works while Rekordbox is open. Add `--dry-run` to preview the playlists, and `--out <path>` to choose where the file goes (the default is `rekordbox.xml` in Overtone's data folder). If the file already exists, Overtone backs it up first, and it leaves the file alone when nothing changed.

To import it, open Rekordbox, go to **Preferences > Advanced > Database**, set **rekordbox xml > Imported Library** to the file, then open the **rekordbox xml** view in the sidebar and drag the `Overtone` folder or single playlists into your collection. Export again after changing vibes, then reload the view.

A vibe with child vibes becomes a folder. If the vibe has tracks of its own, the folder starts with a playlist of the same name holding them, so `Dubs` with a child `Deep` appears as `Dubs / Dubs` and `Dubs / Deep`.

## Traktor sync

`overtone-cli sync traktor` writes your vibe tree as playlists inside an `Overtone` folder at the top of Traktor's playlist tree, in `collection.nml`. Vibes map to folders and playlists the same way as the Rekordbox export. Overtone only changes that folder: your own playlists, smartlists, folders (even one you named `Overtone` inside another folder), cue points, and track metadata stay byte for byte as they were. If you delete every vibe, the next sync removes the folder.

- **Close Traktor first.** Traktor rewrites `collection.nml` when it quits, which would undo the sync, so Overtone refuses while Traktor Pro 3 or 4 is running.
- **Which file:** without options, Overtone uses the only `~/Documents/Native Instruments/Traktor <version>/collection.nml`. If you have several Traktor versions installed, choose one with `--collection <path>`.
- **Tracks must already be in Traktor.** Playlists can only list tracks from Traktor's collection, so Overtone skips tracks Traktor hasn't imported and lists them. Add those files in Traktor, quit it, and sync again.
- **Preview and backups:** `--dry-run` shows the playlists and the skipped tracks without writing. Before changing the file, Overtone saves a copy under `backups/traktor/` in its data folder. It writes nothing when the folder is already up to date.

Tracks on an external drive (under `/Volumes/<Drive>/`) are matched as Traktor's entries for that drive; this mapping hasn't been confirmed in Traktor yet.

## Rekordbox My Tags

`overtone-cli sync rekordbox-tags` writes each vibe as a My Tag in an `Overtone` category of Rekordbox's database (`~/Library/Pioneer/rekordbox/master.db`). Tags are named by the vibe's full path, such as `Dubs/Deep`, and hold the tracks you put in that vibe. Overtone only touches the `Overtone` category: your own My Tag categories, tags, and assignments stay as they are. A tag whose vibe you renamed or deleted is removed from the category on the next sync.

Quit Rekordbox first, including `rekordboxAgent` (its menu bar icon), or the command refuses. Add `--dry-run` to preview the changes without writing anything; it works while Rekordbox is open. Add `--db <path>` to use a different `master.db`, such as a copy you want to try it on.

Tracks must already be in your Rekordbox collection. Overtone matches them by file path and lists the ones Rekordbox doesn't have as skipped; import those in Rekordbox, then sync again.

Before writing, Overtone backs up `master.db`, its `-wal` and `-shm` files, and `masterPlaylists6.xml` into `backups/rekordbox-db/` in Overtone's data folder. It edits a copy, checks the copy, and only then replaces the database. If the tags already match, it writes nothing. To undo a sync, quit Rekordbox and copy the backed-up files back into `~/Library/Pioneer/rekordbox/`, deleting any `master.db-wal` and `master.db-shm` there first.

For crate evaluations, format details, and database safety rules, see [DJ software sync](../engineering/decisions/dj-software-sync.md).
