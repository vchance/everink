# Spike slice 5: storage

Date: 2026-11-12. Code: `spike/store-trial`, and three small additions to `crates/everink-sync`
(`Room::from_objects`, `Room::next_ordinal`, `Room::rng_state`). This answers question 9 of [`README.md`](README.md): **how fast is
storage?** Section 03 of the design has one SQLite file per room and marks every figure for it as unmeasured.

Every timing is from one machine: an Apple M4 (macOS, its internal SSD, APFS), release build, Rust 1.97.1, SQLite
3.54.1 as `rusqlite` 0.42.0 builds it. A Linux server's disk is not measured.

## Answer

- **A durable commit costs about 4 ms on this machine, and it does not matter how many changes are in it.** One
  change or a hundred in a transaction: 3.8 to 4.4 ms at the median, 4 to 8.5 ms at the 95th percentile. That is
  inside the design's estimate of 1 to 5 ms and about 250 commits a second, so group commit, as designed, is what
  carries a busy room.
- **The design's setting alone is not durable on macOS.** With `synchronous=FULL` a commit returns in 0.04 ms,
  because there `fsync` does not make the drive write its cache out. `fullfsync=ON` is what makes it wait, and
  the 4 ms above is with it. The store must set it.
- **A room opens in 28 ms at 10,000 strokes, 0.29 s at 100,000 and 4 to 6 s at 1,000,000**, from a file the
  operating system already holds in memory. The design estimated 5 s to decode a million objects and 1 to 2 s
  more for the spatial index, which is not built here. Reading the chunks out of SQLite is the largest part.
- **The commit does not slow as the file grows**: 4 ms durable, and 0.04 ms of SQLite's own, on a 10 MB file and
  on a 1 GB one alike.
- **The file is 1,008 bytes a stroke.** LZ4 saves 1% on these strokes, which are random walks and pack worse
  than a hand's; the gain on real strokes is not known.
- **The log cannot be replayed exactly as designed.** The room draws random numbers when it makes an order key,
  so a room opened from a snapshot gives the changes after it other keys than they got the first time, unless
  the state of the random numbers is saved with the snapshot. The trial saves it; the design must.

## What was built

`spike/store-trial` is a package of its own, outside the workspace, with its own lock file. It builds the two
workspace crates in, and holds:

- **The store** (`src/lib.rs`): one SQLite file in WAL mode with the design's `log`, `chunk`, `snap` and `meta`
  tables. A log row is a change as the client sent it, with who sent it and a CRC32C. A snapshot is the room's
  objects in chunks of 256 by ordinal, each chunk compressed with LZ4 and stored under its BLAKE3 hash, and a
  manifest of the hashes with a checksum. A second snapshot writes only the chunks that changed. Opening takes
  the newest snapshot, checks every hash, decodes and checks every object, builds the room, and replays the log
  rows after the snapshot through the room's own `submit`, checking that the numbers follow on.
- **Tests** (`tests/store.rs`, 5): the room comes back byte for byte from the log alone, from a snapshot, and
  from a snapshot with a log after it, with strokes put on top, at the bottom and in between, and moves; a
  second snapshot writes at most 4 chunks after 300 strokes and holds the same as a whole one; the log can be
  trimmed to the second-newest snapshot; a changed chunk, a missing chunk, a changed manifest, a missing log row
  and a changed log row are each refused; and replay without the saved random state gives a different room.
- **The timing run** (`src/main.rs`). It fails if the room it opens is not the room it wrote.

`crates/everink-sync` gains `Room::from_objects` (a room from its objects in any order, with the checks
`decode_all` makes; chunks are by ordinal and `decode_all` wants one buffer sorted by id), `Room::next_ordinal`
and `Room::rng_state`, with a test.

Not built: named versions and their retention, the blob, picture and audit tables, the lease, the copy to a
second place, the store thread and the hold on acknowledgements, the daily sweep of chunks.

## Measurements

Command: `cargo run --release --locked --manifest-path spike/store-trial/Cargo.toml`. Two runs with the machine
otherwise idle; a range is over the two. A third run, after review, added the durable commit on the large
files and agreed with the two on the rest. A stroke is 200 points and one change (about 1 KB).

### One commit of the log

