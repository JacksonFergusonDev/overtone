# DJ software sync: Serato, Rekordbox, and Traktor Rust ecosystems

Decided November 26, 2026. This records our evaluation of the Rust crate ecosystems for Serato DJ, Pioneer/AlphaTheta Rekordbox, and Native Instruments Traktor Pro, and the exact integration strategy chosen for each platform.

## Summary of decisions

| Platform | Target format(s) | Crate decision | Integration strategy |
| --- | --- | --- | --- |
| **Serato DJ** | `_Serato_/Subcrates/*.crate` (and future read-only Serato 4 `master.sqlite`) | **Custom ~50 to 100 LOC TLV reader/writer** (do not depend on `triseratops` for crates) | Write `.crate` files directly; let Serato auto-import referenced tracks on launch; never mutate `database V2`. |
| **Rekordbox 6/7** | `rekordbox.xml` first, then `master.db` (SQLCipher) | **`rbox`** (`0.3.1`, `GPL-3.0-only`) | Phase 3: write `rekordbox.xml` via `rbox_xml` (pure Rust, zero DB risk). Phase 5: guarded `master.db` sync via `rbox-masterdb` with process check and automatic pre-write backups. |
| **Traktor Pro 3/4** | `collection.nml` | **Custom ~300 to 500 LOC streaming patcher** on `quick-xml` (no usable crates.io crate) | Stream all XML events unchanged and patch only targeted `<ENTRY>` and `<PLAYLISTS>` nodes; never use `serde` full struct roundtripping. |

---

## 1. Serato DJ (`crates/sync-serato`)

### Ecosystem evaluation

- **`triseratops`** (`Holzhaus/triseratops`, `v0.0.3`, released Nov 2023, `MPL-2.0`): the only real Serato crate published on crates.io. Crucially, its `library` module (`.crate` subcrates and `database V2`) is **read-only**, explicitly marked unstable, tested against only a single 4-track fixture, and has no Smart Crate (`.scrate`) or Serato DJ 4.x `master.sqlite` support. Its embedded audio tag module (`Markers_`, `Markers2`, `BeatGrid`, `Autotags`, `Analysis`, `Overview`) supports read/write roundtripping across ID3, FLAC, and MP4 (paired with `id3` or `lofty`), but has 5 open real-world parsing bugs filed Sept 2026 (`#42` to `#46`: base64 `=`/`==` padding, newline-terminated `Markers2`, omitted trailing `BeatGrid` footer bytes, and 0-marker beatgrids) and no crates.io release since Nov 2023. Mixxx itself does not use `triseratops` in production.
- **`seratodj`** (`v0.0.0`, Jan 2021): obsolete placeholder before rename to `triseratops`.
- **Unpublished GitHub references:** `chrisle/serato-connect-rs` (read-only, useful reference for Serato DJ 4.x `master.sqlite` schemas); `CurePortal/Cueit` (writes Serato `GEOB` cue frames directly via `id3` without `triseratops`).

### Chosen strategy

- **Roll a custom zero-dependency `.crate` binary reader/writer.** The `.crate` format is a minimal stream of big-endian TLV records (`[4-byte ASCII tag][u32 BE length][payload]`):
  1. `vrsn`: UTF-16BE string `"1.0/Serato ScratchLive Crate"` (no BOM).
  2. One `otrk` container record per track, wrapping a `ptrk` record containing the **volume-relative** track path encoded as UTF-16BE (for example `Users/name/Music/Track.flac` on the macOS root volume, or `Music/Track.flac` on an external drive, omitting the leading `/` or `/Volumes/<DriveName>/`).
  3. Optional UI column/sort records (`ovct`/`tvcn`/`tvcw`, `osrt`/`brev`), preserved when reading/updating existing crates.
- **Subcrate nesting** is encoded directly in the filename inside `_Serato_/Subcrates/` using `%%` as the separator (for example `Dubs%%Deep.crate`).
- **Avoid writing `database V2`.** Serato reads `.crate` files and imports referenced tracks automatically on launch. Writing `database V2` risks schema drift across undocumented fields and is legacy as Serato DJ 4.x moves its primary store to plaintext SQLite (`~/Library/Application Support/Serato/Library/master.sqlite` on macOS).
- **Backup before overwrite:** snapshot any existing `.crate` files managed by Overtone before writing via temporary file + atomic rename.
- **Deferred:** if Overtone later needs to edit Serato cues/loops/beatgrids inside audio files, either fork `triseratops` (patching issues `#42` to `#46`) or write targeted `GEOB` encoders over `lofty`.

### Format notes from real crates (November 26, 2026)

Checked against 140 real crates (10,228 tracks) from Serato DJ 4. These correct and extend the notes above.

