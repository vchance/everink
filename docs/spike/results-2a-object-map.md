# Spike slice 2a: the object map

Date: 2026-11-12. Code: `crates/everink-sync`. Design: [`../design/02-document-model.md`](../design/02-document-model.md).

This is the first of three parts of slice 2. It answers question 7 of the [plan](README.md): does Everink's own
object map hold up? Questions 5 and 6 (the text libraries) are part 2b, and question 8 (cursors during a bulk
load) is part 2c.

## Answer

Yes, with one correction to the design.

- **It converges.** Copies that are given the same accepted changes end in the same bytes, whatever order the
  writes arrive in, with repeats, and when whole-object snapshots overlap the stream of changes.
- **It is small and fast enough.** 100,000 strokes take about 1.3 KB of memory each, which is the design's
  estimate, and open in 0.13 s. The server's work on one change is under a microsecond.
- **The design's rule for order keys does not work for the common case.** Putting each new object in front of
  the others reaches the 64-byte key limit after about 450 objects. The prototype counts instead, and 100,000
  objects in front need a 4-byte key. See [Finding 1](#finding-1-order-keys-must-count-at-the-ends).

## What was built

`everink-sync` has no dependency besides `everink-core`.

| Part | What it is |
|---|---|
| `Object` | The fixed part (id, ordinal, creator, `created`), six registers (alive, slot, transform, name, paint, clip) and the stamps of the registers written after creation. Two kinds: stroke and group |
| `Room` | The server's copy. `submit` checks a whole change and either refuses it and changes nothing, or gives it the next `Seq` and returns what it did |
| `Replica` | A client's copy, which may be partial. It merges what the server sends in any order, and lays the client's own unanswered changes over the confirmed state |
| `OrderKey` | Where an object sits among its siblings: `between`, and `after` and `before` for the ends |
| `PackedPoints` | A stroke's points: two zigzag varints and a pressure byte per point, decoded with whole numbers only |
| Canonical bytes | One encoding per value, for an object and for a whole state. Two copies are equal exactly when their bytes are |

## Measurements

Machine: Apple M4 (aarch64, macOS), native, release build, Rust 1.97.1. Command:
`cargo run --release --example bench_object_map <objects> <depth>`, one process per row so that the memory
figure starts from a process that holds nothing. Strokes have 200 points; the points are a random walk with
steps of up to 400 lattice units and a random pressure, so they pack worse than a real pen stroke would.

"Estimated" memory is the room's own count: the hash map's slots, what each object holds outside its struct,
and the children index. "Resident" is how much the process grew while the room was filled, as the operating
system reports it; it includes the allocator's rounding and the pages a growing hash map leaves behind.

| | 100,000, depth 0 | 100,000, depth 100 | 100,000, depth 1,000 | 1,000,000, depth 0 |
|---|---|---|---|---|
| Memory per object, estimated | 1,325 B | 1,590 B | 3,840 B | 1,513 B |
| Memory per object, resident | 1,840 B | 2,164 B | 4,582 B | 2,263 B |
| State as bytes, per object | 1,018 B | 1,270 B | 3,520 B | 1,019 B |
| ... of which not points | 82 B | 334 B | 2,584 B | 83 B |
| Open from bytes (median of 9) | 127 ms | 147 ms | 232 ms | 2,203 ms |
| ... spread (min to 95th percentile) | 124 to 146 ms | 140 to 160 ms | 218 to 264 ms | 1,985 to 2,409 ms |
| Server accepts a new stroke | 0.72 µs | 1.36 µs | 3.17 µs | 0.87 µs |
| Server accepts a move | 0.45 µs | 1.38 µs | 3.48 µs | 0.61 µs |
| Client copy merges a new stroke | 0.16 µs | 0.24 µs | 0.52 µs | 0.71 µs |
| Client copy merges a move | 0.16 µs | 0.26 µs | 0.55 µs | 0.31 µs |
| Client copy ignores a stale move | 0.06 µs | 0.09 µs | 0.06 µs | 0.26 µs |
| Longest order key | 4 B | 4 B | 4 B | 4 B |

The object struct is 256 bytes; a stroke's packed points are 936 bytes (4.68 per point). The per-change times are
one pass over all the objects, not repeated, so they show the size of the cost and not its spread. "Open" is
decoding and checking every object, rebuilding the children index, and checking the tree.

### Against the design's estimates

| Quantity | Design | Measured | |
|---|---|---|---|
| Stroke of 200 samples, packed | about 1 KB (section 02) | 936 B | matches |
| Fixed part of an object near the origin | about 110 B (section 02) | 82 B, body fields included | smaller: no creator link, no provenance yet |
| Server memory per object | 1.3 KB, or 2 KB if memory costs 50% more than the packed form (section 03) | 1.3 to 1.5 KB estimated, 1.8 to 2.3 KB resident | the range the design gave; the spatial index is not built yet and will add to it |
| Load of 1,000,000 objects | about 5 s to decode and 1 to 2 s to index, "unmeasured" (section 03) | 2.2 s to decode, check and build the children index | faster; reading from disk and the spatial index are not in it |
| Server work per change | about 20 µs (section 03) | 0.5 to 0.9 µs at depth 0 | the map is not the cost; the estimate also covers storage and sending, which are not built |

### Depth

A position 100 levels down adds about 250 bytes to an object and 1,000 levels down adds about 2,500, in memory
and in the state bytes alike. At 1,000 levels the positions are 73% of the state. This is the second measurement
that says positions should be stored and sent as differences from a nearby base
([slice 1](results-1-positions.md) found the same for speed). Per-change time also grows with depth, by a factor
of about four at 1,000 levels, and stays in microseconds.

## What the tests show

`cargo test -p everink-sync` runs 18 tests. The random tests drive a room with changes from several clients;
about three in five are refused (a change of several operations is refused when any one is bad), for six
different reasons, and a test asserts that this mix really occurs.

- **Order does not matter.** For 20 random histories of 1,500 changes, a copy is given the objects and then every
  write on its own in a random order, a third of them twice, and again newest first. Its bytes equal the room's.
- **A partial copy converges.** A copy that holds some objects is given the stream in order with a lag, old
  messages again, objects leaving, and whole objects arriving at random moments that overlap the stream. Every
  object it holds is byte for byte the room's.
- **The room agrees with a plain model.** A model written from the rule alone ("replay the accepted changes; the
  last write to a register is its value and its stamp") keeps a stamp for every register. After 8,000 changes
  the room has the same value and stamp for every register, and stores a stamp exactly for those written after
  creation.
- **A change is all or nothing, and is applied once.** A change whose last operation is bad leaves the room's
  bytes and `Seq` untouched. A change sent three times is applied once.
- **The tree stays a tree.** Moves made from an old view of the tree are refused when they would close a loop;
  after each of 20,000 random moves the tree is checked. Nothing moves into a deleted group; moving out is
  allowed. A group and its members can be made in one change.
- **The rules of the design hold:** delete and edit commute, and a restore shows the edit; a change made offline
  wins when it arrives (owner decision 5), except a conditional write, which is refused; a client sees its own
  change at once, a remote write to the same register does not show through, and a refused change falls back to
  the confirmed value.
- **Bytes are canonical and checked.** A state opens to the same bytes. Every cut-off of an object's bytes is
  refused. Of 12,000 single-bit changes, each is either refused or decodes to an object whose bytes are exactly
  the changed bytes. A state whose objects form a loop is refused when opened.

Two tests were checked to be able to fail: with the stamp comparison in `Object::write` broken, the two
convergence tests failed; with the cycle check removed, the three tree tests failed.

## Findings

### Finding 1: order keys must count at the ends

Section 02 gives one rule for a new key: read the two neighbours as digits in base 256, and at the first place
with room pick a random digit from the middle third. That is right between two neighbours. At an end there is no
second neighbour, and each call takes about half of the space that is left, so the key grows by a byte about
every seven calls.

Measured (`always_inserting_at_one_end_grows_the_key_slowly_and_then_fails_cleanly`): the key passes 24 bytes,
where the design has the server rewrite the whole sibling list, after 163 inserts at one end, and no key of 64
bytes or fewer is left after 444 (one seed; the test accepts 100 to 400 and 300 to 1,200). The first timing run failed this way at a few
hundred strokes. A new stroke goes in front of everything, so on a busy surface the server would be rewriting the
top-level list every few hundred strokes.

The prototype keeps the design's rule for "between" and adds `OrderKey::after` and `OrderKey::before` for "front"
and "back". A key that starts with `0xF8` to `0xFE` is a counter with one to seven bytes of whole number, a longer
number sorting above every shorter one; `after` adds one. `0x07` down to `0x01` are the same downwards. Both
kinds are ordinary keys: `between` works on them and next to them (tested from 50,000 random keys). Only the
server resolves "front" and "back", one change at a time, so two calls cannot pick the same key and no randomness
is needed. 100,000 objects in front, or behind, have keys of at most 4 bytes.

A client can still ask for "between the lowest key and nothing" over and over, which is the old worst case. The
rewrite at 24 bytes stays in the design for that; it is not in the prototype.

### Finding 2: an object comes back into a copy whole, never as an old creation

A copy drops a write to an object it does not hold. That is right for a partial copy, and it means the object
must arrive as it is now when it comes back into view. Section 03 already says so (an object entering the view
is sent whole). The first version of the partial-copy test replayed an old creation after the object had left,
and the copy lost the writes made in between. The test `an_object_must_come_back_into_a_copy_whole` pins this
down. For the protocol it means: a server message is never replayed to a connection across the moment an object
left that connection's view. Section 03's "no session resume" already rules it out; the reason is now written
down and tested.

### Smaller points

- **A change sent again gets the newest answer, not its own.** Section 03 keeps the highest change number per
  client and says a resend "is answered with its original pair". With only the highest kept, the room can give
  the pair of the newest change, which is what the prototype returns. Section 03 should say that, or keep more.
- **A refused change is not remembered.** Sent again, it is checked again. That is harmless here, and it means a
  change refused for a passing reason (a parent that was deleted and is then restored) can succeed on a resend.
- **"No two operations write the same register" includes creation.** The prototype refuses a change that creates
  an object and then writes one of its registers, because both would carry the same `Seq`. Section 02 does not
  say; it should.

## What the prototype leaves out

It is one stroke type and a group. Not built: partial erase (cuts), text, shapes and media, delete-by-creator,
undo and its conditional `partial_ok` changes (single conditional writes are built), the rewrite of long keys,
the limits on tree depth and group size, a child's transform relative to its group, the creator's link and
provenance, unknown fields kept for newer clients, purging of deleted objects and the purge horizon, roles,
rates and size caps, storage, the spatial index and the view, and the bytes of a change on the wire (part 2c).
The walk that looks for a loop is not bounded; it relies on the tree being a tree, which opening a state checks.

## Where the code departs from the design

1. **"Front" and "back" count** (Finding 1).
2. **A key of up to 14 bytes is held inline** (the design says 12); the key is 16 bytes either way.
3. **`Create` carries no order key.** The client says front, back or between two keys, and the server picks the
   key in all three cases. The design has the server pick only for front and back.
4. **The transform's linear part is four 16.16 fixed-point numbers.** Section 02 leaves its form to section 06.

## What the design should change

1. Section 02, "Order": add counting keys for the ends, and say the 24-byte rewrite is for the remaining worst
   case only.
2. Section 02, "Changes": say that a new object's registers count as written by the change that creates it.
3. Section 03, "Duplicates": say what a resend of an older change is answered with.
4. Sections 01 and 03: positions as differences from a base, for memory and state size as well as speed.
5. Section 03, capacity table: replace the per-object memory, load and apply estimates with these figures, and
   keep the storage and sending parts as estimates until slices 2c and 5.

These are recorded here and made in a separate change, with the corrections from slice 1.

## Kept afterwards

To decide at the end of slice 2. Part 2c builds the server and the two clients on this crate, so it stays at
least until then.
