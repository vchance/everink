# Spike slice 2c: the connection

Date: 2026-11-12. Code: `crates/everink-sync` (`wire`, `pacing`, `link_model`, two examples).
Design: [`../design/03-sync-and-server.md`](../design/03-sync-and-server.md), "Flow control", and
[`../design/04-presence.md`](../design/04-presence.md), "One connection".

This is the last of three parts of slice 2. It answers question 8 of the [plan](README.md): do cursors stall
during a bulk load on one connection?

## Answer

No, with the design's 20 ms rule, from about 2 Mbit/s up; and yes, badly, without it.

The design leaves 47 ms for queueing inside its 200 ms target for a cursor's whole path (section 04). Every
figure in this table is a 95th percentile.

| | 1 Mbit/s | 5 Mbit/s | 20 Mbit/s |
|---|---|---|---|
| With the rule, in the model | 64 ms: over | 39 ms: within | 26 ms: within |
| With the rule, over real sockets on Linux | 135 ms: over | 52 ms: 5 ms over | 29 ms: within |
| Without it (a megabyte unsent), model and sockets | 8.4 s; over sockets no cursor arrived | 1.7 s | 0.42 s |

- **With the rule** the model gives at most 39 ms at 2 Mbit/s and above while bulk data is sent without a
  pause (39 ms at 2 and 5 Mbit/s, falling to 7 ms at 100). It reproduces the design's own table to within 6 ms
  at every speed.
- **Over real sockets** the figures are the model's plus what the test's own socket buffers hold, about 10 KiB:
  4 ms at 20 Mbit/s, 16 ms at 5, 82 ms at 1. That part is the test's own (Finding 8); without it the 5 Mbit/s
  figure is inside the budget, so it is not counted against the rule.
- **At 1 Mbit/s the target is missed,** by a little in the model (about 217 ms for the whole path) and by more
  over sockets. The design expected the target to hold above about 1.4 Mbit/s, which the model bears out.
  Counting every unsent byte in one place would bring 1 Mbit/s inside it (Finding 5).
- **Without the rule** a cursor message waits for the whole send buffer.
- **The second connection for presence is not called for** on this evidence. Section 04 keeps it as a fallback
  if cursor lag still passes 200 ms above about 1.6 Mbit/s with the rule; in the model it does not from
  2 Mbit/s up.
- **A gap in the protocol was found on the way:** while a view is being filled, a change can overtake the object
  it writes to, and the client then loses it (Finding 1).
- **Two more corrections to the design:** the limit must start small, and the rule only works if the operating
  system is also kept from holding unsent bytes.

**Decided since:** on 2026-11-12 the owner chose to decide two things after cursor lag has been measured over a
real network path: whether to keep the second connection as a fallback, and what to do about 1 Mbit/s. Until
then the design keeps the fallback and its stated target.

## What was built

| Part | What it is |
|---|---|
| `wire` | The bytes of a `Change` (client to server) and of an `Applied` (server to every copy). One encoding per value; the decoders refuse the rest |
| `pacing::Pacer` | The four outbound queues of a connection and the rule for what is written next. It has no clock and no socket: the caller says what the connection took and when |
| `link_model` | A connection in made-up time that sends a fixed number of bytes a second, to see what the rule does without a network. The same on every machine |
| `examples/sim_paced_link.rs` | The model's table |
| `examples/paced_link.rs` | The same pacer over real sockets on the loopback interface: a server writer, a relay that lets through so many bits a second, and a client that puts the streams back together |
| A server and two clients, in a test | They exchange only bytes: changes, answers, the room sent to a late joiner, and cursors |

The server and clients are not yet a program that listens on a port for people: that comes with the product's
server. What is tested here is everything that crosses the connection and the order it crosses in.

## The rule

> Keep the connection's unsent bytes below what it drains in about 20 ms, clamped to between 4 KiB and 64 KiB,
> and write bulk data in chunks no larger than that limit and never over 16 KiB.