- **`osrt` and `ovct` are always written.** Serato puts a `vrsn` record, then one `osrt` (sort column `tvcn` and reverse flag `brev`), then 7 to 10 `ovct` column records (`tvcn` name, `tvcw` width as a decimal string), then the `otrk` tracks. 138 of 140 sampled crates have an `osrt`; new crates should include one. `CrateFile::new()` writes the eight standard columns with width `0` (automatic), as 105 of 140 crates do.
- **`ptrk` is the on-disk Unicode form.** Serato stores the file name exactly as the file system returns it. 7 sampled paths are NFD-only and 101 are NFC-only. Overtone never normalizes a `ptrk` it writes, and compares paths with NFC-insensitive equality.
- **Files tile exactly.** No magic number, padding, or checksum, so parsing is strict: any length that overruns its container is an error, and unknown tags are preserved so parse then serialize returns the original bytes (verified for all 140 crates and `neworder.pref`).
- **One `_Serato_` folder per volume.** Boot volume files belong to `~/Music/_Serato_` with `ptrk` equal to the absolute path minus the leading `/`. Files under `/Volumes/<Name>` belong to that volume's `_Serato_` with `ptrk` relative to the volume root. The external volume layout is unverified by bytes.
- **Owned namespace.** Overtone manages crates only under one top-level crate (default `Overtone`), such as `Overtone%%Dubs%%Deep.crate`, and writes every parent in the chain. It never touches crates outside that namespace. Existing files are updated by replacing only the `otrk` list, so `osrt`, `ovct`, and unknown records survive.
- **Serato DJ 4 context.** Serato 4 keeps its authoritative library in `Library/master.sqlite` and treats `.crate` files and `neworder.pref` as an exported mirror. It records each legacy file's size and MD5 in `last_seen_dbv2_library` (in `root.sqlite`) and appears to re-import files whose hash differs when it starts. Overtone does not touch either SQLite file. Whether Serato 4 keeps externally written crates, or re-exports over them, still needs a manual test in Serato DJ 4 with a throwaway library.
- **Serato must be closed.** `apply` refuses to write while a process runs from a `Serato DJ*.app` bundle (ignoring `seratodj-monitor` helpers).
- **`neworder.pref`** is UTF-16BE with `\n` line breaks and lists crates depth-first, parents before children. New crates go after their parent's subtree. Overtone does not create the file when it is missing.

---

## 2. Rekordbox (`crates/sync-rekordbox`)

### Ecosystem evaluation

