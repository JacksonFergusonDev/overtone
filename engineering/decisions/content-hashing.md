# Audio-only content hashing

Decided November 26, 2026. Implemented in `crates/audio/src/hash.rs` and its submodules.

## Decision

Overtone identifies a track's audio with `blake3-payload-v1:<64 hex>`: the BLAKE3 hash of the bytes that hold the encoded audio stream, found by walking the container, and nothing else. Tag edits, artwork, padding, and chunk reordering never change it; changing a single audio byte always does. Because tag writes rewrite file bytes ([Vibe tags](vibe-tags.md#consequence-for-duplicate-detection)), whole-file hashes would make every retagged track look new.

The scheme name is part of every stored value, so a future scheme can be introduced without comparing unlike digests.

This is exact duplicate detection for the same encoding. A FLAC and a WAV of the same master, or two MP3 encodes, hash differently by design. Perceptual matching is out of scope.

## Payload per format

Format comes from magic bytes, not the extension. Files that match no container, or whose structure doesn't add up, return an error. The scanner then stores the track with no hash and reports it, rather than falling back to a whole-file hash that would silently break the "tags never matter" promise.

| Container | Payload | Skipped |
|---|---|---|
| FLAC (`fLaC`) | Every byte after the metadata block flagged as last, which must start with a frame sync (`0xFFF8`/`0xFFF9`) | STREAMINFO, VORBIS_COMMENT, PICTURE, PADDING, and other blocks; ID3v2 prepended and ID3v1/APEv2 appended by MP3-minded taggers |
| MP3 and ADTS AAC | From the first frame header that chains into two more consistent frames, to the start of trailing tags | Any number of leading ID3v2 tags, padding or junk after them (up to 256 KiB), trailing ID3v1, APEv2 (with or without header), Lyrics3v2, and ID3v2.4 with footer, in any order |
| MP4 / M4A (`ftyp`) | Bodies of all top-level `mdat` atoms, in file order | `moov` (including `udta/meta/ilst` tags), `free`, and everything else. 64-bit sizes and size 0 (to end of file) are handled |
| WAV, RF64, BW64 | Body of the `data` chunk (size from `ds64` when the 32-bit size is `0xFFFFFFFF`) | `LIST`/`INFO`, `id3 `, `bext`, and all other chunks. The RIFF header size is ignored because writers often get it wrong |
| AIFF, AIFF-C | `SSND` sample data after its offset and block-size fields | `ID3 `, `NAME`, `ANNO`, and all other chunks |

Choices worth recording:

- **MP3 start is found by frame chaining, not the ID3v2 size alone.** Taggers leave zero padding or stale bytes past the declared tag size. Scanning for three consecutive frames makes the start a property of the audio. It also recognizes MP3s with leading junk and no tag.
- **The Xing/Info/LAME frame is payload.** It is a real (silent) MPEG frame that tag editors never touch. Recognizing it would mean parsing side information for no gain in stability. Tools that rewrite it, such as VBR header fixers or MP3Gain, change the stream, so a new hash is the honest result.
- **Lyrics3 v1 is not stripped.** It has no length field and has been obsolete since 2000.
- **Truncation is an error where the container can tell.** WAV, AIFF, and MP4 sizes that run past the end of the file fail. FLAC and MP3 frames carry no total length, so a truncated stream hashes as what is there.
- **The hash is plain BLAKE3 over the concatenated ranges**, not keyed, so the value can be checked with `b3sum` on an extracted payload.

## I/O strategy and benchmark

BLAKE3 stays. It is a frozen cryptographic spec with stable output across versions and platforms, which matters more for a value stored in every library than raw speed. The `blake3` crate (1.8.8) uses NEON on Apple Silicon by default.

Payload bytes are streamed with positional reads into a 64 KiB buffer. Memory mapping can't serve here: `blake3`'s mmap helpers hash whole files only, and mapping a sub-range ourselves needs `unsafe`, which the workspace forbids. Parallelism comes from the scanner hashing files concurrently with rayon, not from splitting one file.

Measured with `cargo run --release -p overtone-audio --example hash_bench -- <dir>` on an Apple M3 (8 cores, 16 GB RAM, internal SSD) against a 1,151-file real-world set (flac, mp3, m4a, wav, aiff; 24.2 GB). Payload modes hash 23.9 GB because tags and artwork are skipped. Other processes were running, so treat differences under about 10% as noise.

Cold-ish cache: one pass per mode over the full set, which is larger than RAM, so each pass evicts the previous one's pages.

| Mode | 1 thread, s | 1 thread, MB/s | 8 threads, s | 8 threads, MB/s |
|---|---|---|---|---|
| Find payload ranges only | 0.74 | - | 0.02 | - |
| BLAKE3, 64 KiB reads | 21.08 | 1,133 | 9.57 | 2,496 |
| BLAKE3, 1 MiB reads | 24.08 | 992 | 9.23 | 2,588 |
| BLAKE3, `update_reader` | 20.85 | 1,146 | 9.90 | 2,414 |
| xxh3-128, 1 MiB reads | 13.75 | 1,738 | 8.80 | 2,716 |
| BLAKE3 `update_mmap`, whole file | 62.19 | 389 | 19.13 | 1,265 |
| BLAKE3 `update_mmap_rayon`, whole file | 31.17 | 776 | - | - |

Warm cache: the first 172 files (3.0 GB), third of three passes.

| Mode | 1 thread, s | 1 thread, MB/s | 8 threads, s | 8 threads, MB/s |
|---|---|---|---|---|
| BLAKE3, 64 KiB reads | 1.58 | 1,855 | 0.29 | 10,039 |
| BLAKE3, 1 MiB reads | 1.60 | 1,834 | 0.29 | 10,059 |
| BLAKE3, `update_reader` | 1.61 | 1,824 | 0.28 | 10,389 |
| xxh3-128, 1 MiB reads | 0.29 | 10,249 | 0.09 | 32,647 |
| BLAKE3 `update_mmap`, whole file | 1.57 | 1,901 | 0.28 | 10,735 |
| BLAKE3 `update_mmap_rayon`, whole file | 0.38 | 7,952 | - | - |

What the numbers say:

- **From disk, hashing across files is I/O-bound.** With 8 threads, BLAKE3 reaches 2.5 GB/s and xxh3 2.7 GB/s, a 5 to 10% gap. A first scan of this set takes about 9 seconds either way.
- **xxh3 only wins clearly when data is already cached** (3.3x warm). Hashing runs only for new or changed files, which are rarely cached, so that advantage seldom applies. It does not outweigh BLAKE3's frozen, collision-resistant output.
- **mmap is much slower cold** (page-fault driven reads) and no faster warm.
- **Buffer size barely matters.** 64 KiB matches 1 MiB warm and was faster single-threaded cold, so the scanner uses 64 KiB.
- **Finding the ranges costs under 1 ms per file.**

## Real-world check

`cargo run --release -p overtone-audio --example hash_survey -- <dir>` hashed every audio file in the same set: 366 flac, 652 mp3 (including 2 `.Mp3`), 69 m4a (all AAC), 61 wav, and 3 aiff, with no failures. It found two pairs of identical audio whose files differ in size by 4 bytes and 764 KB respectively, which are the same release with different tags. The smallest payload share was 48% of the file, an MP3 with a 9.5 MB ID3v2 tag holding artwork.

Scanning the set into a fresh library with `cargo run --release -p overtone-core --example scan_folder -- <dir> <library.sqlite>` took 9.0 s, and an unchanged rescan took 6 ms.
