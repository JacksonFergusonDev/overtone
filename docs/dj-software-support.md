# DJ software support

Overtone treats your local library database and embedded audio tags as the source of truth, then exports crates and playlists to your DJ software.

| Software | Target format | Sync mode | Status |
| --- | --- | --- | --- |
| Serato DJ Pro | `_Serato_/Subcrates/*.crate` | Direct `.crate` generation | Planned (Phase 3) |
| Rekordbox 6 and 7 | `rekordbox.xml` | XML export for import | Planned (Phase 3) |
| Rekordbox 6 and 7 | `master.db` (SQLCipher) | Guarded live DB sync | Planned (Phase 5) |
| Traktor Pro 3 and 4 | `collection.nml` | Lossless XML patching | Planned (Phase 5) |

For crate evaluations, format details, and database safety rules, see [DJ software sync](../engineering/decisions/dj-software-sync.md).
