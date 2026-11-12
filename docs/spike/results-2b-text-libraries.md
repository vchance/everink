# Spike slice 2b: the text libraries

Date: 2026-11-12. Code: `spike/text-trial` (a package of its own, outside the Cargo workspace).
Design: [`../design/02-document-model.md`](../design/02-document-model.md), "Text" and "Spike plan".

This is the second of three parts of slice 2. It answers questions 5 and 6 of the [plan](README.md) for
Loro 1.17.1, yrs 0.28.1 and Automerge 0.12.0.

**Which library edits text is the owner's decision** (`AGENTS.md` › Owner decisions). This file gives the
evidence and, at the end, what the evidence points to. It decides nothing.

**Decided since:** on 2026-11-12 the owner chose **yrs**, and to keep this trial until the yrs adapter exists.

## Answers

**Question 5: can any of them sync part of a document? No.**

| Library | What was tried | What happened |
|---|---|---|
| Loro | Its export modes are the whole document or a range of versions; none names a container. So the changes were taken apart: the operations on one text were kept and the rest dropped | Refused: "op counters must be contiguous with the change id". With two peers who each edit their own text and sync now and then, one peer's changes alone are accepted but wait for ever on the other's: of 20 characters, 5 arrive |
| yrs | No call takes a type to limit an update to. Its documented way is the sub-document | Works, and is a separate document: the parent's state does not grow when the sub-document is edited, a copy given only the parent has the pointer and no text, and the text arrives with the sub-document's own update |
| Automerge | The change that made two text objects, and every change that typed into one of them, were applied to a new document | Accepted, and 19 of the 21 wait on changes that were not sent: each change names the one before it, whatever object that one touched. One character arrives |

So the finding the design rests on holds for all three: a library syncs a whole document. One document for a
whole surface cannot be sent in part, and "one small document per text object" is the way to sync text by view.