A cursor message handed to a connection waits behind every byte handed over before it that has not left. So the
writer never hands over much more than the connection can send soon. Cursors and control messages go first and
ignore the limit; the stream of accepted changes and the bulk stream are cut into chunks that fit under it.
Accepted changes go before bulk; after eight chunks of them in a row one chunk of bulk goes, so a load is never
starved.

## Measurements in the model

`cargo run --release --example sim_paced_link`. Made-up time, so there is no machine and no spread between runs.
Another person's cursor arrives 30 times a second; bulk data is sent for the whole 30 seconds. Delay is from the
moment a cursor message is made to the moment its last byte has been sent. Median and 95th percentile after the
first second, in milliseconds.

| Mbit/s | 20 ms rule, two queues | 20 ms rule, one queue | 40 ms rule, two queues | Fixed 64 KiB | Fixed 1 MiB | Design's table (20 ms) |
|---|---|---|---|---|---|---|
| 0.25 | 198 / 257 | 124 / 132 | 198 / 257 | 2,090 / 2,098 | 14,533 / 27,567 | 262 |
| 0.5 | 99 / 131 | 62 / 67 | 99 / 131 | 1,045 / 1,050 | 14,533 / 16,777 | 131 |
| 1 | 50 / 64 | 31 / 33 | 60 / 78 | 522 / 524 | 8,384 / 8,388 | 66 |
| 2 | 30 / 39 | 19 / 21 | 60 / 78 | 261 / 263 | 4,193 / 4,195 | 40 |
| 5 | 30 / 39 | 20 / 21 | 53 / 65 | 104 / 105 | 1,677 / 1,678 | 40 |
| 10 | 27 / 32 | 20 / 21 | 47 / 52 | 52 / 53 | 838 / 839 | 33 |
| 20 | 23 / 26 | 20 / 21 | 30 / 33 | 26 / 27 | 419 / 420 | 27 |
| 50 | 12 / 13 | 10 / 11 | 12 / 13 | 10 / 11 | 167 / 168 | 13 |
| 100 | 6 / 7 | 5 / 6 | 6 / 7 | 5 / 6 | 83 / 84 | 6.6 |

- **Two queues** is how a real server is: the operating system holds up to the limit of unsent bytes and says
  only "there is room" or "there is not", and the writer hands over its next chunk whenever there is room. Up to
  one chunk more than the limit can then be waiting. This column is the one to compare with the design's table,
  which is its worst case; the 95th percentile matches it within 6 ms at every speed.
- **One queue** is the rule as written: the writer knows every byte that has not left. It is up to twice as good
  on slow links. Reaching it needs the operating system's count of unsent bytes, which the design does not use.
- **Fixed 64 KiB** was the design's earlier limit; **fixed 1 MiB** stands for a send buffer left to the operating
  system. In the 0.25 and 0.5 Mbit/s rows of the last, some cursor messages never arrive within the run; they
  are counted with the time they had waited by its end.
- Below 2 Mbit/s the limit sits on its floor of 4 KiB, which is more than 20 ms of sending, so the delay grows
  as the link slows. At 50 Mbit/s and above it sits on its ceiling of 64 KiB.

### The start of a load

The limit is 16 KiB until the first estimate of the connection's speed, which takes one 50 ms window. On a slow
link 16 KiB is far more than 20 ms of sending, and the first cursor messages wait behind it. The worst delay in
the whole run, with the design's starting limit and with a starting limit at the 4 KiB floor:

| Mbit/s | Start at 16 KiB | Start at 4 KiB |
|---|---|---|
| 0.25 | 1,018 ms | 264 ms |
| 0.5 | 493 ms | 132 ms |
| 1 | 230 ms | 66 ms |
| 2 | 99 ms | 41 ms |
| 5 | 46 ms | 40 ms |
| 10 and above | as in steady state | as in steady state |

The limit itself settles within 51 ms at every speed; the design asks for 500 ms.

## Measurements over real sockets

