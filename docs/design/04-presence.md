# Presence, cursors and live ink

Status: detailed design, 2026-11-11, proposal.

## Scope

This section designs the short-lived channel that shows people to each other: cursors with name banners, the colors,
other people's selections, strokes still being drawn ("live ink"), the who-is-here list, and "jump to" and "follow".
It covers the wire messages, the server's routing, the smoothing on the receiving side, the timeouts, the transport
and the limits.

It leaves to other sections:

| Topic | Section |
|---|---|
| What `Position`, `View`, the level cap and exact arithmetic are | 01 |
| Object ids, the committed stroke object, undo | 02 |
| The sync protocol, authentication handshake, the server's spatial index of objects, the shared clock | 03 |
| Brush shape, how a stroke outline is drawn, drawing cursors and banners | 06 |
| Where the who-is-here panel and its accessibility live, pointer events | 07 |
| Roles, invite links, token contents, rate limits per role, the threat model | 08 |

`Position` and `View` are used here as opaque types. Section "What this assumes of other sections" lists the few
operations on them that this design needs.

A word on terms. A **session** is one connection of one person (one browser tab or one app window); a person with
two tabs has two sessions. A **tick** is one run of the server's presence loop, every 33.3 ms. A **snapshot** is the
set of cursor states the server sends one receiver in one tick. **Newest wins** means a message replaces the earlier
state of the same kind, so a lost message costs nothing once the next arrives.

## The problem

Presence has to feel instant, so it cannot wait for anything durable, and it must not become the thing that slows
the room down. The planning figure is a whiteboard with about 50 active people per surface (owner decision,
2026-11-12), so the headline case is a room of 20 to 50 people; rooms of 200 and 2000 are headroom, and the design
degrades in them on purpose rather than failing. Concrete numbers:

- A cursor sampled 30 times a second, each sample carrying a position, is a few hundred bytes of overhead per second
  per person. In a room of 200 where everyone can see everyone, every sample is fanned out 199 times: without
  limits, 200 x 199 x 30 = 1.2 million cursor updates a second leave the server. Everyone-to-everyone does not
  scale, so the server must send each receiver only what it needs, at a rate that fits a budget.
- A position costs bytes in proportion to depth (section 01: 19 bytes at the origin, about 2.5 bytes more per level,
  up to about 3.1 KB at the depth cap). A cursor ten levels down is about 44 bytes, not 19.
- A position at one depth has to be compared exactly with a view at another. A naive loop does senders x receivers
  exact comparisons per tick: 2000 x 2000 = 4 million per tick.
- A stroke is drawn by a pen that samples at 120 to 240 Hz. Sending every sample, to everyone, as separate messages
  is the largest presence traffic there is.
- The network is not steady. If one slow packet holds up the ones behind it, a cursor that is 400 ms old is shown as
  current. For cursors, old data is worse than no data.
- Names come from the embedding app's tokens, but they are still unvetted text, shown over the work of others.

## What others do

