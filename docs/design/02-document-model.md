# Document model, merge and undo

Status: detailed design, 2026-11-11, proposal.

## Scope

This section defines what a surface contains and how concurrent changes to it merge:

- the object kinds and every field of each, and the merge rule for each field;
- object ids, the clock that orders writes, order keys, groups and reparenting;
- text content and rich-text formatting;
- delete, tombstones and when they are collected;
- per-user undo and redo;
- stroke immutability and erasing;
- templates and libraries;
- schema versions and migration;
- the programming interface other sections call (`Doc`);
- how each candidate CRDT library would hold this model when a client has only a partial copy, with a
  plan for the spike.

It leaves these to other sections:

| Topic | Section |
|---|---|
| The `Position` type, exact arithmetic, levels, size band, depth cap, `Linear`, `RelOffset`, `compose`, `localize`, `relevel` | 01 |
| Wire protocol, view subscriptions, spatial index, storage layout, snapshots, named versions, sequencer throughput | 03 |
| Live ink, cursors, drag previews (presence) | 04 |
| Blob store, image and video files, playback clock, fonts | 05 |
| Stroke point format, outline, brush engine, drawing and hit-testing of cuts, text layout | 06 |
| Tool behavior, selection UI, text entry | 07 |
| Roles, the table of limits, rate caps, invite links | 08 |

`Position` is section 01's type: a level plus one fixed-point integer per axis, about 19 bytes at the origin and
about 2.5 bytes more per level. This section treats it as opaque and uses only the operations listed under "What
this assumes of other sections".

## The problem

The high-level design (`docs/design.md`) says the document is a map from object id to object, with last-writer-wins
properties, fractional order keys, a text sequence and delete markers, and that each client holds only a partial
copy. Four facts make this harder than "put it in a CRDT library".

1. **The document can be much larger than any client.** The design's targets include a 10,000-object view and a
   100,000-stroke client cache. The first server version keeps a whole surface in memory, capped at 2 million
   objects (section 03), so take 2 million objects as the planning size. A stroke of about 200 pen samples is
   roughly 1 KB once packed (an estimate; the format is section 06's). The fixed part of each object is roughly 110
   bytes near the origin: id 16 bytes, kind, creator, ordinal, order key about 6 bytes, parent link, a `Position` of
   19 bytes plus 2.5 bytes for each level away from the origin, and stamps of 8 bytes each. Section 03 gives the one
   estimate of what a surface of that size costs in server memory, from this header size, a typical stroke body and
   its index entry; it is far more than any client should hold, and a client sees well under 1% of it. A client must
   never need the rest.
2. **A general-purpose CRDT document is a whole-document object.** To the best of this section's research (details
   under "What others do"), Loro, yrs and Automerge sync and load a document as a unit. None documents a way to hold
   or sync a chosen subset of the containers or objects inside one document.
3. **The server reads, validates and orders every change.** Roles, size limits and "no cycles in the tree" must be
   enforced before a change is accepted, not repaired afterwards. A library update that arrives as an opaque,
   already merged blob is awkward to inspect or refuse one operation at a time.
4. **Users expect undo to be theirs.** "Undo reverses my changes and leaves everyone else's alone" has to stay exact
   when the other person has moved, restyled or deleted the same object.

This section therefore designs a small purpose-built object map, sequenced by the server, with a CRDT library used
only for the text inside text objects (owner decision, 2026-11-12). Which library edits text is still open, and the
spike under "How the model sits on the candidate libraries" compares them; the spike also still checks the finding
that no library syncs part of a document. The reasoning is under "Options considered".

## What others do

**Figma.** Evan Wallace's 2019 post says the server keeps, for each property of each object, the latest value any
client sent. A conflict happens only when two clients change the same property of the same object, and the server's
order of arrival settles it, so no timestamps are needed. Each client puts its own client id inside the object ids
it makes, so objects can be created offline without asking the server. Deleting removes the object's data from the
server and keeps a copy in the deleting client's undo buffer. A child stores a link to its parent; concurrent moves
can form a cycle, and the server rejects the update that would. A second post describes fractional indexing for
ordering siblings, with arbitrary-precision strings, and names two problems: index length can grow, and concurrent
inserts into the same gap can interleave. The post says concurrent identical indices are resolved by the server. On
undo, the post's rule is that undoing several steps and redoing them back to the present must leave the document
unchanged, and that an undo changes the redo history. On reconnect the client downloads a fresh copy and re-applies
its offline edits on top. This is the closest match to Everink's constraints and the design below follows it, with a
different delete and undo model. (Sources 1, 2. These posts date from 2019 and Figma's current system may differ;
unverified.)

**tldraw.** A `Store` holds typed records with a schema, produces and applies diffs of records, and has scopes
(document, session, presence) so that only document records are persisted and synced. The sync server
(`TLSocketRoom`) holds the authoritative copy, and validates documents and runs schema migrations so that clients on
different versions can work together; the server must be updated before the clients. The documentation pages read do
not describe per-property conflict handling; that tldraw merges per property is unverified. (Sources 3, 4.)

**Excalidraw.** Each element carries a `version` counter and a random `versionNonce`. When a remote element meets a
local one, the local one is kept if it is being edited right now, or if its version is higher, or if versions are
equal and its nonce is lower or equal; otherwise the remote one wins. Elements are then sorted by fractional index,
and duplicate indices are repaired. This is last-writer-wins on a whole element with no server order, which is
simple but loses the other person's concurrent change to a different property of the same element. (Source 5, read
from the source file.)

**Loro** (crate `loro` 1.17.1, MIT, updated 2026-10-22). Offers a last-write-wins map, a movable list, a movable
tree with optional fractional-index sibling order (with a jitter setting to reduce collisions, which increases
document size), Fugue-based text with rich-text marks, an `UndoManager`, and shallow snapshots. Details used below:

- Concurrent tree moves that would create a cycle are detected and prevented. The docs warn that fractional indexing
  "has interleaving issues but preserves relative ordering".
- `UndoManager` affects only the local peer's operations and never reverts remote ones. Its default is no merging of
  steps and 100 undo steps. Undoing the deletion of an ordinary map child creates a new container with a new id, so
  concurrent edits to the old one are not redirected to it; "mergeable" children keep their identity.
- A shallow snapshot drops history before a point and keeps current state; updates from before that point cannot be
  imported. This shrinks history, not the set of objects. Recent releases (1.13.x) add lazy loading of snapshot
  state that is still stored as one document, and fix bugs in it, which shows the area is still moving.
- Loro's own docs describe rich text as following Peritext's criteria (stated in a search result; not read in full).
- I found no documented way to export or import the operations of a chosen subset of containers. That absence is
  unverified; the spike must test it.

(Sources 6 to 10.)

**Yjs and yrs** (crate `yrs` 0.28.1, MIT, updated 2026-11-04; a 0.x version). A `Y.Doc` can be embedded in a map as
a subdocument. By default a subdocument is empty until `load()` is called, each has a `guid` that acts as its room
name, and the sync provider (not Yjs itself) syncs it; `destroy()` frees its memory. Deleted content is discarded
when garbage collection is on, leaving a length-only marker. `UndoManager` tracks transactions by origin and merges
steps within a capture timeout (500 ms by default in Yjs); how it treats remote changes is not stated on the pages
read (unverified). The Peritext write-up states that Yjs's formatting model has an overlapping-format anomaly that
Peritext avoids. (Sources 11 to 14.)

**Automerge** (crate `automerge` 0.12.0, MIT, updated 2026-09-16). Concurrent writes to a map key are won
deterministically by operation id (counter first, then actor id), not wall-clock time, and the losing values stay
readable as conflicts. Automerge 3 cut memory use by over 10 times by keeping history compressed at runtime. The
documents read do not mention partial loading of a document; Ink & Switch's sync-protocol notes treat the whole
document as "the unit of sharing" and describe apps working around it with many linked documents, with a
per-document sync cost that grows when there are thousands (from search-result excerpts; not read in full). Whether
Automerge has built-in undo or rich-text marks was not checked; unverified. (Sources 15 to 17.)

**Diamond Types** (ISC). Plain text only in the released form; a branch for other types is in progress, and its own
README calls the published crate out of date. Not a candidate for the object map. (Source 18.)

**Move operation for trees.** Kleppmann et al. give a replicated tree where every move carries a unique, totally
ordered timestamp (Lamport time works). Replicas apply moves as if in timestamp order, by undoing later ones and
redoing them; a move that would create a cycle at its turn is ignored. They truncate the log once a timestamp is
causally stable, which needs knowing what every replica has seen. (Source 19.) With a server that orders everything,
the server's order *is* that timestamp, and the log never needs replaying on clients: the server decides once.

**Hybrid logical clocks (HLC).** A timestamp pair (physical time, counter) that stays close to physical time and
fits in 64 bits, with bounded drift as long as clocks are within a known skew. (Source 20.)

**Fractional indexing.** Evan Wallace's write-up: sort by a fraction in (0,1), with the object id as the tie-break;
insert by averaging the neighbours; use arbitrary-precision numbers because 64-bit floats run out after about 50
halvings; add a random offset to make identical results from two peers unlikely; it can interleave runs inserted
concurrently into one gap, and says so. (Source 21.) David Greenspan's write-up of a string-based variant was not
readable when this section was researched (rate-limited); not used.