`cargo run --release --example paced_link <seconds> <runs>`. Three threads joined by TCP on the loopback
interface. The server writer has 30,000 strokes (30.6 MB) to send to a client that has just joined, and a cursor
to pass on 30 times a second; the relay reads from it at the link's speed; the client decodes every object and
times every cursor message. Each arrangement is run five times for three seconds. Cursor delay in milliseconds
after the first second: the middle run, with the least and the most over the five runs.

### Linux (x86-64, a GitHub-hosted runner)

From the `test` job of CI run [43195119713](https://github.com/vchance/everink/actions/runs/43195119713)
(its second attempt; GitHub did not keep the log of the first), revision 9b65ec8. The repository does not
otherwise take timings from a CI runner; these are delays set by the relay's speed and not by the machine, and
Linux is where the system's unsent bytes can be bounded (`TCP_NOTSENT_LOWAT`), so this is the run that answers
the question. Two earlier runs of four seconds, one run each (CI runs 43192425662 and 43192991412), gave 125 and
135 ms for the 95th percentile at 1 Mbit/s with the rule; five runs of three seconds give 132 to 136.

| Mbit/s | 20 ms rule: median | 20 ms rule: 95th percentile | Fixed 64 KiB: 95th percentile | Fixed 1 MiB: 95th percentile | Worst in the first second, with the rule | Held by the system |
|---|---|---|---|---|---|---|
| 1 | 102 (101 to 103) | 135 (132 to 136) | 721 (720 to 723) | none arrived | 238 (238 to 239) | 10 KiB |
| 5 | 43 (43 to 43) | 52 (52 to 52) | 143 (142 to 143) | 1,715 (1,715 to 1,716) | 55 (55 to 56) | 11 KiB |
| 20 | 26 (26 to 26) | 29 (29 to 29) | 35 (35 to 36) | 429 (429 to 429) | 32 (31 to 32) | 11 KiB |

- The system held 10 to 11 KiB between the writer and the relay in every run (Finding 8). At the link's speed
  that is 82 ms at 1 Mbit/s, 16 ms at 5 and 4 ms at 20, and the figures are the model's two-queue column plus
  that.
- The pacer's estimate of the link was right in every arrangement (0.97, 4.95 and 20.04 Mbit/s).
- The first second at 1 Mbit/s, 238 ms, is the slow start the model shows (230 ms).
- With a megabyte unsent at 1 Mbit/s, one cursor message arrived in three seconds.

### macOS (Apple M4)

| Mbit/s | 20 ms rule: median | 20 ms rule: 95th percentile | Fixed 64 KiB: 95th percentile | Fixed 1 MiB: 95th percentile | Held by the system |
|---|---|---|---|---|---|
| 1 | 1,744 (1,705 to 1,745) | 2,297 (2,157 to 2,319) | 2,481 (2,349 to 2,486) | none arrived | 478 KiB |
| 5 | 33 (33 to 34) | 43 (41 to 45) | 133 (133 to 134) | 1,787 (1,756 to 1,792) | 61 to 66 KiB |
| 20 | 24 (24 to 24) | 27 (27 to 35) | 33 (33 to 34) | 427 (427 to 427) | 49 to 51 KiB |

At 5 and 20 Mbit/s the figures agree with the model within a few milliseconds, in all three columns. At 1 Mbit/s
they do not, and the last column says why: the writer was given 4 KiB socket buffers at both ends, and the
system held 478 KiB between the writer and the relay all the same. Those bytes are ahead of every cursor
message and the pacer cannot see them; its estimate of the link fell to a third of the truth, because once the
system had swallowed that much it took almost nothing more for seconds. This is macOS on the loopback interface
(it also re-sizes a receive buffer when a loopback connection is made); why it holds so much more at 1 Mbit/s
than at 5 was not found. It is recorded because it shows what the rule depends on.

No object was damaged or lost in any run: every object the client received decoded, and the client's copy held
exactly as many as arrived.

## What the tests show

`cargo test -p everink-sync --test wire_and_pacing` runs 11 tests; with slice 2a's, the crate has 29.

- **Bytes on the wire.** 3,000 random changes, accepted or not, and the answers to the accepted ones, decode to
  what was encoded. Every cut-off is refused. Of 12,000 single-bit changes, each is refused or decodes to a
  message whose bytes are exactly the changed bytes. A count of operations over the limit is refused from the
  count alone, and no room is set aside for more operations than the bytes left could hold.
- **A server and two clients that talk only in bytes.** The first client makes 3,000 changes; the server decodes
  each before it looks at it. The second joins after 1,000, on a slow connection: the room as it was then goes
  into the bulk stream, and takes many rounds to arrive while changes keep arriving in the other stream and the
  first client's cursor is passed on. The test checks that the two streams really were mixed. Both clients end
  with the room's bytes. The second sees fewer cursor messages than were made, in order, the last one last.
- **The same run without the fix of Finding 1** ends with a second client whose objects differ from the room's.
- **The pacer.** Presence and control go first whatever is unsent, and only the newest presence message of each
  sender is kept. A chunk never takes the unsent bytes to the limit and is never over 16 KiB, under four rules
  and a connection that sends random amounts. Accepted changes go before bulk, and bulk gets one chunk in nine
  when both are long. The limit follows the connection's speed, ignores idle stretches, and stays between its
  floor and ceiling.
- **The model.** The three results above are asserted: the design's table within 6 ms, a megabyte of delay
  without pacing, and the slow start.

Two tests were checked to be able to fail: with the unsent bytes left out of the chunk size, five tests failed;
with every presence message kept instead of the newest, two failed.

## Findings

1. **While a view is being filled, a change can overtake the object it writes to, and is then lost.** The room
   is sent to a joining client in the bulk stream; accepted changes go in their own stream and go first. A copy
   drops a write to an object it does not hold (slice 2a, Finding 2: the object will come whole). During a fill
   that is wrong: the object that arrives afterwards is as it was when the fill began, and the write is gone for
   good. The first version of this slice's test did not show it, because its room fitted in one turn of the
   pacer and arrived before any change; a reviewer made the streams interleave and it failed.
   The prototype's fix is on the client: between `begin_fill` and `end_fill` a copy keeps writes to objects it
   does not hold and merges them when the object arrives. The stamps decide, so a write the object already
   contains changes nothing. Two other fixes are possible on the server: write each object's bytes when its
   chunk is taken and not when it is queued, with the server holding back changes to an object that is partly
   sent; or send a change for an object in the view only after the object. Section 03's rule for routing changes
   to a connection during `SetView` has to say which.
2. **The limit must start small.** With the design's 16 KiB before the first estimate, the first cursor
   messages after a load begins wait up to a second on a 0.25 Mbit/s link and 230 ms at 1 Mbit/s, over the
   200 ms target for the whole path. Starting at the 4 KiB floor removes it; the limit finds its place within
   one 50 ms window either way. `Pacer::with_first_limit` is there to try it.
3. **The rule holds only if the operating system is also kept from holding unsent bytes.** The pacer counts what
   it has handed over; whatever the system then holds is delay it cannot see, and it also spoils the estimate of
   the link's speed, which is made from how fast the system takes bytes. The design names the right tool
   (`TCP_NOTSENT_LOWAT`); the macOS run above shows what happens without it. Two consequences for the design:
   the option is a requirement of the server's platform and not a tuning detail, and the estimate should be
   checked against something that cannot be fooled by a buffer (the acknowledged bytes the system reports).
4. **The safe wrapper for that option is Linux and Android only.** macOS has the option; `socket2` 0.6.5 and
   `libc` do not expose it there, and the workspace forbids the `unsafe` call that would. The server is Linux,
   so that is enough for the product; a development machine on macOS shows different behaviour unless it makes
   that call.
5. **Counting all unsent bytes in one place would halve the delay on slow links** (the "one queue" column): 33 ms
   in place of 64 at 1 Mbit/s, which is the difference between missing the 200 ms target there and meeting it.
   It needs the system's own count of unsent bytes, which the design does not use.
6. **A 40 ms rule buys nothing.** It doubles the delay between 2 and 10 Mbit/s and changes nothing elsewhere.
   Section 03 says 20 ms is "the first value the spike tries"; it can stay.
7. **Below about 2 Mbit/s the floor, not the rule, sets the delay.** At 0.25 Mbit/s a 4 KiB floor is 131 ms of
   sending. A lower floor would help there and cost more, smaller frames; the design's range starts at 1 Mbit/s.
8. **What the test's own sockets hold is part of the socket figures.** About 10 KiB sat between the writer and
   the relay on Linux in every run: the relay's receive buffer and the bytes in flight on the loopback interface.
   (Linux reports a buffer as twice the size that was set, for its own bookkeeping; the 8,192 the run prints is
   the 4,096 that was asked for.) A real path has no relay, so this part is the test's and not the rule's.

## What the measurements do not show

- **A real network.** Loopback has no loss, no congestion control worth the name, and no other traffic. On a
  real path the speed changes and the rule's estimate has to follow it; only the model's step change was tested
  (`the_limit_follows_what_the_connection_sends`).
- **A WebSocket and TLS.** The frames here are a class, a length and a payload on plain TCP. A WebSocket adds a
  few bytes per frame; TLS adds records and its own buffering, which belongs with Finding 3.
- **Many connections.** One client loads; the server's cost per connection is not measured.
- **The 33 ms tick and the receiver's smoothing.** Section 04 adds both to a cursor's path; only the queueing
  part is measured here.

## Where the code departs from the design

1. **The two streams are byte streams cut into chunks**, with records inside them that the receiver puts back
   together. Section 03 has one message per frame and sends bulk data "in chunks"; it does not say what a chunk
   cuts. A message larger than the limit could otherwise never be sent on a slow link.
2. **Frames and messages are hand-written bytes**, not postcard. Section 03 names postcard for the messages
   around the bodies; the bodies and changes are section 02's bytes either way.
3. **A change's bytes carry no client id.** The connection knows who it is.
4. **In the socket run the system's unsent limit is fixed at 4 KiB**, and the writer keeps the unsent bytes in a
   queue of its own, handing over a chunk whenever that queue is under the limit. Section 03 sets
   `TCP_NOTSENT_LOWAT` to the current limit and changes it with the limit. The two arrangements bound the same
   bytes; the socket figures are for this one.
5. **The client keeps writes during a fill** (Finding 1). Section 03 has no such rule.

Apply latency during a load (how long an accepted change waits behind bulk chunks) is not measured. With
accepted changes going first it is at most one bulk chunk; with the equal turns this slice first had, it would
have been up to eight.

## What the design should change

1. Section 03, "Flow control": the limit starts at the floor (Finding 2).
2. Sections 03 and 04: say that bounding the system's unsent bytes is required for the rule to mean anything,
   name the platform limit, and add the check on the estimate (Findings 3 and 4).
3. Section 03: say what a chunk cuts (departure 1).
4. Section 04, "One connection": replace the table with the model's and the socket figures, say that the target
   is met from about 2 Mbit/s and what would bring 1 Mbit/s inside it (Finding 5), and record that the fallback's
   own trigger is not reached (whether to drop the fallback is the owner's decision).
5. Section 03, "On SetView" and routing: say what happens to a change for an object whose body has been queued
   and not yet sent (Finding 1).

These are recorded here and made in a separate change, with the corrections from slices 1, 2a and 2b.

## Kept afterwards

Slice 2 is complete, so this is the point the plan set for deciding what is kept.

- **`crates/everink-sync`: keep.** The object map answered its question and the rest of the product builds on
  it; `wire` and `pacing` are what the server will use. The link model stays as the pacer's test bench.
- **`spike/text-trial`: keep until the owner has chosen a text library**, then remove (results of slice 2b).
- **`examples/paced_link.rs`: keep until the server exists**, then replace with a test against the real server.
