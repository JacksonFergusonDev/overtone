# DJ software support

Overtone treats your local library database and embedded audio tags as the source of truth, then exports crates and playlists to your DJ software.

| Software | Target format | Sync mode | Status |
| --- | --- | --- | --- |
| Serato DJ Pro | `_Serato_/Subcrates/*.crate` | Direct `.crate` generation under an `Overtone` crate | Available in `overtone-cli sync serato`; not yet confirmed in Serato DJ 4 |
| Rekordbox 6 and 7 | `rekordbox.xml` | XML export for import | Available in `overtone-cli sync rekordbox`; not yet confirmed in Rekordbox |
| Rekordbox 6 and 7 | `master.db` (SQLCipher) | Guarded live DB sync | Planned (Phase 5) |
| Traktor Pro 3 and 4 | `collection.nml` | Lossless XML patching | Planned (Phase 5) |

Embedded Comment hashtags (`overtone-cli tag mirror`) work as a fallback for smart crates and playlists in all three apps. See [vibe tags](../engineering/decisions/vibe-tags.md).

## Rekordbox export

`overtone-cli sync rekordbox` writes a `rekordbox.xml` file with every present track that is in a vibe and an `Overtone` playlist folder holding your vibe tree. It never touches Rekordbox's own database, and it works while Rekordbox is open. Add `--dry-run` to preview the playlists, and `--out <path>` to choose where the file goes (the default is `rekordbox.xml` in Overtone's data folder). If the file already exists, Overtone backs it up first, and it leaves the file alone when nothing changed.

To import it, open Rekordbox, go to **Preferences > Advanced > Database**, set **rekordbox xml > Imported Library** to the file, then open the **rekordbox xml** view in the sidebar and drag the `Overtone` folder or single playlists into your collection. Export again after changing vibes, then reload the view.

A vibe with child vibes becomes a folder. If the vibe has tracks of its own, the folder starts with a playlist of the same name holding them, so `Dubs` with a child `Deep` appears as `Dubs / Dubs` and `Dubs / Deep`.

For crate evaluations, format details, and database safety rules, see [DJ software sync](../engineering/decisions/dj-software-sync.md).