**Interleaving.** Weidner and Kleppmann show that concurrent insertions at one position in a replicated list can
come out woven together, and give the Fugue algorithm, which avoids a defined class of interleaving ("maximal
non-interleaving") for text. (Source 22.)

**Peritext.** Formatting is not stored as characters in the text; `addMark` and `removeMark` operations are anchored
to gaps between characters. Marks such as bold expand when text is typed at their end; links and comments do not.
Compatible marks coexist; mutually exclusive values such as color are settled by last-write-wins on operation ids.
(Source 23.)

**Undo in collaborative editors.** Prakash and Knister (1992, 1994) call undoing only your own change "selective
undo" and note that most shared editors offer only a global undo of the last change by anyone. Later work (a survey
excerpt found by search) says practical support is still hard after nearly three decades. This section takes the
simple per-register rule of Figma's posts and Loro's `UndoManager` rather than operational transformation of
history. (Sources 24, 6.)

## Options considered

| # | Approach | Partial copies | Server can refuse one op | Text | Undo | Verdict |
|---|---|---|---|---|---|---|
| A | One library document for the whole surface (Loro, yrs or Automerge) | No: every client loads the lot | Awkward (opaque merged updates) | Yes | Library | Fails requirement 8 of the design (view-scoped). |
| B | One library document per object (yrs subdocuments, or one Loro/Automerge doc each) | Yes, per object | Awkward | Yes | Per doc, hard to combine into one stack | Per-document sync state and handshake for thousands of objects in view; a root map of millions of ids still needs sharding; unproven. |
| C | One library document per spatial cell | Yes, per cell | Awkward | Yes | Per doc | Rejected by the high-level design: long strokes cross cells, objects move. |
| D | **Server-sequenced register map built for Everink; library only for text bodies** | Yes, per object | Yes, by design | Library, one small doc per text object | Our own, conditional writes | **Decided** (owner decision, 2026-11-12). |
| E | As D, but also our own text CRDT (for example Fugue plus Peritext marks) | Yes | Yes | Ours | Ours | Fallback if no text library passes the spike; much more work and risk. |
| F | Figma-style, text as one whole value | Yes | Yes | No character-level merge | Ours | Fails the design (text edited "character by character"). |

How the clock options compare for ordering writes (the "last" in last-writer-wins):

| Clock | Needs | Winner is | Weakness here |
|---|---|---|---|
| Lamport counter (per client) | nothing | highest counter, ties by client id | Winner is arbitrary but stable; does not use the server's order that already exists. |
| Hybrid logical clock | clients' clocks within a known skew; the server must clamp stamps | latest by real time | A client with a wrong clock wins or loses unfairly unless the server clamps; adds a trust surface. |
| **Server sequence number** (one counter per surface) | one sequencer per surface (already true: one room per surface) | whichever the server accepted last | A single counter limits throughput (section 03 sizes it); offline edits win by arrival, not by when they were made. |

**Decision: D with the server sequence as the clock.** Reasons:

- It is the only option here that gives partial copies without inventing a document-splitting scheme: the object is
  already the unit, and an object is a handful of small registers.
- The server is already the sequencer and reader (owner decision 5), so a total order is free, needs no clock trust,
  and lets the server check each operation (roles, caps, cycles, "reparent into a deleted group") before it gets a
  number.
