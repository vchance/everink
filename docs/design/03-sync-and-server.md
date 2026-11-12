# View-scoped sync, protocol, server and storage

Status: detailed design, 2026-11-11, proposal.

## Scope

This section covers how a room (the server's in-memory copy of one surface) decides which objects each client
receives, how clients and the server talk, and how the server stores a surface and its versions.

It covers:

- the server's spatial index and the subscription algorithm built on it;
- the edges of a client's partial copy (objects leaving, entering or vanishing outside the view);
- the wire protocol: messages, encoding, versioning, ordering, acknowledgement, flow control, reconnect;
- the client cache and its eviction;
- persistence: change log, snapshots, compaction, storage engine, crash recovery;
- versions: automatic restore points, named versions, restore while people edit;
- room lifecycle, placement across machines, and capacity estimates;
- what sections 01, 05, 06 and 08 need from the server: camera queries, media messages and records, overview-picture
  counts and triggers, and authorization hooks.

It leaves to other sections:

| Topic | Section | What this section assumes of it |
|---|---|---|
| `Position`, levels, cells, exact comparison | 01 | See [What this assumes](#what-this-assumes-of-other-sections) |
| Object, change and merge types, undo, canonical encoding | 02 | Same |
| Cursor and live-ink content, routing rules | 04 | The envelope and the view data it needs are here; the content is 04's |
| Blob upload and download, video clock | 05 | Only the clock ping is here |
| Rendering, overview pictures | 06 | The "more here" counts, picture table and builder trigger are here; the pictures are 06's |
| Tokens, roles, limits, invite links | 08 | An auth check that yields a user id and a role |

## The problem

A surface can hold millions of objects over an unbounded range of zoom levels, but a screen shows a few thousand
at most. The goals from `design.md` are that opening a surface costs about the same whatever its size, that
panning shows no holes, and that no change is lost once acknowledged. Concrete numbers (all assumptions, to be
measured in the spike):

- A typical stroke is about 200 points. After delta coding that is about 1 KB (section 02 owns the real figure).
- A view holds 100 to 10,000 objects. At 1 KB each that is 100 KB to 10 MB, so only the first screenful may be sent
  before the user sees anything. The rest must stream, biggest objects first.
- The planning case, the first app (a whiteboard), has about 50 active people on one surface (owner decision,
  2026-11-12). Presence at 30 updates per second each is 1,500 updates per second entering the
  room, and without view filtering up to 73,500 per second leaving it (50 people, each sent the 49 others). So the
  filter, not the sender, is what makes presence affordable.
- A large surface has 2 million objects, the first-version cap. Held fully in server memory that is an estimated 2.6
  GB (built in [Capacity](#capacity-estimates)); a surface ten times larger is not practical to hold in memory on
  one small machine.
- The server must not lose an acknowledged change, even if it crashes one millisecond later.

Why the obvious answers do not work:

- **Send the whole document** (what the Yjs servers and tldraw sync do, as far as their documentation says): open
  time grows with surface size. This is the thing goal 3 of `design.md` rules out.
- **Split the surface into tiles and sync each tile as its own document:** objects cross tile boundaries, objects
  of very different sizes live on many levels, and a view at depth needs the large objects from far above it. The
  high-level design already chose objects, not tiles, as the unit.
- **A single R-tree over everything:** the coordinates have no bound (a position has arbitrarily many digits), and
  a tree does not know that an object smaller than a pixel should be skipped.

## What others do

**Figma.** One server process per document, clients over WebSocket; the server is the central authority and
defines the order of events; per-property last-writer-wins; fractional indexes for order; offline clients download
a fresh copy and reapply their edits
([how multiplayer works](https://www.figma.com/blog/how-figmas-multiplayer-technology-works/)). Persistence is a
journal of changes (DynamoDB, entries batched about every 0.5 s, each with an increasing sequence number) plus a
full checkpoint written to S3 every 30 to 60 seconds; on a crash the server loads the latest checkpoint and applies
journal entries above its sequence number; a lock table keeps one writer per file
([making multiplayer more reliable](https://www.figma.com/blog/making-multiplayer-more-reliable/)). Figma's
numbers: clients send every 33 ms, 95% of edits are saved within about 600 ms. The multiplayer server was moved
from a single-threaded TypeScript process to a Rust child process per document because one slow encode blocked
every document on a worker
([Rust in production](https://www.figma.com/blog/rust-in-production-at-figma/)). Figma now loads only the page
being viewed and sends other pages on demand, together with the pages they depend on. The sources I could read for
this were translations and a third-party summary, so treat the detail as unverified
([page-by-page loading](https://www.figma.com/ja-jp/blog/speeding-up-file-load-times-one-page-at-a-time/)). The
lesson taken: sequence-numbered journal plus periodic checkpoint, a single writer per room, partial loading by a
coarse unit. Everink's unit is finer (a region of the view), so the server needs an index.

**tldraw sync.** One room object per document, a `TLSocketRoom` holding the authoritative document; storage is
in-memory by default and `SQLiteSyncStorage` for production; client and server versions must match, and an old
client may be told to refresh; sessions time out after 20 s unless the platform hibernates
([tldraw sync](https://tldraw.dev/docs/sync)). Its license requires a paid key for production use
([LICENSE](https://github.com/tldraw/tldraw/blob/main/LICENSE.md)), so it cannot be a dependency. Lessons: SQLite
for room storage is a normal choice; strict version matching is a cheap way to avoid protocol compatibility work
but is unkind to offline clients.

**Cloudflare Durable Objects, PartyKit, Liveblocks.** A Durable Object is one single-threaded object per room with
its own SQLite database (10 GB limit per object, a soft limit of about 1,000 requests per second, 32 MiB maximum
received WebSocket message), and a hibernation API lets sockets stay open while the object is evicted from memory,
with only 16 KiB of per-connection state kept
([limits](https://developers.cloudflare.com/durable-objects/platform/limits/),
[WebSockets](https://developers.cloudflare.com/durable-objects/best-practices/websockets/)). Its documentation
advises batching 10 to 100 logical messages per frame or flushing every 50 to 100 ms. Liveblocks runs rooms on
Durable Objects
([case study](https://www.cloudflare.com/case-studies/liveblocks)); its second storage engine streams a room's data
from storage and keeps minimal data in memory
([docs](https://liveblocks.io/docs/guides/about-the-new-storage-engine)). I could not find a primary Liveblocks
write-up of its wire protocol. Lessons: one room, one writer, one thread of control is a model that works at
scale; keeping object bodies out of memory is the route to large documents.

**y-sweet and Hocuspocus.** y-sweet (MIT) persists Yjs documents to S3-compatible storage and uses client tokens
([repository](https://github.com/jamsocket/y-sweet)). Hocuspocus is a TypeScript WebSocket backend for Yjs, scaled
with Redis ([docs](https://tiptap.dev/docs/hocuspocus/introduction)). Both sync a whole Yjs document. A caution
from general knowledge, not verified here: Yjs and similar operation-based CRDTs deliver updates in causal order
per client, so a client that holds only some of a document's operations cannot simply receive "the part it needs".
This is why view-scoped sync needs the document model to be per-object state, not one big operation log (see
[What this assumes](#what-this-assumes-of-other-sections)).

**Interest management in game servers.** Servers that must send each player only what is near them use grids,
spatial hashing and tile-based area-of-interest schemes, with hysteresis at the boundary so entities do not flap in
and out
([Boulanger, Kienzle, Verbrugge 2006](https://www.sable.mcgill.ca/~clump/papers/boulanger-06-comparing.pdf);
[Boulanger thesis](https://www.cs.mcgill.ca/~jboula2/thesis.pdf)). The paper's finding that cheap tile-based
schemes approximate ideal visibility-based management is the reason a grid beats a clever tree here. I did not
find a source on hierarchical grids with differently sized entities by name; the multi-row design below is the
standard "hierarchical grid" idea from general knowledge (unverified as a citation).

**Map tile servers** serve a pyramid of zoom levels, each level a grid of tiles, and pick the level from the zoom.
That is the shape of the index below, except that Everink's objects live on exactly one level of their own, not
rendered into every level.

**Async server stack.** tokio 1.54.1 (MIT), axum 0.8.9 (MIT, WebSocket support built on tokio-tungstenite 0.30.0,
MIT), fastwebsockets 0.10.0 (Apache-2.0), all current on crates.io on 2026-11-11
([tokio](https://crates.io/crates/tokio), [axum](https://crates.io/crates/axum),
[tokio-tungstenite](https://crates.io/crates/tokio-tungstenite),
[fastwebsockets](https://crates.io/crates/fastwebsockets)). WebTransport is supported in Chrome 97+, Firefox 114+
and Safari 26.4+, 92.37% of global usage ([caniuse](https://caniuse.com/webtransport)); the Rust server crate
wtransport is at 0.7.2, pre-1.0 ([crates.io](https://crates.io/crates/wtransport)).

## Options considered

### How the client's partial copy is chosen

| | A. Whole document, client culls | B. Sync per tile or cell | C. Per-object, server index, view subscription |
|---|---|---|---|
| Open cost vs size | Grows with size | Constant | Constant |
| Objects spanning cells and levels | n/a | Duplicated or split; a deep view needs big objects from above | Each object once |
| Server work per change | Fan-out to all | Fan-out per cell subscriber | Index update plus a test per connection |
| Server memory | Document | Document or per-tile documents | Document plus index |
| Fits the CRDT choice | Any library | Any, with sharded docs | Needs per-object state (assumption on 02) |
| Verdict | Fails goal 3 | Contradicts the high-level design | **Chosen** |

### The spatial index

| | Uniform grid per level | Single R-tree | Compressed quadtree | **Sparse rows of hashed buckets, R-tree inside** |
|---|---|---|---|---|
| Unbounded coordinates | Yes (hash key) | No (needs bounded numbers) | Yes, complex | Yes |
| Objects of very different size | Poor: tiny objects scan big cells | Fine | Fine | Fine: size picks the row |
| Skip sub-pixel content | Per level, easy | Per entry only | Per node | Per row, no work |
| Update cost | O(1) | O(log n) with rebalancing | O(log n) | O(log n) in one small tree |
| Query cost | Tiles touched (huge at fine rows) | O(log n + k) | O(depth + k) | Occupied rows times about 4 buckets times O(log n) |
| Deep zoom (hundreds of rows) | Many empty rows | Meaningless | Long single-child chains | Only occupied rows exist |
| Implementation risk | Low | Low | High | Medium (rstar crate does the trees) |

Why not the grid alone: the finest row that can still matter has tiles about one pixel wide, so a full-screen
query would probe about 14 million tiles (a 3,840-pixel padded view squared). The R-tree inside each bucket removes
that, because it walks only occupied space.

### Storage engine

| | SQLite file per room | redb per room | Postgres | Object storage only | Own segment files |
|---|---|---|---|---|---|
| Crash safety | Mature (WAL) | COW B-tree, checksums, repair path | Mature | Atomic put | Must be built and tested |
| Commit latency | fsync, about 1 ms on SSD (unmeasured) | fsync, similar | Network plus fsync | 10 to 100 ms per put (unverified) | fsync |
| Append log plus blobs | Good | Good | Good, bigger ops cost | Poor for small appends | Good |
| Move a room | Copy one file | Copy one file | Nothing to move | Nothing to move | Copy files |
| Self-hosting (library goal) | One dependency, no daemon | Pure Rust, no daemon | A daemon to run | Needs a store | None |
| Debuggability | `sqlite3` shell | Custom tools | SQL | Download blobs | Custom tools |
| License | MIT (rusqlite) | MIT OR Apache-2.0 | PostgreSQL | n/a | n/a |
| Verdict | **Default** | Close second; no read-only view of a saved state | Optional backend later | Replication target | Rejected |

redb has "persistent savepoints" ([design doc](https://github.com/cberner/redb/blob/master/docs/design.md)), which
look like free versions, but restoring one rolls the whole database back, and it does not offer to open one
read-only beside the live data. Versions need both, so it does not help.

### Wire encoding

| | postcard 1.1.3 | bincode 3.0.0 | protobuf (prost 0.14.5) | CBOR (ciborium 0.2.2) | MessagePack (rmp-serde 1.3.1) |
|---|---|---|---|---|---|
| License | MIT OR Apache-2.0 | MIT | Apache-2.0 | Apache-2.0 | MIT |
| Self-describing | No | No | Tags only | Yes | Yes |
| Size | Smallest (varints) | Small | Small | Larger | Larger |
| Add a field later | No (new variant) | No | Yes | Yes | Yes (maps) |
| Wire format promise | Stable since 1.0 | Repository archived 2025-08-15 | Stable | Standard | Stable |
| Maintenance | Last release 2025-07 | Unclear (see Dependencies) | Active (2026-11) | Last release 2024-01 | Last release 2025-12 |
| Verdict | **Envelope and small messages** | No | Considered; not needed | No | No |

Both ends are Rust (the web client is the same crate compiled to WebAssembly), so a schema compiler adds nothing,
and postcard's size matters for presence and view messages. Its cost is that compatibility must be managed by hand,
which the versioning rules below do. Object bodies and changes are opaque blobs encoded by section 02 with its own
version number, so protocol evolution and document evolution do not couple.

### Transport

| | WebSocket only | WebTransport only | **WebSocket now, WebTransport later** |
|---|---|---|---|
| Reach | Everywhere, passes proxies | 92% of browsers; UDP often blocked on company networks | Everywhere |
| Presence blocked behind bulk data | Yes (one TCP stream) | No (datagrams and streams) | Mitigated now: chunks of at most 16 KiB, a presence queue served first, adaptive pacing of unsent bytes |
| Server crates | Mature | wtransport 0.7.2, pre-1.0 | Mature now |
| Verdict | | | **Chosen** |

Presence payloads are self-contained and at most 1,000 bytes, so they can move to WebTransport datagrams later
unchanged. If, in the spike, cursor latency during a bulk load misses its target, section 04's two-socket design is
the documented next step.

## Design

### Overview of a room

One room = one tokio task (the **room actor**) that owns all room state and processes one input at a time. Nothing
inside it is shared across threads, so it needs no locks. Everything slow runs outside it:

```
  connection tasks (one per client)           room actor (one per surface)             store thread
  read frame, decode, check limits   ---->   apply change, assign seq,   --batch-->  SQLite: append, fsync
  write queue, priorities             <----  update index, route Apply,  <--durable-- (also snapshots)
                                             hold Acks until durable
```

Input to the actor is a bounded channel (4,096 messages). When it is full, connection tasks stop reading their
sockets, so TCP pushes back on the senders. This is the only back-pressure the actor needs.

```rust
// From section 02: ObjectId, Ordinal (dense, never reused), Seq (room-wide, assigned to each accepted change,
// strictly increasing, no gaps in the log), ClientChangeId, Doc, Change, Applied, ObjectEffect.
/// The Seq of the last change that touched an object.
pub struct Rev(pub Seq);
pub struct ClientId(pub [u8; 16]);   // stable random id of one device's copy of one surface (this section's)

pub struct Room {
    surface: SurfaceId,
    doc: Doc,                        // section 02: objects by ordinal, tombstones, sharded copy-on-write
    index: SpatialIndex,             // derived from doc, rebuilt on load
    conns: Slab<Conn>,               // at most the room's socket cap (section 08)
    next_seq: Seq,
    dedupe: HashMap<ClientId, (ClientChangeId, Seq)>,  // last change per client, for idempotent resubmission
    pending_commit: Vec<Pending>,    // applied in memory, not yet durable
    store: StoreHandle,
}
```

### The spatial index

**Rows.** A **row** `r` (an `i64`) is a grid of square tiles whose side is `32^r` level-0 units. Rows are absolute,
so they do not depend on any view. Section 01 fixes a cell at 1,024 units of its level and a unit of level `L` at
`1024^-L` level-0 units, so a level-`L` cell is `1024^(1-L) = 32^(2-2L)` level-0 units and **level `L`'s cell grid
is row `2 - 2L`**. Section 01's depth cap (levels within ±1,024) therefore gives cell-grid rows from -2,046 to
2,050; home rows, which add the finest row of each level and the coarser rows that hold large objects, run from
about -2,050 to 2,052. Both fit an `i16`, and the integer test below sums terms no larger than about 20,500, which
fits an `i32`; nothing here
needs more than `i64` for a row or a level. Every other row is a finer or coarser grid in steps of 32. Ratio 32 was
chosen because a ratio of 2 makes ten rows per level and a deep object would need a long chain of rows, while a
ratio of 1,024 (one row per level) leaves tiles that are too coarse to prune anything.

**Flooring a position to a tile is a shift.** Section 01 stores an axis as a fixed-point integer `n` in steps of
`2^-54` of a level unit. For a position at level `l`, a tile of row `r` is `2^(5r + 10l + 54)` steps wide, a power
of two, so `Position::tile_at(row) -> TileKey` is `Axis::shr_floor` of `n` by that many bits, and
`Position::bucket_at(row) -> BucketKey { row, bx, by }` is the same shift plus 10. For an object in its own home row
the shift is never negative: the finest home row of an object is
`2^-10` of its unit, which is `2^44` steps. A view on another level is first expressed on the finer of the two
levels with 01's `at_level`, which never rounds.

**Home row.** An object is filed in the **home row**: the finest row whose tile side is at least the object's
extent. The extent is the longer side of its bounding box, stroke width included. Two consequences, both used
below:

- The object touches at most 2 by 2 tiles of its home row, and its extent is greater than `1/32` of the side.
- Section 01 keeps every object between `2^-12` and `2^16` of its own units (it is re-levelled when it leaves that
  band). An object smaller than `2^-10` of its unit is filed as if it had that extent, so there is a finest row per
  level (row `-2L - 2`) and a dot of zero size does not need row minus infinity. An object of `2^16` units is filed
  in a row whose tile is at most `2^21` units, so its bucket is at most `2^31` units across.

**Buckets.** Hashing tiles one by one would make the finest rows cost millions of probes, so each row groups tiles
into **buckets** of 1,024 by 1,024 tiles (side `32^(r+2)`). A bucket holds an R-tree (the `rstar` crate) of the
entries whose box touches it, with coordinates stored as 64-bit floats relative to the bucket's corner. A bucket can
be up to 2^15 times larger than its smallest entry (1,024 tiles, each up to 32 times an entry's extent), so the
53-bit floats keep about 38 bits for the entry's own scale. The stored box is rounded outward, so the index never
misses; a final exact check removes the rounding.

**Keys and boxes stay small at any depth.** A full `Position` costs 19 bytes at the origin, 272 at level 100 and up
to about 3.1 KB at the cap (section 01). The index therefore never keeps one per object. A `BucketKey` (two big
integers) is as large as a position axis, but there is one per bucket, interned to a `u32` `BucketId`, not one per
entry. An object's box is stored as a small offset from its bucket's corner, which the size band above bounds
at 85 bits, so it fits an `i128` at any depth.

```rust
pub struct Row(pub i64);
pub struct BucketId(pub u32);               // interned (Row, BucketKey); the key is stored once, in `bucket_keys`

pub struct SpatialIndex {
    rows: BTreeMap<Row, RowIndex>,          // occupied rows only, coarse to fine by key order
    bucket_keys: Vec<BucketKey>,           // 01's BucketKey { row, bx, by }: two big integers
}
struct RowIndex {
    buckets: HashMap<BucketKey, (BucketId, RTree<Entry>)>,
    entries: u64,
}
struct Entry {
    ord: Ordinal,
    lo: [f64; 2], hi: [f64; 2],             // box relative to the bucket corner, rounded outward
    rel_extent: f32,                        // extent / tile side, in (1/32, 1]
}
/// Per object, outside the index: the exact box used for the final check. About 80 bytes at any depth.
/// Built from 01's `ExactBox { min, w, h }`: `bucket` is `min.bucket_at(home)`, `off` is
/// `min.offset_from_corner(&bucket_key)`, and `from_corner_offset` rebuilds `min` exactly.
pub struct ObjBox {
    home: Row, bucket: BucketId,            // bucket holding the box's minimum corner
    level: Level,                           // the lattice `off` is expressed in: the object's own level
    off: [i128; 2],                         // steps of 2^-54 level units from the bucket corner, below 2^85
    w: f64, h: f64,                         // size in units of `level`
}
```

An object that touches up to four buckets has up to four entries; queries remove duplicates with a bitmap of
ordinals. The exact minimum corner is rebuilt as a `Position` with `Position::from_corner_offset` when it is needed.

**Where boxes come from.** The index never computes a box itself. `Doc::apply` returns one `ObjectEffect` per
touched object (and, when the server asks, per descendant of a moved group), with `old_box` and `new_box` as 01's
`ExactRect` that already include the effect of ancestors. The room converts a `new_box` to an `ExactBox` (its
minimum corner and the longer sides rounded up) and then to an `ObjBox`, files it in its home row, and removes the
entry built from `old_box`. The boxes themselves come from 02's `ExtentOracle`, which section 06 supplies for every
kind (a stroke from its points and width, a shape from its geometry, an image or video from its size, a group from
its children). A text object's box is its stored `TextFrame` size, written by the client that last edited the text;
the server never lays text out. A client that writes a wrong frame harms only the visibility of its own object. The
object table also keeps 06's home octave `h(o)` per object (a pure function of the world box, recomputed from each
effect) as derived data for the picture counts.

**Costs.**

| Operation | Cost |
|---|---|
| Insert or remove | Compute row and at most 4 buckets (a shift of the fixed-point integers), then O(log n) per bucket tree |
| Move or resize | Remove, then insert; the old box is read from the object table, so the index cannot drift |
| Query | For each occupied row that can matter: at most about 4 buckets (about 64 to 81 in the finest row, for a padded view), one hash probe and one tree range query each, so O(R · (log n) + k) where R is the number of occupied rows that pass the test below and k the results |
| Memory | About 60 bytes per entry plus about 110 bytes per object for the object table (`ObjBox` about 80 bytes, ids, rev); about 170 bytes per object together, at any depth (estimate) |
| Rebuild | One pass over the document, estimated 1 to 2 microseconds per object; 1 million objects in 1 to 2 s |

The index is derived data: it is rebuilt at every room load and can be rebuilt on demand. A debug build checks it
against a full scan after every change (see tests).

**What `design.md` stored and what changes.** `design.md` lists, per cell, the largest object and whether anything
lies beneath. The first is not stored: the home-row rule makes the largest possible object in a row known without
looking (its extent is at most the tile side). The second is replaced by per-tile counts kept for the overview
pictures (see [Overview pictures](#overview-pictures-outline-build-step-6)), so there are no per-level counters
to keep consistent across levels.

### Which rows can matter

An object is sent when it crosses the padded view and its longer bounding-box side covers at least `min_px` screen
pixels (default 1); the query below applies this rule **down to the size threshold** and no further. Section 01's
`ViewRect` carries `pixel_log2`, the base-2 logarithm of the world size of one screen pixel in level-0 units,
rounded down. The view's own level has no meaning here (01). The test is done on logarithms because a tile's pixel
size overflows a float at depth, and it is an integer plus `log2(min_px)`:

```
tile_px_log2(r) = 5·r - pixel_log2                       // log2 of the row's tile side in pixels; rounded up
row r can hold an object that reaches min_px   iff   tile_px_log2(r) >= log2(min_px)     // its largest object is the tile
```

- **Coarse rows** (large `tile_px_log2`) always pass. In such a row every entry is at least `tile_px / 32` pixels,
  so if `tile_px_log2(r) >= 5 + ceil(log2(min_px))` the per-entry size test is skipped.
- **Fine rows** are cut off by the test: a row whose whole tile is under `min_px` holds nothing that reaches it.
  How many fine rows pass depends only on the view's size in pixels against `min_px`; for a 4,000-pixel view and
  `min_px = 1` it is five rows (tiles of 1, 32, about 1,000, about 33,000 and about 1,000,000 pixels), whatever
  level the content is on.
- **Rows with nothing in them are never visited:** `rows` holds only occupied rows, so a surface that has content
  at levels 0 and 400 costs two row visits however many levels lie between.
- **Rounding is conservative.** `pixel_log2` is rounded down, so the size test is conservative to within a factor of
  two: the server may send objects down to half a pixel. Section 06 culls with the exact pixel size and with the
  `eff_min_px` that the
  server reports, so nothing is drawn that should not be. The brute-force test in "How it is tested" uses the same
  rounded definition.

### The subscription algorithm

A **view** as sent by a client. Section 01's `ViewRect` is exact, unpadded and rounded outward; this is the only
place a view travels (never in presence):

```rust
pub struct ViewMsg {
    rect: ViewRect,              // section 01: level, min/max axes, pixel_log2
    min_px: f32,                 // default 1.0; the server clamps to >= 0.5
}
```

On the wire `max` is carried as a difference from `min`, and `min` as a difference from the previous `SetView`'s
`min` when both are on the same lattice, so a pan at level 100 costs tens of bytes instead of the 540 bytes of four
whole axes.

The server derives three regions from the rectangle with 01's `ExactRect::grown(num, den)`, all exact (growing by
half or a full size is a shift):

| Region | Grown by | Used for |
|---|---|---|
| `padded` | `grown(1, 2)`: half the size on each side | Which objects are sent (`N`) |
| `keep` | `grown(1, 1)`: a full size on each side | When a sent object is dropped (`K`); the gap is hysteresis |
| `routed` | `grown(1, 8)`: one eighth on each side | Cursor routing (section 04): `RoutedView { rect, routed }` |

Padding is the server's job, so a client cannot ask for more than the limits allow. Each connection's current
`RoutedView` is exposed **in-process** to the presence router (`fn view_of(&self, conn: ConnId) ->
Option<&RoutedView>`); presence messages never carry a view, so there is one source of truth.

```
query(view, region) -> sorted list of Ordinal       // objects crossing `region` and at least min_px
    seen = empty bitmap
    for (row, ri) in index.rows (any order):
        if 5·row - view.pixel_log2 < log2(view.min_px): continue                   // the row is below the size threshold
        for bucket in buckets_touching(ri, region):                  // 1..4 usually; exact bucket arithmetic
            for e in bucket.tree.locate_in_envelope_intersecting(local(region)):
                if seen.insert(e.ord) is false: continue
                if tile_px(row) < 32·min_px and e.rel_extent·tile_px(row) < min_px: continue
                if !exact_intersects(obj_box[e.ord], region): continue   // removes rounding slack
                out.push(e.ord)
    return out
```

The walk through coarser rows is the loop above (each costs at most four bucket probes because the view is tiny
against a coarse bucket); it ends at the size threshold.

**Per-connection state.**

```rust
pub struct Conn {
    id: ConnId, session: SessionId, user: UserId, role: Role, via: Option<LinkId>, client: ClientId,
    view: Option<RoutedView>, view_id: u32,
    sent: RoaringBitmap,        // ordinals the client has been sent and must be kept up to date on
    pending_enter: RoaringBitmap,   // chosen to send, not yet written to the socket
    dirty: RoaringBitmap,       // changed while the write queue was backed up (see back-pressure)
    out: OutQueues, credits: Credits,
}
```

`sent ∪ pending_enter` is the **subscribed set** S. Ordinals are small dense numbers, so the bitmaps are a few
kilobytes to a few hundred (100,000 clustered objects compress to well under 100 KB; estimate).

**On `SetView`:**

1. Compute `N = query(view, padded)` and `K = query(view, keep)`.
2. `enter = N \ S`; `exit = S \ K` (the gap between the padded and keep regions is hysteresis, so an object near the
   edge does not flap in and out; the idea is standard in interest management).
3. For each id the client listed in `held` (below): if it is in `N`, and the client's `rev` equals the current
   `Rev`, take it out of `enter` (no body needed); if it is not in `N` (deleted, moved away, or now too small), add
   it to `drop`.
4. Drop from `pending_enter` anything not in `K`; stop sending chunks of the older `view_id`.
5. For every object in `enter`, add its **ancestor groups** if the client does not hold them (see groups below).
6. Apply the budgets below, then the picture decision (see overview pictures).
7. Queue chunks in this order: inside the unpadded view before the padding; within each, larger objects first
   (extent in pixels, descending) so the shape of the content arrives before the detail; ancestors before the
   children that need them.
8. Finish with `ViewEnd { view_id, up_to: Seq, eff_min_px, far_dropped, pictures, more }`. After it, the client has
   every object in `N` as of `up_to` and receives `Apply` for later changes.

**Budgets.** A view that would return more than `max_view_objects` or `max_view_bytes` is cut after the objects are
ordered biggest first. The numbers (50,000 objects, 64 MiB) are in section 08's table; the reasoning is here: a
screen shows a few thousand objects at most, 50,000 is an order of magnitude above that, and 64 MiB is what a client
can hold in memory without evicting what it is drawing. The cut
is reported by raising `eff_min_px` to the smallest extent that was sent; the client and section 06 use that value
for culling, so what is drawn and what was sent agree. Opening a very dense surface zoomed out is the case the
overview pictures are for; until they exist, the budget is the guard.

**Far objects.** A **far** object is one whose home row has `tile_px_log2` above 40 (its tile is over a trillion
pixels, so the view lies inside a small part of it). Such objects are backdrops and frames; every occupied coarse
row can contribute some, and an abusive surface can stack one per level over a thousand levels. At most
`max_far_objects` (256, proposed here and held in section 08's table; a few hundred is enough for any honest
nesting) are sent per view, smallest tile first, and `far_dropped` in `ViewEnd` reports how many were left out.

**Client send rate.** While the view is moving the client sends `SetView` **at least every 100 ms, however small the
move**, so that cursor routing (which works from the view padded by one eighth) never uses a stale view; section
08's view-change rate allows more. Most of these messages are cheap: the server stores the new `RoutedView` at once,
and recomputes `N` and `K` only when the view has moved by an eighth of its size since the last computation or the
size of a pixel has changed by more than a factor of 1.25. The half-screen padding leaves at least a quarter screen
of margin in between, so no object enters the real view unsent.

**Routing a change.** After a change is applied and is durable, for each effect (one per touched object: its old and
new box, new rev, deleted or not) and each connection `c` with a view:

| In S before | In `N(new state)` | Sent to `c` |
|---|---|---|
| yes | yes | `Patch` (the change's per-object update) |
| yes | no, but in `K` | `Patch` (it stays subscribed, hysteresis) |
| yes | no, and outside `K` | `Exit { id, rev }`; the client keeps its copy as cold |
| no | yes | `Enter` with the full body, and any missing ancestor groups |
| yes (deleted) | | `Remove { id, rev }` |
| no | no | nothing |

The loop over connections costs about 100 ns per connection per touched object (one exact box test), so a change
touching 100 objects in a room of 50 people is about 500 microseconds. A change touching many thousand objects (a
huge selection moved) is sent as one `Apply` with a compact form and routed by testing each object once; the budget
is 1 ms per 1,000 objects (estimate). There is no reverse index from objects to subscribers; with at most a few
hundred connections it would cost more to maintain than it saves.

### The edges of a partial copy

**The client's copy is a set of objects with enough information to draw them, never a prefix of a log.** Each
object body is a self-contained state that merges into a local copy of the same object in any order
(requirement on section 02). So a client can hold any subset, and an object can arrive late or twice.

1. **An object edited while it leaves the view.** The editor's own edit is applied locally at once. The server
   acknowledges it as for any change. If the object is now outside the keep region, the editor gets `Exit` for it
   after the ack; the client keeps its body as a cold cache entry stamped with the rev from the ack. Objects the
   client has unacknowledged edits on are never evicted until acknowledged. Other people's edits to an object
   outside
   `K` are not sent. The next `SetView` that brings it back lists it in `held` with its old rev, and the server
   sends the new body if the rev differs. The same covers a client whose view is huge in a short time (a jump to
   another person).
2. **A group whose children are only partly loaded.** Children carry their parent id (as in Figma, and as
   `design.md` implies); a group has no list of children. When any object is sent, so are its ancestor groups
   (small objects), so the client can compute the object's world transform. The stored placement is 02's `Xform`
   (`TopXform` for a top-level object, `ChildXform` relative to the parent), so the world box is composed through
   the ancestors by 01's `compose` inside 02's `Doc::apply`, and a child is at most `MAX_LEVEL_SPAN` (8) levels from
   its group. Each group body also carries
   `GroupSummary { bbox, descendants }` that the server computes (not part of the merged state), so selection
   handles and "move the group" work without the children. Moving, scaling or restyling the group is a change to
   the group itself, so it is correct with any subset of children. Commands that need all children (ungroup,
   copy, delete with content, export) first send `FetchSubtree { group }` and wait for the result; the client
   shows "loading" meanwhile. Children outside the view are not drawn and need not be loaded; children that are
   sub-pixel are not sent and need not be drawn.
   *The cost on the server:* moving a group is one write to the group, but it changes the world box of every
   descendant, so `Doc::apply` returns one `ObjectEffect` per descendant and the room re-files each, at about 1 to 2
   microseconds per descendant. A group with more
   than `max_group_descendants` (20,000, in section 08's table; the reasoning is this paragraph) cannot be created
   or grown, because a drag
   would stall the room; the user is told to split it.
3. **An object moved into a view by someone else.** The change's effect has an old box outside and a new box
   inside, so the row "no, yes" of the routing table fires: the client gets `Enter` with the full body. It
   appears at its new place; a client that held a cold copy at the old place gets the new body too, because
   `Enter` replaces the cold copy's state by merging.
4. **Deletes of objects a client never saw.** Nothing is sent. The server keeps the tombstone (section 02, purged
   after 30 days) so that a later edit from someone who had it agrees. A client that held a cold copy finds out at
   its next `SetView`:
   the id is in `held` but not in `N`, so it receives `drop`. A client that sends an edit to an object it holds
   cold, after a delete elsewhere, gets the normal merge result (section 02) and a corrective `Enter` or `Remove`.
5. **A change that is rejected.** The sender receives `Reject { change_id, code, touched }`, followed by `Enter`
   for each touched object in its view, which overwrites the client's optimistic local state with the server's.
6. **An object grows past the pixel threshold or the view zooms.** Objects that become visible by zoom come in the
   `SetView` that the zoom causes. An object that grows (a resize by someone else) while sub-pixel is routed by the
   same table using its new size.

### Wire protocol

**One WebSocket carries everything** in the first version: document changes, view traffic, presence, clock pings and
media messages. Presence is a message class of its own, not a second socket (section 04 keeps the two-socket design
as a documented fallback, triggered if cursor latency during a bulk load misses its target in the spike).

**Handshake (section 08 owns it).** The WebSocket sub-protocol is `everink.v1`; later versions are offered as
`everink.v2` and so on, and the server picks the highest it supports, which is the version negotiation. The server
sends nothing until the client's first frame, an `Auth` of at most 4 KiB, has been verified; the reply is `AuthOk`
(08's `auth_ok`). The first frame is only `auth { token }`: there are no anonymous sessions (owner decision,
2026-11-12), and everyone is identified by the embedding app. The session records `via: Option<LinkId>`, the invite
link the token names (section 08), so that turning a link off closes the sockets that came through it and so that an
owner can suspend or delete the work of everyone who did. Only after `AuthOk` does the room accept the messages
below. Then the client sends `Hello` and
the room answers `Welcome`.

**Framing.** Every WebSocket message is one binary frame holding one `postcard`-encoded `ClientMsg` or `ServerMsg`.
The frame length is checked against section 08's cap before the body is read (the WebSocket library is configured
with that maximum, so an oversized frame is refused while its header is parsed; the exact option names are
unverified). **Bulk data is written in chunks of at most 16 KiB**, so that a cursor update waits behind at most one
chunk. Object bodies and changes inside messages are `Bytes` produced by section 02, optionally LZ4-compressed
(`lz4_flex`) when above 1 KiB, with a flag byte. `permessage-deflate` is not negotiated (08).

**Presence class.** `Presence(Bytes)` is opaque to this section; section 04 defines the payload. A presence payload
is at most 1,000 bytes and self-contained, so that it can move to WebTransport datagrams later without change.
The connection task checks the size before the payload reaches the room. Follow requests travel inside the presence
payload (section 04): the room's presence module refuses a 51st follower of one person with a refusal message from
section 04, and the number is in section 08's table.

**Versioning.** Besides the sub-protocol, the rules for evolving the messages are:

1. Message enums only grow at the end. A variant's fields never change after release; a changed message is a new
   variant (`SetViewV2`) and the old one is kept for the support window.
2. The server accepts clients up to 6 months old (owner decision, 2026-11-12): the current version and every version
   released in the last 6 months. A client older than that gets `Notice { UpgradeRequired }`.
3. An unknown variant is a protocol error: the connection is closed with code `ProtocolError`. Because versions
   are negotiated, this should only happen with a bug.
4. Object and change blobs have their own version in their header (section 02). A server too old for a blob
   version refuses the change with `Reject { code: BlobTooNew }`.

**Anchors are delta-coded inside a message.** A whole `Position` is 19 bytes at the origin, about 272 at level 100
and up to about 3.1 KB at the depth cap (section 01), so a chunk of 200 objects deep in the surface would spend
50 KB on anchors alone. Every message that carries object placements (`Enter`, `Apply`, `FetchResult`) starts
with a `base: Position`. A body is 02's canonical `Doc::body` bytes; on the wire the codec re-encodes each
`Position` inside it (the anchor of an `Xform`) with 01's `Position::encode_delta(base, out)`, a few bytes when the
two are on nearby lattice points, and the receiver restores it with `Position::decode_delta(base, input)`, which is
exact, so the canonical bytes come back identical. 01's encoding falls back to a long difference when positions are
far apart and still never costs more than a whole position plus a few bytes. The server picks the base per chunk
(the centre of the view for `Enter`; the first effect for `Apply`). Incoming positions are checked with
`check_cap_bytes` on the raw bytes before any number is built.

```rust
pub enum ClientMsg {
    Auth { token: Bytes },                 // section 08: the signed connect token, at most 4 KiB
    Hello {
        surface: SurfaceId,
        client: ClientId,
        open_at: Option<VersionId>,        // read-only view of a saved version (08 says who may)
        caps: u32,                         // bit set: lz4, ...
        acked_upto: ClientChangeId,        // highest id this client has seen acknowledged
    },
    SetView { view_id: u32, view: ViewMsg, held: Vec<(ObjectId, Rev)> },   // held: at most 20,000 entries
    Submit { changes: Vec<ChangeFrame>, acked_upto: ClientChangeId },   // changes in order
    Fetch { req: u32, what: FetchWhat },
    Locate { req: u32, what: LocateWhat }, // nearest content, bounds of all content (section 01)
    Presence(Bytes),                       // section 04's payload, at most 1,000 bytes
    Ping { client_ms: u64 },
    VersionCmd { req: u32, cmd: VersionCmd },
    UploadGrantRequest { req: u32, blob: BlobId, size: u64, kind: BlobKind },   // reference for 05 and 08
    BlobGrantRequest { req: u32, blobs: Vec<BlobId> },                         // at most 100 ids (08's table)
    Bye,
}
pub struct ChangeFrame {
    id: ClientChangeId,                          // next number for this ClientId
    replay: bool,                          // made offline: send back the merged state of what it touched
    blob: Bytes,                           // section 02's change encoding, including any `if_stamp` conditions
}
pub enum FetchWhat { Objects(Vec<ObjectId>), Subtree(ObjectId) }
pub enum LocateWhat { Nearest { to: Position }, Bounds }
pub enum VersionCmd {                      // Save and Delete: editor members and the owner; Restore: owner and editor members (08)
    List, Save { name: String }, Delete(VersionId), Rename(VersionId, String), Restore(VersionId),
}

pub enum ServerMsg {
    AuthOk { session: SessionId, expires: u64, role: Role },                      // section 08
    Welcome {
        user: UserId, head: Seq, server_ms: u64,
        window: Window,                    // how much the client may have unacknowledged (numbers: section 08)
        limits: Limits,                    // section 08's table, as the client needs it
        surface: SurfaceMeta,              // bounds, depth caps, home position (opaque to this section)
    },
    ViewBegin { view_id: u32 },
    Enter { view_id: u32, base: Position, bodies: Vec<BodyMsg> },          // chunked, at most 16 KiB each
    ViewEnd {
        view_id: u32, up_to: Seq, eff_min_px: f32, far_dropped: u32, drop: Vec<ObjectId>,
        pictures: Vec<PictureRef>,         // tiles drawn from a picture instead of objects (section 06)
        more: Vec<(PicTile, u32)>,         // tiles with objects not sent, and how many ("more here")
    },
    Apply { seq: Seq, origin: Option<UserId>, base: Position, effects: Vec<Effect> },
    Ack { acked: Vec<(ClientChangeId, Seq)> },   // one (id, seq) pair for every change covered; all are durable
    Partial { change: ClientChangeId, skipped: Vec<u16> },   // `Applied.skipped` of a `partial_ok` change (undo only)
    Reject { change: ClientChangeId, code: RejectCode, touched: Vec<ObjectId> },   // RejectCode wraps 02's `Rejected`; RateLimited carries retry_after
    FetchResult { req: u32, base: Position, bodies: Vec<BodyMsg>, missing: Vec<ObjectId> },
    Located { req: u32, result: LocateResult },
    Presence(Bytes),                       // section 04's payload, at most 1,000 bytes
    Pong { client_ms: u64, server_ms: u64 },        // clock offset for video (sections 04 and 05)
    Versions { req: u32, result: VersionResult },
    UploadGrant { req: u32, grant: Bytes },         // section 08's upload grant, for section 05's HTTP upload
    BlobGrants { req: u32, grants: Vec<(BlobId, Bytes)>, denied: Vec<BlobId> },   // one grant covers a blob's variants
    BlobStatus { blob: BlobId, status: BlobStatus },   // section 05's enum; pushed, never stored in the document
    PictureUpdate { tile: PicTile, picture: Option<PictureRef> },              // section 06; None = removed
    Notice(NoticeKind),                    // RoleChanged, LinkDisabled, RestoreHappened{by,version}, Draining{retry_ms}, UpgradeRequired
    Resync,                                // the server cannot continue incrementally: send SetView again
    Close { code: CloseCode, retry_ms: Option<u32> },
}
pub enum Effect {
    Patch { id: ObjectId, rev: Rev, patch: Bytes },
    Enter { body: BodyMsg },            // includes ancestors the client lacks, ahead of the child
    Exit { id: ObjectId, rev: Rev },
    Remove { id: ObjectId, rev: Rev },
}
/// The wire form of one object: 02's `ObjectBody` (canonical bytes from `Doc::body`) plus what the room adds.
pub struct BodyMsg { rev: Rev, body: ObjectBody, group: Option<GroupSummary> }
```

`BlobStatus`, `BlobKind`, `BlobId` and `RejectReason` are defined by section 05 and `PicTile` and `PictureRef` by
section 06 (see the outlines below); this section carries them unchanged. Message count: 12 client messages, 21
server messages. `Welcome` is sent only after `AuthOk`, and nothing at all is sent before it.

**Ordering.** `seq` is assigned by the room actor and gives a total order of accepted changes; it is also the stamp
of every register a change writes (section 02). `Apply` messages to one connection are in increasing `seq` order
(with gaps for changes it was not sent). The client may rely on this. `Ack { acked }` carries a `(ClientChangeId,
Seq)` pair for every change it covers, so the client learns the sequence number of each of its own changes (section
02's undo and redo need it). Every pair is durable. For a local change, any `Apply` with `seq` below its pair's
`seq` is "before it", and any above is "after it". The recommended client rule, from Figma's design, is to hold back
incoming patches to
a property while a local change to that property is unacknowledged, and apply them after the ack; this prevents a
value flickering back. It is section 02's rule to confirm.

**Accept or reject, never rewrite.** The room accepts or rejects a change as a whole. It never edits a change,
re-stamps part of it or merges parts away. Conditional writes are part of the change (section 02's `if_stamp`: apply
this write only if the register still has this stamp, which is how undo avoids overwriting someone else's later
work). The conditions are checked inside `Doc::apply(&change, &ApplyCtx { seq, author, ext })` against the state at
the moment the change reaches the front of the actor's queue. If one fails, the change is rejected with `Reject {
code: Conflict, touched }`, and the sender
then receives the current bodies of `touched` (the rule in "The edges of a partial copy"). The room may also author
changes of its own (`Author::Server`: pausing playing videos, rebalancing order keys, `DeleteBy`); those are new
changes with their own `Seq`, not rewrites of a client's. The only exception to all-or-nothing is a change marked
`partial_ok` (undo only, section 02), which is accepted with some operations skipped; the room tells
the sender which with `Partial`. The log stores the change exactly as submitted, and because conditions are decided
by the state that the log order produces, replay reaches the same decisions.

**Acknowledgement and durability.** The actor applies a change in memory and assigns its `seq` at once, but
**releases both the `Ack` and the `Apply` messages to others only after the store reports the batch durable** under
the room's durability mode (local disk by default; local disk and a second copy in strict mode).
Reason: if `Apply` went out first and the server then crashed, other clients would hold a change the log lacks and
their copy would diverge with no one to reconcile it. The cost is one commit latency (about 1 to 5 ms on an SSD,
unmeasured) added to remote delivery, not to the author, who sees the change at once. The store thread commits
whatever has arrived while the previous commit was running (group commit), so load does not increase latency per
change. Presence and `Pong` are never held for the commit.

**Idempotent resubmission.** A client resends every unacknowledged `ChangeFrame` after a reconnect. The room
keeps, per `ClientId`, the highest `ClientChangeId` it has logged and the `(ClientChangeId, Seq)` pairs of the
changes that client has not yet confirmed receiving an `Ack` for. Every `Submit` and every `Hello` carries
`acked_upto`, the highest id the client has seen acknowledged, and the room drops the pairs at or below it, so
the list holds at most the unacknowledged budget in section 08's table and is normally a handful. A frame with `id
<= highest` is
not applied again and is answered with its original pair in an `Ack`. The map is saved in every snapshot and
rebuilt from the log tail, so it survives a crash. It is trimmed for clients idle 30 days. An idle client costs
about 40 bytes (4 MB for 100,000 devices); a client that vanished with a full window of unconfirmed changes costs
up to about 4 KB until it is trimmed, which is 400 MB only if all 100,000 did so at once (an estimate).

**Flow control and back-pressure.**

- *Client to server:* `Welcome.window` gives the client a budget of unacknowledged bytes and changes: 1 MiB and 256
  changes, proposed here and held in section 08's table. `Ack` returns the budget. A client past the budget waits
  (it keeps queueing locally). A
  client that sends beyond the budget plus a margin is closed. Section 08's rate limits apply on top, per role.
- *Server to client, classes.* Each connection has four outbound queues:

  | Class | Content | Rule |
  |---|---|---|
  | P | `Presence` | Newest state per sender only; older ones are replaced, never queued; served first |
  | C | `Pong`, `Ack`, `Reject`, `Notice`, `Close` | Small, never dropped; served after P and before A and B; `Pong`
  is written as soon as the reader task sees `Ping`, without passing through the room actor or the change queue |
  | A | `Apply`, `BlobStatus`, `PictureUpdate` | Never dropped; coalesced under pressure (below) |
  | B | `Enter`, `ViewEnd`, `FetchResult`, `Located`, `Versions` | Bulk; chunks of at most the current unsent limit
  and never over 16 KiB; lowest priority but never starved: after every 8 chunks of A or B one chunk of the other is
  served |

- *Pacing, adaptive.* The rule: **keep the connection's unsent bytes below what it drains in about 20 ms, clamped
  to between 4 KiB and 64 KiB, and write bulk data in chunks no larger than that limit and never over 16 KiB.** A
  fixed 64 KiB fails slow links (131 ms of queue at 5 Mbit/s, 655 ms at 1 Mbit/s, by section 04's arithmetic).
  With this rule a cursor update waits about 40 ms at most (one chunk plus the unsent bytes, 20 ms each), which fits
  the 47 ms left of the 200 ms cursor budget. The 4 KiB floor takes over below about 1.6 Mbit/s (4 KiB drains in
  20 ms at 1.64 Mbit/s); there the wait is two floors' drain time, 66 ms at 1 Mbit/s and 660 ms at 100 kbit/s.
  On such links the server sends fewer objects first by raising `eff_min_px` for the opening fill, so less bulk
  data is queued. 20 ms is the first value the spike tries. The bound counts bytes unsent on this host; bytes
  already queued in the network (a router in front of a slow last hop) are not modelled, so such a link can add
  delay beyond the figures here. Examples: 5 Mbit/s gives a limit of 12.5 KB, 13 Mbit/s
  gives 32 KB, and 26 Mbit/s or more gives 64 KiB.
  With tokio it is done in two layers. First, the accepted `TcpStream` gets `TCP_NOTSENT_LOWAT` through the
  `socket2` crate before the WebSocket upgrade, set to the current limit (and set again when the limit changes);
  the kernel then reports the socket writable only when its unsent queue is below it, so `poll_write` returns
  `Pending` by itself (Linux supports the option; macOS has a similar one; elsewhere unverified). Second, as the
  rule that works where the option is missing or when TLS or a proxy sits between, the writer task keeps its own
  count of bytes handed to the WebSocket sink and not yet flushed, and takes a class A or B chunk only when the
  count plus the chunk is under the limit; it flushes after each chunk. The writer is one task per connection that
  loops: take from P, else C, else (if under the limit) A or B, `feed`, `flush`.
  *Estimating the drain rate.* While the writer has a backlog, every time `poll_write` or `flush` accepts bytes it
  records (time, bytes accepted). Because the kernel accepts bytes only as its queue drains (with the option set
  at the limit), bytes accepted per second during a backlog is the drain rate. The writer averages it over windows
  of at least 50 ms (exponentially weighted, weight 1/4 for the newest window), ignores windows with no backlog
  (an idle link tells nothing), and sets `limit = clamp(rate × 20 ms, 4 KiB, 64 KiB)`. Before the first estimate
  the limit is 16 KiB. Behind a TLS-terminating proxy the option only covers the hop to the proxy and the estimate
  then measures that hop; the second layer still holds.
- *A slow client:* when its write queue exceeds 4 MiB or 2 s of backlog, the server throws away the queued class A
  messages, adds their ordinals to `dirty`, and when the socket drains sends the current body of each dirty
  object as `Enter` (state-based merge makes the latest state enough). Memory is therefore bounded by the bitmap.
  If the backlog does not drain in 60 s, the connection is closed with `Slow`, and the client reconnects normally.
- *Browsers:* the WebSocket API gives the page no way to pause receiving, so the client's only back-pressure is
  its own reading speed; the server's queue limits are what protect it.

**Heartbeat and time.** The client sends `Ping` every 5 s and the server answers `Pong` from the connection task
(class C, outside the ordered change queue). Section 05 keeps the minimum-delay sample of the last 8 and uses it for
video; section 04 uses the same offset. No `Pong` for 15 s closes the connection. A presence timeout for others is
section 04's.

### Reconnect and offline replay

The client keeps a durable **outbox**: every local change it has produced, in order, until acknowledged, in
IndexedDB
or OPFS (web) or a file (native). It keeps the **cache** next to it (below).

```
on connect (first time or after a loss):
    send Auth{ token } ← AuthOk;  send Hello{ client, surface } ← Welcome
    send SetView{ view, held = cache entries intersecting the new view that are cold }
    send Submit{ outbox in order, replay = true for those made while disconnected }
    server: dedupe by ClientId/ClientChangeId; apply the rest; Ack; for replay frames, route the merged state of
            touched objects back to this client as Effect::Enter or Patch
```

There is no session resume and no per-connection history on the server. The `held` list is the whole catch-up
mechanism: the server compares revs and sends only what differs, so it is correct after a one-second drop, after a
crash and restart of the server, and after two weeks offline. It uploads at most 20,000 entries of 24 bytes (480
KB),
typically a few hundred (about 10 KB). If a client holds more cold objects in the view than that, it lists the
nearest 20,000; the rest are simply re-sent.

Offline changes merge by section 02's rules against whatever happened meanwhile. Stamps are the room's `Seq` at
acceptance, so a replayed change is stamped when it is accepted and wins over older online edits to the same field.
Replayed offline changes are unconditional by default (02); only undo carries `if_stamp`. This is a question for the
owner and may change; the protocol carries either. If a restore happened meanwhile, the offline changes
apply on top of the
restored state; the client is told with `Notice::RestoreHappened`.

A stroke that was in progress when the connection died is not kept by the server: the preview fades on other screens
(section 04), and the author's device, which saves a stroke to its outbox every 250 ms while it is being drawn
(section 07), sends it later as an ordinary finished change (owner decision, 2026-11-12). That holds when the
app dies as well as when the connection does. The server never commits a stroke on someone's behalf. A change whose
author's role was removed while offline is refused with `Reject { code: Forbidden }`; the client
keeps
it in a "not accepted" list and shows it (section 07 decides how) instead of discarding it silently.

### The client cache and its eviction

The client holds three sets: **subscribed** (kept current by the server), **cold** (kept only as a copy with the
rev it had), and **pinned** (unacknowledged edits, selected objects, objects being dragged; never evicted).

- Objects the server sends `Exit` for, and objects outside `K` after the client moves, become cold.
- Cold entries are kept in the same local spatial index as subscribed ones (the client index is section 01/06's) so
  the client can find the cold objects that intersect a new view.
- The cache has a byte budget (default 256 MiB native, 128 MiB web; to be tuned by the spike). Over budget, cold
  entries are evicted least recently used first. If still over, the client narrows its padding to a quarter screen,
  and as a last resort sends a smaller `View.w/h` (visible effect: holes at the screen edge while panning).
- An evicted object is simply absent; nothing needs to be told to the server.
- On disk (native) or in IndexedDB, the cache is the same entries, written lazily. Signing out clears it
  (`design.md`).

### Persistence

**One SQLite file per room** (`rusqlite` 0.42.0, MIT, with the `bundled` feature), in WAL mode with
`synchronous=FULL`, so a commit is durable when it returns (with `NORMAL`, the SQLite documentation says a commit
might roll back after power loss, which would break the acknowledgement rule;
[WAL](https://www.sqlite.org/wal.html)). WAL has one writer at a time, which is how a room already works. Readers
(a snapshot job, a version opened read-only) do not block it. The file is on local disk; WAL does not work over a
network file system.

```sql
CREATE TABLE log   (seq INTEGER PRIMARY KEY, time_ms INTEGER, user BLOB, client BLOB, change_id INTEGER,
                    crc INTEGER, blob BLOB);                       -- one row per accepted change
CREATE TABLE chunk (hash BLOB PRIMARY KEY, data BLOB, blobs BLOB);  -- content-addressed, compressed; blob ids inside
CREATE TABLE snap  (id INTEGER PRIMARY KEY, seq INTEGER, time_ms INTEGER, manifest BLOB, -- chunk hashes
                    dedupe BLOB, kind INTEGER, name TEXT, by BLOB, ord_next INTEGER);   -- kind: auto, named
CREATE TABLE meta  (k TEXT PRIMARY KEY, v BLOB);                   -- format version, tenant, surface id, lease epoch
CREATE TABLE blob_ref (blob BLOB PRIMARY KEY, first_seen INTEGER, last_referenced INTEGER);   -- media support
CREATE TABLE picture (octave INTEGER, tx BLOB, ty BLOB, version INTEGER, blob BLOB,
                      order_min BLOB, order_max BLOB, built_seq INTEGER, PRIMARY KEY (octave, tx, ty));
CREATE TABLE audit (seq INTEGER, time_ms INTEGER, session BLOB, user BLOB, action TEXT, detail BLOB);
```

**The log** is the sequence of accepted changes exactly as the room applied them (section 02's encoding), with a
CRC32C per row against bit rot. A batch from the actor is one transaction. Replaying the log from a snapshot with
the same code must give the same document, so the room has no hidden state that is not in the log.

**Snapshots by cohort chunks.** The document is divided into **chunks** by creation order: chunk `c` holds the
objects with ordinals `256·c .. 256·c + 255`, serialized as section 02's canonical `Doc::body` bytes (same state,
same bytes), compressed with LZ4, and stored under its BLAKE3 hash. A snapshot is a **manifest**: the
list of chunk hashes in order, plus the sequence number and the dedupe table. Because most objects never change
after the pen lifts, and new objects fall into the newest chunks, a snapshot only rewrites chunks that changed
since the last one: the room keeps a set of dirty chunk numbers, and taking a snapshot serializes only those. A
snapshot of a 1-million-object surface after 600 new strokes writes about 3 chunks plus a manifest of about
4,000 hashes (128 KB). The trade-off: editing one old object rewrites its 256-object chunk (about 128 KB), so
heavy editing of old content costs write amplification; the chunk size (256) is a starting value to tune.

*Taking a snapshot without stopping the room:* the document is held in 4,096 reference-counted shards. The actor
clones the shard pointers (microseconds), hands the clone to a background job, and then mutates shards with
copy-on-write, so the job sees a frozen state at a known `seq`. The job writes the dirty chunks and the manifest in
one transaction.

*When:* on the first of: 5 minutes since the last snapshot with at least one change since, 16 MiB of log since the
last snapshot, or room unload. A final snapshot at unload makes the next load fast.

**Compaction and retention.**

- *Log:* rows with `seq` at or below the second-newest snapshot's `seq` are deleted. Keeping the log from the second
  newest means that if the newest snapshot is found corrupt, the previous one plus the log still gives the current
  state. A debug setting keeps the log for 24 hours for support.
- *Chunks:* a chunk is deleted when no retained manifest names it (mark and sweep by hash, run after the daily
  expiry job).
- *Space:* `auto_vacuum=INCREMENTAL`; the file is shrunk by the expiry job, not on the hot path.
- *Not done in the first version:* squashing runs of property writes in old log rows. Dragging an object sends one
  patch per tick, so a long drag fills the log; the remedy is on the client (section 02 and 04: preview a drag as
  presence, commit it on release or at most 5 times a second, every 200 ms), not in the store.

**Recovery after a crash** (runs at every room load, because a clean unload and a crash look the same):

1. Open the SQLite file; SQLite's own WAL recovery restores a consistent state (no row of a batch is partial).
2. Read the newest `snap` row whose manifest verifies (manifest checksum, and every named chunk exists and its
   hash matches). If it does not verify, take the next older one and record an alarm.
3. Decode the chunks into the document. Replay `log` rows with `seq > snapshot.seq` in order; check `seq` is
   contiguous and each CRC matches; stop and alarm at the first gap or mismatch (this can only be bit rot, since
   transactions are atomic).
4. Set `next_seq`, rebuild `dedupe` from the snapshot plus replayed rows, rebuild the spatial index from the
   document, open for connections.
5. Clients reconnect on their own; each resends unacknowledged changes; `dedupe` makes that safe. A change that was
   applied in memory but not durable when the server died was never acknowledged and was never sent to anyone else,
   so it exists only in the author's outbox and is resent.

`everink-server repair <room-file>` runs steps 1 to 3 on a copy and reports. Estimated load time for 1 million
objects: decode about 5 s, index 1 to 2 s (unmeasured; the spike measures it).

**Durability modes (owner decision, 2026-11-12).** The `RoomStore` trait (append batch, read snapshot manifest, read
chunks, write snapshot, list/expire) hides SQLite, so a Postgres store can be added for a hosted service. A room
runs in one of two modes, chosen by the operator (per deployment, or per tenant where 08's quotas allow):

- **Default: confirm after the local disk, copy in the background.** A change is confirmed (`Ack`, and `Apply` to
  others) once its batch has been committed and synced on the local disk. A background task copies each new snapshot
  and the log tail to a second place (a directory in single-node setups, S3-compatible storage otherwise). If the
  whole machine is lost, the room reloads elsewhere from the copy and the changes confirmed since the last copy are
  lost: the loss window is the copy lag, aimed at under a minute (estimate). The authors' outboxes still hold
  anything not yet confirmed.
- **Strict: confirm only after a second copy.** The `Ack` and `Apply` are released only after the batch is durable
  locally **and** a second copy has confirmed it. The second copy is a replica: another node, in another failure
  domain, that keeps a copy of the room's log and snapshots; the store thread ships each batch (with the lease
  epoch) to the replica's `LogSink::append(epoch, batch)`, which syncs it to its own disk and replies. Strict mode
  needs: a second machine with local disk; a network round trip per commit, which adds an estimated 10 to 50 ms to
  remote delivery and to the `Ack` (group commit keeps throughput, since a batch carries everything that arrived
  meanwhile); the same lease and fencing as above, so that on failure the replica takes the lease with the next
  epoch and loads from its copy with no confirmed change lost; and a rule for a replica that is unreachable. The
  rule is to refuse writes (`Reject { Unavailable }`, the room stays readable) after 5 seconds rather than quietly
  fall back to the default mode, because quietly falling back would break the promise strict mode makes.
  Operators who want availability over the promise run the default mode.

### Versions

**Restore points.** Every snapshot is a restore point, in the `snap` table with `kind`:

- `auto`: created by the snapshot rule above, plus one immediately before a restore and one before any change that
  deletes more than 1,000 objects. Thinned with age, see Retention.
- `named`: created when an editor member or the owner saves a version (the same rule as restoring, which the owner
  decided on 2026-11-12; saving and deleting follow from it). The room takes a
  snapshot of the current state at `seq` right
  now (it is cheap: dirty chunks only) and marks it `named` with the name and author.

**Retention (owner decision, 2026-11-12).** Automatic points thin with age exactly as follows: every one is kept for
24 hours, then one an hour for a week, then one a day up to 30 days, after which they are deleted. Named points are
never thinned and never deleted by the system. The cleaning job applies the rule daily and marks chunks no retained
manifest names for the sweep. Because a version is a manifest over shared chunks, a named version costs only the
chunks that differ from other retained snapshots: a rough figure is the size of the content changed since the
nearest other snapshot (estimate).

**Opening a version (read-only).** The client connects with `Hello { open_at: Some(version) }`. The server creates a
temporary read-only room from that manifest (no log to replay), builds its index, and serves it with the same
protocol; `Submit` is refused with `Forbidden`. This reuses the whole view machinery, so a version can be panned and
zoomed like the live surface. Copying objects out of it into the live surface is a client-side copy-paste across
two connections (section 07). The temporary room is dropped when its connection closes or after 10 idle minutes.

**Restoring while people are editing.** A restore is a change like any other, so everyone's view stays valid and no
one is disconnected.

1. An owner or an editor member sends `VersionCmd::Restore(v)` (owner decision, 2026-11-12; the restore is audited).
   The room first takes an automatic snapshot called "before restore".
2. A background job computes a **restore change** from the live document (state at `s0`) to the version's state,
   using
   section 02's `Doc::diff_to(&Doc) -> Change`. The change sets every differing property with a fresh stamp
   (so it wins over everything earlier), resurrects objects that exist in the version and are deleted now, and
   deletes objects that do not exist in the version.
3. Because the diff may take seconds on a big surface, the job repeats: after applying, it computes the difference
   for
   objects whose `Rev` is above `s0`, applies that, and repeats until it is empty; the last round runs with the
   actor briefly refusing new `Submit`s (queued, not dropped), normally under 50 ms (estimate).
4. All connections receive the restore as ordinary `Apply` effects (for objects in their view) and, right after,
   everyone on the surface is told with `Notice::RestoreHappened { by, version, at: Seq }`. No undo stack is
   cleared: section 02's conditional undo
   (`if_stamp`) already fails for any entry whose registers the restore re-stamped, so a stale entry reports
   "Nothing to undo" instead of undoing the wrong thing. Schema migrations in section 02 read old versions from
   their snapshot chunks, because the log is trimmed.
5. Changes that were in flight from other people arrive after the restore and are applied on top of it by the
   normal merge rules. They are not lost, and they may re-create things the restore deleted, as `design.md`
   already says. Undoing the restore means restoring the "before restore" point.

Other people are not warned beforehand; they are told right after (owner decision, 2026-11-12).

### Media support (section 05)

Section 05 owns the media pipeline (upload, derivation, tiles, the blob store). This section carries the messages
and keeps the records that tie blobs to a surface. Files travel over HTTP, never on the WebSocket.

- **Upload and grants.** The message names and fields here are the reference that sections 05 and 08 use.
  `UploadGrantRequest { req, blob, size, kind }` is checked against the sender's role and section 08's media limits,
  and answered with `UploadGrant`: the upload grant defined in section 08, bound to user, surface, blob and size,
  valid until the session ends and for at most 24 hours. It is the only credential for section 05's resumable HTTP
  upload. `everink-server` checks every upload request (`POST`, `PATCH`, `HEAD`) against the revoked-session list
  and the session's upload floor (section 08), so an upload is refused after a link is turned off, after the session
  ends, and after its role loses upload rights. The room raises the floor on each of those events.
  `BlobGrantRequest { req, blobs }` (at most 100 ids, from section 08's table) is answered with `BlobGrants`: one
  grant per blob, which covers all of the blob's
  variants and tiles, and is issued only if this surface references the blob (the reference index below
  decides). The grant itself (a MAC over tenant, surface, blob hash, session id and expiry) is built by section 08's
  code; this section only checks the reference and relays the bytes.
- **`BlobStatus` push.** The media worker reports each blob's state (receiving, stalled, processing, partial,
  ready, abandoned, rejected, as defined by section 05) to the room over an in-process channel. The room keeps one
  status per blob **in memory and in no document**, throttles pushes to 2 per second per blob, and sends them
  (class A) to every connection that holds an object using that blob, found as `blob_objs[blob] ∩ conn.sent`. A
  client that connects later gets the current status of the blobs in its first `Enter` chunks.
- **Blob records and the reference index.** Per room file (that is, per surface):

  ```sql
  CREATE TABLE blob_ref (blob BLOB PRIMARY KEY, first_seen INTEGER, last_referenced INTEGER);   -- Seq values
  -- chunk.blobs (see Persistence): the blob ids named inside one snapshot chunk
  ```

  `Doc::apply` reports, per change, the blobs that new or changed objects reference (a requirement on 02's
  `ObjectEffect`). The room then upserts `blob_ref` in the same transaction as the change, and updates the
  in-memory `blob_objs: HashMap<BlobId, RoaringBitmap>` used for status routing. A reference is never removed when
  an object is deleted (05: undo can bring it back, and snapshots still hold it).
- **Liveness and the daily sweep.** Section 05 runs the sweep once a day per tenant. This section supplies the
  function it requires:

  ```rust
  /// Blobs referenced by one stored state of a surface: the current state, one retained snapshot, one named
  version.
  /// Loads the state and calls section 02's pure `blobs_in_state(&Doc)`; the tenant comes from the store's scope.
  fn blobs_in_store(store: &dyn RoomStore, state: StateRef) -> impl Iterator<Item = BlobId>;
  ```

  For a snapshot it is the union of the `blobs` lists of the chunks in its manifest (each list is written when the
  chunk is written, so no chunk has to be decoded); for the current state it is the keys of `blob_objs` plus the
  not-yet-snapshotted tail. The same daily job purges deleted objects 30 days after deletion, the same period
  automatic snapshots are kept; the purge rewrites the chunks that held them and never reaches into named versions.
- **Videos are paused at the right moments.** After recovery on room start, the room asks the document for every
  video whose shared state is `Playing` and commits one change, authored by the server (`Author::Server`), that sets
  each video's `PlaybackState` register to `Paused` at its last known position (sections 02 and 05). It does the
  same when the last connection leaves. A
  snapshot or version opened read-only has `Playing` shown as `Paused` in memory, with no change written.

### Overview pictures (outline, build step 6)

Section 06 defines the pictures and the builder; this section stores the counts and the picture table, decides per
tile whether to send a picture or objects, and triggers the builder. Detail is left to build step 6; the shapes
below are what the earlier steps must not make impossible.

- **Home octave.** The core computes each object's home octave with section 06's `home_octave` (the octave at
  which its longer side is 4 to 8 px) once, from its `ExactBox`, and the object table stores it (an `i32`, which
  the depth cap keeps in range). Octaves are 06's, in steps of `2^1`, and are independent of this section's rows.
- **Counts.** `pic_count: HashMap<PicTile, u32>`, where `PicTile` is 06's `{ octave, tx: Cell, ty: Cell }` with
  global tile coordinates. An object is counted once in each tile that 06's `picture_tiles` gives for it: the five
  octaves `h(o) - 4 ..= h(o)`, at most four tiles each, so at most 20 updates per insert, move or delete, whatever
  the depth. They are built in the background after a room opens (pictures are not needed to open it) and updated
  with the index. Cost estimate: under 1 microsecond per update.
- **The per-tile decision.** In `SetView` processing, for each tile of the view's tile octave `Z_v`, the objects
  with `h(o) >= Z_v` that touch it are counted: `pic_count[(Z_v, tile)]`. If the count is above the threshold
  (starting value 2,000, 06's) and a picture exists, those objects are left out of `enter` and the tile's
  `PictureRef` goes in `ViewEnd.pictures`. Otherwise the objects go out by the normal rule. Objects with
  `h(o) < Z_v` are always sent as objects.
- **The "more here" marker.** `ViewEnd.more` lists, per tile in view, `pic_count` minus the objects actually sent,
  when that is above zero, so section 06 can draw its marker where content exists that was not sent.
- **The picture table** in the room store: `picture(octave, tx, ty, version, blob, order_min, order_max,
  built_seq)`. Pictures are ordinary hash-named blobs in section 05's store; the table says which blob is current.
  `order_min` and `order_max` are the smallest and largest order key among the members when it was built (06 uses
  them to place the picture in the draw order).
- **Trigger.** A change marks dirty each tile whose count it altered. A scheduler task in the room enqueues a build
  job for a dirty tile whose count is over the threshold, after 5 s without further change to it and at the latest
  60 s
  after it first went dirty (06). A job names `(tile, as_of_seq)`; the builder is a separate process that reads the
  tile's members through a read-only view of the room and uploads the result as a blob. When it reports back, the
  room writes the table row, bumps `version`, and pushes `PictureUpdate` to connections whose view includes the
  tile.
  A tile whose last member is gone has its picture removed (`PictureUpdate { picture: None }`).

### Queries for the camera (section 01)

Two queries that section 01 needs, both answered from the index:

- **Nearest content.** `Locate { Nearest { to } }` returns the object whose bounding box is nearest to `to` across
  all levels, as its id and an exact anchor `Position`. The search visits occupied rows, in each doing a
  nearest-neighbour walk in the buckets around `to` with `rstar`, and compares candidates by exact distance (01's
  `sub`). It stops with the best so far after a 10 ms budget and marks the answer approximate. This is the "go to
  nearest content" button.
- **Bounds of all content.** `Locate { Bounds }` returns 01's `Bounds`: the exact union of all boxes. The room keeps
  it as the union of each bucket's root envelope, recomputed lazily when a delete or move might have shrunk it
  (a dirty flag, then one pass over buckets, which is O(buckets)), and caches it with the `Seq` it was computed at.
  This is "fit all" and the "home" fallback.

Both count as view changes for the rate limits in section 08.

### Hooks for identity and security (section 08)

- **Nothing before `AuthOk`; frame length first.** The connection task sends no frame (not even `Notice` or `Close`
  with a reason) before the first `Auth` frame has been verified, and checks each frame's length against the
  limit before it reads the body. Unauthenticated sockets are handled by 08's handshake code, not by a room.
- **Every message is authorized.** The connection task calls `Authz::check(session, operation)` for every
  `ClientMsg` after `AuthOk`, and the HTTP handlers (upload, blob grants, export) call it for every request,
  against the session as it is at that moment (a role change takes effect on the next message).
- **Storage takes a tenant scope.** `RoomStore::open(scope: TenantScope, surface: SurfaceId)` is the only way to
  get a store. The file path includes the tenant, `meta` records tenant and surface, and a mismatch on open is a
  fatal error. Nothing in a store is addressable across tenants (chunk hashes are per room file).
- **Restores are audited.** Version create, rename, delete and restore each write a row in `audit(seq, time,
  session,
  user, action, detail)` in the same transaction as the change, and are sent to 08's audit sink. A restore records
  who, which version and the "before restore" point.
- **Revoked sessions on blob requests.** `everink-server`, which serves the media endpoint, keeps the list
  `RevokedSessions`: session ids with the time they were revoked, in memory behind a read-write lock, shared
  between the room tasks and the HTTP blob handlers. An entry is kept for the longest grant lifetime (2 hours,
  video), or until the room is loaded again, when the new lease epoch makes every earlier grant invalid. The room
  adds a session synchronously, before it answers the owner who turned an invite link off, in the
  same
  step that closes that link's sockets. Every blob request is checked against the list after the grant's MAC. One
  limit remains, and it is section 08's and 05's to state: where the store hands out a short-lived presigned
  address to object storage, up to 60 seconds of access can outlive a revoked session. On several nodes the list
  is also published through the coordination store (outline).

### Room lifecycle and placement

```
Unloaded --first connection--> Loading --recovery done--> Active --last connection leaves--> Idle(60 s)
   ^                                                         |                                   |
   |                                                         +--connection arrives--------------+
   +------ final snapshot, close file, free memory ----- Draining <-- idle timeout or memory pressure
```

- *Loading:* the connection is accepted and authenticated, but `Welcome` waits for recovery. Clients see a spinner.
  After recovery the room pauses every playing video (see Media support) before it opens.
  Loads of different rooms run in parallel on a bounded pool.
- *Idle:* no connection for 60 s (setting); costs memory only. Under memory pressure, idle rooms are drained oldest
  first. A room with a connection is never drained, except for a deploy or a move, which send `Notice::Draining`
  and `Close { retry_ms }` so clients reconnect (to the new host, if moved).
- *Draining:* stop reading input, wait for the pending batch to be durable (Figma does the same for deploys, with
  under 1 s at the 99th percentile), final snapshot, close.

**Placement.** The room for `surface_id` is on exactly one machine at a time. Machines are chosen by rendezvous
(highest random weight) hashing of the surface id over the live node list, so adding or removing a node moves only
`1/n` of the rooms. A room is only loaded after taking a **lease** (a row in a small coordination store: Postgres,
etcd or a file lock on a single node) with an increasing epoch, renewed every 5 s; the epoch is written into `meta`
and into each store transaction so that a machine that lost its lease cannot append (a stale writer's commit is
refused). This is the same single-writer rule as Figma's lock table. The epoch also names the room's incarnation for
grants: section 08 puts it inside every blob read grant and upload grant, and the blob and upload endpoints refuse a
grant whose epoch is not the room's current one. A room that is loaded again after a crash, a restart or an idle
unload has a new epoch, so the revoked-session list and the upload floors, which are held in memory, never need to
survive it. When the room is not loaded (idle, crashed, moving), the blob and upload endpoints have no room to ask,
so they read the room's current lease epoch from the coordination store, which is also how an endpoint on a node
other than the owner learns it (cached for a few seconds and re-read on a mismatch; with one node the epoch is
local). A grant whose epoch is not the current one is refused with a status that tells the client to reconnect, and
a reconnect loads the room and returns fresh grants. Clients connect to any gateway node (`axum` behind a load
balancer), which forwards the WebSocket to the owner node by surface id; or the load balancer does it by a hash of
the surface id in the URL, which is simpler and is the recommended first step. Single-node deployments skip all of
this.

The room data is on the owner's local disk. Moving a room is: drain it (final snapshot, WAL checkpoint), copy the
single file or restore from the replicated copy, take the lease on the new node, load. A hot room cannot be split
across machines (one actor);
the ceiling is one machine's core and memory (see capacity), the same limit Durable Objects have.

### Capacity estimates

The headline case is the first app, a whiteboard, with about 50 active people per surface (owner decision,
2026-11-12). Assumptions (all unmeasured; the spike replaces them with numbers):

| Assumption | Value |
|---|---|
| Planning case: a whiteboard, active people on one surface | about 50 (socket cap: section 08) |
| Stroke bytes at commit | 1.2 KB |
| Stroke rate per active drawer | 0.3 per second |
| Presence message | 60 bytes, 30 per second per user |
| Objects in a large surface | 2 million (the first-version cap) |
| Object header (section 02) | about 110 bytes near the origin |
| Stroke body (section 02's typical, packed) | about 1 KB for 200 pen samples; every object is taken to be a stroke |
| Index entry and object table (this section) | about 170 bytes per object |

Derived:

| Quantity | Calculation | Result |
|---|---|---|
| Changes into a busy room | 50 · 0.3 | 15 per second, 18 KB/s |
| Changes out, worst case (all views overlap) | 18 KB/s · 49 | 880 KB/s, 7 Mbit/s |
| Presence into a busy room | 50 · 30 · 60 B | 90 KB/s |
| Presence out, worst case (all see all) | 50 · 49 · 30 · 60 B | 4.4 MB/s, 35 Mbit/s |
| Presence out, typical (each sees a fifth) | 4.4 MB/s / 5 | 0.9 MB/s |
| Actor CPU for changes | 15 · (20 µs apply + 50 · 0.1 µs route + encode) | well under 1% of a core |
| Actor CPU for presence worst case | 73,500 per second · about 1 µs | about 7% of a core |
| First fill of 2,000 objects | 2,000 · 1 KB, LZ4 about 2 times | 1 MB, 0.4 s at 20 Mbit/s |
| Server memory at the cap, estimate | 2M · (110 B header + 1,000 B stroke body + 170 B index) | about 2.6 GB; about 4 GB if the in-memory form costs 50% more than the packed form |
| Per connection | two bitmaps (up to 100 KB), 256 KB buffers | under 0.5 MB, 50 people is 25 MB |
| Log growth, busy | 18 KB/s | 65 MB per hour |
| Log growth, ordinary (5 people, 0.1 strokes/s each) | 0.5 · 1.2 KB | 0.6 KB/s, 2 MB per hour, trimmed at the second snapshot |
| Commits per second per room | one per commit round trip, 1 to 5 ms | up to 200; usually 20 to 50 |
| Active rooms per host, memory-bound | 32 GB / (100 MB typical big board) | about 300; about 10,000 for 1,000-object boards |

Presence worst-case fan-out is the largest figure, and it is section 04's to reduce (lower rate when many receive,
cursors beyond a count shown as a list). Past the 2-million cap, the route is to keep object bodies in the chunk
cache instead of RAM: chunks are the paging unit already, and the index (170 B per object, 1.7 GB at 10 million)
stays resident. That is the design Liveblocks describes for its second engine. It is not part of the first version;
the first version rejects creating objects past `max_objects` (2,000,000; owner decision, 2026-11-12; section 08).
The cap follows from
the memory estimate: 2.6 to 4 GB for the surface, plus under 0.1 GB for 200 connections and a transient copy of the
shards a snapshot job is writing, fits a host with 8 GB; ten times the cap (13 to 20 GB) does not.

## Failure modes

| Failure | What happens | Why it is safe |
|---|---|---|
| Room process crashes | Clients reconnect, resend unacknowledged changes; room recovers from snapshot plus log | Ack only after durable; resubmission is idempotent |
| Crash between apply and commit | The change was never acknowledged or broadcast; the author's outbox resends it | Nothing outside the room saw it |
| Machine loses its disk | Room reloads on another node from the replicated snapshot and log tail; changes after the last replication are lost | Default mode: the author's outbox still has changes that were not acked; acked changes inside the copy lag are lost (stated limit). Strict mode: none are lost |
| Strict mode: the replica is unreachable | After 5 s the room refuses writes with `Reject { Unavailable }` and stays readable; it does not fall back to the default mode | Strict mode never confirms a change that has no second copy |
| The room is not loaded when a blob or upload request arrives | The endpoint reads the current lease epoch from the coordination store, refuses a grant with another epoch, and the client reconnects and gets fresh grants | The epoch names the incarnation of the room |
| The connection dies in the middle of a stroke | The preview fades on other screens; the author's device sends the finished stroke from its outbox on reconnect; the server keeps nothing | The server never commits a stroke on someone's behalf |
| Two machines both load a room | The one without the current lease epoch cannot commit; its connections are closed | Epoch check in every store transaction |
| Log row corrupted | Detected by CRC at load; recovery stops there and alarms; the previous snapshot path is tried | Second-newest snapshot is retained |
| Newest snapshot corrupted | Fall back to the previous snapshot and replay | Log kept from the second newest |
| Index disagrees with document | Rebuilt at load; a debug build checks after every change | Index is derived |
| Client sends a hostile view (huge, deep, every second) | Size and rate clamps, `max_view_objects`, and the debounce (the server also ignores a `SetView` within 50 ms of the last) | Budgets, section 08 limits |
| Client holds a stale cold copy | Revalidated by `held` at the next view | Rev comparison |
| Client's `held` is wrong or forged | Worst case it gets bodies it already has, or drops its own copies; the server never trusts it for access control (every object sent is checked against the view) | Held list only skips bodies |
| Slow client | Queue limits, dirty bitmap, then `Slow` disconnect | Bounded memory |
| Very dense view | Budget raises `eff_min_px`; overview pictures later | Reported to the client |
| Restore during edits | Restore is a change; in-flight changes merge on top | See Versions |
| A conditional write fails (`if_stamp`) | The whole change is rejected with `Conflict`; the sender gets current bodies | Accept or reject, never rewrite |
| Picture builder is down or slow | Clients get objects up to the budget, with `eff_min_px` raised; the tile stays dirty | Pictures are an optimisation |
| An invite link is turned off | Sockets closed, session ids added to the revoked list before the owner is answered; blob requests fail at once | Revoked-session check on every blob request |
| Media worker dies mid-upload | Status becomes `Stalled`, then `Abandoned`; the document is untouched | Status is not in the document |
| Object count or group size above caps | Change refused with `Reject { code: Limit }` | Caps |
| Server restart during deploy | Drain, final snapshot, `Close { retry_ms }` with jitter | Avoids a thundering reconnect |
| Many clients reconnect at once | Clients retry with exponential backoff and jitter, the room loads once; `held` makes each catch-up small | Section 07 owns the client retry timers |

## How it is tested

| Rule | Proved by |
|---|---|
| The query returns exactly the objects crossing the padded view and at least `min_px` | Property test: random objects over random levels (including levels 0, 1, -1, 100, -100 and rows straddling level boundaries), random views; `query()` compared with a brute-force scan using the exact predicate. Run with exit margin 0 for equality |
| Subscribed set is between `N` and `K` after any sequence of views and changes | Property test: random walks of views interleaved with random changes; after each `ViewEnd`, check `N ⊆ S ⊆ K` |
| The index never drifts | After every change in a test run, rebuild the index from the document and compare |
| Row skipping does not skip a visible object | Same brute force; objects of every size from 2^-10 to 2^40 units, views at pixel sizes from 2^-2000 to 2^2000 level-0 units |
| Routing table | Scripted two- and three-client scenarios for each of the six rows, including a drag out of view, a move into view, and a delete outside view |
| Group with partly loaded children | Scenario: group of 1,000 children, client views 3; move the group, ungroup (must fetch), delete |
| Held-list revalidation | Scenario: client caches region, disconnects, others edit and delete, client reconnects; compare final cache with server truth |
| Ack only after durable | Store test double that delays commits: no `Ack` or foreign `Apply` before the commit callback; kill the actor after apply and before commit, restart, resubmit, compare |
| Idempotent resubmission | Submit every change twice, and in a loop with random disconnects; log has each exactly once, and each resubmission is answered with the original `(ClientChangeId, Seq)` pair |
| Crash safety | Fault injection: kill the process at random points across 10,000 runs of random workloads; recovered state equals the acknowledged prefix; also truncated and bit-flipped log rows |
| Snapshot fallback | Corrupt the newest manifest, then a chunk, then both; recovery still yields the correct state |
| Snapshot equals log | At any seq, snapshot plus replay equals replay from empty |
| Chunk dedupe | Two snapshots after N strokes share all but O(N/256) chunks |
| Restore while editing | Randomized: several clients edit while a restore runs; all converge, and the "before restore" point restores the pre-state |
| Flow control | Throttled socket test: presence latency stays near 40 ms (the adaptive limit) while a 50 MB view fill is running; slow client is bounded in memory and then disconnected |
| Protocol compatibility | Golden files of every message for version N, decoded by build N+1 in CI; an unknown variant closes with `ProtocolError` |
| Lease | Two nodes race for a room; the loser's commits are refused |
| Strict mode | Kill the replica, then the primary, at random points over 10,000 runs: every confirmed change is present after failover; with the replica down, writes are refused after 5 s |
| Endpoint epochs | Reload a room (new epoch): an old grant is refused at an endpoint on another node, and a grant from the new connection works |
| Retention thinning | Simulated 40 days of snapshots: all kept for 24 h, one per hour to 7 days, one per day to 30 days, none after; named versions untouched |
| Accept or reject, never rewrite | Change with a failing `if_stamp`: rejected whole, log unchanged; `partial_ok` replay gives the same skipped set |
| Pong outside the change queue | Block the actor for 1 s; `Pong` still arrives within one chunk time |
| Pacing | Throttled links at 1 and 5 Mbit/s and unlimited, 10 MB queued: unsent bytes stay under the adaptive limit (4 to 64 KiB) plus one chunk, the estimate converges within 500 ms, and a presence update overtakes the queue within about 40 ms (66 ms at 1 Mbit/s, where the floor applies) |
| Far objects | Surface with one backdrop per level over 1,000 levels: at most `max_far_objects` sent, `far_dropped` correct |
| Index at depth | Objects at levels 0, 100 and 1,024: `ObjBox` stays about 80 bytes; query equals brute force |
| Anchors delta-coded | Chunk of 200 objects at level 100 is under a tenth of the whole-position size; decodes to identical positions |
| Nothing before `AuthOk` | A raw client sends garbage, then a valid `Auth` late: no byte comes back before verification |
| Picture decision | Tile with 1,999 then 2,001 small objects: objects, then picture; counts equal a brute-force count |
| Nearest and bounds | `Locate` equals a brute-force scan over random surfaces at several depths |

## Dependencies

All checked on crates.io on 2026-11-11 unless marked. All licenses are compatible with MIT OR Apache-2.0.

| Crate | Version | License | Maintenance | Why |
|---|---|---|---|---|
| tokio | 1.54.1 | MIT | Active (2026-10-21) | Async runtime |
| axum | 0.8.9 | MIT | Active (2026-04-14) | HTTP and WebSocket endpoint |
| tokio-tungstenite | 0.30.0 | MIT | Active (2026-07-11) | WebSocket (through axum); fastwebsockets 0.10.0 (Apache-2.0, 2026-01) is the alternative if profiling shows the need |
| postcard | 1.1.3 | MIT OR Apache-2.0 | Stable format; last release 2025-07 | Envelope encoding |
| rusqlite | 0.42.0 | MIT | Active (2026-11-11) | Room storage; bundles SQLite (public domain) |
| rstar | 0.14.0 | MIT OR Apache-2.0 | Active (2026-11-01) | R-tree in each bucket |
| roaring | 0.12.0 | MIT OR Apache-2.0 | Active (2026-10-28) | Per-connection bitmaps |
| lz4_flex | 0.14.0 | MIT | Active (2026-07-14); pure Rust, builds to WebAssembly | Compression of bodies and chunks |
| blake3 | 1.8.7 | CC0-1.0 OR Apache-2.0 OR Apache-2.0 WITH LLVM-exception | Active (2026-08-20) | Chunk hashes |
| redb | 4.5.0 | MIT OR Apache-2.0 | Active (2026-11-05) | Not used by the server; candidate for the client's local cache and outbox on native. Version and license from docs.rs, one read |
| socket2 | not checked | MIT OR Apache-2.0 (unverified) | Widely used | Sets `TCP_NOTSENT_LOWAT` on the accepted socket |
| wtransport | 0.7.2 | MIT OR Apache-2.0 | Pre-1.0 (2026-08-11) | Not used now; future transport |

Not chosen: bincode 3.0.0 (MIT): its GitHub repository was archived on 2025-08-15 with a note that development moved
to sourcehut; I could not confirm the status of crate releases, so avoid. ciborium 0.2.2 (last release 2024-01) and
prost 0.14.5 (Apache-2.0) are fine but unnecessary. zstd 0.14.3 shows BSD-3-Clause on crates.io and bundles C code,
which is awkward for WebAssembly, so lz4_flex is used. imbl (persistent maps) is MPL-2.0 and not needed: the sharded
copy-on-write map is a few hundred lines. Hocuspocus, y-sweet and tldraw sync are not usable as dependencies (other
language, different document model, or license).

## What this assumes of other sections

Names below are copied from the "API by name" table in `01-positions.md` and the "Types and operations by name"
table in `02-document-model.md`.

**Section 01 (positions).** `Position` is `{ level, x: Axis, y: Axis }` with a fixed-point `Axis { cell, frac }` in
steps of `2^-54` of a level unit, a canonical wire format of 19 bytes at the origin, about 2.5 bytes more per level,
and at most about 3.1 KB at the depth cap (levels within ±1,024, cell magnitude up to 12,288 bits). Checked against
those numbers: row `2 - 2L` is the level-`L` cell grid (a cell is 1,024 units, a unit is `1024^-L`), home rows run
from about -2,050 to 2,052, and the tile and bucket shifts are never negative for an object in its home row. Used,
all exact:

- Types: `Level`, `Axis`, `Fixed`, `Position`, `Delta`, `ExactRect`, `ViewRect` (with `pixel_log2`), `ExactBox`,
  `Bounds`, `TileKey`, `BucketKey { row, bx, by }`, `Round`, `DepthCap`.
- `Position::at_level`, `sub`, `add_delta`, `tile_at(row)`, `bucket_at(row)`, `Axis::shr_floor`, `Axis::shl`.
- `Position::offset_from_corner(&BucketKey) -> Option<[i128; 2]>` (this is `ObjBox.off`; `None` means the box does
  not
  fit and the change is refused with a limit error), `Position::from_corner_offset(&BucketKey, level, off)` for the
  exact rebuild, and `Position::offset_in(&BucketKey, unit_level, Round)` for the outward-rounded floats of an
  `Entry` (`Round::Down` for `lo`, `Round::Up` for `hi`).
- `ExactRect::intersects`, `contains`, `grown(num, den)`; `ViewRect::exact_rect`.
- `Position::encode_delta(base, out)` and `decode_delta(base, input)` for anchors inside a message, and
  `check_cap_bytes(&[u8], &DepthCap)` to refuse a hostile position on raw bytes before any number is built.
- `compose`, `localize`, `relevel` and `MAX_LEVEL_SPAN` (group maths, used through 02).
- The client sends the exact, unpadded `ViewRect` (01 says so); the server pads.

**Section 02 (document model).** The object map is purpose-built and sequenced by the server: each mutable field is
a
`Reg<T>` stamped with the per-surface `Seq`, so each object's state merges in any order; the server accepts or
rejects a whole change; a CRDT library is used only inside text objects behind `TextDoc`. This is decided (owner
decision, 2026-11-12). Which library edits text (Loro, yrs or another) is still open and the spike compares them;
the spike also still checks the finding that no library syncs part of a document. Used:

- `Doc::apply(&Change, &ApplyCtx { seq, author, ext }) -> Result<Applied, ApplyError>`, where
  `Applied { effects: Vec<ObjectEffect>, skipped }` and `ObjectEffect { id, ordinal, old_box, new_box, deleted,
  new_blobs, changed }`; the boxes are `Option<ExactRect>` and include the effect of ancestors. The server's index
  sets the `ApplyCtx` flag (added by 02) that returns one effect per descendant when a group moves. Conditions
  (`if_stamp`) and `partial_ok` are checked
  inside.
- `Doc::diff_to(&Doc) -> Change` (restore), `Doc::body(id) -> Option<ObjectBody>` and `Doc::encode_all()`, both
  canonical, `Doc::decode_body`, `Doc::get`, `Doc::children`.
- `ExtentOracle` (supplied by 06) is passed in `ApplyCtx`; a text object's box is its stored `TextFrame` size, so
  the server never lays text out.
- `Change`, `Op`, `Author` (a user or the server), `Rejected`, `ApplyError`, `Missing`, `ClientChangeId`, `Ordinal`,
  `Seq`, `ObjectId`, `OrderKey`, `Xform` (`TopXform`, `ChildXform`), `PlaybackState`, `BlobId`,
  `blobs_in_state`, `DUPLICATE_WINDOW`, `TOMBSTONE_RETENTION`, `purge_horizon`, and the limit names
  `MAX_CHANGE_BYTES` and the others whose numbers are in section 08's table.
- A rule for dragging that does not write a `Set` per pointer event (preview as presence; 02 sends the commit on
  release and at most 5 times a second during a long drag).
- `ClientChangeId` is a counter per client device that survives reconnects (02), and duplicates are keyed by client
  and counter. This section therefore keys duplicate detection by `(ClientId, ClientChangeId)` and the client keeps
  the counter in its outbox. 02's `DUPLICATE_WINDOW` (10 minutes) is the minimum; this section keeps the highest
  id per `ClientId` for 30 days.
- 02 agrees that this section may delta-encode the `Position`s inside a canonical body for the wire and for snapshot
  chunks, provided decoding restores the canonical bytes exactly; `encode_delta` and `decode_delta` do.

**Section 04 (presence).** Defines the payload (at most 1,000 bytes, self-contained, no view inside) and its rate
and
size rules. Needs from this section, in-process: each connection's `RoutedView` and its `ConnId`. This section only
enforces the size and the queue class. The reduction of presence fan-out in large rooms is its job.

**Section 05 (media).** Owns the upload protocol, the worker, `BlobStatus`, `BlobId`, `BlobKind` and the store.
Needs
from this section: the messages and records in "Media support", `blobs_in_store`, the status push, pausing of
playing videos, and `Ping`/`Pong` outside the change queue for its offset estimate (minimum-delay sample of the
last 8). A video's `PlaybackState` is an ordinary register beside `VideoFields`.

**Section 06 (rendering).** Owns picture format, the builder, `PicTile` and `PictureRef`, the home octave `h(o)` (a
pure function of an object's world box, with the picture tiles it touches) and the 2,000-object threshold, and
supplies the `ExtentOracle`. Uses `eff_min_px` and `more` from `ViewEnd`, and consumes cold and subscribed objects
through the same local index. The pixel size of an object is the longer side of its bounding box in pixels, as here.
The "octave" is 06's, not a row.

**Section 07 (shells).** Provides durable storage for the outbox and cache (IndexedDB or OPFS; files), reconnect
timers with jittered backoff, the "not accepted changes" display, and the `Transport` for one WebSocket.

**Section 08 (identity).** Provides the handshake (`Auth`, `AuthOk`, sub-protocol `everink.v1`), `Authz::check` for
every message and HTTP request, role changes and invite-link shutdown pushed as `Notice` and `Close`, the grant
format,
`TenantScope`, the audit sink, and the limits table with all the numbers: sockets per room and per user, the
unacknowledged window, change and view rates, frame sizes, media sizes, depth cap, `max_objects` (first-version
default 2,000,000), `max_far_objects`, and the three view limits `max_view_objects`, `max_view_bytes` and
`max_group_descendants`. The reasoning for those three is in this section.

## Questions for the owner

Answered on 2026-11-11 and 2026-11-12 and now stated in the Design: the merge model, the first app and its planning
figure, replayed offline changes, who may restore, retention and thinning of versions, the two durability modes, the
protocol window of 6 months, and the cap of 2 million objects. Still open:

1. **Which library edits text** (Loro, yrs or another). The merge model does not depend on the answer: Everink's own
   object map is ordered by the server and a library is used only inside text objects. The spike compares the
   candidates on text, and also re-checks the finding that no library syncs part of a document (one document per
   object, for example yrs subdocuments, is a route to partial copies that is unproven at scale, section 02).
2. **Whether a hosted offering (if any) should run on Cloudflare Durable Objects.** The room model is the same, and
   the core compiles to WebAssembly, but the storage layer and the actor would need to be rebuilt for that
   platform. Not recommended for the first version; the `RoomStore` trait keeps the option open.

## Sources

- Figma, [How Figma's multiplayer technology works](https://www.figma.com/blog/how-figmas-multiplayer-technology-works/)
- Figma, [Making multiplayer more reliable](https://www.figma.com/blog/making-multiplayer-more-reliable/)
- Figma, [Rust in production at Figma](https://www.figma.com/blog/rust-in-production-at-figma/)
- Figma, [Speeding up file load times one page at a time](https://www.figma.com/ja-jp/blog/speeding-up-file-load-times-one-page-at-a-time/)
  (translation only was readable; claims unverified)
- tldraw, [Sync](https://tldraw.dev/docs/sync) and [LICENSE](https://github.com/tldraw/tldraw/blob/main/LICENSE.md)
- Cloudflare, [Durable Objects limits](https://developers.cloudflare.com/durable-objects/platform/limits/) and
  [WebSocket best practices](https://developers.cloudflare.com/durable-objects/best-practices/websockets/)
- Cloudflare, [Liveblocks case study](https://www.cloudflare.com/case-studies/liveblocks)
- Liveblocks, [About the new storage engine](https://liveblocks.io/docs/guides/about-the-new-storage-engine)
  (read through a search summary only)
- [y-sweet](https://github.com/jamsocket/y-sweet), [Hocuspocus](https://tiptap.dev/docs/hocuspocus/introduction)
- Boulanger, Kienzle, Verbrugge, [Comparing interest management algorithms for massively multiplayer games](https://www.sable.mcgill.ca/~clump/papers/boulanger-06-comparing.pdf)
  (abstract only), and [Boulanger's thesis](https://www.cs.mcgill.ca/~jboula2/thesis.pdf) (not read)
- SQLite, [Write-ahead logging](https://www.sqlite.org/wal.html)
- redb, [design document](https://github.com/cberner/redb/blob/master/docs/design.md) and
  [docs.rs](https://docs.rs/crate/redb/latest)
- postcard, [wire format](https://postcard.jamesmunns.com/wire-format)
- bincode, [repository](https://github.com/servo/bincode)
- Can I use, [WebTransport](https://caniuse.com/webtransport)
- AWS, [S3 conditional writes](https://aws.amazon.com/about-aws/whats-new/2024/08/amazon-s3-conditional-writes)
  (considered for object-store leases; not used)
- crates.io pages for tokio, axum, tokio-tungstenite, fastwebsockets, postcard, bincode, prost, ciborium, rmp-serde,
  rusqlite, rstar, roaring, lz4_flex, blake3, zstd, wtransport, loro (1.17.1, MIT), yrs (0.28.1, MIT);
  all read 2026-11-11 through the crates.io API.
- Not verified: that `TCP_NOTSENT_LOWAT` is available on every target (Linux assumed; macOS similar option) and the
  WebSocket library's maximum-frame option names; the Yjs partial-replica limitation; hierarchical grids as a named
  technique; SQLite versus redb
  speed; S3 put latency; all capacity figures; Figma's current multiplayer sizing.
