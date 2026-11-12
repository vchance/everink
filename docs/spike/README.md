# The spike

Status: started 2026-11-12. Slices 1 and 2 and the first two parts of slice 3 are done; the rest is planned.

The spike is step 0 of the order of work in [`../design.md`](../design.md). Its purpose is to replace the largest
guesses in the detailed design with measurements before anything is built on them. It is one stroke type, two
clients and a server, native and web. Code written for it is kept only where a slice says so.

## Rules

- **A question is answered by a measurement or a test, never by an opinion.** Each answer names the machine, the
  revision and the command that produced it.
- **Each slice is one reviewed change** with its own results file in this folder. Slice 2 answers four
  questions that do not depend on each other, so it is three changes: 2a the object map, 2b the text libraries,
  2c the server, the clients and the paced connection. Slice 3 is three: 3a the wasm32 target, stroke
  outlines and the cut; 3b the renderer natively; 3c the renderer in a browser. Pen latency (question 11) is the owner's to run.
- **Results change the design.** Where a measurement contradicts a section of [`../design/`](../design/README.md),
  the slice's results file says so and names the section; the design is corrected in a separate change.
- **What the agent cannot measure is the owner's to run.** Those rows are marked "owner".

## Questions and what settles them

The questions are the spike table of [`../design/README.md`](../design/README.md). "Done when" is the acceptance
criterion for each.

| # | Question | Slice | Done when | Who |
|---|---|---|---|---|
| 1 | What does exact position arithmetic cost? | 1 | Subtraction, re-base of 10,000 objects and one frame are timed at depths 0, 1, 10, 100 and 1,000 levels and compared with the estimates in section 01 | agent |
| 2 | Are positions the same on every machine? | 1, 3 | One golden file of positions, level changes and float conversions passes byte for byte on aarch64 and x86-64 (slice 1) and on wasm32 (slice 3) | agent |
| 3 | Are stroke outlines bit-identical everywhere? | 3a | The same strokes give byte-identical outlines on x86-64, aarch64 and wasm32 | agent |
| 4 | What does cutting a large shape cost? | 3a | A curve from 10, 100 and 1,000 levels above the view is cut to the view and timed against the 2 ms per frame budget | agent |
| 5 | Can any of Loro, yrs or Automerge sync part of a document? | 2b | Each library is read and tried: either a documented way to export and import a subset is shown working, or the attempt and where it fails is recorded | agent |
| 6 | Which library edits text best in one small document per text object? | 2b | For each candidate: bytes and memory for 1,000 small text documents, time to apply an edit, and whether the server can inspect an update before applying it | agent |
| 7 | Does Everink's own object map hold up? | 2a | 100,000 strokes: memory per object and time to open; randomized tests in which the same changes applied in different orders end in the same state | agent |
| 8 | Do cursors stall during a bulk load on one connection? | 2c | Cursor delay is measured while a client loads a large surface over a link limited to 1, 5 and 20 Mbit/s, with the 20 ms pacing rule, against the 200 ms target | agent |
| 9 | How fast is storage? | 5 | Commit latency and load time for one SQLite file per room at 10,000, 100,000 and 1,000,000 objects | agent |
| 10 | Does the renderer carry the load? | 3b, 3c | `vello_gpu` on WebGPU and on WebGL2: tile raster rate, 10,000 objects at 60 frames per second or the measured shortfall, and the size of the wasm build | agent, with the owner for browsers the agent cannot drive |
| 11 | What is pen latency really? | 3 | A 240 frames-per-second camera on named devices, with and without the low-latency canvas | owner |
| 12 | Which controls toolkit? | 4 | The same toolbar and text field built with Everink's own small toolkit and with egui: text composition, a screen reader on desktop and web, wasm size, lines of code and effort | agent, with the owner for screen-reader checks |
| 13 | Does the owner's further candidate make sense here? | 4 | It is put behind the renderer interface next to the Vello path and measured the same way; its details and results are kept outside this repository | agent and owner |

## Slices

| Slice | Contents | Answers | Kept afterwards |
|---|---|---|---|
| 1. Positions | The Cargo workspace, CI, and `everink-core` with the position type, its canonical bytes, the camera, tests and a timing run | 1, 2 (two of three targets) | Yes: `crates/everink-core` |
| 2. Object map and sync | 2a: the server-ordered object map and its convergence tests (`crates/everink-sync`). 2b: the text-library trials (`spike/text-trial`, a package outside the workspace). 2c: the bytes of a change on the wire, a server and two clients that talk in them, and the paced connection (`crates/everink-sync`) | 5, 6, 7, 8 | Decided from the results, at the end of the slice |
| 3. Rendering and ink | 3a: the wasm32 target, stroke outlines and the cut (`crates/everink-core`). 3b: Vello natively (`spike/render-trial`). 3c: Vello in the browser, and the tile cache | 2 (wasm32), 3, 4, 10, 11 | Decided from the results |
| 4. Controls and the further candidate | The same toolbar both ways; the owner's candidate behind the renderer interface | 12, 13 | Decided from the results |
| 5. Storage | One SQLite file per room: commit and load timings | 9 | Decided from the results |