- Conditional writes (compare-and-set on a register's stamp) make selective undo exact, and are only possible
  because the server decides atomically.
- Text is the one place where a real sequence CRDT earns its complexity, and the libraries are good at exactly that.

**Given up:**

- Peer-to-peer or server-less merging (two clients with no server cannot merge). The design already has a server.
- Offline edits win over newer online edits to the same property (arrival order); the owner accepted this on
  2026-11-12.
- A library's built-in history and time travel for the object map. Named versions are snapshots (section 03), not
  replay.
- Some code to write: roughly a register map, an op validator, an order-key generator and an undo stack, all small
  and testable. The text adapter keeps the library swappable.
- Using one library for the whole object map is not done. The spike (below) still measures option B (a document per
  object) so that the finding that no library syncs part of a document rests on a measurement, not on the absence of
  a documented feature.

## Design

### 1. Principles

1. **Server-sequenced.** Clients send a `Change` (an atomic batch of operations). The server validates it against
   its full state, then either accepts it and gives it the next surface-wide number `Seq`, or rejects the whole
   change. Every client applies accepted changes. The server never rewrites an operation; it only accepts or
   rejects.
2. **Registers.** Every mutable field is a register: a value plus the `Seq` that wrote it. A write with a higher
   stamp replaces the value; a lower one is ignored. Merging is therefore independent of delivery order, which makes
   a snapshot plus a live stream safe even if they overlap or arrive crossed. (Cheap, and needed by partial copies.)
3. **Optimistic local apply.** A client shows its own pending change at once. While a local change to a register is
   unacknowledged, incoming remote writes to that register are held back from display (they cannot win: the pending
   change will get a higher `Seq` if accepted). On rejection the client reverts to the newest accepted value. This
   is Figma's flicker rule.
4. **Closure.** A client that holds an object also holds its whole chain of ancestors (groups). This is needed to
   draw it and lets the client check cycles itself. Section 03 sends ancestors with any object.
5. **Immutable parts are not registers.** Kind, creator, creation time, stroke points, provenance and the object id
   never change, so they need no merge rule.

### 2. Ids, stamps and time

```rust
/// 128 random bits from the operating system's secure source (crypto.getRandomValues on the web).
pub struct ObjectId(pub [u8; 16]);

/// Assigned by the room when it accepts the Create. Never reused, not even after a purge. Compact handle for the wire
/// and for section 03's index; the ObjectId stays the name that survives export and copy.
pub struct Ordinal(pub u32);

/// Position in the surface's accepted history. One counter per surface, assigned by the room. Never reused.
pub struct Seq(pub u64);

pub struct UserId(pub u64);         // per surface, interned from the token's user id (everyone is identified by the embedding app)
pub struct LinkId(pub u64);         // the invite link a person joined through (section 08)
pub struct ClientChangeId(pub u64); // per client DEVICE counter, survives reconnects; with the client id it names a change

/// One register of type T. 'stamp' is the Seq of the change that wrote it.
pub struct Reg<T> { pub value: T, pub stamp: Seq }
```

- **Ids.** Random 128-bit ids need no coordination, so objects can be created offline. A collision among 10^10
  objects is about 10^-19. The server rejects a `Create` whose id exists, except as a replay of the same change. A
  `ClientChangeId` is a counter per client device (not per connection), saved with the device's offline queue so it
  survives reconnects and restarts. A change is named by the pair (client, `ClientChangeId`), and a duplicate is a
  change whose pair has been seen. Section 03 keeps the highest id seen per client for 30 days and answers a replay
  with the original `Seq`; `DUPLICATE_WINDOW` (default 10 minutes) is the minimum time every accepted pair is
  remembered, and 03 may remember longer. The same ids are used for library objects; built-in templates get fixed
  ids derived once at build time so that they are stable across releases.
- **Ordinals.** The room hands out `Ordinal`s in increasing order from a counter that is saved with the surface and
  never goes back, so a purged object's ordinal is never given to another. A `u32` holds 4 billion, far above the
  cap of 2 million live objects (section 03), and a surface that exhausts it is read-only until migrated.
- **Why not Lamport or HLC stamps?** See the table above. The room already assigns one total order; adding a second
  clock would add skew handling, clamping and a hostile-client surface with no benefit. Wall-clock times shown to
  people ("edited 3 minutes ago") come from a table `Seq -> server time` that the room keeps, not from the stamps.
- **Stamp cost.** A stamp is 8 bytes. To avoid paying it on every field of every stroke, an object stores stamps
  only for fields written after creation, in a small sorted list; a field not in the list has the object's `created`
  stamp. The server may keep the full form.
- **Stale clients.** Each change carries `observed: Seq`, the newest `Seq` the client had seen when it made the
  change. The server holds a `purge_horizon` (see tombstones); a change with `observed` older than that is refused
  and the client does a fresh load, as Figma does on reconnect.
- **Authors.** A change has an author: a user (with a role, section 08) or the server itself. The server authors
  changes of its own: rebalancing order keys, purging, `DeleteBy` (deleting everything by one creator), and pausing
  videos (section 05). Server-authored changes are stamped like any other and logged like any other. This does not
  conflict with "the server never rewrites a client's change": the server may add a change of its own, but it never
  edits, re-stamps or merges away part of a change a client sent.
- **Replayed offline changes.** A change a client made offline and sends on reconnect is **unconditional**:
  it is stamped when the server accepts it, so it wins over older online edits to the same register. Only undo
  carries `if_stamp`. (Owner decision, 2026-11-12.)

### 3. The object

```rust
pub struct Object {
    pub id: ObjectId,
    pub ordinal: Ordinal,            // immutable, server-assigned
    pub header: Header,
    pub body: Body,
    pub extra: Vec<(FieldId, Reg<Bytes>)>,   // fields this version does not know; kept and re-sent unchanged
}

pub struct Header {
    pub kind: Kind,                  // immutable
    pub creator: Creator,            // immutable
    pub created: Seq,                // immutable; also the default stamp of every field
    pub origin: Option<Provenance>,  // immutable; set when placed from a template
    pub alive: Reg<bool>,            // false = deleted (a tombstone); see section 7
    pub slot: Reg<Slot>,             // parent and order, one register so they change together
    pub xform: Reg<Xform>,           // where the object is, with its rotation and scale: ONE register
    pub name: Reg<Option<SmallText>>,// optional label (length limit in section 08)
}

/// Who made the object. Section 08 uses it so an owner can delete everything made by one person or through one invite link.
pub struct Creator {
    pub user: UserId,
    pub via: Option<LinkId>,         // Some: the invite link the person joined through
}

pub enum Kind { Stroke, Shape, Text, Image, Video, Group, Unknown(u8) }  // on the wire a u8; unknown kinds are kept opaque

pub struct Slot {
    pub parent: Option<ObjectId>,    // None = top level of the surface
    pub order: OrderKey,             // among siblings with the same parent; ties broken by ObjectId
}

/// What is stored. Types and names are section 01's ("Groups: compose, localize, relevel"):
///   TopXform   { anchor: Position, linear: Linear }
///   ChildXform { dl: i8 /* -8..=8 */, off: RelOffset, linear: Linear }
/// Level, anchor, rotation and scale are one register, so a concurrent move and rotate of one object lose one of the two.
pub enum Xform {
    Top(TopXform),
    Child(ChildXform),
}
// Linear is 01's rotation-and-scale type, [a, b, c, d] as f64 (no angle, no platform math function).
// RelOffset is 01's exact offset of the child's origin from the parent's origin, in steps of 2^-54 child units.
// 01's `Placement { level, anchor, xf }` is a different thing: the WORLD placement that `compose` computes from an
// Xform and its ancestors. It is never stored.
```

`Xform` is one register on purpose. Re-leveling (moving an object to the level that suits its size) changes the
anchor's level and the scale together, so they must be written atomically. The price: if one user moves an object
and another rotates it at the same moment, one of the two wins. A move and a restyle never conflict.

A child's `Xform` is relative to its parent, so moving, rotating or scaling a group is one write to the group,
however many children it has, and the group stores no child list. A child may be at most `MAX_LEVEL_SPAN = 8` levels
from its group (`dl` in -8..=8, 01's constant and this section's limit); grouping objects further apart is refused
with a message. This is the one place the limit is stated. Section 01 supplies `compose` (child `Xform` and parent
world placement give world placement), `localize` (the inverse, for making a group and for reparenting) and
`relevel`, with refusals (span above the limit, singular matrix, out of range) returned as errors that this section
reports to the user. Section 03 computes an object's world box through its ancestors with `compose`.

**Regroup drift, accepted.** `localize` followed by `compose` is exact when the parent's `linear` has a power-of-two
scale or a rotation that is a multiple of 90 degrees. Otherwise each ungroup-and-regroup cycle can move a child by
about 2^-51 of its offset from the group origin (01's estimate of the bound, about 4 ulps). This section accepts
that, because a group is not normally dissolved and re-formed over and over. Undo of a regroup restores the stored
`Xform` values whole, so undo itself adds no drift.

### 4. Fields and merge rule, by kind

"Reg" means last-writer-wins by `Seq`. "Imm" means set at creation, never merged. Header fields are in section 3.

| Kind | Field | Type | Merge | Notes |
|---|---|---|---|---|
| Stroke | `brush` | `StrokeBrush` (`BrushId`, `base_width`, `style_ver`) | Imm | Brush engine and its versions are section 06's. |
| Stroke | `points` | `PackedPoints` | Imm | Format is section 06's; see section 9. |
| Stroke | `paint` | `Reg<Paint>` (color, opacity) | Reg | Restyle after drawing. |
| Stroke | `cuts` | add/remove set of `Cut` | OR-set | Partial erase, section 9. |
| Shape | `geom` | `Reg<ShapeGeom>` | Reg (whole value) | Rect, ellipse, line, arrow, polygon, path. Two users editing different points of one path: one wins. |
| Shape | `fill`, `stroke` | `Reg<Option<Paint>>` each | Reg | |
| Shape | `stroke_style` | `Reg<StrokeStyle>` (width, dash, cap, join, arrowheads) | Reg | |
| Text | `body` | text sequence CRDT | library (section 10) | Characters and marks. |
| Text | `frame` | `Reg<TextFrame>` (width or auto, wrap, padding, laid-out size) | Reg (whole value) | See below. |
| Text | `defaults` | `Reg<TextDefaults>` (`FontRef`, size, color, align, line height) | Reg | What new characters get. |
| Image | `image` | `Reg<ImageFields>` | Reg (whole value) | Replacing the picture is one write. Files: section 05. |
| Image | `size`, `crop` | `Reg<Size>`, `Reg<Option<Rect>>` | Reg | Box size in the object's own units; `crop` is in the same units, see below. |
| Video | `video` | `Reg<VideoFields>` without `playback` | Reg (whole value) | Section 05 defines the struct. |
| Video | `playback` | `Reg<PlaybackState>` | Reg (whole value), never queued offline | Section 05 defines the struct. |
| Video | `size`, `crop` | as Image | Reg | |
| Group | `clip` | `Reg<bool>` | Reg | Children are not listed in the group; see section 6. |

Section 05 defines `ImageFields`, `VideoFields`, `PlaybackState`, `FontRef`, `BlobId` and `Placeholder`, and this
section stores them exactly as 05 writes them (names and types are 05's). Two storage rules are 02's:

- `VideoFields` contains a `playback: PlaybackState` member. In the document that member is **not** part of the
  `video` register. It is its own register, `playback`, so changing the file facts never overwrites a play or pause
  and the reverse.
- `PlaybackState` (status, anchor media time, anchor server time, rate, looping) is **one** last-writer-wins
  register, written whole, so a play by one person and a seek by another never combine into a mixture. It is never
  queued offline (see the rules below), and the server may author a write to it.

**Text frame and the stored laid-out size.** The server never lays text out, yet it must know a text object's box to
decide what to send. `TextFrame` therefore stores the result of layout:

```rust
pub struct TextFrame {
    pub width: Option<Fixed32>,   // fixed wrap width in object units, or None = grows with its text
    pub wrap: bool,
    pub padding: Padding,
    pub laid_out: Size,           // size of the laid-out text in object units, written by the client that last edited
}
```

`laid_out` is written only by the client that last edited the text, after its layout, in the same change as the text
update or frame change (section 06, "Extents and the home octave", has the full rule: it is rewritten when the text,
the width, wrap or padding, or the metric-changing defaults change, or a waited-for font arrives; other clients
never write it). `TextFrame` is one register, so the width and the laid-out size it produced are never from
different writers. The box only decides what is sent, cached and invalidated; a client that writes a wrong size
harms only the visibility of its own object. The validator checks that `laid_out` is finite and within the size band
of section 01.

**Image and video `crop`.** `size` is the box in object units and `crop` is a rectangle in the same units, relative
to the box's own origin, that selects the part of the source shown. The source pixel rectangle is the crop rectangle
scaled by source pixel size over object size: `px = crop * (px_w / size.w, px_h / size.h)` per axis, where `px_w`
and `px_h` are `ImageFields` (or `VideoFields`) pixel sizes. A missing `crop` shows the whole source. The object's
box (06's extent) is `[0, size.w] x [0, size.h]`; `crop` never enlarges it.

Rules that follow:

- Two fields never merge into a mixture. A register holds the whole value of one writer.
- Fields that must change together share a register: `slot` (parent and order), `xform`, `geom`, `playback`,
  `image`, `video`.
- `PlaybackState` is never queued offline. A client without a connection refuses a play, pause or seek instead of
  storing it, because replaying it later would claim the video has been playing for hours. A write rejected for a
  stale anchor is dropped by the client, not retried. The server may write it as the author (see section 2).
- Box kinds (shape, text, image, video) are resized by writing `size` or `geom` and keep `linear` a pure rotation.
  Strokes and groups are scaled by `linear`, since a stroke's points cannot change.
- Unknown fields are kept (see section 12).

### 5. Operations, changes and validation

```rust
pub struct Change {
    pub id: ClientChangeId,
    pub observed: Seq,
    pub ops: Vec<Op>,             // atomic: all or none; at most MAX_OPS_PER_CHANGE ops and MAX_CHANGE_BYTES (section 08)
}

pub enum Op {
    Create    { id: ObjectId, header: NewHeader, body: NewBody },
    Set       { id: ObjectId, field: FieldId, value: Value, if_stamp: Option<Seq> },  // None = unconditional
    SetSlot   { id: ObjectId, parent: Option<ObjectId>, at: At, if_stamp: Option<Seq> },
    CutAdd    { id: ObjectId, cut: Cut },
    CutRemove { id: ObjectId, tag: (Seq, u16) },
    Text      { id: ObjectId, update: Vec<u8> },           // opaque update for the text library, section 10
    DeleteBy  { creator: CreatorFilter },                  // owner only; the server expands it, see section 7
}

pub enum At { Between(Option<OrderKey>, Option<OrderKey>), Front, Back }   // Front and Back are decided by the server
```

All operations in one change get the same `Seq`. No two operations in a change may write the same register. A `Set`
with `if_stamp: Some(s)` is applied only if the register's stamp is still `s`; otherwise the operation fails, and
(by the atomic rule) so does the change, unless the change is marked `partial_ok`, which only undo uses.

The server checks, in this order:

1. The sender's role allows each operation (section 08), and rate and size caps hold.
2. Ids: no `Create` of an existing id, no operation on a purged or unknown id.
3. Shape: the field exists for the kind and the value is in range. Numbers must be finite. A `Position` must be
   within the surface's depth cap (section 01, listed in 08's table), checked on the raw bytes before any number is
   built. Order keys, text length, update size, cuts per stroke and points per stroke must be within their limits
   (`MAX_ORDER_KEY_BYTES`, `MAX_TEXT_CHARS`, `MAX_TEXT_UPDATE_BYTES`, `MAX_CUTS_PER_STROKE`, `MAX_STROKE_POINTS`;
   all in section 08's table, which sets points per stroke to 20,000).
4. Tree: for `SetSlot`, the new parent exists, is alive, and is not the object or one of its descendants. Nesting
   depth is within `MAX_TREE_DEPTH` (section 08). A `SetSlot` into a deleted group is refused.
5. Conditions: every `if_stamp` still holds.

On a pass, the room writes the change to its log, then acknowledges. On a failure it sends `Rejected { id, reason
}`. Clients apply accepted changes in `Seq` order (per object; a partial copy sees only its objects' changes, and
gaps in `Seq` are expected).

### 6. Order keys, groups and reparenting

**Order key.**

```rust
/// Bytes compared as an unsigned lexicographic string; a shorter string that is a prefix sorts first.
/// A key never ends in 0x00. At most MAX_ORDER_KEY_BYTES (section 08).
pub struct OrderKey(pub SmallVec<[u8; 12]>);
// The sort key of an object is (OrderKey, ObjectId), so two equal keys are fine.

fn between(a: Option<&OrderKey>, b: Option<&OrderKey>, rng: &mut impl Rng) -> OrderKey;
```

`between(a, b)` works on base-256 digits. Starting at the first position where `a` and `b` differ, if at least two
digit values lie strictly between them, choose a random digit in the middle third of that range and stop. Otherwise
copy `a`'s digit and continue on the next position with `b` treated as absent. With `a` absent, `b` is treated as
`0x00…`, with `b` absent as `0xFF…`. A key has no terminating byte; the rule "no trailing 0x00" keeps `between`
always possible.

This answers the known problems:

- **Identical keys from two writers.** The sort key includes the object id, so equal keys never break the order. No
  server repair is needed. The random digit makes them rare anyway.
- **Interleaving.** Each user's own run stays in order, because every new key lies between the previous key and the
  same right neighbour. Two users' runs inserted into one gap can interleave. For z-order this is harmless: it
  affects only which of two overlapping objects is on top, never any object's content. Anything that must stay
  contiguous (a pasted selection) is a group, with one key.
- **Growth.** Using the middle of a whole digit, not a bit, adds about one byte per 8 repeated inserts into the
  *same* gap. When a sibling list's longest key passes `REBALANCE_KEY_BYTES` (default 24), the server, as author,
  issues a `Rebalance`: a change that rewrites keys of the affected neighbourhood with evenly spread short keys
  (ordinary `SetSlot` writes). A concurrent insert that used old neighbour keys may land a few positions off; that
  is accepted. Keys over the limit are refused, so hostile growth is bounded.
- **Front and back.** `At::Front` and `At::Back` are resolved by the server, which keeps the largest and smallest
  key per parent. A client sees only part of a large sibling list, so only the server can say "above everything";
  the client shows a guess (largest key it holds, extended) until the acknowledgement brings the real key.
- **Order across levels** is just this one list: all top-level objects share the parent `None`.

**Groups.** A group has no list of children. A child names its parent; the room and each client build the children
index from `slot`. A group is made by one atomic change: `Create` the group, then `SetSlot` each member under it,
with each member's `Xform` rewritten into the group's frame (`localize`, section 01). Ungroup is the reverse
(`compose`), plus `alive = false` on the group, in one change. Members get evenly spread keys in the group's old
slot.

**Visibility is derived.** An object is visible when it and every ancestor is alive. Deleting a group is one write;
its descendants need none. This makes "delete a group while someone moves a child out of it" commute: if the move is
accepted first, the child is outside and stays; if the delete is first, a move *out of* a deleted group is still
allowed (it leaves the hidden tree and shows again), while a move *into* it is refused by the server (step 4), so
nothing is silently swallowed.

**Cycles.** Two users move A into B and B into A at once. The server accepts whichever it receives first and rejects
the other because, against the state after the first, it would make a cycle. This is Kleppmann's rule ("the later
timestamp is ignored") with the server's order as the timestamp, and Figma's server-side rejection. The loser's
client, which applied its own move optimistically, sees `Rejected` and reverts. If the client receives the other
move first, it sees that its pending move would form a cycle among objects it holds (it has all ancestors, principle
4) and hides its pending move until the verdict, so no cycle is ever drawn.

### 7. Delete, tombstones and collection

- **Delete** writes `alive = false`. **Restore** writes `alive = true`. They are ordinary register writes, so a
  delete and a concurrent edit of any other field commute: the edit is applied to the hidden object, and a later
  restore shows it with the edit. This is the "delete and a concurrent edit agree" rule.
- **Delete by creator.** `Op::DeleteBy` is for owners only (section 08). The server expands it, in one change, into
  `alive = false` for every live object whose `creator.user` or `creator.via` matches, so one person's or one invite
  link's work can be removed at once. It is an ordinary delete: the objects can be restored, and are purged after
  the retention period. The room keeps an index from creator to objects for this.
- A tombstone is the whole object, hidden. It keeps its data so undo and "restore" work. Its geometry may be moved
  to cold storage after 24 hours (section 03).
- **Hard purge.** The room purges an object when it has been invisible (deleted itself, or under a deleted ancestor)
  for `TOMBSTONE_RETENTION = 30 days`, equal to the period automatic snapshots are kept. Purging removes it
  completely and advances `purge_horizon` to the `Seq` of the newest purge; a later change that names a purged id,
  or has `observed` older than the horizon, is refused. Subtrees are purged together. Named versions are separate
  snapshots (section 03) and keep their copies; a purge does not reach into them. Blob garbage collection (section
  05) counts current state, retained snapshots and named versions as live; `blobs_in_state` (part 14) is the pure
  function that lists the blobs of one document state; section 03's `blobs_in_store` loads each stored state and
  calls it.
- **Versions.** Restoring is for the owner and editor members (owner decision, 2026-11-12; sections 03 and 08).
  Creating and deleting named versions follows the same rule; the owner's decision was about restoring, and this
  extends it. A restore is itself a server-authored change
  that writes the old state on top, using `Doc::diff_to` (section 14).
- **Who can still reference a tombstone?** (a) A connected client's undo stack (in memory, at most 100 entries per
  session; a refused undo is reported and dropped). (b) An offline client: a client away longer than the retention
  period reloads, and its queued changes are re-checked; those naming purged objects are dropped with a notice. (c)
  A text library's own tombstones (inside a text doc).
- **Privacy note.** Deleted content stays on the server for 30 days and in any named version. Users who think
  "delete" means "gone now" would be wrong; this is a product point (see the open questions).
- Registers keep no old values, so the document does not grow with edits; the only unbounded growth is tombstones
  and text history, both bounded as above.

### 8. Per-user undo and redo

Each client session keeps two stacks of **entries**. An entry is one user gesture: a drag, a stroke, a paste, a
restyle, a group, a delete. It holds, for each register it wrote, the old value and the `Seq` the write got, and for
creates and deletes the object id.

```rust
pub struct UndoEntry { pub parts: Vec<UndoPart>, pub label: &'static str }
pub enum UndoPart {
    Reg   { id: ObjectId, field: FieldId, before: Value, wrote: Seq },   // wrote = the stamp my write got
    Slot  { id: ObjectId, before: Slot, wrote: Seq },
    Create{ id: ObjectId, wrote: Seq },       // undo = alive false
    Delete{ id: ObjectId, wrote: Seq },       // undo = alive true
    Cut   { id: ObjectId, tag: (Seq, u16) },  // undo = CutRemove
    Text  { id: ObjectId, session: TextSessionId },
}
```

**Undo** sends one change made of conditional writes, one per part, with `if_stamp = Some(wrote)`:

| Part | What undo writes | Applied only if |
|---|---|---|
| `Reg` | `before` | the register's stamp is still `wrote` (nobody wrote it since) |
| `Slot` | `before` (parent and order) | slot stamp is `wrote`, and the old parent is alive and the move makes no cycle |
| `Create` | `alive = false` | the alive stamp is `wrote` (nobody restored or deleted it since) |
| `Delete` | `alive = true` | same |
| `Cut` | `CutRemove` | the tag exists |

The change is marked `partial_ok`: parts whose condition fails are skipped, the others apply, and the server returns
which ones applied. This is the exact meaning of "undo reverses only my changes":

- If another user has since changed a register I changed, that register is left as they set it. My undo of the
  *other* fields of the same gesture still happens.
- If another user deleted the object, my undo of a move or restyle of it does nothing visible (the writes are
  skipped because their stamps were replaced, or, if no one wrote those registers, they apply to a hidden object).
  Restoring is the deleter's undo, or any user's explicit restore.
- If I undo my own creation, the object is deleted even if others edited it since (their edits stay on the hidden
  object and return if someone restores it). This is deliberate: it is my object. (See the open questions.)
- If an entry applies nothing, it is consumed, the user sees "Nothing to undo: changed by someone else", and it is
  not pushed to the redo stack. Applied parts become a redo entry, built from the *actual* values replaced, with the
  new stamps (the acknowledgement gives the `Seq` of each of the client's own changes), so redo obeys the same rule
  against later changes by others.
- A new gesture clears the redo stack. Undoing several steps, copying, and redoing back leaves the document
  unchanged when nobody else interfered, which is Figma's stated rule.
- The acknowledgement carries a `(ClientChangeId, Seq)` pair for every change it covers, so the client learns the
  `Seq` of each of its own changes, and that `Seq` is the `wrote` of the undo parts that change produced. A replayed
  change is answered with its original pair, so `wrote` is the same after a reconnect. An undo made while one of
  its parts is unacknowledged waits for the acknowledgement (one round trip) so it knows `wrote`.
- A restore of a version (section 03) clears no undo stack. The restore is a change that writes new stamps, so a
  stale entry's condition (`if_stamp = wrote`) fails and that part is skipped, exactly as if someone else had
  changed the register.
- Because the conditions are checked by the server, undo works on objects that left the client's cache. The client
  need not hold them.
- Undo is per session. Two devices of one user have separate stacks; the stack is not saved across reloads (first
  version).

**Text** inside an open text object uses the text library's own undo for that user (section 10) while the editor is
focused, as every editor does. When editing ends, the whole editing session is pushed as one `Text` part, and
undoing it reverses that session's own insertions and deletions through the library, leaving others' text.

### 9. Strokes and erasing

A stroke is finished when the pen lifts and is then one `Create`; the stroke in progress exists only in presence
(section 04). Its points never change. This section holds the field; section 06 defines the format.

**A stroke in progress when the connection dies.** The preview on other screens fades (section 04). The author's
device keeps the stroke: it finishes it as if the pen had lifted and puts the finished stroke, as one `Create`, in
the client's durable outbox (the device's saved queue of changes not yet acknowledged, which also keeps
`ClientChangeId`s). It is sent when the device reconnects, and a replay after an unacknowledged send is answered
with the original pair like any other change. The server never commits a stroke on someone's behalf (owner decision,
2026-11-12). If the device is closed before reconnecting, the outbox is on disk and the stroke is sent at the next
start, subject to the purge horizon rule in part 2. The format is:

```rust
/// Defined by section 06 and held here word for word. Points are converted to units of the stroke's own level,
/// relative to the stroke's anchor, and rounded to an i32 lattice of 2^-16 of an object unit; one pressure byte per
/// point (0 = no pressure given, 1..=255); there is no time field. Immutable once the pen lifts. At most
/// MAX_STROKE_POINTS points (section 08: 20,000); a longer stroke is split by the client into several strokes.
pub struct PackedPoints { pub count: u32, pub bytes: Arc<[u8]> }
/// bytes: per point, zigzag-LEB128 varints of dx, dy (lattice steps from the previous point, the first from the
/// anchor), then the pressure byte. Decoding is integer-only.

/// The style block of a stroke (06). `style_ver` names the outline algorithm version, so a later algorithm never
/// alters old strokes.
pub struct StrokeBrush { pub brush: BrushId, pub base_width: Fixed32, pub style_ver: u8 }
```

A stroke's anchor (its `TopXform` or `ChildXform` origin) is the **center of the stroke's bounding box** (section
06), chosen by the stroke tool when the pen lifts. An `i32` at 2^-16 reaches 2^15 units from the anchor. Section 06
has the one stroke rule that makes every point fit (the stroke's level is fixed when the pen goes down, and the tool
ends the stroke and starts a new one when a point would lie too far from the pen-down point); this section relies on
it and does not restate its numbers. Integers decode identically on every platform.

**Erasing.**

- **Whole-stroke erase** is `alive = false`. Any object kind can be erased this way.
- **Partial erase** of a stroke does not touch its points and does not split it. It adds `Cut`s:

  ```rust
  pub struct Cut { pub from: StrokeParam, pub to: StrokeParam }  // from < to
  pub struct StrokeParam { pub index: u32, pub t: u16 }          // sample index and a 16-bit fraction toward the next
  // set element identity: (Seq of the change, index within the change)
  ```

  The stroke draws the parts of its points not inside any cut. Cuts form an add/remove set, so two users erasing
  different bits of one stroke at once both take effect (union), unlike "delete and re-create pieces", where two
  concurrent partial erases would each re-create the other's erased part. Undo of a partial erase removes exactly
  the cuts that gesture added (by tag), and other people's cuts remain. Cut positions are in the immutable sample
  indices, so they never go stale. Section 06 draws strokes with their cuts and hit-tests them; section 07's eraser
  tool produces the `Cut`s.
- **Default tool.** The whole-object eraser (`alive = false`) is the default tool; the partial stroke eraser, which
  produces `Cut`s, is a second tool (owner decision, 2026-11-12).
- At `MAX_CUTS_PER_STROKE` cuts (64) the eraser falls back to replacing the stroke: one change deletes it and
  creates the remaining pieces as new strokes. This path can duplicate in the rare case of two such replacements at
  once; it is accepted as a corner case.

### 10. Text and rich text

A text object has a header, the registers `frame` and `defaults`, and a **body**: a sequence of characters.

**One small document per text object.** The server holds each body in a text-library document. A client that is
*editing* a text object loads that document and exchanges `Op::Text` updates (opaque bytes the library produced). A
client that is only *viewing* receives a materialized copy:

```rust
pub struct RichText { pub rev: Seq, pub runs: Vec<Run> }
pub struct Run { pub text: SmallString, pub marks: Marks }
pub struct Marks { pub bold: bool, pub italic: bool, pub underline: bool, pub strike: bool,
                   pub color: Option<Rgba>, pub size: Option<f32>, pub font: Option<FontRef>, pub link: Option<LinkUrl> }
```

The server re-materializes after a text update (at most 4 times a second per text object) and sends `RichText` to
subscribers of the object, each with its `rev`; a copy with a lower `rev` is dropped. Viewers therefore need no
library document, no history and no tombstones; the cost is that a viewer cannot edit text offline until it has
opened that text once. (This is a choice; sending the document to everyone is the simpler alternative if the spike
shows small documents are cheap.)

**Library boundary.** The model only needs this from the library; the choice of library stays open and the adapter
is swappable:

```rust
pub trait TextDoc {
    fn new_empty() -> Self;
    fn apply_update(&mut self, bytes: &[u8]) -> Result<(), TextError>;         // untrusted input; must not panic
    fn export_update_since(&self, v: &TextVersion) -> Vec<u8>;
    fn insert(&mut self, at: usize, s: &str);   fn delete(&mut self, range: Range<usize>);
    fn mark(&mut self, range: Range<usize>, key: MarkKey, value: MarkValue);
    fn materialize(&self) -> RichText;
    fn undo_manager(&self, user: UserId) -> Box<dyn TextUndo>;                 // per-user, own edits only
    fn compact(&mut self);                                                      // drop history not needed for merging
}
```

- **Concurrent typing** merges by the library's sequence rules. Fugue-style algorithms avoid interleaving of two
  users' runs (Source 22); Loro uses Fugue (Source 7).
- **Formatting** follows Peritext's model (Source 23): marks are anchored to gaps, not characters. Expanding marks
  (bold, italic, underline, strike, color, size, font) grow when text is typed at their end; links do not.
  Concurrent exclusive values (two colors on one word) are settled by last-write-wins on operation ids. One
  paragraph is text up to a newline; paragraph alignment is the object-level `defaults.align` in the first version
  (lists and headings are later).
- **What the first version includes (whiteboard).** The first app is a whiteboard (owner decision, 2026-11-12), so
  text is for labels, sticky notes and short passages. Proposed here, not an owner decision: the first version
  supports **bold, italic, size and color**
  as marks, and per-object alignment (`defaults.align`). The `Marks` fields `underline`, `strike`, `font` and `link`
  exist in the model so that old documents and clients keep them (unknown parts are kept, part 12), but the first
  version's tools do not offer them. **Lists, headings, links and per-run font choice wait.** Lists are left out
  because a whiteboard note rarely needs them and a list adds paragraph-level structure (indent, numbering) that the
  mark model does not have; adding them later is a new field and so a minor schema change.
- **Scripts.** Text on the surface is Latin script only at first (owner decision, 2026-11-12). The model stores any
  Unicode text unchanged, so nothing is lost and no document changes when support is added; characters outside Latin
  show as missing-glyph boxes until section 06 ships fonts for them. No validation here rejects non-Latin text.
- **Unwanted markup.** Text is plain runs plus these marks. Links are limited to `http`, `https` and `mailto`
  addresses. No markup is interpreted.
- **Hostile updates.** `apply_update` runs on the server on bytes from clients. The server applies it to a scratch
  copy first, checks the length and mark limits (`MAX_TEXT_CHARS`, `MAX_TEXT_UPDATE_BYTES`, section 08), and only
  then keeps it. The adapter is fuzzed.
- **Size.** A larger paste than `MAX_TEXT_CHARS` becomes several objects.
- **History.** The text library keeps what it needs to merge (Loro's design avoids permanent tombstones per its
  docs; yrs discards deleted content with garbage collection on). `compact` runs when a text object has had no
  editor for 10 minutes.

### 11. Templates and libraries

- A **library** is a surface of a special kind (`SurfaceKind::Library`), so the same code edits it. A **template**
  is a top-level object in it, normally a group, with a name and tags in `name`.
- **Place = copy.** Placing makes new objects with new ids for the template and all its descendants, with the same
  relative layout and relative order keys; the root gets a new order key (`At::Front`) and an `Xform` from the drop
  point and the current view (section 01 provides the conversion). The target surface's server never reads the
  library; the client sends ordinary `Create` operations, so the copy is checked like any other content. Each copy
  records the placing user as its creator.
- **Provenance.** The root of the copy gets `origin = Some(Provenance { library: LibraryId, template: ObjectId,
  template_rev: Seq })`, immutable. It is for "find where this was used" and "update from library". It is never
  followed automatically; editing the original does not change copies. Built-in libraries ship with the app as
  read-only data with stable ids and `LibraryId::Builtin(name, edition)`.
- **Deep copies of images** share the same `BlobId`, so no file is duplicated (section 05).
- **Libraries across surfaces** need the placing user to have read access to the library surface (section 08).

### 12. Schema versions and migration

- The surface has `schema: SchemaVersion { major: u16, minor: u16 }`. Each kind's field list is fixed per version.
- **Additive changes** (a new optional field, a new kind) raise `minor`. Old clients keep and re-send fields and
  kinds they do not know, unchanged (`Object::extra`, `Kind::Unknown(u8)`), show an unknown kind as a labelled box,
  and never edit it.
- **Breaking changes** raise `major`. The room runs migration functions `migrate(n -> n+1)` over every object, in
  order, as a maintenance step: it stops accepting old clients, rewrites the snapshot, and starts a new `epoch`. The
  log is trimmed after snapshots (section 03), so old versions are read from their snapshot chunks, not from the
  log: each chunk carries the schema version it was written under and is run through the migration chain when opened
  (for example when opening a named version). This follows tldraw's rule that the server migrates and is upgraded
  first (Source 3).
- Clients with an older `major` are told to update and are not allowed to write.
- Migrations are pure functions of one object; each has a test over stored sample documents of every earlier
  version.
- Template and library surfaces carry the same version and migrate the same way.

### 13. How the model sits on the candidate libraries

What the model needs from a library, and who can give it, as best I could check:

| Criterion | Loro 1.17.1 | yrs 0.28.1 | Automerge 0.12.0 |
|---|---|---|---|
| Hold or sync some objects of a document | not documented; lazy snapshot loading exists but is within one document; test export by container | subdocuments, loaded on demand, synced by the provider per `guid` | whole document is the unit (unverified for partial loading) |
| Map-of-objects scale (millions of ids) as one doc | untested | untested | untested |
| Let the server reject one operation before it merges | would need decoding and a scratch document | same | same |
| Use the server's `Seq` as the last-writer rule | no (own ids and lamports) | no | no |
| Movable tree with cycle handling | yes | no tree type found in the pages read (unverified) | not found in the pages read (unverified) |
| Rich text with Peritext-style marks | yes, per Loro's docs | marks exist; anomaly noted by Peritext authors | unverified |
| Per-user undo | `UndoManager`, local ops only | `UndoManager` with origins; remote behavior unverified | unverified |
| Text sequence without interleaving | Fugue | YATA (not checked for interleaving) | RGA-family (not checked) |
| License, status | MIT, active | MIT, 0.x, active | MIT, active |

Reading: for text only, Loro and yrs both qualify on paper, and Loro has the stronger documented rich-text and undo
story. For the whole object map, B (a document per object) would be the only route to partial copies, and it is
unproven at the scale of thousands of objects in view. The text library is not yet chosen; the spike decides.

**Spike plan for the text library and for the partial-copy finding.** Build the object map of this section once
(about 1,500 lines, since it is only registers) and run these against it:

1. **Text-only trial.** Loro and yrs behind the `TextDoc` trait: memory per empty and per 200-character document;
   time to apply 1,000 updates; update size per keystroke; concurrent typing in three clients; marks overlapping and
   exclusive; undo of own edits while others edit; fuzzed `apply_update`; wasm size added.
2. **Document-per-object trial (option B)** with yrs subdocuments and with one `LoroDoc` per object: load 5,000
   objects into a client; time to first draw; memory; sync handshake count; whether a single root map of 2 million
   entries is tolerable.
3. **Whole-surface single document trial (option A)** at 100,000 strokes as a size reference only.
4. **Loro container-subset export**: check by code whether updates for chosen containers only can be exported and
   imported.
5. **Pass criteria:** text trial within a 2x budget on memory and latency of the best, and no failure in the fuzz
   and merge tests. For B to displace D, it must open a view of 5,000 objects in under 1 second and keep memory per
   object within 3x of D's, while still giving the server a per-operation check.

### 14. The `Doc` interface

Other sections call the model through one type. It holds a set of objects (all of them on the server, a partial copy
on a client) and performs no I/O.

```rust
impl Doc {
    /// Apply one change. 'ctx' carries the Seq the room gave it, its author and the extent oracle. The conditions
    /// (`if_stamp`) are checked inside; a failed condition rejects the whole change (Err) unless the change is
    /// `partial_ok`, in which case the skipped operations are reported in the Ok value. Returns what happened to each
    /// object, so section 03's index, section 05's blob table and section 06's scene update without scanning.
    pub fn apply(&mut self, change: &Change, ctx: &ApplyCtx) -> Result<Applied, ApplyError>;

    /// The Change that turns this document into 'target' (restore, reload, resync after a gap). Every differing
    /// register gets a write, so the room stamps it fresh. 'target' may be a snapshot.
    pub fn diff_to(&self, target: &Doc) -> Change;

    /// Full state of one object as canonical bytes, and the bytes of the whole document. Canonical: fixed field
    /// order, registers sorted by FieldId, minimal-length integers, no padding, no map-order dependence. Equal states
    /// give equal bytes on every platform. Section 03 wraps these in its framing.
    pub fn body(&self, id: ObjectId) -> Option<ObjectBody>;
    pub fn encode_all(&self) -> Vec<u8>;
    pub fn decode_body(bytes: &[u8]) -> Result<Object, DecodeError>;      // untrusted input

    pub fn get(&self, id: ObjectId) -> Option<&Object>;
    pub fn children(&self, parent: Option<ObjectId>) -> impl Iterator<Item = ObjectId> + '_;  // in order
}

pub struct ApplyCtx<'a> {
    pub seq: Seq, pub author: Author, pub ext: &'a dyn ExtentOracle,
    pub descendants: bool,   // true: also return an ObjectEffect for every descendant of a moved, re-leveled or hidden group
}
pub struct Applied { pub effects: Vec<ObjectEffect>, pub skipped: Vec<u16> }   // 'skipped' only for partial_ok changes

/// The effect of a change on one object. (Section 03's `Effect` is a wire message; this is the model's report.)
pub struct ObjectEffect {
    pub id: ObjectId,
    pub ordinal: Ordinal,
    pub old_box: Option<ExactRect>,       // world box before; None if the object was not visible (or did not exist)
    pub new_box: Option<ExactRect>,       // world box after; None if not visible now
    pub deleted: bool,                    // hidden by this change (itself or through an ancestor)
    pub new_blobs: Vec<BlobId>,           // blobs this change references that the object did not reference before
    pub changed: FieldSet,                // which registers or fields were written
}

/// Supplied by section 06, which also defines `LocalBox` (a box in the object's own frame on 01's lattice). It sees ONE
/// object: strokes, shapes, images and videos from their fields, text from the stored `TextFrame::laid_out`. For a
/// group it returns the empty box, and `Doc` forms the group's box (see below). Boxes are bit-identical on every client.
pub trait ExtentOracle { fn extent(&self, object: &Object) -> LocalBox; }

/// Every blob referenced by the object / by the whole state: image and video files, posters, and fonts in text
/// defaults and marks. A pure function of one document state (the live document, or a snapshot or version after 03 has loaded it).
pub fn blobs_in_object(object: &Object) -> impl Iterator<Item = BlobId> + '_;
pub fn blobs_in_state(state: &Doc) -> impl Iterator<Item = BlobId> + '_;
```

- `old_box` and `new_box` include the effect of ancestors. **A group's box** is the union of its children's boxes,
  each mapped into the group's frame by 01's `compose` and formed by `Doc`, because `ExtentOracle::extent` sees one
  object. When a group moves, `apply` returns one `ObjectEffect` for the group and, if `ctx.descendants` is true,
  one for each descendant the caller holds (the server's index sets it; a client sets it only to refresh descendants
  it holds). It is off by default because computing it for a 20,000-member group (the per-group limit in section 08)
  is real work and a client rarely needs it.
- On the server `apply` does not fail for a change the room validated. On a client it can fail for a change that
  names objects the client does not hold, which it reports as `Missing(ids)` so section 03 can fetch them.
- The canonical bytes are the format used for storage, for convergence tests (two documents are equal when their
  canonical bytes are equal) and for chunks. Section 03 may delta-encode the positions inside a canonical body for
  the wire and for snapshot chunks (for example against a base position per message), provided that decoding
  restores the canonical bytes exactly; the canonical form stays the form that is compared, hashed and stored in the
  log. `new_blobs` is how the room keeps its blob reference table (sections 03 and 05) without scanning.

### 15. Types and operations by name

One line each. These names are used by other sections; they will not change without a note in this section.

| Name | What it is |
|---|---|
| `ObjectId`, `Ordinal`, `Seq`, `UserId`, `LinkId`, `ClientChangeId` | Ids and counters (section 2); `ClientChangeId` is per client device and survives reconnects. |
| `Reg<T>` | A value and the `Seq` that wrote it. |
| `Object`, `Header`, `Kind`, `Creator` | An object, its fixed part, its kind, and who made it (user and, if any, the invite link). |
| `Slot { parent, order }` | The tree register: parent and `OrderKey` together. |
| `OrderKey`, `between`, `At` | Sibling order key, key generator, and the `Between`/`Front`/`Back` insert position. |
| `Xform`, `TopXform`, `ChildXform`, `RelOffset`, `Linear` | The one placement register, with 01's types. |
| `MAX_LEVEL_SPAN` | 8 levels between a child and its group. |
| `Paint`, `ShapeGeom`, `StrokeStyle`, `TextFrame`, `TextDefaults`, `Size`, `Rect` | Style and geometry values of shapes, text and boxes. `TextFrame::laid_out` is the stored laid-out size. |
| `PackedPoints`, `StrokeBrush`, `Cut`, `StrokeParam` | Stroke fields (06's format) and the partial-erase cut. |
| `ImageFields`, `VideoFields`, `PlaybackState`, `FontRef`, `BlobId`, `Placeholder` | Media fields (05's definitions). |
| `RichText`, `Run`, `Marks`, `TextDoc`, `TextUndo` | Text content for viewers, and the library adapter. |
| `Provenance`, `LibraryId` | Where a placed copy came from. |
| `SchemaVersion`, `Kind::Unknown`, `Object::extra` | Schema version and forward-compatible unknown parts. |
| `Change`, `Op`, `Author` | An atomic batch, its operations, and who wrote it (a user or the server). |
| `Op::Create`, `Set`, `SetSlot`, `CutAdd`, `CutRemove`, `Text`, `DeleteBy` | The operations. `Set` and `SetSlot` carry `if_stamp`. |
| `partial_ok`, `if_stamp` | Conditional write and the undo-only change flag. |
| `Rejected`, `ApplyError`, `Missing` | Refusal of a change; apply failures. |
| `UndoEntry`, `UndoPart` | The client's undo stacks. |
| `Doc`, `Doc::apply`, `Doc::diff_to`, `Doc::body`, `Doc::encode_all`, `Doc::get`, `Doc::children` | The model's interface (section 14). |
| `ApplyCtx` (with `descendants`), `Applied`, `ObjectEffect`, `ExtentOracle` and `LocalBox` (including each kind's extent and the group rule), `LocalBox` (06's) | Inputs and outputs of `apply`; a group's box is formed by `Doc` through `compose`. |
| `blobs_in_object`, `blobs_in_state` | Blob references of one object or of one document state (pure; 03's `blobs_in_store` loads stored states and calls it). |
| `purge_horizon`, `TOMBSTONE_RETENTION`, `DUPLICATE_WINDOW`, `REBALANCE_KEY_BYTES` | This section's own parameters (sections 2, 7, 6). |
| `MAX_OPS_PER_CHANGE`, `MAX_CHANGE_BYTES`, `MAX_ORDER_KEY_BYTES`, `MAX_TREE_DEPTH`, `MAX_TEXT_CHARS`, `MAX_TEXT_UPDATE_BYTES`, `MAX_CUTS_PER_STROKE`, `MAX_STROKE_POINTS` | Limits; the numbers are in section 08's table. |

## Failure modes

| Failure | What happens |
|---|---|
| Two users change one register at once | The later accepted write wins on every client; the other client's pending value is replaced when the acknowledgement arrives. No dialog. |
| A change is refused (cycle, role, limit, into a deleted group) | `Rejected`; the client reverts that change and shows a short notice. An undo that fails shows "Nothing to undo". |
| The room crashes after accepting a change but before the acknowledgement | The client resends; the room answers with the original `(ClientChangeId, Seq)` pair instead of applying twice. |
| The client is offline for a long time | On reconnect: if `observed` is older than the purge horizon, reload and re-apply what still applies; otherwise changes apply by arrival order and may override newer online edits to the same register. A play, pause or seek is not queued at all. |
| An object is evicted from a client cache while it is in the undo stack | Undo is decided by the server by stamp, so it still works. |
| Order keys grow | The server rebalances at `REBALANCE_KEY_BYTES` and refuses keys over `MAX_ORDER_KEY_BYTES`. |
| A hostile client sends a deep tree, a huge stroke, a deeply nested position or a malicious text update | Refused by the validator (limits in section 08), and the text adapter is fuzzed. |
| A text update that the library cannot decode | `TextError`; the change is rejected; the room's text document stays as it was (scratch-copy rule). |
| A client with a gap in `Seq` (partial copy) | Registers are order-independent, so a snapshot followed by live changes is correct even if they overlap. |
| Two users partially erase one stroke | Both erasures apply (union of cuts). |
| A migration bug | Migrations run on a copy first; the old snapshot and log remain; the room only switches after a full check of every object. |
| Template deleted after use | Copies are independent; `origin` then points at nothing, shown as "template removed". |
| The single sequencer is a bottleneck | Section 03 sizes it; one room per surface (about 100 changes a second for 20 busy users is far below any limit). |
| Someone who joined through an invite link defaces the surface | The owner turns the link off, suspends those people, and runs `DeleteBy` for the link; the objects are hidden in one change and can be restored. |

## How it is tested

"The same on every client" means bit-identical canonical bytes, outlines and boxes, not identical GPU pixels
(sections 01 and 06). Convergence tests compare `encode_all` output.

| Rule | Test |
|---|---|
| Registers converge whatever the delivery order | Property test: random changes on random objects, delivered to several simulated clients in random orders and with random overlap between snapshot and stream; all end with equal canonical bytes. |
| Delete and a concurrent edit agree | Same test with `alive` writes mixed in; a restore shows the edit. |
| Visibility is derived through ancestors | Test: delete a group, move a child out, undo each, in every order. |
| No cycles, ever | Property test with many concurrent `SetSlot` operations; check acyclic after each accepted change on the server and on every client, including during optimistic apply. |
| A move into a deleted group is refused | Scripted two-client case. |
| Order key invariants | Property test: `between` result is strictly between its inputs, never ends in 0x00, stays under the limit after 10,000 random inserts; runs keep internal order; rebalance preserves order. |
| Own run order survives interleaving | Two simulated users insert 100-object runs into one gap concurrently; each run is in order. |
| Undo reverses only my changes | Scripted two-user scenarios: other user moved, restyled, deleted or restored the object, before and after my undo and redo; table of expected results from section 8. |
| Undo of a pending change waits for the acknowledgement | Scripted with a delayed connection. |
| Partial erase union | Two clients cut one stroke at once; both cuts present; undo removes only mine. |
| Stamp cost saving gives the same result as the full form | Property test comparing the two representations. |
| Duplicate resend is idempotent | Resend every change twice in a random test. |
| Ordinals are never reused | Create, delete, purge and create again; the new object gets a higher ordinal. |
| Purge | Time-travel test with a fake clock: purge after 30 days, refusal of stale changes. |
| `blobs_in_state` finds every blob | Generate documents with every blob-bearing field; the iterator equals a brute-force walk of the encoded bytes, for live state, snapshot and version. |
| `diff_to` | For random pairs of documents, applying `diff_to` to the first gives the second (equal canonical bytes). |
| Regroup drift stays within 01's bound | Ungroup and regroup a rotated, non-power-of-two-scaled group 1,000 times; the child's offset error stays within 1,000 x 2^-51 of its offset; with a power-of-two scale it is zero. |
| `ObjectEffect` boxes | Compare `old_box`/`new_box` with boxes computed from scratch through ancestors, after random moves of groups. |
| Delete by creator | Objects of two people who joined through different invite links and one who did not; `DeleteBy` one link hides exactly that link's objects, and a restore brings them back. |
| Playback is not queued offline | A disconnected client's play is refused, and nothing is sent on reconnect. |
| Schema forward compatibility | Old client model reads documents with unknown fields and kinds, writes, and the unknown parts survive. |
| Migrations | Golden documents of each earlier version migrate to the current one and pass the validator. |
| Text adapter | Shared conformance suite run against every `TextDoc` implementation (convergence, marks, undo, fuzz). |
| Library comparison | The spike plan in design part 13. |

## Dependencies

| Crate | Version checked | License | Status | Why |
|---|---|---|---|---|
| `loro` | 1.17.1 (2026-10-22) | MIT | Active; fast-moving (many 1.13 to 1.14 fixes in lazy loading, tree and undo) | Candidate for text bodies only. |
| `yrs` | 0.28.1 (2026-11-04) | MIT | Active; 0.x, so API may change | Candidate for text bodies only. |
| `automerge` | 0.12.0 (2026-09-16) | MIT | Active | Considered; not preferred for text (undo and marks unverified). |
| `smallvec` | any recent | MIT OR Apache-2.0 | Widely used (not re-checked) | Order keys and small strings. |
| `getrandom` / `rand` | any recent | MIT OR Apache-2.0 | Widely used (not re-checked) | Ids and key digits; `getrandom` has a wasm backend. |
| `proptest` | dev only | MIT OR Apache-2.0 | (not re-checked) | The property tests above. |

MIT is compatible with MIT OR Apache-2.0 for use as a dependency. Everything else in this section is Everink's own
code. No library is needed for the object map by design.

## What this assumes of other sections

- **01:** `Position` is exactly comparable, of bounded size, and has a depth cap checkable on raw bytes. It owns
  `TopXform`, `ChildXform { dl, off: RelOffset, linear }`, `Linear`, `compose`, `localize` and `relevel` as written
  in its "Groups: compose, localize, relevel" subsection, the `ExactRect` box type, and the refusals (span above 8,
  singular matrix, out of range) as errors. This section stores 01's types as they are and states the span limit
  once (design part 3).
- **03:** One room per surface assigns `Seq` and `Ordinal`. Every object is sent to a client together with its
  ancestors, with all register stamps. A client reloading an object it holds receives stamps newer than its own. The
  room keeps the log, snapshots, `purge_horizon`, the `Seq -> time` table, the duplicate window, the children index
  and the creator index, and runs `Rebalance`, the purge and `DeleteBy`. The spatial index consumes `ObjectEffect`s
  (its wire message `Effect` is its own). Wire encoding of `Value`, `Position` and `OrderKey` is theirs, wrapped
  around this section's canonical bytes. It accepts conditional writes (`if_stamp`) and `partial_ok`.
- **04:** Drag previews and live ink travel as presence messages; this section's `Set` for a drag is sent on release
  and at most 5 times a second during a long drag.
- **05:** The definitions of `ImageFields`, `VideoFields`, `PlaybackState`, `FontRef`, `BlobId` and `Placeholder`;
  the playback clock. Blob garbage collection calls `blobs_in_state`; the room uses `ObjectEffect::new_blobs`.
- **06:** The `PackedPoints` format, `StrokeBrush`, the stroke anchor at the bounding-box center, `ExtentOracle`,
  and drawing and hit-testing of strokes with cuts.
- **07:** Gesture boundaries (what makes one undo entry); text-editor focus (which switches undo to the text
  library's); the eraser tool producing `Cut`s.
- **08:** The table of limits (`MAX_OPS_PER_CHANGE`, `MAX_CHANGE_BYTES`, `MAX_ORDER_KEY_BYTES`, `MAX_TREE_DEPTH`,
  `MAX_TEXT_CHARS`, `MAX_TEXT_UPDATE_BYTES`, `MAX_CUTS_PER_STROKE`, `MAX_STROKE_POINTS` = 20,000, name length); the
  role check including owner-only `DeleteBy` and the version operations; the meaning of `LinkId`.

## Questions for the owner

Open. The merge model, offline replay, the whole-object eraser as default, the first app, who may restore versions
and Latin script first were decided on 2026-11-11 and 2026-11-12 and are in the Design.

1. **Text library.** Which library edits the text bodies (Loro, yrs or another) is decided after the spike in design
   part 13; the owner chooses on its result. The object map itself does not use a library.
2. **Undo of my own creation.** It deletes the object even if others have since edited or grouped it. The
   alternative is to refuse the undo when others have touched it. Which do users expect?
3. **Deleted content retention.** Deleted objects stay 30 days on the server and forever in named versions. Is that
   acceptable, or should there be a "delete permanently now" action?
4. **Offline text.** Viewers can edit text offline only for texts they opened once. Accept, or ship the text library
   document to every viewer (more bytes, simpler rules)?

## Sources

All pages read 2026-11-11 unless noted. "Search excerpt" means the claim comes from a search result, not a full
read.

1. Evan Wallace, How Figma's multiplayer technology works.
   https://www.figma.com/blog/how-figmas-multiplayer-technology-works/
2. Figma, Realtime editing of ordered sequences. https://www.figma.com/blog/realtime-editing-of-ordered-sequences/
3. tldraw, Sync. https://tldraw.dev/docs/sync
4. tldraw, Store reference. https://tldraw.dev/reference/store/Store
5. Excalidraw, `reconcile.ts`.
   https://github.com/excalidraw/excalidraw/blob/master/packages/excalidraw/data/reconcile.ts
6. Loro Rust docs, UndoManager. https://docs.rs/loro/latest/loro/struct.UndoManager.html
7. Loro docs (full text extract; tree, map, shallow snapshots, text). https://www.loro.dev/llms-full.txt
8. Loro Rust docs, LoroTree. https://docs.rs/loro/latest/loro/struct.LoroTree.html
9. Loro changelog (npm package 1.14.1; lazy loading, tree, undo fixes).
   https://cdn.jsdelivr.net/npm/loro-crdt@1.14.1/CHANGELOG.md
10. Loro repository (license, features). https://github.com/loro-dev/loro ; crate data
    https://crates.io/api/v1/crates/loro ; rich text (search excerpt) https://loro.dev/docs/tutorial/text
11. Yjs, subdocuments. https://docs.yjs.dev/api/subdocuments
12. Yjs, UndoManager. https://docs.yjs.dev/api/undo-manager
13. Yjs internals (deleted items, garbage collection). https://github.com/yjs/yjs/blob/main/INTERNALS.md
14. yrs UndoManager docs. https://docs.rs/yrs/latest/yrs/undo/struct.UndoManager.html ; crate data
    https://crates.io/api/v1/crates/yrs
15. Automerge 3.0. https://automerge.org/blog/automerge-3/
16. Automerge, conflicts. https://automerge.org/docs/reference/documents/conflicts/ ; crate data
    https://crates.io/api/v1/crates/automerge
17. Ink & Switch, Keyhive notebook 05 (documents as the unit of sharing; search excerpt).
    https://www.inkandswitch.com/keyhive/notebook/05/
18. Diamond Types. https://github.com/josephg/diamond-types
19. Kleppmann, Mulligan, Gomes, Beresford, A highly-available move operation for replicated trees.
    https://martin.kleppmann.com/papers/move-op.pdf
20. Kulkarni et al., Logical Physical Clocks and Consistent Snapshots in Globally Distributed Databases (HLC), read
    only in part. https://cse.buffalo.edu/tech-reports/2014-04.pdf
21. Evan Wallace, Fractional indexing for CRDTs. https://madebyevan.com/algos/crdt-fractional-indexing/
22. Weidner, Kleppmann, The Art of the Fugue: Minimizing Interleaving in Collaborative Text Editing.
    https://arxiv.org/abs/2305.00583
23. Litt, Lim, Kleppmann, van Hardenberg, Peritext. https://www.inkandswitch.com/peritext/
24. Prakash and Knister, A framework for undoing actions in collaborative systems (search excerpt, not read in
    full). https://web.eecs.umich.edu/~aprakash/papers/undo-tochi94.pdf

## Claims left unverified

- That any of Loro, yrs or Automerge cannot sync a subset of a document's containers (only that no such feature was
  found in the pages read).
- Memory and size cost per tiny library document; per-operation metadata size in each library.
- That tldraw merges per property; that Figma's current system matches the 2019 posts; that Figma keeps transform as
  one register (this design chooses it on its own reasoning).
- yrs `UndoManager` behavior for remote changes; Automerge undo and marks; yrs tree support; interleaving behavior
  of YATA and Automerge's list.
- The byte estimates for strokes and object headers (derived, not measured).
- Ink & Switch's statement on Automerge documents being the unit of sharing and per-document sync cost (search
  excerpt only).
- David Greenspan's string-based fractional indexing write-up (could not be read), and Loro's rich-text compliance
  with Peritext (search excerpt).