| Setting | Changes in the commit | Median | 95th | 99th | Most | A change |
|---|---|---|---|---|---|---|
| `synchronous=FULL`, `fullfsync=ON` | 1 | 3.8 to 4.0 ms | 4.1 to 5.1 ms | 5.9 to 6.1 ms | 6.4 ms | 3.9 ms |
| the same | 10 | 3.8 to 3.9 ms | 4.0 to 4.6 ms | 6.0 to 7.0 ms | 15 ms | 0.39 ms |
| the same | 100 | 4.3 to 4.4 ms | 8.2 to 8.5 ms | 20 to 21 ms | 24 ms | 0.044 ms |
| `synchronous=FULL` (the design's), which on macOS does not reach the disk | 1 | 0.041 to 0.044 ms | 0.08 to 0.11 ms | 0.10 to 0.13 ms | 0.32 ms | 0.04 ms |
| the same | 10 | 0.056 to 0.066 ms | 0.08 to 0.10 ms | 0.09 to 0.17 ms | 3.4 ms | 0.006 ms |
| the same | 100 | 0.23 ms | 0.31 to 0.40 ms | 4.7 to 5.6 ms | 8.8 ms | 0.002 ms |
| `synchronous=NORMAL` (no sync at a commit; one run) | 1 | 0.012 ms | 0.029 ms | 0.032 ms | 2.2 ms | 0.012 ms |

200 commits for the first setting, 400 for the second, 2,000 for the third, after 20 not counted.

- The first rows are the cost of the disk. The others are the cost of SQLite: 12 to 44 microseconds a commit,
  and 2 microseconds a change inside a larger transaction.
- With a durable commit of 4 ms, a room that commits each change alone manages about 250 changes a second; one
  that commits what arrived during the last commit (the design's group commit) writes 100 changes in the same
  4 ms. The design's busy room is 15 changes a second.
- The occasional commit of 15 to 24 ms is most likely the write-ahead log being moved into the main file (a
  checkpoint), which SQLite does on the committing thread by default every 1,000 pages: about every 40 commits
  of 100 changes. That is inferred from the sizes, not measured.

On a file that already holds a room, one change a commit:

| Strokes in the file | `synchronous=FULL` (300 commits) | with `fullfsync=ON` (100 commits; one run): median | 95th |
|---|---|---|---|
| 10,000 | 0.036 to 0.040 ms | 3.97 ms | 4.4 ms |
| 100,000 | 0.038 to 0.044 ms | 3.96 ms | 4.2 ms |
| 1,000,000 | 0.032 to 0.033 ms | 3.79 ms | 6.0 ms |

The file's size does not show in either.

### Snapshots

| Strokes | Chunks | Stored | A stroke | After LZ4 | Building the chunks | Writing them (one transaction) | The file |
|---|---|---|---|---|---|---|---|
| 10,000 | 40 | 10.1 MB | 1,006 bytes | 99% | 10 to 11 ms | 33 to 34 ms | 10.1 MB |
| 100,000 | 391 | 100.7 MB | 1,007 bytes | 99% | 110 ms | 405 to 466 ms | 101.1 MB |
| 1,000,000 | 3,907 | 1,007.9 MB | 1,008 bytes | 99% | 1.3 to 1.5 s | 5.4 to 7.2 s | 1,010.6 MB |

That is a whole snapshot, which a room writes once. The design's example is a snapshot after 600 new strokes:

| Strokes in the room | Chunks written | Bytes | Manifest | Building | Writing |
|---|---|---|---|---|---|
| 10,000 | 3 of 42 | 620 KB | 1.5 KB | 0.8 ms | 1.5 to 1.7 ms |
| 100,000 | 3 of 393 | 767 KB | 14.2 KB | 3.1 to 3.5 ms | 2.4 ms |
| 1,000,000 | 3 of 3,909 | 669 KB | 140.8 KB | 63 to 88 ms | 2.6 to 10.9 ms |

- "About 3 chunks plus a manifest of about 4,000 hashes (128 KB)", says the design for a million objects: 3
  chunks and 141 KB measured (the trial's manifest has the chunk's number beside each hash).
- Building takes 63 to 88 ms at a million objects because the trial sorts every object by ordinal to find the
  chunks; the design's shards would not. It is not the cost of the three chunks.
- The transactions in both tables are with `synchronous=FULL` and without `fullfsync`; add the 4 ms of a
  durable commit to each.

### Opening a room

The snapshot, the 600 strokes' snapshot, and a log tail of 1,000 changes after it. Median of 5 opens (3 at a
million), each on a new connection. The file is in the operating system's cache: it was just written.

| Strokes | Whole | Reading the chunks from SQLite | Checking hashes, decompressing | Decoding the objects | Building the room | Replaying 1,000 log rows |
|---|---|---|---|---|---|---|
| 10,600 | 28 ms | 7 ms | 5 ms | 9 ms | 4 ms | 1.9 ms |
| 100,600 | 291 to 293 ms | 108 to 112 ms | 46 to 48 ms | 82 to 85 ms | 46 to 48 ms | 1.9 to 2.0 ms |
| 1,000,600 | 4.0 to 6.0 s (4.2 s in the third run) | 1.6 to 2.9 s | 0.53 s | 0.9 to 1.1 s | 0.9 s | 2.4 to 7.7 ms |

- Up to 100,000 strokes the time is 2.6 to 2.9 microseconds a stroke. At a million it is 4 to 6, and the part
  that grew and varied is reading from SQLite (1 GB of chunks, each about 258 KB, which SQLite keeps as a chain
  of 4 KB pages). The machine has 16 GB of memory and the run holds the file's pages and the room at once; how
  much of the 1.6 to 2.9 s is that is not known.
- Decoding and building together are 2.0 s at a million. Slice 2a measured 2.6 s for the same work from one
  buffer; the two agree.
- Replaying the log costs 2 to 8 microseconds a change, so a tail of 16 MiB (the design's limit before a
  snapshot, about 16,000 strokes) adds about 0.1 s.
- A file that is not in the operating system's cache must also be read from the disk: 1 GB at this drive's speed
  is under a second, on a network volume much more. Not measured.

## Findings

1. **SQLite is not the cost; the disk is.** A commit is tens of microseconds of SQLite and 4 ms of waiting for
   the drive. The design's 1 to 5 ms holds, and so does its reasoning that group commit keeps latency flat.
2. **`synchronous=FULL` must come with `fullfsync=ON` on Apple systems**, or "durable when it returns" is not
   true there. This matters for a native client that keeps a local store, and for anyone running the server on a
   Mac; a Linux server is not affected.
3. **Snapshots must carry the room's random state**, or the log is not replayed exactly (a test shows both). The
   alternative is to log what the room did (the effects, with the keys it chose) and not what the client asked.
4. **Opening a million-object room takes several seconds**, as the design estimated, and most of it is not
   decoding. Reading large chunks back out of SQLite is the first thing to look at: a larger page size, or
   chunks beside the file, were not tried.
5. **Compression earned nothing on this data.** Whether LZ4 is worth its place depends on real strokes.
6. **A checkpoint lands on a commit now and then** (15 to 24 ms). If that matters, the store thread can run
   checkpoints itself between commits.

## What this does not show

- **A server's disk.** One laptop SSD. A Linux machine with an NVMe drive, a cloud volume, and a disk with power
  loss protection will each differ, mostly in the 4 ms.
- **Power loss.** Nothing here cuts the power or kills the process mid-commit; the design's crash tests
  (10,000 runs of fault injection) are not built.
- **A cold file**, and memory use while opening.
- **Real strokes**, deep positions (an object 1,000 levels down is about 2,500 bytes larger), and objects other
  than strokes.
- **Concurrent readers**: a snapshot job or a version opened read-only while the room writes.
- **redb or any other engine.** Only the design's default was tried.
- **The rest of the store**: versions and retention, the sweep of old chunks, the copy to a second place.

## Where the code departs from the design

- The manifest holds the chunk's number beside each hash, and the room's random state.
- `chunk` has no `blobs` column and `snap` no `dedupe`, `kind`, `name`, `by` or `ord_next`; the table of clients'
  newest changes is not saved, so a client that resends a change after a restart is not recognised.
- A chunk is hashed after compression, so it can be checked before it is decompressed, and a chunk that says
  it holds more than 256 of the largest objects is refused before memory is taken for it. The hash and the
  manifest's checksum guard against damage, not against a file made to deceive: neither is keyed.
- A whole snapshot is found by sorting the objects, not from the design's 4,096 shards.

## What the design should change

1. Section 03, storage: add `fullfsync=ON` beside `synchronous=FULL`, and say why.
2. Section 03, snapshots and recovery: the manifest carries the room's random state (or the log holds effects).
3. Section 03, capacity: commit 4 ms measured (Apple M4, one disk); a commit on SQLite's side 0.01 to 0.04 ms;
   open 0.29 s at 100,000 strokes and 4 to 6 s at 1,000,000 without the spatial index; 1,008 bytes a stroke.
4. Section 03, snapshots: LZ4's gain is unproven; and checkpoints should be scheduled by the store thread.

## Kept afterwards

`spike/store-trial` stays until the server's store is written; its tests are the start of that store's.
`Room::from_objects`, `Room::next_ordinal` and `Room::rng_state` stay in `crates/everink-sync`.
