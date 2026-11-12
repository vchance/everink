# Spike slice 2c: the connection

Date: 2026-11-12. Code: `crates/everink-sync` (`wire`, `pacing`, `link_model`, two examples).
Design: [`../design/03-sync-and-server.md`](../design/03-sync-and-server.md), "Flow control", and
[`../design/04-presence.md`](../design/04-presence.md), "One connection".

This is the last of three parts of slice 2. It answers question 8 of the [plan](README.md): do cursors stall
during a bulk load on one connection?

## Answer

No, with the design's 20 ms rule, down to about 1 Mbit/s; and yes, badly, without it.

- **With the rule,** another person's cursor is delayed by 23 to 30 ms (median) and at most 39 ms (95th
  percentile) at 2 Mbit/s and above while bulk data is sent without a pause, and by 50 to 64 ms at 1 Mbit/s.
  That fits the 47 ms the design leaves for queueing at 2 Mbit/s and above, and is within the 200 ms target for
  the whole path at 1 Mbit/s. The model reproduces the design's own table to within 6 ms at every speed.
- **Without it,** a cursor message waits for the whole send buffer: 420 ms at 20 Mbit/s, 1.7 s at 5 Mbit/s and
  8.4 s at 1 Mbit/s behind one megabyte. Measured over real sockets as well as in the model.
- **The second connection for presence is not needed** on this evidence. Section 04 keeps it as a fallback if
  cursor lag still passes 200 ms above about 1.6 Mbit/s with the rule; it does not.