- **`rbox`** (`dylanljones/rbox`, `v0.3.1`, Oct 2026, `GPL-3.0-only`): built by the author of `pyrekordbox`, this is the only comprehensive read/write Rust library covering the full Rekordbox stack via modular feature flags:
  - `xml` (`rbox_xml`): pure-Rust read/write for `rekordbox.xml` (tracks, nested playlists, cue points, beatgrids) using `quick-xml` + `serde`.
  - `master-db` (`rbox-masterdb`): Diesel ORM over SQLCipher for Rekordbox v6 and v7 local `master.db`.
  - `one-library` (`rbox-onelib`) and `device-library` (`rbox-devlib`): read/write for modern OneLibrary / Device Library Plus (`exportLibrary.db` on CDJ-3000X, XDJ-AZ, OPUS-QUAD, OMNIS-DUO) and legacy `export.pdb` / `exportExt.pdb`.
  - `anlz` and `setting`: read/write for `.DAT`/`.EXT`/`.2EX` analysis files and `*SETTING.DAT`.
  - **Caveats:** early `0.3.x` API churn; `GPL-3.0-only` license; `master-db` and `one-library` require SQLCipher + OpenSSL on macOS (macOS system SQLite lacks SQLCipher, so builds require either `rbox`'s `bundled-vendored-openssl` feature or Homebrew `sqlcipher` + `OPENSSL_DIR`).
- **`rekordcrate`** (`Holzhaus/rekordcrate`, `v0.3.0`, Oct 2026, `MPL-2.0`): mature, pure-Rust parser used in the Mixxx ecosystem, but **read-only** and limited to USB exports (`export.pdb`, `USBANLZ`, `xml`, settings) with no `master.db` or OneLibrary support.
- **`rekordbox-pdb`** (`fragmede/rekordbox-pdb-rs`, `v0.1.0`): zero-dependency surgical in-place editor for legacy `export.pdb` only; no `master.db`, XML, OneLibrary, or `ANLZ`.

### Chosen strategy

- **Phase 3 (Safe interchange first):** use `rbox` with only the `xml` feature (`rbox_xml`) to generate `rekordbox.xml` containing tracks, metadata, and nested playlist trees. Zero SQLCipher build friction and zero risk of corrupting Rekordbox's live database.
- **Phase 5 (Seamless native sync):** enable `rbox`'s `master-db` feature behind strict, non-bypassable safety invariants:
  1. Refuse to open `master.db` for writing if a `rekordbox` process is running.
  2. Automatically copy `master.db` (plus `-wal` and `-shm` files) into a timestamped backup directory under Overtone's app data folder before every write session.
  3. Keep `rekordbox.xml` export available as a read-only dry-run or safe fallback.
- **USB exports deferred:** let Rekordbox's own Sync Manager export to CDJ USBs from the updated `master.db` until direct USB export (which requires generating or preserving `ANLZ` waveforms and beatgrids) is explicitly needed.

---

## 3. Traktor Pro (`crates/sync-traktor`)

### Ecosystem evaluation

- **crates.io is empty for Traktor:** `traktor` (`v0.1.0`, May 2017) is an abandoned 3 KB stub depending on `xml-rs ^0.4`, predating Traktor Pro 3 and 4 by years.
- **GitHub reference apps (internal workspace crates, not published):**
  - **`adamtdm/ReTraktor`** (`crates/retraktor-formats/src/nml.rs`, Apache-2.0, active Nov 2026): clean schema reference for NML v19/v20 (Traktor Pro 3 and 4), including `/:` POSIX path encoding, `<CUE_V2>`, TP4 flexible beatgrids (`<GRID BPM="..."/>` inside `<CUE_V2 TYPE="4">`), `<STEMS>`, and multiline attributes such as `INFO/@KEY_LYRICS`. Not lossless on write because `to_xml()` regenerates XML from structs and drops unmodeled elements.
  - **`suitux/Backspn`** (`crates/formats/src/nml.rs`, GPL-3.0, active Nov 2026): read/import focused.

### Chosen strategy

- **Build a custom `collection.nml` module (~300 to 500 LOC) using `quick-xml` (`0.42+`).**
- **Lossless streaming passthrough on write (required):** never deserialize `collection.nml` into Rust structs and re-serialize with `serde`, which silently strips undocumented TP3/TP4 attributes, multiline attribute values, `<INDEXING>`, `<SORTING_INFO>`, smartlists, and stem flags. Instead, stream every XML event from `quick-xml::Reader` to `quick-xml::Writer` unchanged, intercepting and patching only the specific `<ENTRY>` or `<PLAYLISTS>` nodes Overtone owns.
- **Path encoding:** map POSIX paths to Traktor's `<LOCATION VOLUME="..." DIR="/:Users/:name/:Music/:" FILE="..."/>` and playlist `<PRIMARYKEY TYPE="TRACK" KEY="VOLUME+DIR+FILE"/>`.
- **Audio tags:** Traktor stores proprietary cues/grids/stripes in an undocumented binary `TRAKTOR4` payload inside ID3 `PRIV` frames (and `NITR` RIFF chunks in WAV). No Rust crate parses it; use **`lofty`** for standard tags so opaque `TRAKTOR4` frames are preserved untouched.
- **Safety guardrails:** refuse to write while Traktor is running (Traktor overwrites `collection.nml` on exit), save a timestamped backup first, and write via temp file + atomic rename.

## Rekordbox XML: written with quick-xml, not rbox

Decided November 27, 2026, after a spike of `rbox` 0.3.1 with only its `xml` feature. It builds without SQLCipher, but it still pulls in about 85 crates (its `rbox-core` dependency brings `image`, `rayon`, and `sysinfo`) and its serializer has two output problems: every unset field is written as an empty attribute (`Size=""`, `BitRate=""`), and it prepends `file://localhost/` to the absolute path, producing `file://localhost//Users/...`. Rekordbox writes `file://localhost/Users/...`.

`overtone-sync-rekordbox` writes the file itself with the `quick-xml` writer: `DJ_PLAYLISTS Version="1.0.0"`, a `PRODUCT` element, a `COLLECTION` of `TRACK` elements carrying only the attributes Overtone knows, and a `PLAYLISTS` tree of folder (`Type="0"`) and playlist (`Type="1"`, `KeyType="0"`) nodes under `ROOT`. `Location` is `file://localhost` plus the absolute path, percent-encoded except unreserved characters and `/`, taken byte for byte from the path on disk without Unicode normalization. No `TEMPO` or `POSITION_MARK` elements are written, so the export carries no cue or beatgrid data. `rbox` stays the plan for `master.db` in Phase 5. Sources: Pioneer's rekordbox XML format list and the [pyrekordbox XML notes](https://pyrekordbox.readthedocs.io/en/latest/formats/xml.html).

## Traktor: splice the Overtone playlist folder into collection.nml

Decided November 27, 2026, after a survey of the NML format and a `quick-xml` 0.42 spike. The only crate on crates.io (`traktor` 0.1.0, 2017) is abandoned. A `Reader` to `Writer` event passthrough reproduced every sample NML byte for byte (entities such as `&#xA;` in `KEY_LYRICS`, `&quot;` in TP4 `STEMS` JSON, attribute order and quoting, CRLF), except that it drops a UTF-8 byte order mark.

`overtone-sync-traktor` therefore never rewrites the file as a whole. It scans the file once to collect every `COLLECTION` entry's `VOLUME` + `DIR` + `FILE` key and the byte span of the `NODE TYPE="FOLDER" NAME="Overtone"` directly under `$ROOT`, then copies the original bytes around that span unchanged and writes a newly generated subtree in its place (adjusting the `$ROOT` `SUBNODES COUNT` when the folder is added or removed). Everything outside the Overtone folder stays byte-identical, which the writer verifies before replacing the file.

- **Keys:** `VOLUME` + `DIR` + `FILE`, where `DIR` turns each `/` into `/:` and a literal `:` into `//`, for example `Macintosh HD/:Users/:me/:Music/:a.mp3`. Library paths are matched to collection entries ignoring Unicode normalization, and the collection's own spelling is written. Tracks missing from the Traktor collection are skipped and reported, never added.
- **Layout:** Traktor's own style (no indentation, LF, explicit end tags); playlist UUIDs are derived from the vibe path so reruns are stable.
- **Safety:** refuses while Traktor Pro 3 or 4 is running (it rewrites `collection.nml` on quit), backs up the file to Overtone's backups folder first, and writes through a temporary file and atomic rename.

Sources: [alchimestre/traktor-nml](https://github.com/alchimestre/traktor-nml), [wolkenarchitekt/traktor-nml-utils](https://github.com/wolkenarchitekt/traktor-nml-utils) fixtures, [ReTraktor](https://github.com/adamtdm/ReTraktor), and Native Instruments' [collection backup article](https://support.native-instruments.com/support/solutions/articles/69000879494-how-to-restore-the-traktor-track-collection-from-a-backup). Test fixtures are synthetic, modeled on those files, because real collections contain other people's track metadata.

## Rekordbox master.db: own My Tag writer over rusqlite with SQLCipher, not rbox

Decided November 27, 2026, after a spike of `rbox` 0.3.1 with `master-db` on copies of a real Rekordbox 7.0.8 `master.db` (`db_version` 6000). `rbox` reads and writes it, but it pins `libsqlite3-sys = "=0.38"`, and Overtone's `rusqlite` 0.43 needs `^0.40`. Cargo allows only one crate linking `sqlite3` per build, so the two cannot share a binary. The workarounds were a separately built helper process, or downgrading the whole workspace to `rusqlite` 0.40 and keeping it in lockstep with `rbox` from then on.

Overtone only writes My Tags to `master.db` (playlists go through `rekordbox.xml`, see [vibe tags](vibe-tags.md)), and that is a small, well-defined write: rows in `djmdMyTag` and `djmdSongMyTag` plus the `agentRegistry` `localUpdateCount` bookkeeping. So the workspace keeps `rusqlite` 0.43 and switches its bundled SQLite to the SQLCipher build (`bundled-sqlcipher-vendored-openssl`, SQLCipher 4.20 on SQLite 3.54.1, compiled from source with no Homebrew dependency). An unkeyed database behaves exactly like plain SQLite, so Overtone's own library is unaffected: library and core tests pass unchanged, and scanning the 1,151-file `local/` testing ground took 9.9 s cold, the same as before within run-to-run noise. `overtone-sync-rekordbox` opens `master.db` with Rekordbox's published SQLCipher passphrase (the same for every installation, as documented by pyrekordbox and used by `rbox`) and ports `rbox`'s My Tag bookkeeping: random integer IDs, UUIDs, timestamps, and per-row `rb_local_usn` stamped from an incremented `localUpdateCount`.

Spike findings that shape the design:

- My Tags are a two-level tree: categories (`Attribute = 1`, `ParentID = "root"`) holding tags (`Attribute = 0`). Overtone owns one category named `Overtone` and writes one tag per vibe named by its full path, such as `Dubs/Deep`.
- `djmdContent.FolderPath` holds each track's absolute path. Tracks Rekordbox doesn't know are skipped and reported, never added: added rows would have blank metadata and no analysis.
- The database runs in WAL mode, and opening it creates `-wal` and `-shm` files, so Overtone never opens the live file. Planning and applying both work on a copy, and the edited copy replaces the original only after `PRAGMA integrity_check` and a row check pass, with Rekordbox closed and the database, its WAL files, and `masterPlaylists6.xml` backed up first.
- `agentRegistry` also holds Rekordbox account credentials. Overtone reads only the `localUpdateCount` row and never logs the table.