## Decisions taken from the results

Owner decisions of 2026-11-12, recorded in `AGENTS.md` › Owner decisions:

- The text library is **yrs** (questions 5 and 6).
- The fallback of a second connection for presence, and what to do about the 1 Mbit/s case, are decided after a
  real-network test (question 8). That test is not one of this spike's slices; it needs the product's server.
- `spike/text-trial` is kept until the yrs adapter exists.

The corrections the results asked for were applied to `docs/design/` after slice 2. Later slices record theirs
in their results files in the same way.

## Results

| Slice | Results | Summary |
|---|---|---|
| 1 | [results-1-positions.md](results-1-positions.md) | Exact arithmetic costs almost nothing per frame at any depth (0.004 ms for 10,000 objects). Subtraction and re-basing at depth cost about three times the design's estimate, and a re-base does not reliably fit in a frame at depth, so it has to be spread. Positions match one golden file byte for byte on aarch64 and x86-64. Zooming back to a spot by hand is not exact at depth; an object's position and a jump to it are. |
| 2a | [results-2a-object-map.md](results-2a-object-map.md) | The object map holds up. Copies converge whatever order writes arrive in. 100,000 strokes take about 1.3 KB of memory each, as the design estimated, and open in 0.14 s; at 100,000 objects the server's work on a change is under a microsecond at depth 0 and 1 to 3 microseconds with positions 100 to 1,000 levels down. One correction: the design's rule for order keys runs out of room after about 450 objects put in front one after another, so "front" and "back" count instead. |
| 2b | [results-2b-text-libraries.md](results-2b-text-libraries.md) | None of Loro, yrs or Automerge can sync part of a document, so one small document per text object stands. yrs is the smallest and fastest by a wide margin (12 bytes per keystroke, 2 KB of memory per 200-character document, 3 µs for a checked keystroke); Loro is the only one that never panicked on damaged bytes and never let damage change the text, at about 14 times the memory on the machine measured. A client can get past the server's length check by sending updates out of order unless the server refuses updates that have to wait; the design must add that. No candidate passes the design's bar on every count. The choice is the owner's. |
| 2c | [results-2c-paced-connection.md](results-2c-paced-connection.md) | Cursors do not stall during a bulk load under the design's 20 ms rule from about 2 Mbit/s up: at the 95th percentile, at most 39 ms of delay in a model that matches the design's table, and 29 ms at 20 Mbit/s and 52 ms at 5 over real sockets on Linux (of which about 16 ms is the test's own buffers), against a budget of 47. At 1 Mbit/s the 200 ms target is missed (64 ms in the model, 135 ms over sockets). A gap in the protocol was found: while a view is being filled a change can overtake the object it writes to and be lost; the client now keeps such writes until the object arrives. Without the rule a cursor waits for the whole send buffer, seconds on a slow link. Two corrections: the limit must start small, and the rule only works if the operating system is kept from holding unsent bytes. The fallback of a second connection for presence is not called for on this evidence. |
| 3a | [results-3a-wasm-outlines-cut.md](results-3a-wasm-outlines-cut.md) | Positions and stroke outlines are the same, byte for byte, on aarch64, x86-64 and wasm32. The outline the design specified leaves holes over the stroke where it doubles back or curls tightly (7% of a stroke's own points in the test corpus); the prototype builds the union of discs and bands instead, which cannot. Cutting one curve to the view costs 0.37 ms at 100 levels above it and 11 ms at 1,000 on an Apple M4, three to four times that on wasm32: about ten times the design's estimate at 100 levels, and too long for one frame from about 300 levels, so the cut must be able to stop part-way. |
| 3b | [results-3b-renderer.md](results-3b-renderer.md) | Natively on an Apple M4, redrawing a whole 4K view of 10,000 strokes takes 78 ms with the design's outline and 420 ms with the robust outline of slice 3a; the design reads 60 ms as acceptable and 400 ms as not. The cost is processor time preparing paths, 20 to 35 ns a segment, not GPU time, and `vello_cpu` is as fast as `vello_gpu` on this content. The robust outline has nine times the segments, so its form has to change. The tile cache, which is what makes a steady view cheap, is not built yet; no browser figure yet. |