**Figma and FigJam.** Figma's engineering write-up says clients talk to a cluster of servers over WebSockets and
that multiplayer syncs document changes; it does not describe cursors ([Figma, How Figma's multiplayer technology
works](https://www.figma.com/blog/how-figmas-multiplayer-technology-works/)). Figma's help center states limits that
show what a large room needs: it shows at most 200 multiplayer cursors to other users in a file at once, prefers
people with edit permission, always shows the cursor of anyone being followed (not counted in the 200), and at 500
participants new joiners get a static view with no multiplayer ([Figma
Help](https://help.figma.com/hc/en-us/articles/1500006775761)). Cursor chat messages in Figma Design are limited to
52 characters per a help page found by search; that limit was not re-checked on the FigJam page (unverified for
FigJam).

**Miro.** Miro publishes no hard cap; staff advise about 100 simultaneous users per board as a recommendation, and
community reports describe boards of 180 to 200 users working well when cursors were turned off. Cursor visibility
is per user, with no board-wide switch ([Miro Community, how many users on a
board](https://community.miro.com/ask-the-community-45/how-many-users-can-be-on-a-board-at-once-130), [Miro
Community, laggy board with 200
users](https://community.miro.com/ask-the-community-45/laggy-board-with-200-users-4449)). These are community
statements, not specifications (unverified as policy). The lesson: in big rooms people turn cursors off, so a cap
and a user-side switch are expected.

**Liveblocks presence.** Presence is a JSON object per user, temporary: "A user's Presence resets every time they
disconnect." Updates are throttled by default to one WebSocket message per 100 ms (10 per second; configurable from
16 to 1000 ms). After a connection is lost, other users' lists are cleared only after `lostConnectionTimeout`,
default 5000 ms, to avoid flicker on quick reconnects ([Liveblocks client
API](https://liveblocks.io/docs/api-reference/liveblocks-client)). Presence-size limits were in the part of the page
not read (unverified).

**Yjs awareness.** Each client's state is a schemaless JSON object with a per-client clock that only increases; a
state is overwritten only by a newer clock. A client with no update from a peer for 30 seconds marks it offline, so
clients re-announce regularly. A new peer receives the full awareness state, then increments ([Yjs docs,
Awareness](https://docs.yjs.dev/api/about-awareness)). A 15-second renewal interval that I remember from the source
was not on the page (unverified).

**tldraw.** Cursor, selection and viewport travel as "presence records"; `OverlayUtil` draws collaborator cursors,
brushes, scribbles and selections; there are pages on cursor chat and user following ([tldraw collaboration
docs](https://tldraw.dev/docs/collaboration)). The pages I could read gave no update rate or idle timeout
(unverified). Its cursor smoothing library, perfect-cursors (MIT), "interpolat[es] the cursors' locations based on a
set of connected curves", is driven by `addPoint` calls and a per-animation-frame callback, and its README says
services typically throttle to one update per 50 to 80 ms
([perfect-cursors](https://github.com/steveruizok/perfect-cursors)). I did not read its source, so the exact spline
and timing are unverified.

**Game netcode.** The same problem, remote things whose position arrives in bursts, is solved in games. Glenn
Fiedler's snapshot interpolation article: keep received snapshots in a buffer and render slightly in the past so the
next snapshot has usually arrived; choose the delay to survive two lost packets in a row (about 3 send intervals at
2 to 5 percent loss: 300 ms at 10 snapshots per second, about 150 ms at 30, about 85 ms at 60); extrapolation can
lower the delay but works only for roughly 50 to 250 ms on mostly straight motion ([Gaffer on Games, Snapshot
Interpolation](https://gafferongames.com/post/snapshot_interpolation/)). Valve's Source networking documents an
interpolation delay of 100 ms by default for remote entities; the page returned HTTP 403 to me, so that figure is
from memory and unverified ([Valve, Source Multiplayer
Networking](https://developer.valvesoftware.com/wiki/Source_Multiplayer_Networking)). **Interest management** is the
game term for what requirement 7 asks for: send each client only the entities inside its area of interest. Everink's
area of interest is the view.

**Transports.** WebTransport (QUIC, with unreliable datagrams) is marked "Baseline 2026" by MDN ([MDN, WebTransport
datagrams](https://developer.mozilla.org/en-US/docs/Web/API/WebTransport/datagrams)); caniuse shows 92.37 percent
global support, with Chrome 97, Edge 98, Firefox 114, and Safari (desktop and iOS) from 26.4; Chrome for Android
from 157 and Firefox for Android from 160 ([caniuse](https://caniuse.com/webtransport)). On the Rust side
`wtransport` 0.7.2 (MIT OR Apache-2.0, updated 2026-08-11) serves WebTransport with datagrams ([crates.io
wtransport](https://crates.io/crates/wtransport), [docs.rs](https://docs.rs/wtransport/latest/wtransport/)). The
Rust `webrtc` crate is 0.21.1 (MIT/Apache-2.0, updated 2026-11-07) ([crates.io
webrtc](https://crates.io/crates/webrtc)).

**Accessible colors.** The Okabe-Ito palette (vermillion, bluish green, sky blue, blue, reddish purple, and
orange/yellow tones, avoiding yellow-green) is described as unambiguous to people with and without color-vision
deficiency; the authors also advise redundant coding by shape, not color alone ([Okabe and Ito, Color Universal
Design](https://jfly.uni-koeln.de/color/)). ColorBrewer and Tableau offer categorical palettes of 8 to 12 colors,
but their color-vision behavior was not checked here (unverified).

## Options considered

Three questions have real alternatives: the transport, the smoothing, and how a receiver is chosen.

**Transport**

| | A. Same WebSocket, presence as a priority class, bulk data paced (first version) | B. Second, dedicated WebSocket (documented fallback) | C. WebTransport datagrams | D. WebRTC data channels |
|---|---|---|---|---|
| Head-of-line blocking | Presence waits behind at most the bytes already queued (an adaptive unsent limit, 4 to 64 KiB, plus one chunk of at most 16 KiB) and behind any lost TCP segment | Isolated from document bursts; still TCP, so one lost segment stalls later presence for about one RTT | None: datagrams are independent | None when set unordered, unreliable |
| Works everywhere (corporate proxies, old browsers) | Yes | Yes | No: needs UDP/HTTP3; 7.6 percent of browsers lack it | Mostly; needs STUN/TURN |
| Native client | Same crate as sync | Same | `wtransport` client | `webrtc` crate |
| Server work | A writer that serves two classes | A second listener, credential exchange and set of timeouts | A QUIC endpoint and certificates | Signaling, ICE, DTLS, SCTP, a media-grade stack |
| Fallback needed | No | No (it is the fallback) | Yes, the WebSocket path | Yes |
| Priority of presence over changes | In the server's writer only; bytes already in the kernel cannot be reordered | Full: separate queues and separate TCP streams | Full | Full |

Section 03 chooses **A** for the first version. I agree on cost, not on delay: A leaves one connection to
authenticate, rate-limit, time out and close (sections 03, 07 and 08 all describe one socket), and needs only a
writer that serves presence first. What it gives up is quantified under "Transport details": with section 03's
adaptive unsent limit (20 ms of drain) a cursor message waits behind bulk data for about 40 ms at most at any link
speed above about 1.6 Mbit/s, and more below that. **B** is the fallback and has a trigger: if in the spike the
cursor lag during a bulk view fill still exceeds the target (200 ms end to end, see "Smoothing on the receiver")
with that rule, on links the product must support, presence moves to a second socket. **C** is not enough alone,
because networks that block UDP need the WebSocket path anyway; it is added behind the same message format when
measurement shows that TCP stalls matter. **D** is rejected: it is the heaviest to run, needs signaling and TURN
servers, and its only benefit over C is older-browser reach that C already has at 92 percent.

**Smoothing of remote cursors**

| | 1. Show latest sample, low-pass filter | 2. Spline through recent points, small delay (perfect-cursors style) | 3. Snapshot interpolation: fixed render delay, Hermite curve, short bounded extrapolation | 4. Dead reckoning: sender sends velocity, receiver predicts |
|---|---|---|---|---|
| Added delay | About 1 sample | 1 to 2 samples | 2 to 3 samples (66 to 100 ms) | Near 0 |
| Smooth at 30 Hz | Weak: steps show on fast motion | Good | Good | Good while prediction holds |
| Handles loss and jitter | Poor | Moderate | Good (buffer is sized for it) | Corrections jump when motion changes |
| Cursor motion is erratic by nature | Fine | Fine | Fine | Poor: hands stop and turn within a few frames |
| Cost | Trivial | Small | Small | Needs velocity on the wire, correction logic |

I choose **3**. A pointing hand changes direction too often for dead reckoning (4), which Fiedler also found fails
once motion is not straight. Option 2 is the closest relative; 3 is the same idea with an explicit, adaptive delay
and a defined behavior on loss. Given up: about 100 ms of delay compared with 4.

**Who receives what**

| | I. Everyone gets everyone | II. Cursor goes to those whose view contains it, by an index (chosen) | III. Rooms split into zones |
|---|---|---|---|
| Meets requirement 7 | Only if the client filters, wasting bandwidth | Yes, directly | Only roughly |
| Cost with 2000 people | 4 million deliveries per tick | Proportional to deliveries actually due | Zone borders are wrong at any zoom |

II is chosen: it is requirement 7 and design.md already requires it.

## Design

### Overview

Presence travels on the session's one WebSocket as a separate message class (section 03 carries it as opaque
`Presence(Bytes)` messages; this section owns their format). The client sends its own state: pointer, selection,
live ink, object drags, activity. The server keeps, per room, a table of sessions and an index that answers "which
views contain this cursor" and "which cursors lie in this view". Every tick it builds, for each receiver, a snapshot
of the cursors that receiver should see, at a rate and size that fit that receiver's budget. The client keeps a
small buffer per remote cursor and draws it slightly in the past. Presence is never stored; when a room process
restarts, presence starts empty and refills within a second as clients re-announce.

The view of a session is never sent in a presence message. Section 03's `SetView` already carries it (a `ViewMsg`
holding 01's `ViewRect`, exact and unpadded), and the server's presence module reads it from there in-process: one
source of truth, no second copy to disagree with. Presence carries only the cursor's position and the sender's zoom
scale (01's `ViewScale`).

### Identifiers

```rust
/// Small id for a session, unique within one room while the session lives. Reused only after 60 s.
/// The server assigns it; clients can never pick one.
pub struct SessionIdx(pub u16);

/// Millisecond clock of the room, wraps every 49.7 days; compare with wrapping subtraction.
pub struct RoomMs(pub u32);


```

A room supports at most 65,535 live sessions by this index width; the real cap is in section 08's limits table.

### Wire format

All messages are binary, little-endian, with LEB128 varints where marked (v). Every message starts with a one-byte
type. Every message is **self-contained and at most 1000 bytes**, so that the same format can later ride in a QUIC
datagram (the largest datagram a QUIC path accepts is about 1200 bytes minus headers; the exact browser limit is
read from `transport.datagrams.maxDatagramSize` at run time, unverified here). Nothing depends on the order in which
messages arrive; where order matters, a sequence number lets the receiver decide.

**Positions on the wire.** A cursor position is section 01's `QPos`: a `Position` whose fractions are rounded to a
multiple of 2^-14 level units (carrying into the cell), which the sender does before it sends. On the wire it is the
canonical `Position` serialization: a level, then per axis a length-prefixed cell and an 8-byte fraction. Its size
is `P(d)` for a position d levels from the origin level: `P(0)` is 19 bytes and `P(d)` is about `19 + 2.5 d` bytes,
up to about 3.1 KB at the depth cap (01). Routing and the exact "inside the view" test use the `QPos` as sent, so
every machine agrees. Rounding to 2^-14 unit is at most 1/16 pixel at the finest in-level zoom (2048 pixels per unit
gives 1/8 pixel), invisible for a pointer. One exception to the 1000-byte rule follows from the size: a position
deeper than about 350 levels (`P` over 900 bytes) makes its message larger, up to about 3.3 KB. Such a message is
legal on the WebSocket and would have to use the reliable path if datagrams are added. The decoder checks the depth
cap on the raw bytes with 01's `Position::check_cap_bytes` before building any number.

**Delta mode for snapshots.** A snapshot may carry a `base` position and write each entry's position with 01's
`Position::encode_delta(base)` and read it with `decode_delta`, which costs 3 to 6 bytes for a nearby position
instead of `P(d)`. Every entry in a receiver's snapshot lies inside that receiver's padded view, so entries are near
each other and near the base (the first entry's position). The snapshot stays self-contained, because the base is in
it. Delta mode is built per receiver, so it gives up the encode-once saving below; the server uses it when `P(d)` of
the entries exceeds 60 bytes (depth over about 16 levels), where the saving is large (a cursor 100 levels down costs
about 12 bytes in an entry instead of 277). At depth 0 it would save 27 to about 14 bytes an entry and is left for
the spike to decide.

```rust
/// What a client sends. Sizes are payload bytes at depth 0, P = P(0) = 19.
pub enum ToServer {
    /// First presence message after the connection is up. About 20 bytes. `resume` lets a reconnect keep session and
        /// color. The name is not here: it comes from the token (section 08).
    PresenceHello { proto: u8, resume: Option<[u8; 16]>, locale_dir: u8 },
    /// Hot path, up to 30 per second. 9 + P = 28 bytes. Not sent while an ink or drag message is (they carry the
    /// pointer themselves), so a session never sends more than about 30 presence messages a second.
    Pointer(PointerMsg),
    /// The person turns "may others jump to or follow me" on or off (section 08, `locatable`). 2 bytes.
    Locatable { on: bool },
    /// Only on change, at most 10 per second. 6 + 16 n bytes (n <= 32 ids), plus P + 16 when `bounds` is set.
    Selection(SelectionMsg),
    /// Live ink, see below. Not allowed from viewers.
    Ink(InkMsg),
    /// Object drag preview, see below. Not allowed from viewers.
    Drag(DragMsg),
    /// Only on change. 3 bytes.
    Activity { seq: u16, state: Activity },
    /// Every 2 s when nothing else was sent, and at least every 250 ms while snapshots arrive. 3 bytes. Carries the
    /// last snapshot number seen, for back-pressure. (Connection liveness and the clock are section 03's Ping/Pong.)
    Heartbeat { last_snapshot: u16 },
    /// "Jump to this person" and "follow". 5 bytes.
    Jump { target: SessionIdx },
    Follow { target: SessionIdx },
    Unfollow,
    /// The who-is-here list: open or close the live list, and ask for a page. 8 bytes.
    WhoSubscribe { open: bool },
    WhoQuery { prefix: String, offset: u16, limit: u8 },
}

pub struct PointerMsg {
    pub seq: u16,                // wraps; newest wins
    pub dt_ms: u8,               // sender's own time since its previous Pointer, saturating at 255
    pub flags: PointerFlags,     // has_cursor, pen_down, hover, discontinuity (teleport: do not interpolate)
    pub tool: ToolId,            // u8, from section 06/07's tool list
    pub scale: ViewScale,        // 01's `ViewScale { level, q }`, 3 bytes (v); q is z in 1/64 octave, 0 to 640
    pub pos: Option<QPos>,       // absent when the pointer is off the surface or a touch is not down
}

pub struct SelectionMsg {
    pub version: u16,            // increases on each change
    pub total: u32,              // number of objects selected
    pub ids: Vec<ObjectId>,      // at most 32; the first 32 by id order
    pub bounds: Option<ExactBox>, // set when total > 32: 01's `ExactBox { min, w, h }`, the sender's box of the selection
}

pub enum Activity { Active, Idle, Away }   // see "Timeouts"

pub enum InkMsg {
    /// One per stroke. 24 + P + 1 bytes: id, brush, the pen-down position, and the sender's zoom scale.
    /// `first` is the exact `Position` of the first point; it is the provisional anchor of the preview.
    Begin { stroke: ObjectId, brush: StrokeBrush, first: Position, scale: ViewScale },
    /// 12 + about 7 bytes per point. Never more than 120 points or 1000 bytes. Also stands in for `Pointer`: the
    /// cursor is the last point of the newest batch.
    Points { stroke: ObjectId, first_index: u32, seq: u16, flags: PointerFlags, pts: Vec<WirePoint> },
    /// Pen lifted. `count` is the number of points drawn in all. 24 bytes.
    End { stroke: ObjectId, count: u32 },
    /// The client dropped the stroke (undo while drawing, tool change). 20 bytes.
    Cancel { stroke: ObjectId },
}

/// The lattice of 06's `PackedPoints`: steps of 2^-16 of an object unit, relative to the previous kept
/// point (the first is relative to `Begin.first`), plus the pressure byte (0 = none, 1 to 255). The same zigzag
/// varints as 06's `bytes`. The receiver decodes each into 06's `InkPoint` and feeds it to `OutlineBuilder::push`.
pub struct WirePoint { pub dx: i64 /*zigzag v*/, pub dy: i64 /*zigzag v*/, pub pressure: u8 }

/// A drag of selected objects before the move is committed (section 02 sends an `Op::Set` of the `Xform` register on
/// release and at most 5 times a second during a long drag).
pub enum DragMsg {
    /// One per drag. 6 + 16 n bytes (n <= 32 ids), + 8 for the count of the rest; the sender's zoom scale.
    Begin { drag: u16, ids: Vec<ObjectId>, total: u32, bounds: Option<ExactBox>, scale: ViewScale },
    /// Up to 30 per second (15 under pressure). 8 + P + the `Delta`: about 40 to 60 bytes at depth 0. Also stands in
    /// for `Pointer`.
    Move { drag: u16, seq: u16, flags: PointerFlags, pointer: QPos, offset: Delta },
    /// Released (the `Op::Set` follows on the document channel) or abandoned. 3 bytes.
    End { drag: u16, committed: bool },
}
```

`ObjectId` is the id the finished stroke will have in the document (section 02 lets a client choose it before the
pen lifts); I assume 16 bytes. `StrokeBrush` is the style block of 06 and 02 (`brush`, `base_width`, `style_ver`), a
few bytes. `Delta` is 01's exact difference with its canonical serialization.

```rust
/// What the server sends.
pub enum ToClient {
    /// Reply to PresenceHello. Gives the session its index, color, glyph and sanitized name, and the room's tick.
    PresenceWelcome { you: SessionIdx, color: ColorSlot, glyph: Glyph, name: String, resume: [u8; 16], tick_ms: u8 },
    /// Describes a session the receiver is about to see: sent on first appearance and when it changes.
    Peer(PeerInfo),
    PeerGone { who: SessionIdx },
    /// The cursors that should be shown. Several per tick if there are many (each <= 1000 bytes).
    /// `base` is set in delta mode (see "Positions on the wire"); then each entry's `pos` is delta-coded against it.
    Snapshot { number: u16, server_time: RoomMs, base: Option<QPos>, entries: Vec<CursorEntry> },
    /// Another session's selection or live ink, sent when the receiver starts seeing that session and on change.
    Selection { who: SessionIdx, sel: SelectionMsg },
    Ink { who: SessionIdx, msg: InkMsg },
    Drag { who: SessionIdx, msg: DragMsg },
    /// "+N more cursors in view that were not sent"; once per second.
    Omitted { count: u16 },
    Who(WhoUpdate),
    /// Answer to Jump: the target's view now. Also used for each Follow update. This is the only way a view reaches
    /// another client, and only the one who asked (the view comes from section 03's `SetView`).
    /// `ViewMsg` is 03's wire view (it holds 01's `ViewRect`), exact and unpadded.
    JumpTo { who: SessionIdx, view: ViewMsg },
    /// Someone started or stopped following you (to you); a jump or follow was refused; rate limited; ...
    PresenceNotice { code: NoticeCode },
}

pub struct PeerInfo {
    pub who: SessionIdx, pub user: UserKey /* opaque; same for two tabs of one person */,
    pub name: String,            // sanitized, disambiguated, <= 24 grapheme clusters (see names)
    
    pub color: ColorSlot, pub glyph: Glyph,
    pub locatable: bool,         // section 08's per-session flag; false hides the jump action
}

pub struct CursorEntry {              // 8 + P = 27 bytes at depth 0
    pub who: SessionIdx,              // 2
    pub flags: PointerFlags,          // 1
    pub age_ms: u8,                   // 1: server_time minus the time of this sample, saturating
    pub scale: ViewScale,             // 3
    pub tool: ToolId,                 // 1
    pub pos: QPos,                    // P, or 3 to 6 bytes in delta mode
}
```

A snapshot of n entries is `1 + 2 + 4 + 1 + 27 n` bytes at depth 0 without a base, so at most 36 entries fit in 1000
bytes. Outside delta mode the entry is the same for every receiver (it contains no receiver-specific field), so the
server encodes each sender's entry once per tick and builds each receiver's snapshot by copying slices.

The cursor's zoom ratio is not sent. The receiver compares `scale` with its own view's scale, which it has. The
other person's **view is never sent to other clients**; only `JumpTo` and the follow stream send a view, and only to
the person who asked, under the privacy rule below.

A cursor deep below the origin is expensive (2.5 bytes a level in every entry) unless delta mode is used. The budget
below uses the real encoded size, so both modes are accounted for.

### Color and glyph

A cursor's identity is a **color slot** plus a **glyph** (a small shape at the start of the banner), and always the
name. The redundant shape follows the Okabe-Ito authors' advice not to rely on color alone.

- **Palette.** Seven hues taken from Okabe-Ito (orange #E69F00, sky blue #56B4E9, bluish green #009E73, yellow
  #F0E442, blue #0072B2, vermillion #D55E00, reddish purple #CC79A7; hex values from memory of the published
  palette, to be checked against the source in the first test) each in two lightness steps, giving **14 slots**, and
  **4 glyphs** (circle, square, triangle, diamond): **56 distinct (color, glyph) pairs**. Past 56 live sessions in a
  room, pairs repeat and the name tells people apart.
- **Contrast against any background.** The cursor arrow and the banner are drawn with a 1.5 px white inner outline
  and a 1 px near-black outer outline, so they separate from light, dark and busy backgrounds whatever their fill.
  The fill therefore need not contrast with the background; it must be distinguishable from the other fills, and the
  banner text must be readable on it. Text is black or white, whichever has more contrast with the fill. For any
  fill, the better of the two gives at least 4.58:1 (the minimum over all luminances is at relative luminance 0.179,
  computed from the WCAG formula), so the 4.5:1 rule for normal text is always met.
- **Color-vision deficiency.** The slot list is accepted only if a build-time test passes: for every pair of slots,
  in normal vision and in simulated protanopia, deuteranopia and tritanopia (Machado et al. 2009 matrices), the
  distance in OKLab must be at least a threshold. The seven base hues are chosen to satisfy it by the Okabe-Ito
  design; the seven second-lightness steps are generated and tuned until the test passes, and the thresholds (start
  with 8 for normal vision and 5 under simulation, on a 0 to 100 scale) are test targets I have not verified. If the
  test cannot be met for 14, the palette shrinks and the glyphs carry more of the load.
- **Assignment.** On join the server picks the (color, glyph) pair as follows: let `pref = hash(user_key) mod 56`;
  take the first free pair at or after `pref`, scanning color slots before glyphs (so the first 14 sessions all get
  different colors). "Free" means unused by a live session in the room. When no pair is free, take the pair with the
  fewest live holders, ties broken by `pref` order.
- **Stability.** The same person (same `user_key`, the token's user id) gets the same pair on every reconnect as
  long as it is free. The room remembers the pair of a departed session for 10 minutes in memory only; the memory is
  released earlier if the pair is needed. A restart of the room process loses it, which only changes a color. Two
  sessions of one person share the pair (same person, same color), and the who list shows one entry with a "2
  devices" note.
- **Large rooms.** With more than 56 sessions colors repeat by design. Only cursors in view are drawn, and the most
  that can be drawn is bounded by the cap below, so repeats are rare in practice.
- **Own cursor.** The local pointer is the system pointer, not a banner. The user's own color shows in the who list
  and in their selection handles.

### Names and banners

- **Source.** Everyone is identified by the embedding app, so every name banner shows the display name from the
  person's token (section 08). The server sanitizes it; the client never sends a name in a presence message. A
  person who joined through an invite link is an ordinary member and gets the same banner, with no label.
- **Sanitizing.** Unicode NFC; remove control characters, bidirectional override and isolate characters (U+202A to
  U+202E, U+2066 to U+2069), zero-width characters and tag characters; collapse runs of whitespace; trim; allow at
  most 3 combining marks in a row (stops stacked "zalgo" text from covering the work); cut to the display-name limit
  in section 08's limits table (I count grapheme clusters). A banner shows at most 24 grapheme clusters and ends
  longer names with an ellipsis; the who list shows all. A name that is empty after this becomes the short form of
  the user id.
- **Plain text only.** Names are drawn by the core's text layout as plain text, wrapped in a first-strong isolate so
  that a right-to-left name cannot reorder the banner around it. Never markup. Banner height is fixed and clipped.
- **Collisions.** If two live sessions of different people have the same name after case folding and compatibility
  normalization, the later one gets " (2)", " (3)" and so on. For confusable names (one name that looks like another
  by Unicode confusable skeleton, for example Latin and Cyrillic letters) the same rule applies, so a person cannot
  borrow another's name.
- **Level of detail on screen.** Drawing every banner in a crowd hides the work. A receiver draws at most **50
  banners** at once: the followed person, anyone hovering near the receiver's own pointer, the most recently active,
  then the nearest to the pointer. The other visible cursors are drawn as arrows in their color and glyph with the
  name on hover. This is a client choice and costs no bandwidth.

### Server: tables and index

```rust
pub struct PresenceRoom {
    sessions: Slab<SessionState>,            // by SessionIdx
    views: ViewIndex,                        // which sessions' views contain a point
    cursors: CursorIndex,                    // which cursors lie in a rectangle
    follow: HashMap<SessionIdx, Vec<SessionIdx>>,   // target -> followers
    colors: ColorBook,                       // pairs in use, remembered pairs with expiry
    budget: RoomBudget,
}

pub struct SessionState {
    peer: PeerInfo,
    last_pointer: Option<(PointerMsg, RoomMs)>,   // newest only
    selection: Option<SelectionMsg>,
    ink: Option<LiveInk>,                          // at most one stroke per session
    view: Option<RoutedView>,                      // padded view, read in-process from section 03's SetView
    visible: HashSet<SessionIdx>,                  // cursors this session currently receives
    sent_version: HashMap<SessionIdx, u16>,        // selection versions it has been sent
    inflight_bytes: u32, last_acked: u16, rate_divisor: u8,   // back-pressure state
    locatable: bool,                               // section 08; mirrored here for the jump and follow checks
    activity: Activity, last_heard: Instant,
}

pub struct LiveInk { stroke: ObjectId, brush: StrokeBrush, first: Position,
                     points: Vec<WirePoint> /* capped at 08's points-per-stroke limit */, bbox: ExactBox,
                     started: RoomMs }
pub struct LiveDrag { drag: u16, ids: Vec<ObjectId>, total: u32, last_offset: Delta, started: RoomMs }
```

**Routing region.** `RoutedView` is the session's view padded by 1/8 of its width and height on each side (section
03 pads the sync view by half a screen; presence uses a smaller pad so it does not send many cursors that cannot be
seen). The server delivers a cursor when it is inside the padded region; **the client draws it only when it is
inside the exact view** (exact comparison, by its own position arithmetic). The pad is hysteresis: the cursor
arrives slightly before it is visible, so it enters already smoothed and does not pop, and a cursor at the edge does
not flicker on and off when view updates and cursor updates cross. Every cursor in the exact view is delivered (the
padded region contains the view), and the client shows exactly those: requirement 7 holds on the screen. This holds
on one condition: **the view moves less than the 1/8 pad between two `SetView` messages**. Section 03 sends
`SetView` at least every 100 ms while a view moves, however small the move, and the server pads the exact view it
receives by 1/8 for this routing; the client sends the exact, unpadded `ViewRect`. A pan faster than 1/8 of the view
width in 100 ms (a fling) breaks the condition. Then a cursor that has just come into the exact view may be missing
until the next `SetView` arrives and the server routes again: at worst about 100 ms plus half a round trip late (the
interval to the next `SetView`, plus its travel to the server). The cursor then appears, already smoothed, with the
next snapshot. This lateness is accepted for flings; the spike measures it.

**Views index.** For each level that some session's view is on, a hash map from cell to the list of sessions whose
padded region overlaps that cell. A view covers few cells at its own level: its width is at most a screen's worth of
pixels divided by the zoom within the level, which is at least 0.5 pixel per unit (the camera range of section 01),
so a 2,000-pixel screen spans about 4,000 units, 4 to 5 cells of 1024 units per axis with the pad. The set of levels
in use, `view_levels`, is kept with a count per level.

**Cursor index.** A sparse, path-compressed tree over (level, cell): a node exists only for a cell that holds a
cursor, or where two cursor paths branch, and each node keeps the cursors directly in it and a count beneath. A
cursor update moves one entry; a query by rectangle descends only nodes that overlap the rectangle. Because chains
of single-child nodes are compressed, depth is bounded by the number of cursors, not by the number of levels. This
is the same kind of occupied-cells-with-counts structure that section 03 builds for objects; if 03's index exposes a
point-item interface, `CursorIndex` should be an instance of it, not a second implementation.

**Routing a cursor update (a sender moved).** Once per tick, for each sender whose pointer changed:

```text
for level l in view_levels:                       // usually 1 to 3 levels, one per distinct view level
    c = cell_of(pos, l)                           // exact: 01's at_coarser_floor if l is coarser than pos.level,
                                                  //   at_level if l is finer (never rounds)
    for receiver in views[l][c]:
        if padded_region(receiver).contains_exact(pos at level l):  add receiver to the sender's new watcher set
diff new watcher set with the sender's previous set:
    entered -> send Peer (if unknown) + current Selection + current Ink catch-up; mark visible
    left    -> remove from receiver.visible (receiver also removes it on timeout, see below)
the sender's entry is encoded once and appended to each watcher's pending list
```

Cost per moved sender: `|view_levels|` hash lookups plus a few exact rectangle tests; each exact test costs O(digits
of the cell index): 01 keeps cells that fit 128 bits in plain integers, but at depth 100 a test compares about 17
machine words. With `|view_levels|` of 3 and 2000 senders all moving, that is 2000 x 3 = 6000 lookups per tick, or
180,000 a second. At an assumed 0.3 microsecond for small positions that is about 5 percent of a core; at an assumed
1 microsecond for positions 100 levels deep it is about 18 percent (both are estimates to measure, not measurements,
and a room whose cursors are all that deep is unusual). The worst case is hostile: sessions on many different levels
make `|view_levels|` large, up to the room size. Bound it: if `|view_levels| > 16`, routing keeps running but for
the extra levels beyond 16 (least populated first) it runs once every 4th tick, and the room's operator metrics
report it. Honest users sit on few levels at once because a shared surface is worked on at one place.

**Routing a view change (a receiver moved).** When a session's view changes (at most once per tick, from 03's
messages), query the cursor index with the new padded region and diff with `visible`:

```text
for level l in cursor_levels that are shallower than the view's level (cursors on coarser levels):
    r = region expressed at level l (exact, 01's at_coarser_floor on its corners, widened outward); look up the 1 to 4 cells it touches; test exactly
for the view's own level and all deeper cursor levels:
    descend the cursor index from the cells the region overlaps; prune nodes whose cell misses the region
diff with visible -> entered / left
```

Cost is the number of cells in the region (at most about 25) plus the cursors found, plus a descent whose length is
the number of occupied branching nodes, not the number of levels. With 2000 receivers all panning at once: 2000
queries per tick at an assumed 3 microseconds is 6 ms of the 33 ms tick, once. Typical rooms have a few people
moving at a time, so it is much less. A receiver who is zoomed far out (its region at a coarse level covers cells
holding very many deeper cursors) hits the cap below instead of receiving them all.

The result of both directions is symmetric in effect but computed separately, as requirement 7 demands: A sees B
when B is in A's region; B sees A when A is in B's region; neither implies the other.

**Caps and priority.** A receiver is sent at most `K` cursors (`K` set by the budget below, and never above the
visible-cursor cap in section 08's limits table; I propose 200, the number Figma uses). When more than `K` are in
the region, the order is: the followed person, then anyone selected or being dragged against the receiver's
selection (conflict avoidance), then most recently moved, then nearest to the receiver's own cursor (compared on the
receiver's screen). The rest are counted in `Omitted`, which the client shows as "+N".

### Send rates and the bandwidth budget

Constants: `tick` 33.3 ms; payload sizes from the wire section (`P(0)` = 19 bytes, so an entry is 27 bytes and a
`Pointer` is 28); per-message transport overhead for a TLS WebSocket frame carrying one message of this size is
taken as 66 bytes (TCP 20 + IP 20 + TLS 22 + frame 4 to 8, rounded up; section 03's own envelope adds 1 to 3 bytes
more, ignored here); for QUIC datagrams about 55 bytes. A snapshot of n entries at depth 0 is `7 + 27 n` bytes, plus
overhead once per message. A snapshot of more than 36 entries is split into `k = ceil(n / 36)` messages of at most
1000 bytes, each with its own header and overhead, so the per-snapshot cost is `S(n) = k x (66 + 7) + 27 n`. In this
section "KB" is 1000 bytes.

The budgets below are named limits, `presence_down_budget` (per receiver) and `presence_room_egress` (per room), and
the cursor cap is `max_visible_cursors`. Section 08's limits table owns their values; the figures used here (32
KB/s, 100 Mbit/s, 200) are what I propose for that table and what the arithmetic assumes.

**Uplink (one client sending).** `Pointer`: 28 + 66 = 94 bytes, 30 per second = 2.8 KB/s = 22.6 kbit/s while the
pointer moves. Nothing is sent while it is still, except a heartbeat every 2 s. A client drawing live ink or
dragging sends ink or drag messages in place of `Pointer`, not in addition, so it still sends about 30 presence
messages a second. This matters for section 08's presence rate limit (see "Limits and hostile senders").

**Downlink budget.** Each receiver has a presence budget `B` (default 32 KB/s, 256 kbit/s), and the room has a total
egress budget for presence (default 100 Mbit/s, 12.5 MB/s). The effective per-receiver budget is `B_eff = min(B,
room_budget / receivers)`. For a receiver with `n` cursors to show, the server picks:

1. the rate `r` = the highest of 30, 15, 10, 6, 5 snapshots per second (every 1st, 2nd, 3rd, 5th or 6th tick) with
   `r x S(n) <= B_eff`;
2. if even 5 per second does not fit, it keeps rate 5 and reduces `n` to the largest value with `S(n) <= B_eff / 5`.

Only entries whose sender changed since the last snapshot to that receiver are included (a **dirty** filter), plus a
refresh of every visible cursor at least once a second (this is also how a missed update on a lossy path heals).
Snapshots to different receivers are staggered by `receiver index mod divisor` so the ticks are even.

| Room | Visible cursors per receiver (worst case: everyone in everyone's view, all moving) | Rate chosen | Downlink per receiver | Server egress, presence | Uplink per client |
|---|---|---|---|---|---|
| 2 | 1 | 30 Hz | 30 x S(1) = 3.0 KB/s (24 kbit/s) | 6.0 KB/s | 2.8 KB/s |
| 20 | 19 | 30 Hz (S = 586 B) | 30 x 586 = 17.6 KB/s (141 kbit/s) | 352 KB/s (2.8 Mbit/s) | 2.8 KB/s |
| 50 (headline case) | 49 | 15 Hz (k = 2, S = 1,469 B; 30 Hz = 44 KB/s is over 32 KB/s) | 15 x 1,469 = 22.0 KB/s (176 kbit/s) | 1.1 MB/s (8.8 Mbit/s) | 2.8 KB/s |
| 200 | 199 | 5 Hz (k = 6, S = 5,811 B; 6 Hz = 34.9 KB/s is over 32 KB/s; 10 Hz = 58 KB/s) | 29.1 KB/s (232 kbit/s) | 5.8 MB/s (47 Mbit/s) | 2.8 KB/s |
| 2000 | 200 (cap) | room budget 12.5 MB/s / 2000 = 6.25 KB/s; 5 Hz needs S <= 1,250 B: n = 40 gives k = 2, S = 1,226 B; n = 41 gives 1,253 B, over | 6.25 KB/s (50 kbit/s) | 12.5 MB/s (100 Mbit/s) | 2.8 KB/s |

The 50-person row is the planning case. In a whiteboard few people move at once: with 10 of 49 cursors moving, a
snapshot holds 10 moving entries plus the refresh share (49 / 30 = 2), so 12 entries, S = 397 B, and the rate stays
at 30 Hz with 11.9 KB/s per receiver and 0.6 MB/s (4.8 Mbit/s) of server egress; the worst case of all 49 moving at
once drops to 15 Hz and a smoothing delay near 140 ms. In general the numbers are lower than the table, because the
dirty filter sends only senders that moved. With 10 percent of a 200-person room moving, a snapshot holds about 20
moving entries plus the 1 Hz refresh share (199 / 30 = 7 at 30 Hz): 27 entries, S = 802 B, 24 KB/s, which fits the
budget at 30 Hz instead of 5 Hz. These are arithmetic on assumed sizes, not measurements; the first benchmark
replaces them.

The 2000-person worst case shows what the design gives up: with everyone in one place and everyone moving, a
receiver sees 40 cursors at 5 Hz, delayed by about 400 ms (see smoothing). That is by design: the alternative is a
100 Mbit/s bill for cursors. The operator can raise the budgets. With depth, each entry grows by 2.5 bytes per level
(a cursor 10 levels down costs 52 bytes, not 27; 100 levels down, 277 bytes), which steps 1 and 2 account for
because they use the actual encoded size. At the depth cap an entry is 3.1 KB and a receiver would see about two
such cursors at 5 Hz inside 32 KB/s; this is why relative encoding is listed as the known remedy.

Server CPU for snapshots is copying: each sender's entry is encoded once per tick, and each snapshot is built by
gathering slices. For 2000 receivers at 5 Hz and 40 entries, that is about 2000 / 6 x 40 = 13,000 entry copies per
tick, 400,000 a second, at an assumed 30 ns each: about 1.3 percent of a core. For the 200 room at 5 Hz and 199
entries it is 200 / 6 x 199 = 6,600 per tick, 0.2 million a second, under 1 percent.

### Smoothing on the receiver

The receiver keeps, per visible remote cursor, a ring of the last 8 samples `(t, pos)`, where `t` is the sample time
in room time (`server_time - age_ms`).

**Clock.** The receiver estimates room time with the shared clock offset: section 03's `Ping`/`Pong` on the
WebSocket, answered outside the ordered change queue, with the minimum-delay sample of the last 8 (the estimate
itself is section 05's). Its accuracy only has to be steady, not exact: a constant error shifts all cursors equally
and cancels in `render_time`. The server converts a sender's samples to room time by estimating the sender's clock
offset as the minimum of `arrival - sender_time` over the last 64 messages (the standard minimum-delay filter), so
uplink jitter is removed from the sample times and only the sender's real cadence remains.

**Render time.** `render_time = room_now - D`. `D` is the interpolation delay:

```text
interval = measured mean gap between samples of this cursor, from the receiver's snapshot rate (33 ms at 30 Hz,
           200 ms at 5 Hz)
D        = clamp(2 x interval + 2 x jitter, 66 ms, 400 ms)      // jitter = running mean absolute deviation of arrival gaps
D changes by at most 1 ms per frame (slew), so a change in network quality never speeds cursors up visibly.
```

At 30 Hz and a steady connection, jitter is about 5 ms, so D is about 76 ms; this keeps two lost snapshots from
starving the buffer (Fiedler's rule is about 3 intervals, 100 ms; 2 intervals plus 2 jitters is a deliberate
reduction, because a cursor tolerates a brief hold better than a physics object). At 5 Hz, D is 400 ms (the clamp).
So the smoothing delay stays under the 100 ms only while a receiver is at 30 Hz; at lower rates the design trades
delay for bandwidth, on purpose.

**Interpolation.** Find samples `a` and `b` with `a.t <= render_time < b.t`. Interpolate with a cubic Hermite curve
between them, with tangents `(b - prev_a) / (b.t - prev_a.t)` at `a` (centered, if the sample before `a` exists) and
`(b - a) / (b.t - a.t)` at `b` (one-sided, because there is no sample after `b` yet). This is smooth in position and
nearly smooth in velocity, and costs a few multiplications. To keep the arithmetic safe at any depth, the receiver
does not interpolate absolute positions. It takes the newest sample as an anchor and keeps each older sample as a
small `[f64; 2]` offset from it in the units of the anchor's level (computed by exact subtraction, section 01's
`Position::sub` followed by `Delta::to_f64`); interpolates those small vectors; and adds the result to the anchor
once per frame with the exact step to get the draw position. If `to_f64` returns `Rel::Far`, or the `discontinuity`
flag is set, or the step is longer than half the receiver's view diagonal, the cursor jumps (no interpolation) to
the new sample. The last case makes a zoomed-out person's pointer, whose steps are huge in the receiver's scale,
appear where it is rather than sweep.

**Extrapolation (buffer empty).** If `render_time` is after the newest sample (late or lost snapshot), continue at
the last velocity for at most 100 ms, with the velocity decaying linearly to zero over that time, then hold still.
The limit is inside the 50 to 250 ms range where Fiedler found straight-line extrapolation tolerable. When the next
sample arrives, the error between the drawn and the true position is removed over the next 100 ms, not at once.

**Own cursor and live ink** are never delayed: the local pointer is drawn immediately by the local renderer.

**Delay as seen by the user.** One-way cursor lag = uplink latency (about RTT/2) + wait for the tick (mean 17 ms, at
most 33) + downlink (RTT/2) + D + any wait behind queued bulk data (see "Transport details"). At RTT 60 ms and 30
Hz: 30 + 17 + 30 + 76 = about 153 ms, which is inside the 200 ms target with 47 ms to spare for queueing.
design.md's earlier "no more than 100 ms" cannot be met by a design that buffers, so the target is restated. For
rooms of at most 25 visible cursors the server may forward a pointer update on arrival instead of waiting for the
tick (saves 17 ms mean); this is an optimization to take only if measurement says it matters.

### What a cursor looks like at a different depth

A cursor is drawn at the screen position of its exact world position, with a fixed screen size (the arrow does not
grow or shrink with the surface). Let `rel` be the ratio of the sender's zoom to the receiver's, computed from the
two `ViewScale` values (level difference x 10 octaves, plus the difference within the level).

| `rel` | Appearance |
|---|---|
| between 0.5 and 2 | Normal cursor and banner |
| 2 or more (the person is deeper, zoomed in on a spot inside my view) | Normal cursor, plus a badge on the banner: a down-arrow and the factor, "1,000x"; for more than 10^6, "10^n x" |
| 0.5 or less (the person is shallower, zoomed out) | Normal cursor, plus a badge with an up-arrow and the factor |
| 10^6 or more in either direction | The cursor is drawn at 60 percent opacity with a double-line badge, because the person cannot see what I am drawing at my scale and I cannot see what they point at |

The badge is the "note of how much deeper" from design.md. Text of the badge is part of the accessible description
of the cursor (section 07). The banner is placed to the lower right of the arrow and moves to the left or above when
it would leave the screen, so it stays readable at the edge.

A person far shallower than me can have a cursor inside my view whose real hand movement is a very large movement in
the world; the discontinuity rule above shows it where it is. A person far deeper than me whose cursor is in my view
moves it in tiny world steps: it sits nearly still on my screen, which is correct, and it is the reason the badge
exists.

### Selections of other people

A selection is shown while the receiver sees the owner's cursor, in the owner's color: a 2 px outline around each
selected object that the receiver has locally, and a small banner-colored tab with the owner's glyph. Ids that the
receiver does not hold are skipped. If `total > 32`, the owner's `bounds` rectangle is drawn dashed, with the count.
The server sends a selection record to a receiver when the owner's cursor enters the receiver's region and then on
each version change, never in the periodic refresh. The server does not route selections by object position: that
would need a rectangle index and a second routing path, and the cost is small. What is given up: a selection whose
owner's cursor is off the receiver's screen is not shown even if the selected objects are on it. The objects
themselves are, and a move of them arrives as a document change.

Viewers (role) send cursor and view (requirement 7 needs both) but not live ink, as section 08 specifies; I also
refuse `Selection` and `Drag` from viewers, because a viewer cannot change objects. The server drops those messages.

### Live ink

The pen's samples are collected by the client in the order they arrive, thinned and sent.

- **Thinning.** The points sent are a subset of the stored stroke's own points (the `i32` lattice of 2^-16 object
  unit plus pressure, defined by 02 and 06). The client drops a sample only if it lies within 0.15 screen pixel (at
  the sender's zoom) of the line from the last kept sample to the next one, the tolerance 06 uses for the stored
  stroke; since the next sample is not yet known, it keeps a sample if it is at least 0.5 pixel from the last kept
  sample, or the direction has turned by more than 5 degrees, or 16 ms have passed. A 240 Hz pen drawing at normal
  speed ends at about 60 to 120 kept points a second.
- **One stroke rule (owned by 06).** A stroke's level is the camera's level when the pen goes down and does not
  change while it is drawn. The tool ends the stroke and starts a new one (a new `ObjectId`, so `End` then a new
  `Begin`) when a point would lie 2^14 object units or more from the pen-down point on either axis. So every preview
  point relative to `Begin.first` fits an `i32` at 2^-16 of a unit, and the extent stays under 2^15 units. A delta
  between two points is just under 2^31 steps, so the receiver decodes `dx` and `dy` as `i64` and checks that each
  resulting point fits `i32`; a point that does not is a malformed message and is dropped. The preview relies on
  this rule, including for strokes drawn while the view auto-pans.
- **Batching.** Kept points are sent in `Points` messages every tick (33 ms), 2 to 8 points in each. The client also
  sends a batch sooner if 120 points have gathered.
- **Size.** A point is two zigzag varints and a pressure byte. A step of half a pixel is about 32,000 lattice steps
  at one pixel per unit (more when zoomed out), so each varint takes 3 bytes: a point is about 7 bytes. A message
  has about 12 bytes of header, so at 90 points a second the payload is about 90 x 7 + 30 x 12 = 990 bytes a second
  (7.9 kbit/s), 3.0 KB/s with transport overhead. `Begin` adds about 45 bytes once (at depth 0).
- **Routing.** Ink is content, not a cursor: a long stroke can be in view while its pen is not. The server keeps the
  bounding box of each live stroke, and every 3 ticks (10 Hz) tests each live stroke against each view region,
  exactly, by brute force. Live strokes per room are capped (`max_live_strokes`, a name for section 08's table; I
  assume 64), so with 2000 sessions the cost is 64 x 2000 = 128,000 rectangle tests per run, 1.3 million a second;
  at 100 ns each that is 13 percent of a core, and the cap on strokes and the 10 Hz rate are the lever to reduce it.
  A receiver that newly sees a stroke gets `Begin` and the stored points so far (thinned to at most 2000 points if
  longer).
- **Ink budget.** Ink counts against the receiver's budget `B_eff`. When over budget, the server first thins ink
  points (keeps every 2nd, then 4th) and then delivers ink for only the strokes nearest the receiver's cursor; the
  rest appear when committed. Delivery of ink in lower detail is safe because the preview is replaced by the final
  stroke.
- **Gaps.** `Points` carries `first_index`. A receiver that sees a gap (a message was dropped) draws a straight line
  across it; nothing is retransmitted. With 90 points a second, a lost 33 ms batch is about 3 points, and the final
  stroke is exact.
- **Drawing the preview.** The receiver builds the outline with 06's `OutlineBuilder::push` (the same function as
  for the finished stroke; `finish` equals a one-shot build, which 06 tests), on the points so far, into the overlay
  layer ("live ink" of the rendering section), in the sender's color tint only at the pen position (a small dot);
  the stroke itself is drawn in its own brush color so the preview matches the final.
- **Provisional anchor.** While drawing, the stroke's center is not known, but 06 anchors a finished stroke at the
  center of its bounding box. The preview therefore uses the pen-down `Position` (`Begin.first`) as a provisional
  anchor and the points as lattice offsets from it. For preview and final to be the same world points, 06 places the
  final anchor a whole number of 2^-16 object units from `first` when the pen lifts ( agreed with 06), so preview
  and final stroke coincide exactly.
- **The swap, without a flicker.** The preview is keyed by the stroke's `ObjectId`, which is the id of the finished
  object. Each frame the renderer checks, for each preview, whether the local copy of the document contains an
  object with that id. If it does, the preview is not drawn in that frame and the object is, in the same frame: one
  drawing replaces the other with no frame in which both or neither are visible. If `End` arrives and the object is
  not yet here, the preview freezes at its last points and waits up to 5 seconds for the object, then fades out over
  300 ms. If the object arrives and the preview never existed (the stroke was too short to bother, or out of view),
  nothing special happens. For the swap to be invisible, the finished outline must equal the preview outline to
  within a quarter of a pixel: the preview is built from thinned points and the object from all points, so the
  difference is at most the thinning tolerance of 0.15 pixel at the sender's zoom (above, and 06's), which is under
  the quarter pixel the swap test allows. A receiver at 1000 times the sender's zoom would see the difference, but
  it can see the stroke only through the part in its tiny view, and the swap there changes a corner at most.
- **The drawer disconnects mid-stroke (owner decision, 2026-11-12).** The stroke is not lost, and the server never
  commits a stroke on someone's behalf (it would be a change authored by a client that never sent it). What happens:
  - On other screens, when the server declares the connection lost (6 s without a message) it removes the live ink,
    and receivers freeze the preview for the time they have already waited and then fade it out over 300 ms. The
    stroke thus vanishes about 6 seconds after the last point.
  - On the drawer's device, the client keeps the stroke: it finishes it locally as an ordinary stroke (the pen lift
    commits it to the local copy and the offline queue, with the `ObjectId` it already announced), and saves the
    queue to local storage as soon as the loss is detected, not only at the lift.
  - When the connection returns, the queue sends the finished stroke as one `Op::Create`. Others then see the whole
    stroke appear at once as an ordinary new object, with no preview and no animation, however long afterwards
    (seconds to minutes, or after a restart of the app). It takes its place in the order by the normal rules, which
    is on top if nothing else was added after it was made on the drawer's device, and may be under objects others
    drew meanwhile.
  - If the device is destroyed before it saved anything, the stroke is lost; nothing the server holds could restore
    it.
- **Limits.** One live stroke per session; points kept by the server per stroke are capped at section 08's
  points-per-stroke limit (20,000, in section 08's table); live strokes per room at `max_live_strokes`. A stroke
  that exceeds the point cap still commits normally, only its preview stops growing.

### Object drag previews

Section 02 sends a drag's `Op::Set` of the `Xform` register on release and at most 5 times a second during a long
drag. Between those writes the other people would see the objects jump. The preview uses the presence channel, as
live ink does, at this depth of design (messages, size, rate; details are for the implementation):

- **Messages.** `DragMsg::Begin` names up to 32 of the dragged objects (the rest only counted, with a `bounds` box
  for all of them, as an `ExactBox`), `Move` carries the pointer `QPos` and the exact displacement `offset` (a
  `Delta`) since the drag began, and `End` says whether it was committed.
- **Size and rate.** `Begin` is about 6 + 16 n bytes, once. `Move` is about 12 bytes of header plus the pointer
  position (`P`, 19 bytes at the origin) plus an `offset` of 5 to 30 bytes for moves of a few screens: about 40 to
  60 bytes at depth 0, at up to 30 a second (15 under pressure, same as ink), so about 1.5 KB/s with transport
  overhead per dragging person. `Move` replaces `Pointer` while the drag lasts.
- **Routing.** Like a cursor: a receiver gets the drag while it receives the sender's cursor, so a preview follows
  requirement 7. Entering receivers get `Begin` and the latest `Move`.
- **Drawing.** The receiver draws each named object it holds, with its world `Placement` moved by `offset`
  (`Position::add_delta` on the placement's anchor), in the overlay layer and hides its document copy, in the
  sender's color outline. Objects it does not hold are skipped; for `total > 32` it draws the dashed `bounds`
  displaced the same way.
- **The swap.** On `End { committed: true }` the preview stays until the document copy shows a placement change for
  one of the named ids (the `Op::Set` of its `Xform`), then the preview is dropped and the object drawn at its new
  place in the same frame; if no change arrives within 2 seconds, the preview is dropped and the object is where the
  document says. On `committed: false` or a lost connection the preview disappears after 300 ms and the objects stay
  where the document has them. Nothing in the preview is ever written to the document.
- **What it depends on.** The preview moves the composed world placement. The final `Op::Set` writes the stored
  `Xform`: a `TopXform` for a top-level object, or, for a member of a group, a `ChildXform` obtained with 01's
  `localize`. Both are 02's; the preview needs neither. Objects of different levels in one selection move by the
  same displacement in world terms, which is why `offset` is an exact `Delta`.

### Timeouts

| State | Rule |
|---|---|
| Active | Input in the last 10 s. Sends Pointer at up to 30 Hz when it changes. |
| Idle | No input for 10 s to 2 minutes. The cursor is drawn at 50 percent opacity; nothing but heartbeats is sent. |
| Away | No input for 2 minutes, or the tab hidden for 30 s. The cursor is hidden; the who list shows "away". |
| Silent | No message for 6 s (3 missed heartbeats): the server stops forwarding the cursor and tells receivers to grey it. |
| Gone | Transport closed: cursor removed at once; the who list keeps the entry for 3 s so a page reload does not flicker. Silent for 30 s: removed (the same figure as Yjs's offline rule). |

A receiver also removes a cursor it has not heard about (no snapshot entry) for 3 s, which covers a lost `PeerGone`
on a lossy transport. Browsers throttle timers in hidden tabs, to once a second and, after some minutes, once a
minute (unverified; known from Chrome's documentation). A hidden tab therefore sends `Activity::Away` before it is
throttled, and the server uses a 150 s silence limit for an Away session instead of 6 s.

### Who is here, at scale

The who list is a panel (section 07); its data comes from here.

- For up to 50 sessions the server sends the whole list and then each join, leave or change, and the client shows
  all.
- Above 50, the list is virtual: the server sends only the total count and the first page (the followed person, then
  nearest to the viewer, then recently active, then by name), and answers `WhoQuery` with a prefix of a name, an
  offset and a limit. The panel scrolls through pages.
- The server sends list changes only to sessions that have the panel open (`WhoSubscribe { open: true }`). Others
  receive nothing but the count (once per 5 s). This prevents the join storm: 2000 people joining in a minute would
  otherwise send 2000 x 2000 = 4 million updates. Changes are batched to one message per second per subscriber.
- Two sessions of one person are shown once, with a device count.  An entry shows the glyph and color, name, state,
  and a "jump" button unless the person is not `locatable`.

### Jump to and follow

- **Jump.** The client sends `Jump { target }`. The server checks that the sender's role allows it, that the sender
  is within the jump rate limit of section 08's table (1 a second, 30 a minute), and that the target is `locatable`;
  then it replies `JumpTo { who, view }` with the target's view at that moment (read from the target's `SetView`
  state, never asked of the target). A refusal looks the same as a person who has left (08). The client animates the
  camera there (camera is section 01's; the animation is a straight move of the view center in log-scale, so a jump
  across many levels does not fly over empty space). The target's view reaches only the asker, once.
- **Follow.** `Follow { target }` needs the target to be `locatable`, as a jump does. It makes the server push
  `JumpTo`-form updates of the target's view at 15 Hz (only when it changed). Following is a server-pushed stream at
  15 updates a second, one per session, not repeated jumps; jumps keep their own rate limit. It reveals nothing more
  than repeated jumps would. The follower's camera eases toward each (critically damped, response 150 ms). Any
  manual pan or zoom by the follower ends the follow (a `PresenceNotice` confirms). The target's cursor is always
  delivered to a follower, regardless of the region test and the cap, as Figma does. Followers see the target's
  cursor delayed by D, and the view stream is delayed to match, so the cursor and the camera stay together.
- **Follower cap.** At most 50 followers per person; a 51st `Follow` is refused with a `PresenceNotice`. The cost is
  stream rate times message size times followers. A `JumpTo` carrying a `ViewMsg` is about 50 bytes of payload at
  depth 0 (two positions and `pixel_log2`) and about 120 bytes with transport overhead, so one follower costs 15 x
  120 = 1.8 KB/s of server egress and 50 followers cost 90 KB/s (0.72 Mbit/s) for one person, encoded once and
  copied. The followed cursor is also exempt from the visible-cursor cap, so a follower never loses it; 50 of them
  add 50 entries to that person's fan-out, not to anyone else's. Without a cap, one person followed by a
  2,000-person room would cost 3.6 MB/s (29 Mbit/s) for one camera. The limit is named `max_followers_per_person`
  for section 08's table. A "present" mode that broadcasts one person's view to an audience over 50 is out of scope
  for now (owner decision, 2026-11-12).
- **Follow notice.** The followed person sees who follows them (owner decision, 2026-11-12): the server sends the
  target a `PresenceNotice` when a follow starts and ends ("Ana is following you"), and the who list shows the names
  of followers to the target.
- **Switching it off for invite-link people.** An owner can turn jump and follow off for people who joined through
  an invite link (owner decision, 2026-11-12; the surface setting is section 08's `invited_locatable`). The
  session carries `via: Option<LinkId>` from the token. When the setting is off it works in both directions:
  `Jump` and `Follow` from a session with `via` set are refused with a `PresenceNotice`, and `Jump` and `Follow`
  aimed at such a session are refused too, as if it had opted out. The person's cursor, name and place in the who
  list are unaffected.
- **Privacy position and the cost of an opt-out.** The high-level design accepts that jump reveals where a person is
  working. Section 08 specifies the opt-out as a per-session `locatable` flag (default from the token's `loc` claim,
  otherwise on), and this design implements it as `ToServer::Locatable { on }` and `PeerInfo.locatable`. When off,
  the server refuses `Jump` and `Follow` aimed at that session, the who list shows no jump action for it, and its
  view is never put in any message; its location is visible only through its cursor, which by requirement 7 anyone
  who is looking at that place already sees. What an opt-out costs: (1) the person cannot be found from the list,
  which defeats the common case of a teacher or presenter being followed, so presenters would leave it on; (2) it
  adds a state to test and explain; (3) the owner's setting for invite-link people above is separate and does not
  depend on it. The engineering cost is small (one flag, three checks, one test). Jump and follow are on by default
  and each person can opt out (owner decision, 2026-11-12).

### Transport details

- **First version: one WebSocket.** Presence is the `Presence(Bytes)` message of section 03 on the session's one
  socket; there is no second connection, credential or timeout. Each presence message is one binary WebSocket
  message. Compression (permessage-deflate) is off: messages are small and compression adds delay and CPU.
- **Priority class.** The server's writer for a connection has a presence queue that holds the newest state only (a
  snapshot being built replaces an unsent one), and it is served before every other class (section 03 lists the
  others: control and `Ack`, `Apply`, bulk view fill). The server never queues a backlog of presence.
- **Pacing of bulk data (owned by 03).** The server keeps a connection's unsent bytes below `U`, the amount the
  connection drains in about 20 ms, clamped to 4 KiB to 64 KiB (`TCP_NOTSENT_LOWAT` or the writer's own count; the
  server estimates the drain rate from how fast its writes complete). Bulk data (section 03's `Enter` chunks and any
  other large message) is written in chunks no larger than `U` and never larger than 16 KiB, and a chunk is written
  only while unsent bytes are under `U`. Presence is written whenever the socket accepts it, ignoring `U`, but its
  queue holds one snapshot per receiver at most. 20 ms is the first value the spike tries.
- **Worst-case wait behind bulk data.** A presence message written now can wait behind every unsent byte plus the
  chunk being written: at most `A = U + chunk`, and the wait is `A / link speed` on the receiver's bottleneck link.
  Where `U` is not clamped, `U / speed` is 20 ms and `chunk / speed` is at most 20 ms, so the wait is **at most
  about 40 ms at any link speed** between the 4 KiB floor (about 1.6 Mbit/s) and the 16 KiB chunk limit (about 6.5
  Mbit/s); above that the chunk limit and the 64 KiB cap make it smaller. The wait in milliseconds, with two older
  rules for comparison:

  | Downlink | 0.25 Mbit/s | 0.5 | 1 | 2 | 5 | 10 | 20 | 50 | 100 |
  |---|---|---|---|---|---|---|---|---|---|
  | **Adaptive, 20 ms drain (the rule)** | 262 | 131 | 66 | 40 | 40 | 33 | 27 | 13 | 6.6 |
  | Adaptive, 40 ms drain (comparison only) | 262 | 131 | 80 | 80 | 66 | 53 | 33 | 13 | 6.6 |
  | Old fixed 64 KiB limit (comparison only) | 2621 | 1311 | 655 | 328 | 131 | 66 | 33 | 13 | 6.6 |

  Below the floor the bound is lost: with `U` = 4 KiB and a 4 KiB chunk, `A` = 8 KiB = 65,536 bits, so the wait is
  262 ms at 0.25 Mbit/s, 131 ms at 0.5 and 655 ms at 0.1. The floor exists so a chunk is never too small to be worth
  a frame header and a flush. On links below about 1.6 Mbit/s cursors lag during a bulk load; there the server sends
  fewer objects first by raising `eff_min_px` (section 03), which shortens the load.
- **What the bound means for the 200 ms target.** At RTT 60 ms and 30 Hz the cursor already takes about 153 ms
  (previous section), leaving 47 ms for queueing. The 40 ms bound fits: the target holds during a bulk load above
  about 1.4 Mbit/s (below that the 4 KiB floor dominates: `65,536 bit / 0.047 s`; the 1.6 Mbit/s of the previous
  paragraph is where the floor starts to apply). The 40 ms drain rule would give 80 ms and hold the target only
  above 13.9 Mbit/s, which is why 20 ms was chosen. The cost of a smaller unsent queue matters for throughput only
  if the drain-rate estimate is poor, since the congestion window, not the unsent counter, governs how much is in
  flight. Bytes sent but not yet acknowledged add delay only if a buffer in the network fills (bufferbloat); I did
  not model or measure that (unverified). These are worst cases: the average wait behind a chunk in progress is
  about `U + chunk / 2`, and outside bulk loads the wait is zero.
- **Fallback trigger: a second WebSocket.** If in the spike the cursor lag during a bulk load still exceeds 200 ms
  with the 20 ms rule on a link above the floor (about 1.6 Mbit/s) that the product must support, presence moves to
  its own socket: path `/presence` on the same host, authenticated by the same session credential as the sync
  socket, with its own queue and no bulk data on it. The message format does not change; only the framing around
  `Presence(Bytes)` is removed, and section 03's `Hello`/`Welcome` (the connection's own messages) must say how the
  second socket attaches to the session. This costs a second handshake, a second set of timeouts and a second place
  where a client can be half-connected (see the failure table). It does not remove the TCP stall from a lost
  segment.
- **Back-pressure.** Clients send `Heartbeat.last_snapshot` at least every 250 ms while receiving. The server keeps
  `inflight_bytes` = bytes sent in snapshots numbered after `last_acked`. If `inflight_bytes` exceeds `max(16 KB, 4
  x B_eff x RTT)`, the server skips that receiver's snapshot this tick (the state stays in the table, the next
  snapshot carries it) and raises its `rate_divisor`; when inflight is below half the limit for 1 s it lowers it
  again. TCP's own buffers are not visible to the server, so the figure is conservative: the point is to stop
  writing when the client falls behind, not to model the connection.
- **Drop order** when a receiver is over budget: (1) cursors at the end of the priority order; (2) a lower snapshot
  rate; (3) ink thinning; (4) delayed selection updates. Never dropped: `PeerGone`, the followed person's cursor,
  answers to `Jump`, `PresenceNotice`.
- **Later: WebTransport datagrams.** The same messages in datagrams (snapshot split into datagrams of at most 1000
  bytes, each with whole entries), with the WebSocket kept as the fallback and chosen when a WebTransport handshake
  fails or times out in 2 s. The path is added when the benchmark below shows that stalls on the WebSocket (a lost
  segment holding presence for an RTT or more) happen often enough to matter. The message format needs nothing new
  for this: every message is independent, entries carry sample times, losses heal through the 1 Hz refresh, and
  `Points.first_index` handles ink gaps. Native clients can use `wtransport`'s client.
- **No WebRTC data channels** (see options).

### Limits and hostile senders

- The server writes `who` itself; a client can never claim another's index. Every float in a message must be finite
  and every `Position` within the depth cap (01; checked on the raw bytes before any number is built), or the
  message is dropped and counted.
- **Rates and caps are section 08's**. This section only names what it needs in that table: the presence message
  rate per session (60 a second with a burst of 120); the jump rate; the points per stroke; and four limits I
  propose adding: `max_live_strokes` per room, `max_visible_cursors` per receiver, `presence_down_budget` per
  receiver and `presence_room_egress` per room. Over the limit, presence is dropped without reply (08). The design
  keeps a drawing or dragging session at about 30 messages a second so the 60-a-second figure holds with room to
  spare: ink and drag messages replace `Pointer`, and `Selection`, `Activity`, `Heartbeat` are rare.
- Message sizes are checked before decoding (1000 bytes, or the depth-dependent size of one position for the rare
  deep message) and decoders never allocate more than they were told (a declared length is checked against the bytes
  present).
- `Selection.ids` are only ever used as keys into the receiver's own document; an id it does not hold is ignored.

## Failure modes

| Failure | What happens |
|---|---|
| A presence packet is lost (datagram path, later) | The next snapshot replaces it; interpolation holds or extrapolates up to 100 ms; the 1 Hz refresh repairs any lost entry. |
| TCP stalls the socket (a lost segment) | Cursors freeze, then extrapolate and stop after 100 ms; when the stall ends, the newest state arrives first (the server never queued a backlog) and the cursor eases to it over 100 ms. |
| The client falls behind | `inflight_bytes` rises, the server skips snapshots, the rate drops; ink thins. |
| The connection drops (one socket) | Sync and presence go together. The client reconnects and sends `PresenceHello` with `resume`; others see the cursor grey after 6 s and removed after 30 s if it does not return; if it returns within 10 min it has the same color. |
| Fallback only: presence socket drops but the sync socket lives, or the reverse | A half-connected session: the server treats it as Silent for routing after 6 s without a presence message (or without a view update) and the client reconnects the missing socket. |
| A bulk load on a slow link | Cursors lag by the wait in the transport table, up to about 40 ms above 1.6 Mbit/s with the adaptive limit and more below the 4 KiB floor (262 ms at 0.25 Mbit/s); interpolation extrapolates and eases; the lag ends with the load. |
| Room process restarts | Presence starts empty. Clients reconnect and send `PresenceHello` and `Pointer`; cursors reappear in under 2 s. Colors may change. |
| Many cursors in one place | Cap, `Omitted` count, 50-banner limit. |
| Two tabs of one person | Same color, one list entry, two cursors. |
| A cursor on a level the receiver has not been sent | Cursor display does not need the object data; it needs only its `Position`. It is drawn even where the receiver has no content yet. |
| Position arithmetic out of range (`Delta::to_f64` returns `Far`) | The cursor jumps to the new sample. |
| Clock offset estimate jumps | `D` slews 1 ms a frame, so a correction of 100 ms takes 100 frames; an offset step larger than 500 ms resets all buffers. |
| A hostile name | Sanitized, truncated, drawn as plain text; a confusable of another present name is disambiguated. |
| The drawer disconnects mid-stroke | See Live ink. |
| Receiver's view changes during interpolation | Interpolation is in world offsets, converted at draw time, so the cursor follows the world, not the screen. |

## How it is tested

| Rule | Proved by |
|---|---|
| A cursor in the receiver's exact view is delivered; the receiver shows exactly those in its exact view (replaces design.md's "delivered exactly") | Random sessions with views at levels -5 to 100, random cursors, 100,000 cases: the indexed result equals a brute-force scan over all pairs, for the padded region (server) and for the exact view (client draw list). Includes cursors exactly on edges and on cell borders, and across the level boundary |
| Cursors stay correct while the view moves | A scripted receiver panning at 0.5, 1 and 2 times the pad per 100 ms with cursors crossing its edge: every cursor in the exact view is present when the pan is below the pad per `SetView`; above it, a missing cursor appears within 100 ms plus half the simulated round trip, and the measured worst case is reported |
| Routing is per direction | A scripted pair where A's view contains B's cursor and B's view does not contain A's: A receives B and B does not receive A, and back |
| The index agrees after any sequence of moves, joins and leaves | A model test: random operations on the index and on a plain list; compare after each |
| Deep and shallow views see each other correctly | Scripted scenarios with the sender 1, 10, 100 levels deeper and shallower than the receiver, against a reference big-number implementation |
| A snapshot is at most 1000 bytes and decodes to what was encoded | Round-trip and fuzz tests of every message; truncated and oversized input is rejected without allocating |
| The rate and cap follow the budget | Simulated rooms of 2, 20, 200, 2000: assert chosen rate and n against the table, and that room egress stays under the budget |
| Dirty filter and refresh | A still cursor produces one entry per second; a moving one every snapshot |
| Smoothing is smooth and bounded | Recorded cursor traces with 0, 2 and 10 percent loss and 50 ms jitter: the drawn path has no step larger than 1.5 times the true step per frame, extrapolation never exceeds 100 ms, a teleport flag snaps |
| Interpolation does not depend on depth | The same trace at levels 0 and 100 gives the same draw offsets to within 1/16 pixel |
| Color assignment | The first 14 sessions in a room all differ in color; a rejoin gets the same pair; with 100 sessions, no pair has more than 2 holders; the palette passes the pairwise distance test under normal vision and the three simulated deficiencies; the black/white text choice meets 4.5:1 for every slot |
| Names | A corpus of hostile names (bidi overrides, zero-width joiners, stacked combining marks, very long, confusables, empty, markup, emoji sequences) produces bounded, plain, collision-free banners; no person can display a name equal to another present person's |
| Live ink swap | A scripted drawer and viewer: for every frame between the last `Points` and the object arriving, exactly one of preview and object is drawn (no frame with none or both); the outlines differ by at most a quarter pixel; `End` before the object, and the object before `End`, both work |
| Drawer disconnect | Cut the drawer's connection mid-stroke: other screens fade the preview within 7 s and no object appears; the drawer's device keeps the stroke, also across an app restart; reconnect: the whole finished stroke appears once on every screen as an ordinary object; the server never creates a stroke itself |
| Timeouts | Fake clock tests for Active, Idle, Away, Silent, Gone, and hidden-tab behavior |
| Back-pressure | A receiver that reads slowly: the server's queue for it never holds more than one snapshot of unsent presence |
| Jump and follow | A session with `locatable` off refuses both and a refusal looks like a departed person; the jump rate limit holds; the target receives the follow notice; a follower gets the followed cursor outside the region and cap; manual pan ends a follow; no message to another client contains a view that was not requested (scan of all outputs, shared with 08's N87) |
| Drag preview | A scripted dragger and viewer: the viewer's preview follows `Move`; after the `Set` arrives exactly one of preview and object is drawn in each frame; a lost `End` times out in 2 s; nothing from a preview reaches the document |
| One socket keeps cursors timely | Throttled-socket test with a 50 MB view fill running: the wait is measured at 0.25, 1, 5, 20 and 100 Mbit/s with the 20 ms drain rule (and, for comparison, 40 ms) against the transport table (bound about 40 ms above the 4 KiB floor); cursor lag stays under 200 ms where the table says it can; the result decides whether the second-socket fallback is needed |
| A session sends at most about 30 presence messages a second | Drive a drawing session and a dragging session at full rate: the count stays under 60 a second (08's limit) |
| Deep positions | A cursor at depth 100 and at the cap: the message decodes, the budget formula uses the real size, and an over-cap position is rejected from the raw bytes |
| Limits | Rate-limit, size, non-finite number and impossible-position cases are dropped and counted |
| Who list at scale | 2000 simulated joins: no more than the count message reaches closed panels; open panels get at most one batch a second |
| Benchmarks (not pass/fail rules) | Routing cost with 2000 senders and receivers at 1, 3 and 16 view levels; snapshot build cost; end-to-end cursor lag at RTT 20, 60 and 200 ms with and without a bulk load; WebSocket stall frequency on real networks (the data that decides when to add datagrams) |

## Dependencies

| Crate | Version checked | License | Maintenance | Use |
|---|---|---|---|---|
| `wtransport` | 0.7.2 (updated 2026-08-11; 1.2 million downloads) | MIT OR Apache-2.0 | Active | Later: WebTransport datagrams on server and native client. Not needed for the first version |
| `unicode-segmentation` | 1.13.3 (updated 2026-06-01) | MIT OR Apache-2.0 | Active | Grapheme counting for name limits |
| `unicode-normalization` | 0.1.26 (updated 2026-10-16) | (MIT OR Apache-2.0) AND Unicode-3.0 | Active | NFC and compatibility folding of names. The Unicode-3.0 term covers the data tables; it is a permissive license, but the project should confirm it against its license policy |
| `unicode-security` | 0.1.3 (updated 2026-11-02) | MIT/Apache-2.0 | Active | Confusable skeletons for name collisions |

Not used: `webrtc` 0.21.1 (rejected, above); `perfect-cursors` (TypeScript; the algorithm here is its own, in Rust);
Yjs awareness (JavaScript; its timeouts informed the numbers). The wire format is written by hand with a small
varint and reader/writer module, no serialization crate, so byte counts are exactly as stated (section 03 mentions
postcard for its own messages; presence is carried inside as opaque bytes, so the two choices do not conflict). The
WebSocket crate is whatever section 03 chooses; the first version adds no transport dependency to presence. Versions
come from crates.io on 2026-11-11; their health beyond the update date was not examined.

## What this assumes of other sections

- **01.** `Position` with its canonical serialization (19 bytes at the origin, about 2.5 bytes more per level, about
  3.1 KB at the cap), `Position::sub` and `Delta::to_f64(unit_level) -> [Rel; 2]` with `Rel::Far`, `at_level` and
  `at_coarser_floor` for exact cells, `QPos`, `Position::add_delta`, `encode_delta` and `decode_delta`,
  `check_cap_bytes`, `ExactBox`, `ViewScale { level, q }`, and `ExactRect::contains` and `grown` on the view's
  rectangle (`ViewRect::exact_rect`).
- **02.** A client can choose the `ObjectId` of a stroke before the pen lifts, and the committed stroke uses it.
  `ObjectId` is at most 16 bytes. 02 sends a drag's `Op::Set` of the `Xform` register on release and at most 5 times
  a second during a long drag; the preview in this section covers the time between. The stored stroke is 06's
  `PackedPoints` (the `i32` lattice at 2^-16 plus pressure), and live ink uses the same lattice.
- **03.** The server's presence module is told every session's current view (and each change) in-process from
  `SetView` (a `ViewMsg`), sent at least every 100 ms while the view moves and unpadded, and the server pads it by
  1/8 for cursor routing. `Presence(Bytes)` is a priority class served before other classes, with the adaptive
  unsent limit and chunks (4 to 64 KiB, chunks at most 16 KiB). `Ping`/`Pong` give the clock offset. The object
  spatial index offers a point-item interface that `CursorIndex` can reuse. If the fallback is ever used, 03's
  handshake says how a second socket attaches to a session.
- **06.** An outline function that builds a stroke from a point list incrementally, identical for preview and final;
  a way to draw the cursor, banner, badges, glyphs, selection outlines and the dashed bounds; the thinning tolerance
  agrees with the stroke's own smoothing. `OutlineBuilder::push` and `finish` as in 06, and the final stroke's
  anchor (the center of its bounding box) placed a whole number of 2^-16 object units from the pen-down point
  `Begin.first`, so preview and final points are the same world points (agreed with section 06).
- **07.** Pointer events with an activity signal and hover state, hidden-tab events, the who-list panel and its
  accessibility description, tool ids; one socket in `Transport`.
- **08.** Which roles may send which messages (viewers: cursor and view, no ink); the presence message rate (60 a
  second, burst 120); the jump rate; the display-name limit; the token's user id as `user_key`; `via:
  Option<LinkId>` on every session; the per-session `locatable` flag with its `loc` claim; the follow notice; the
  owner setting that turns jump and follow off for people who came through an invite link; and a place in the limits
  table for `max_followers_per_person` (50), `max_live_strokes`, `max_visible_cursors`, `presence_down_budget` and
  `presence_room_egress`, with a room cap on sessions.

## What this changes in docs/design.md

- **"One WebSocket per client"** is kept for the first version, with presence as a priority class and paced bulk
  data. My earlier proposal of a second socket is the fallback, with a measured trigger. The cost of keeping one
  socket is a cursor wait of up to 80 KiB of link time during a bulk load (table under "Transport details").
- **"Delivered exactly when inside the view"** (the Presence section and the test table) becomes: delivered when
  inside the view padded by 1/8, shown only when inside the exact view. Reason: the view and cursor messages cross,
  so an exact server test makes cursors pop in late at the edge, and the receiver needs a sample or two to smooth.
  The visible rule is unchanged.
- **"A remote cursor lags by no more than 100 ms on a good connection"** (Targets) cannot hold with smoothing: lag
  is about RTT + 17 ms + D, about 150 ms at RTT 60 ms. Proposed target: under 200 ms at RTT 60 ms in rooms up to 25
  visible cursors; the delay added by smoothing itself under 100 ms.
- **"If a connection is slow the server sends only the newest state"** is kept and made concrete (newest-state
  queue, skipping snapshots when the client's unacknowledged bytes grow).
- **"Palette is finite; names disambiguate past it"** is kept and made concrete: 14 colors x 4 glyphs = 56 pairs.
- **"The server assigns each user a color unique among those present"** holds up to 56 sessions in a room; beyond
  that it cannot be true with any finite palette, and the design says so.
- **"Presence carries the view"** is changed: the view travels once, in `SetView`; presence carries only the
  cursor's scale.
- **Live drag previews** are added to presence (section 02 relies on them).

## Questions for the owner

1. **Cursors in big rooms.** For rooms over the planning figure of about 50 people: is a cap of 200 visible cursors,
   50 banners and a low update rate acceptable as the product behavior? Should people be able to switch off other
   people's cursors for themselves (as in Miro)?
2. **Choosing a color.** Should people be able to choose their color, which makes uniqueness weaker and shifts the
   problem to who wins a clash? This design assigns colors only.
3. **Presence cost.** The default budgets (32 KB/s per receiver, 100 Mbit/s per room) are operating costs for
   whoever runs the server. Are they right as defaults for the first version? At 50 people the worst case needs 22
   KB/s per receiver and 8.8 Mbit/s per room, far below both.

## Sources

- Figma, How Figma's multiplayer technology works:
  https://www.figma.com/blog/how-figmas-multiplayer-technology-works/
- Figma Help, cursor and participant limits: https://help.figma.com/hc/en-us/articles/1500006775761
- Miro Community, how many users on a board:
  https://community.miro.com/ask-the-community-45/how-many-users-can-be-on-a-board-at-once-130
- Miro Community, laggy board with 200 users:
  https://community.miro.com/ask-the-community-45/laggy-board-with-200-users-4449
- Liveblocks client API (throttle, lostConnectionTimeout):
  https://liveblocks.io/docs/api-reference/liveblocks-client
- Yjs awareness: https://docs.yjs.dev/api/about-awareness
- tldraw collaboration docs: https://tldraw.dev/docs/collaboration
- perfect-cursors: https://github.com/steveruizok/perfect-cursors
- Gaffer on Games, Snapshot Interpolation: https://gafferongames.com/post/snapshot_interpolation/
- Valve, Source Multiplayer Networking (not readable, figures unverified):
  https://developer.valvesoftware.com/wiki/Source_Multiplayer_Networking
- MDN, WebTransport datagrams: https://developer.mozilla.org/en-US/docs/Web/API/WebTransport/datagrams
- caniuse, WebTransport: https://caniuse.com/webtransport
- crates.io: https://crates.io/crates/wtransport, https://crates.io/crates/webrtc, https://crates.io/crates/quinn,
  https://crates.io/crates/unicode-segmentation, https://crates.io/crates/unicode-normalization,
  https://crates.io/crates/unicode-security
- wtransport documentation: https://docs.rs/wtransport/latest/wtransport/
- Okabe and Ito, Color Universal Design: https://jfly.uni-koeln.de/color/

Claims left unverified, collected: Valve's 100 ms default interpolation; tldraw's presence rate and timeouts; Yjs's
15 s renewal; Liveblocks presence size limit; Figma cursor-chat length for FigJam; Miro figures (community
statements); Okabe-Ito hex values (from memory); the CVD distance thresholds and the 14-slot palette passing them;
browser datagram size limits; Chrome's timer throttling in hidden tabs; every performance figure in microseconds and
percentages of a core (assumptions to benchmark); the sizes `P(d)`, which depend on section 01's encoding;
ColorBrewer and Tableau color-vision behavior; the Machado 2009 matrices (from memory).