- **Two corrections to the design** came out of it: the limit must start small, and the rule only works if the
  operating system is also kept from holding unsent bytes. See [Findings](#findings).

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
ignore the limit; the stream of accepted changes and the bulk stream are cut into chunks that fit under it, and
take turns of eight chunks.

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

`cargo run --release --example paced_link <seconds>`. Three threads joined by TCP on the loopback interface. The
server writer has 30,000 strokes (30.6 MB) to send to a client that has just joined, and a cursor to pass on 30
times a second; the relay reads from it at the link's speed; the client decodes every object and times every
cursor message. Cursor delay in milliseconds after the first second: median / 95th percentile / maximum.

LINUX_SECTION

### macOS (Apple M4), 6 seconds per run

| Mbit/s | 20 ms rule | Fixed 64 KiB | Fixed 1 MiB | Most bytes ever held by the system |
|---|---|---|---|---|
| 1 | 2,787 / 3,454 / 3,554 | 3,285 / 4,076 / 4,168 | none arrived | 479 KiB |
| 5 | 34 / 44 / 45 | 122 / 134 / 138 | 1,696 / 1,748 / 1,792 | 60 to 65 KiB |
| 20 | 24 / 27 / 27 | 30 / 34 / 65 | 423 / 426 / 427 | 48 to 50 KiB |

At 5 and 20 Mbit/s the figures agree with the model within a few milliseconds, in all three columns. At 1 Mbit/s
they do not, and the last column says why: the writer had asked for 4 KiB socket buffers at both ends, and the
system held 479 KiB between the writer and the relay all the same. Those bytes are ahead of every cursor
message and the pacer cannot see them; its estimate of the link fell to nothing, because once the system had
swallowed that much it took almost nothing more for seconds. This is macOS on the loopback interface (it also
re-sizes a receive buffer when a loopback connection is made); why it holds so much more at 1 Mbit/s than at 5
was not found. It is recorded because it shows what the rule depends on.

No object was damaged or lost in any run: every object the client received decoded, and the client's copy held
exactly as many as arrived.

## What the tests show

`cargo test -p everink-sync --test wire_and_pacing` runs 10 tests; with slice 2a's, the crate has 28.

- **Bytes on the wire.** 3,000 random changes, accepted or not, and the answers to the accepted ones, decode to
  what was encoded. Every cut-off is refused. Of 12,000 single-bit changes, each is refused or decodes to a
  message whose bytes are exactly the changed bytes. A count of operations over the limit is refused from the
  count alone.
- **A server and two clients that talk only in bytes.** The first client makes 3,000 changes; the server decodes
  each before it looks at it. The second joins after 1,000: it is sent the room as it is in the bulk stream
  while changes keep arriving in the other stream and the first client's cursor is passed on. Both clients end
  with the room's bytes. The second sees far fewer cursor messages than were made, in order, the last one last.
- **The pacer.** Presence and control go first whatever is unsent, and only the newest presence message of each
  sender is kept. A chunk never takes the unsent bytes to the limit and is never over 16 KiB, under four rules
  and a connection that sends random amounts. The two streams take turns of eight. The limit follows the
  connection's speed, ignores idle stretches, and stays between its floor and ceiling.
- **The model.** The three results above are asserted: the design's table within 6 ms, a megabyte of delay
  without pacing, and the slow start.

Two tests were checked to be able to fail: with the unsent bytes left out of the chunk size, five tests failed;
with every presence message kept instead of the newest, two failed.

## Findings

1. **The limit must start small.** With the design's 16 KiB before the first estimate, the first cursor
   messages after a load begins wait up to a second on a 0.25 Mbit/s link and 230 ms at 1 Mbit/s, over the
   200 ms target for the whole path. Starting at the 4 KiB floor removes it; the limit finds its place within
   one 50 ms window either way. `Pacer::with_first_limit` is there to try it.
2. **The rule holds only if the operating system is also kept from holding unsent bytes.** The pacer counts what
   it has handed over; whatever the system then holds is delay it cannot see, and it also spoils the estimate of
   the link's speed, which is made from how fast the system takes bytes. The design names the right tool
   (`TCP_NOTSENT_LOWAT`); the macOS run above shows what happens without it. Two consequences for the design:
   the option is a requirement of the server's platform and not a tuning detail, and the estimate should be
   checked against something that cannot be fooled by a buffer (the acknowledged bytes the system reports).
3. **The safe wrapper for that option is Linux and Android only** (`socket2` 0.6.5). The server is Linux, so
   that is enough for the product; a development machine on macOS cannot reproduce the server's behaviour.
4. **Counting all unsent bytes in one place would halve the delay on slow links** (the "one queue" column). It
   needs the system's own count of unsent bytes. Not needed to meet the target; worth knowing it is there.
5. **A 40 ms rule buys nothing.** It doubles the delay between 2 and 10 Mbit/s and changes nothing elsewhere.
   Section 03 says 20 ms is "the first value the spike tries"; it can stay.
6. **Below about 2 Mbit/s the floor, not the rule, sets the delay.** At 0.25 Mbit/s a 4 KiB floor is 131 ms of
   sending. A lower floor would help there and cost more, smaller frames; the design's range starts at 1 Mbit/s.

## What the measurements do not show

- **A real network.** Loopback has no loss, no congestion control worth the name, and no other traffic. On a
  real path the speed changes and the rule's estimate has to follow it; only the model's step change was tested
  (`the_limit_follows_what_the_connection_sends`).
- **A WebSocket and TLS.** The frames here are a class, a length and a payload on plain TCP. A WebSocket adds a
  few bytes per frame; TLS adds records and its own buffering, which belongs with Finding 2.
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

## What the design should change

1. Section 03, "Flow control": the limit starts at the floor (Finding 1).
2. Sections 03 and 04: say that bounding the system's unsent bytes is required for the rule to mean anything,
   name the platform limit, and add the check on the estimate (Findings 2 and 3).
3. Section 03: say what a chunk cuts (departure 1).
4. Section 04, "One connection": replace the table with the model's, note the one-queue figures, and record that
   the second connection is not needed on this evidence (the decision to drop the fallback is the owner's).

These are recorded here and made in a separate change, with the corrections from slices 1, 2a and 2b.

## Kept afterwards

Slice 2 is complete, so this is the point the plan set for deciding what is kept.

- **`crates/everink-sync`: keep.** The object map answered its question and the rest of the product builds on
  it; `wire` and `pacing` are what the server will use. The link model stays as the pacer's test bench.
- **`spike/text-trial`: keep until the owner has chosen a text library**, then remove (results of slice 2b).
- **`examples/paced_link.rs`: keep until the server exists**, then replace with a test against the real server.