**Question 6: which edits text best in one small document per text object?** yrs is the smallest and fastest on
every figure measured, by a wide margin. Loro is the most careful with bad input: it never panicked, and never
let damaged bytes change the text. No candidate passes the design's own bar on every count; see
[Against the design's bar](#against-the-designs-bar).

## Measurements

Machine: Apple M4 (aarch64, macOS), native, release build, Rust 1.97.1. Command:
`cargo run --release --locked --manifest-path spike/text-trial/Cargo.toml`. A text object is 200 characters typed
one at a time, each keystroke its own update. Times are the median of 7 runs with the minimum and maximum.
Memory is how much a process grew while it loaded 10,000 documents, divided by 10,000; each figure is taken in a
process of its own.

| | Loro 1.17.1 | yrs 0.28.1 | Automerge 0.12.0 |
|---|---|---|---|
| Update for one keystroke, median (min to max) | 85 B (81 to 85) | 12 B (10 to 12) | 113 B (106 to 113) |
| Whole document, 200 characters | 397 B | 211 B | 398 B |
| Whole document, empty | 81 B | 2 B | 117 B |
| Whole document, 1,000 characters | 451 B | 1,011 B | 255 B |
| Memory per document held, 200 characters | 28.2 KB | 2.1 KB | 37.8 to 48.5 KB (three runs) |
| Memory per document held, empty | 17.7 KB | 1.4 KB | 16.5 KB |
| Memory for 1,000 documents of 200 characters | 28 MB | 2.1 MB | 38 to 49 MB |
| Apply one keystroke | 3.13 µs (2.86 to 3.57) | 0.51 µs (0.51 to 0.63) | 66.9 µs (66.0 to 67.2) |
| Apply one keystroke with the server's check, document growing to 1,000 characters | 61.6 µs (60.1 to 65.6) | 2.97 µs (2.96 to 3.06) | 140 µs (139 to 142) |
| The same on a document that stays at 200 characters | 42.9 µs (42.8 to 44.4) | 2.90 µs (2.89 to 2.97) | 75.1 µs (74.9 to 75.3) |
| Load 5,000 documents of 200 characters | 33.3 ms (32.5 to 38.1) | 4.2 ms (4.2 to 4.5) | 1,565 ms (1,560 to 1,829) |
| Crates the library brings in | 141 | 40 | 47 |

Notes on the table:

- **The server's check** is the rule from section 02, with one addition from Finding 1: apply the update to a
  scratch copy, refuse it if any of it has to wait for changes the server has not seen, check the length limit,
  and only then apply it to the real document. Loro and Automerge copy with `fork`. yrs has no fork; its copy is
  a new document loaded from the whole state, and it is still the cheapest by far.
- **Automerge's apply time** is for decoding the change and applying it. Its shorter call, `load_incremental`,
  cannot be used by a server; see Finding 4.
- **Whole-document sizes** cross over: yrs stores typed text one run per keystroke until it is compacted, so its
  1,000-character document is the largest; Loro and Automerge compress their histories. At the 200 characters of
  a whiteboard label yrs is the smallest.
- **Ratios are this machine's.** On the M4, Loro holds a 200-character document in 13.6 times the memory yrs
  does. On the CI runner (x86-64 Linux, a different allocator) the same run gave 20.0 KB against 1.8 KB, about
  11 times. The order is the same on both.
- **The text typed** is plain English, ASCII. Other scripts are not measured (the first release is Latin script,
  owner decision 14).

## What each library does

`cargo test --manifest-path spike/text-trial/Cargo.toml` runs 12 tests. A test that records a limit of a library
asserts the behaviour that was seen, so a new version that behaves differently fails and this file is read again.

| | Loro | yrs | Automerge |
|---|---|---|---|
| Three clients typing and deleting at once, updates late, reordered and repeated, end with the same text | yes | yes | yes |
| A refused update leaves the server's document unchanged, byte for byte | yes | yes | yes |
| An update that depends on changes not yet seen | accepted and parked; `fork` drops what is parked | accepted and parked; a copy keeps it | accepted and parked; `fork` keeps it |
| Overlapping marks of different kinds (bold, italic) from two clients converge | yes | yes | yes |
| Two values of one mark (red, blue) on overlapping stretches from two clients | converge: red, then blue to the end | converge, but the last stretch, which only the second client coloured, ends with no colour | converge: two stretches that do not overlap |
| Undo takes back my edit and leaves theirs | yes | yes, if remote updates are applied with an origin of their own | no undo manager |
| An update can be read before it is applied | only counts and versions; the operations are readable as JSON after importing into a scratch copy | yes: it decodes on its own and shows what was inserted | yes: it decodes on its own into its operations |

### Damaged bytes

Two seeds; per library 5,000 damaged updates, each given to a new copy of a 40-character document, and 1,000
damaged whole documents. The damage is simple: one bit, one byte, a cut, two bytes set to `0xff`, or random
bytes. Each good update is one keystroke or one deletion that applies on its own.

| Damaged updates (5,000) | Loro | yrs | Automerge |
|---|---|---|---|
| Refused | 4,547 | 4,123 | 3,135 |
| Accepted, and the text is what the undamaged update gives | 453 | 280 | 240 |
| Accepted, and the text did not change (parked, or empty) | 0 | 392 | 891 |
| Accepted, and the text is something that was not sent | 0 | 195 | 45 |
| Panicked | 0 | 10 | 689 |
| Text changed after a refusal | 0 | 0 | 0 |

| Damaged whole documents (1,000) | Loro | yrs | Automerge |
|---|---|---|---|
| Refused | 976 | 748 | 998 |
| Loaded | 24 | 249 | 2 |
| Panicked | 0 | 3 | 0 |

Where the panics were: yrs, all 13 at `src/block.rs:93`. Automerge, at six places: `src/op_set2/change/batch.rs`
lines 1121 (651 times), 920, 859, 949 and 136, and `src/change_graph.rs:569`. These are the places two seeds
found; other inputs will find others.

## Findings

1. **A client can get past the server's check by sending updates out of order, unless the server refuses
   updates that have to wait.** Sent keystrokes 22 to 30 before keystroke 21, every library answers "ok" and
   parks them. The design's rule (try on a scratch copy, check the limit, then keep) is not enough:
   - With Loro, `fork` drops what is parked, so the copy that checks keystroke 21 sees 21 characters and
     passes it, and the real document then has 30. The length limit is bypassed.
   - With yrs and Automerge the copy keeps what is parked and sees 30, so keystroke 21 is refused. But the nine
     early updates were already accepted unchecked, they wait in memory without bound, and the document is
     stuck: the one update that would unblock it is the one that is refused.

   The trial's rule therefore refuses any update of which some part has to wait. Each library says so: Loro in
   the `pending` of its import status, yrs in its store's pending update, Automerge through its missing
   dependencies. With that, all three refuse the early updates, keep nothing of them, and accept keystroke 21
   (`updates_sent_out_of_order_do_not_get_past_the_server`). Section 02's "Hostile updates" does not cover this.
2. **yrs and Automerge can panic on bytes from outside**, Automerge on 14% of the damaged updates in this run.
   The design requires that applying an update "must not panic". With either library the adapter has to catch a
   panic around every call that takes outside bytes, and throw away the document it was working on. Loro needed
   nothing.
3. **yrs and Automerge accept some damaged updates that change the text to something nobody sent** (195 and 45
   of 5,000). Loro accepted a damaged update only when the result was exactly what the undamaged one gives.
   A connection over TLS will not flip bits, so this is about a faulty or hostile client more than about the
   wire; a hostile client can send any text anyway. It does mean the library is not a check on what arrives.
4. **Automerge's `load_incremental` says "ok" to bytes it cannot read**, and applies nothing. A server that used
   it could not tell a bad update from a good one. The trial decodes each change itself (`Change::from_bytes`)
   and applies that, which costs more per keystroke; the decoding refuses most damaged bytes and not all, as
   the table shows. It also means an update must be exactly one change: `save_incremental` returns every change
   since it was last called, received ones included, so the adapter calls it and drops the result before each
   edit.
5. **yrs undo needs remote updates marked.** Applied in a plain transaction, as a local edit is, a remote edit is
   taken for mine and undone with it. Applied with `transact_mut_with(<an origin>)`, it is left alone. This
   settles the item section 02 lists as unverified ("yrs `UndoManager` behaviour for remote changes").
6. **Two values of one mark merge badly in yrs.** When two clients colour overlapping stretches differently at
   the same time, both copies agree, but a stretch that only one of them coloured ends with no colour. Loro and
   Automerge keep a colour on every character that was given one.
7. **Automerge 0.12.0 has marks and no undo manager.** Section 02 listed both as unverified. An application
   would build undo from inverse edits.
8. **Smaller differences.** In Automerge the text is an object some change created, so every client must start
   from the same document that already holds it; Loro and yrs name a root text and need no such start. Loro
   wants a mark's name declared before it is used. yrs counts a text's length in UTF-8 bytes unless told
   otherwise, so the product's length limit needs its UTF-16 or character setting.
9. **Licences.** The trial's lock file lists 201 packages over all platforms. All offer MIT or Apache-2.0 except
   five: `im`, `bitmaps` and `sized-chunks` (MPL-2.0-or-later) and `xxhash-rust` (BSL-1.0), all brought in by
   Loro, and `zlib-rs` (Zlib), brought in by Automerge. These are compatible with MIT OR Apache-2.0 for use as
   dependencies; MPL-2.0 puts conditions on changes to those crates' own files. yrs brings in none of them. The
   licences were read from the crates' metadata and not audited further.

## Against the design's bar

Section 02 sets the bar for the text trial: within two times the best on memory and latency, with no fuzz or
merge failures. The ratios are this machine's.

| | Loro | yrs | Automerge |
|---|---|---|---|
| Memory within 2 times the best | no: about 14 times | best | no: 18 to 24 times |
| Latency within 2 times the best | no: 6 times to apply, 15 to 21 times with the check | best | no: about 130 times to apply, 26 to 47 times with the check |
| No merge failures | pass | text: pass. Two values of one mark: a stretch loses its colour | pass |
| No failures on damaged bytes | pass | fail: panics, and damage that changes the text | fail: panics, and damage that changes the text |

No candidate passes every row. yrs's failures are ones an adapter can contain (catch the panic) or the
application can avoid (it decides how colours are applied). Loro fails only on size and speed, and in absolute
terms its cost is small at this scale: 28 MB for 1,000 text objects and 43 µs for a checked keystroke. Every
candidate needs the rule of Finding 1.

## What the evidence points to

For the owner, not a decision:

- **yrs** if size and speed decide. On this machine it is 7 times smaller per keystroke on the wire and about
  14 times smaller in memory than Loro, has the fewest dependencies and only MIT or Apache-2.0 licences. Its
  costs: panics have to be caught, damaged input can change the text, remote updates must be applied with an
  origin for undo to be right, two values of one mark merge badly, it has no fork, and it is at version 0.x.
- **Loro** if care with bad input and a richer API decide (it never panicked and never let damage through;
  fork, operations as JSON, undo that follows local changes without extra care). Its costs: memory and bytes as
  measured, 141 crates, three MPL-2.0 crates, and a `fork` that drops parked changes, which makes Finding 1's
  rule essential.
- **Automerge** is behind on every measured figure except the size of a long document, and panicked most. It
  also takes 1.6 s to load 5,000 text documents. For scale, section 02's bar for opening a view of 5,000 objects
  is 1 s; that bar was written for another option (a document per object of every kind) and is borrowed here.

Whichever is chosen sits behind the `TextDoc` adapter of section 02, so the choice can be revisited.

## Not done

- **WebAssembly size.** Section 02 asks what each library adds to the wasm build. That is measured in slice 3,
  where the wasm32 target arrives.
- **The larger trials of section 02's spike plan:** a document per object for every object (option B) with 5,000
  objects in a view beyond text, a single root map of 2 million entries, and one document for a whole surface of
  100,000 strokes as a size reference (option A). Slice 2a measured the chosen design (Everink's own object map)
  at 100,000 and 1,000,000 strokes instead; the loading row above is the one figure here that bears on option B.
- **A real fuzzer** (coverage-guided), memory under editing over time, compaction, and text in other scripts.
- **A `TextDoc` adapter in the product.** The trial's `Candidate` trait is smaller than section 02's `TextDoc`.

## What the design should change

1. Section 02, "Text" and "Hostile updates": the server refuses an update of which any part has to wait for
   changes it has not seen (Finding 1).
2. Section 02, "Text": the adapter catches a panic from the library and drops the document it was applied to
   (Finding 2); the library is not relied on to detect damaged input (Finding 3).
3. Section 02, the comparison table and the list of unverified claims: replace the per-document memory and size
   guesses with these figures, and strike the claims settled here (yrs undo and remote changes, Automerge undo
   and marks, whether any library syncs a subset).
4. Section 02, "Spike plan", pass criteria: the bar as written is passed by no candidate; say which failures can
   be contained by the adapter and which cannot.
5. Section 02, limits: say in what unit `MAX_TEXT_CHARS` counts, and that the adapter must count in it
   (Finding 8).

These are recorded here and made in a separate change, with the corrections from slices 1 and 2a.

## Kept afterwards

The trial stays until the owner has chosen a library. After that, the chosen library's adapter moves into the
product and `spike/text-trial` can be removed.
