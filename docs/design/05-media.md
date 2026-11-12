# Media: images, video, fonts and the blob store

Status: detailed design, 2026-11-11, proposal.

## Scope

This section covers what happens to a file from the moment someone pastes it to the moment other people see it:
which files are accepted, how they are checked, where they are stored, how they are delivered at the right size for
the zoom, and how a video stays in step on every screen. It also covers fonts as files.

Left to other sections:

| Topic | Section |
|---|---|
| Fields and merge rules of the object itself (this section says which fields image and video objects need) | 02 |
| How the view reaches the server, the WebSocket protocol, the room, snapshots and named versions | 03 |
| Drawing decoded images and video frames, text layout, shaping, the overview pictures | 06 |
| The platform API (this section lists what media needs from it), shells, accessibility of the controls | 07 |
| Roles, tenants, who may upload, the limits table, the grant and response headers, the threat model | 08 |
| The exact arithmetic that turns an image's position into "which part of the picture is on screen" | 01 |

Terms. A **blob** is an immutable file stored by the server and named by the hash of its bytes. A **variant** is a
file the server derived from a blob (a tile, a manifest). A **tenant** is one embedding app and the data it owns
(owner decision 4). A **grant** is a short-lived signed value that lets its holder read one blob.

## The problem

Concrete numbers, because they drive every choice below.

- **Photos are large and the surface is zoomed out most of the time.** A phone photo is about 12 megapixels
  (4000 by 3000) and 3 to 6 MB. Fitted into a 1000-pixel-wide view, the screen needs about 0.75 megapixels. Fetching
  the original is about 40 times more data than needed, and a board with 50 photos would pull 150 to 300 MB to show
  thumbnails. Decoded to RGBA, one 12-megapixel photo is 48 MB of memory; a 100-megapixel scan is 400 MB.
- **Unlimited zoom (owner decision 1) applies to pictures too, but a picture has finite detail.** A 4000-pixel image
  zoomed in until one of its pixels fills a 4000-pixel screen is magnified 4000 times at most. Beyond that the
  client can only magnify pixels. The design must stay correct, cheap and free of float overflow when the zoom is
  10^6 or 10^300 times past that.
- **Files are hostile data from strangers.** A 40 KB PNG can declare 65,000 by 65,000 pixels and ask a decoder for
  16 GB (a decompression bomb). Metadata can carry a person's GPS position. A font is a binary format with a long
  history of memory-safety bugs. Video decoders are large C and C++ code bases.
- **Uploads fail.** A 1 GB video on a mobile connection will be interrupted. Other people need to see something
  honest meanwhile.
- **Video must be in step.** Owner decision 3 says playback is synchronized for everyone, and the high-level design
  targets two viewers within 100 ms. Network round trips are 20 to 200 ms, so the clocks of the machines have to be
  reconciled; no amount of hurrying messages gets under that.
- **Browsers block sound until the person has interacted** with the page, and limit how many hardware decoders run
  at once (the limit varies by machine; no figure verified).

## What others do

- **tldraw** keeps a small *asset record* in the document (dimensions, MIME type, a URL) and the bytes elsewhere.
  An application supplies an asset store with `upload` and `resolve`; `resolve` receives the display scale so a host
  can return a resized URL. Without a store, images and videos are inlined into the document as data URLs.
  Scaling is the host's job, not the library's. [tldraw assets]
- **Excalidraw** keeps image files in a separate `files` map keyed by a file id, stored in browser storage locally
  (unused ones cleaned up after 24 hours) and, in its hosted collaboration, in a cloud store with a time-to-live for
  deleted files (from a fork's architecture notes and a DeepWiki page; upstream code not read). [Excalidraw notes]
- **Figma** names an image by a hash of its contents (`imageHash`; the API describes it as a hash of the file),
  accepts PNG, JPEG and GIF, and limits plugin-created images to 4096 pixels per side. [Figma images]
- **Miro** accepts bmp, gif, svg, jpeg, png, psd, heic, heif and pdf, with a 30 MB file limit and a 32-megapixel
  limit (an 8192 by 4096 dimension cap is also stated); users ask for automatic downscaling and third-party apps
  slice big images, which suggests the product does not tile. A Miro staff member is reported to say the API allows
  6 MB images. FigJam and Miro video internals were not found. [Miro formats]
- **Deep-zoom formats.** Microsoft's Deep Zoom stores an image as a pyramid, each level half the size in each
  direction, cut into 256-pixel tiles named `column_row`, and supports "sparse" images that have more resolution in
  one region. IIIF's Image API asks for a `region` and a `size` and describes pre-cut tiles with `scaleFactors`;
  map tile pyramids work the same way. All assume a finite source with a fixed finest level. [Deep Zoom] [IIIF]
- **Placeholders.** BlurHash packs a few cosine components into a short text string. ThumbHash, by another author,
  uses similar ideas in a binary form: it also encodes the aspect ratio, supports alpha, gives more accurate colors,
  and does not allow tuning; MIT-licensed. [ThumbHash]
- **Resumable uploads.** tus 1.0.0 (2016) is a small HTTP protocol: create an upload (`POST` with `Upload-Length`),
  ask where it stopped (`HEAD` returns `Upload-Offset`), send bytes (`PATCH` with `Upload-Offset`); optional
  extensions add checksums and concatenation. S3 multipart upload splits an object into 1 to 10,000 parts of 5 MiB
  to 5 GiB, each uploaded separately (including through presigned URLs), then completed. [tus] [S3 limits]
- **BLAKE3.** A 256-bit hash built as a tree, so it parallelizes across threads and SIMD lanes; its Rust crate has
  SSE, AVX, NEON and WebAssembly code paths; it is CC0 or Apache-2.0; and its tree structure supports verified
  streaming (Bao). [BLAKE3]
- **Synchronized playback.** Syncplay compares each player to a shared position and reacts by thresholds; its source
  defaults are: slow down to 0.95 speed when 1.5 s ahead, rewind when 4 s off, fast-forward when 5 s behind, and a
  seek threshold of 1 s. Those thresholds suit people on separate media players, not a 100 ms goal. [Syncplay
  constants] The W3C Multi-Device Timing Community Group's **timing object** is a shared motion vector (position,
  velocity, acceleration, timestamp) plus a clock-skew estimate from periodic exchanges that account for round-trip
  time, applied gradually so time never moves backward; a media element follows it by small `playbackRate` changes
  or by seeking, and never pauses the shared object to wait for data. The group has closed (early 2025) and the spec
  says work was discontinued. Its reference library `timingsrc` is LGPL-3.0, so Everink cannot take code from it.
  The design below reuses the ideas, not the code. [timing object] [timingsrc] Watch2Gether and Teleparty were not
  studied (no primary source read).

## Options considered

### Who derives the smaller versions

| | A. Uploading client | B. Server media worker (chosen) | C. External image service (imgproxy-style) |
|---|---|---|---|
| Trust | Server cannot trust what a client sends | Trusted code on a trusted machine | Same, plus another service to run |
| Upload cost | About 1.33 times the bytes | Original only | Original only |
| Hostile input | Decoded on the uploader's machine and again by the server to verify | Decoded once, in an isolated process | Decoded in a C library service |
| Slow devices | Heavy work on phones | None on the client | None |
| Given up | Server must re-derive anyway | Server CPU and a job queue | Rust-only toolchain; tile layout we do not control |

### Upload protocol

| | tus subset through the Everink server (chosen) | S3 multipart with presigned URLs | Plain PUT |
|---|---|---|---|
| Resume | Yes, `HEAD` offset | Yes, part list | No (restart) |
| Works with a local-directory backend | Yes | No, S3-style stores only | Yes |
| Server hashes and sniffs while receiving | Yes | No, must read the object back | Yes |
| Server bandwidth | All upload bytes pass through | None | All pass through |
| Off-the-shelf clients | tus-js-client, Uppy | AWS SDKs | Anything |
| Given up | Server bandwidth for big videos | Simplicity; early rejection of bad files | Resume on 1 GB files |

### Placeholder in the document

| | BlurHash | ThumbHash (chosen) | Tiny WebP/JPEG (about 16 px) |
|---|---|---|---|
| Form | Text, 20 to 30 characters | Binary, tens of bytes (exact size unverified) | 150 to 400 bytes plus header |
| Alpha | No | Yes | WebP yes |
| Aspect ratio in the data | No | Yes | In the header |
| Decoder | Small cosine sum | Small cosine sum | A full image decoder per placeholder |
| Quality per byte | Lower | Higher (author's claim; not measured here) | Highest, at about 10 times the size |
| Given up | | Fixed parameters | Document grows with every image |

### How video reaches the screen

| | Web | Native |
|---|---|---|
| 1. Platform player, read frames out of it (chosen) | Hidden `HTMLVideoElement`: hardware decode, audio and A/V sync, `playbackRate` for nudging, `requestVideoFrameCallback` for frame times, frames to WebGPU by `importExternalTexture` | AVFoundation, Media Foundation, GStreamer: the OS player decodes and plays audio; we copy frames out |
| 2. Own decode loop | WebCodecs plus a demuxer (Mediabunny is MPL-2.0 TypeScript); audio output and A/V sync become ours to write | FFmpeg or crates: LGPL/GPL and patent duties pass to every embedder |
| 3. Server renders one stream, clients watch it | Perfect sync | Server cost per video; breaks "clients draw" |

### Synchronization model

| | Server-clock register, client chases it (chosen) | Peer-to-peer timing object | One leader client, others follow |
|---|---|---|---|
| Fits the CRDT document and roles | Yes, one register | Needs its own transport | Needs leader election |
| Late joiner | Reads the register | Reads the vector | Waits for the leader |
| Leader leaves | No effect | No effect | Breaks |
| Given up | Clock error is a server round trip | Not applicable here | |

## Design

### Overview

```
 paste --> probe (shared Rust code) --> BLAKE3 + ThumbHash --> object created at once (local preview)
              |                                                   |
              +--> POST /uploads (tus subset, HTTP) --> PATCH chunks --> server hashes while receiving
                                                                         |
                          staging/ --verify hash, sniff, probe--> t/<tenant>/b/<hash>/orig
                                                                         |
                                                            media worker process (OS limits)
                                                            image: tile pyramid   font: validate
                                                            video: probe only (no frames decoded)
                                                                         |
          others: ThumbHash (in document) --> BlobStatus pushes --> grant --> tiles / range reads
```

`everink-media` is a package of its own. It holds what the client and the server must agree on exactly (format
sniffing, limits, header probing, tile geometry, the ThumbHash encoder and the BLAKE3 helper), and the media worker
and the blob store code. Its `derive` feature (decoders and encoders, used only by the server's media worker, the
`everink-media-worker` binary) is off in the wasm and native client builds. The HTTP endpoints, the revoked-session
list and the room-side reference index live in `everink-server`, which depends on `everink-media`.

### What is in the document and what is in the blob store

The document holds only what a client needs to lay out the object and to ask for the right files. Everything else is
derived and lives in the blob store. Section 02 holds the schema (it stores `ImageFields`, `VideoFields`,
`PlaybackState`, `FontRef`, `BlobId` and `Placeholder` exactly as written here); this section gives the meaning of
each field. Storage rule from 02: `PlaybackState` is a register of its own beside the `video` register, so
the `playback` member of `VideoFields` below is not stored inside `video`; it is written to the `playback` register,
and a change to the file facts never overwrites a play or pause.

```rust
/// 32-byte BLAKE3 hash. Written "b3-" + 64 lowercase hex characters in URLs and logs.
pub struct BlobId(pub [u8; 32]);

/// A ThumbHash. The server refuses a document change that carries more than 64 bytes.
pub struct Placeholder(pub Vec<u8>);

pub struct ImageFields {
    pub blob: BlobId,           // the original; derived files are found from it
    pub px_w: u32,              // pixel size AFTER applying EXIF orientation
    pub px_h: u32,
    pub placeholder: Placeholder,
    pub alt: Option<String>,    // at most 500 bytes; feeds section 07's accessibility description
}

pub struct VideoFields {
    pub blob: BlobId,
    pub px_w: u32, pub px_h: u32,      // after container rotation
    pub duration_us: u64,
    pub has_audio: bool,
    pub poster: Option<BlobId>,        // an ordinary image blob, at most 2048 px on its long side
    pub placeholder: Placeholder,      // ThumbHash of the poster
    pub playback: PlaybackState,       // ONE last-writer-wins register for the whole struct
}

/// A font run in a text object names a font like this, never by a system font name.
pub enum FontRef {
    Builtin { family: BuiltinFamily, build: u16 }, // from the table compiled into the core
    Blob(BlobId),
}
```

The object's box on the surface is not `px_w` by `px_h`. It comes from the stored `size` register (and `crop`, if
set) of section 02, in the object's own units, and section 06's `ExtentOracle` turns that into the `LocalBox` that
decides what is sent and cached; it needs nothing but the stored fields. `px_w` and `px_h` are the source pixel
size, used only for tile geometry and for the screen pixels per source pixel `s` below (the box width in screen
pixels divided by the cropped source width). An image object is about 90 bytes.

`px_w`, `px_h`, `duration_us` and `has_audio` are claims by the pasting client. The server probes the stored file
with the same `everink-media` code and marks the blob `Rejected(Mismatch)` if they differ, so a client cannot make
tile addresses that do not exist or a video of the wrong length.

### Accepted image formats and decoders

Detection is by content (magic bytes), never by file name or the client's MIME type. The server and the client use
the same `sniff()`.

| Format | Accepted | Decoder (memory-safe Rust unless noted) | Notes |
|---|---|---|---|
| PNG | Yes | `png` (README: no `unsafe`; fuzzed on OSS-Fuzz) | APNG with more than one frame: refused |
| JPEG (baseline, progressive) | Yes | `zune-jpeg` (through `image`) | Uses SIMD intrinsics, which the project says is its only kind of `unsafe`; isolation below |
| WebP, still | Yes | `image-webp` (README: no `unsafe` in it or its dependencies) | Animated: refused |
| GIF, one frame | Yes | `gif` | More than one frame: refused |
| JPEG XL | Phase 2 | `jxl-oxide` (pure Rust, "spec-conforming", has a fuzz directory) | Add once a conformance run is in CI |
| AVIF | Phase 2, only if a safe decoder qualifies | `image`'s AVIF path calls the C library dav1d, so it does not qualify. `rav1d` is a Rust port of dav1d that can use assembly; the extent of its `unsafe` is not stated on its page. `rav1d-safe` and `zenavif` claim a no-`unsafe` path but are young (unverified) | Decide after a spike |
| HEIC / HEIF | No | None that is safe and patent-clear (HEVC is licensed through patent pools) | Message: export as JPEG or PNG |
| SVG | No | n/a | See below |
| Animated GIF, APNG, animated WebP | No | n/a | Message: "Animated pictures are not supported; paste a video." |
| TIFF, BMP, ICO, PSD, EXR, HDR, PDF, RAW | No | n/a | Each adds a decoder to audit; none is needed to draw on a board |

**Why SVG is refused as an image.** An SVG file is an XML document that can reference other files and URLs, embed
scripts and fonts, and describe filters of unbounded cost. Rendering it needs a full engine, a second drawing system
beside section 06's. With unlimited zoom, vector content is attractive, so the better route is **import**: parse the
SVG once, in a bounded way, and convert it to ordinary Everink objects (paths, shapes, text) that are editable and
sharp at any depth. That is a later feature and a product choice (Questions for the owner, 2); this section only
fixes that a `.svg` is never stored or served as a blob.

**Output is normalized.** The server applies EXIF orientation, converts an embedded ICC profile to sRGB with
`moxcms` (pure Rust), reduces 16-bit and floating-point channels to 8 bits, and **drops all metadata** (EXIF, GPS,
XMP, ICC) from every derived file. Colors outside sRGB are therefore clipped; the help text says so. The original
keeps its metadata, so it is served only to members (Questions for the owner, 7).

### Limits

Section 08's limits table is the home of these figures; it adopts the member figures below and has stricter guest
figures (guests may upload only images, and smaller ones). The values shown are for member sessions so the reasoning
can be read next to them; if the two ever differ, 08 is right. The limits the server enforces from this section
(file sizes and pixels, decoder memory and time, metadata size, font glyphs and axes, placeholder and alt-text size,
playback writes per user and per object and the anchor tolerance, presigned address lifetime) are mirrored in 08's
table as rows "set by section 05", with the figures given here. Each limit is checked at the earliest point that can
know it.

| Limit | Default | Where checked | How the number was chosen |
|---|---|---|---|
| Image file size | 50 MiB | `Upload-Length` at creation; running count while receiving | Above Miro's 30 MB, below a typical 100 MB proxy cap |
| Image pixels | 128 megapixels; each side at most 32,768 | From the header after the first 4 KiB, before any pixel buffer exists | 128 MP times 4 bytes is 512 MiB, the `image` crate's default `max_alloc` |
| Decoder memory | `Limits::max_alloc` 768 MiB, plus a 1.5 GiB process limit | Worker process | `max_alloc` is documented as non-strict (some decoders ignore it), so the OS limit is the real guard |
| Decode plus derive time | 60 s wall, 45 s CPU | Worker process, killed on expiry | A 128 MP image should finish well inside (estimate; measure in the spike) |
| Metadata | EXIF, XMP and ICC together at most 1 MiB | While parsing | It is dropped anyway |
| Frames | 1 | While parsing | |
| Font file | 20 MiB, 65,535 glyphs, 16 variation axes | Probe | |
| Video file | 2 GiB, at most 4096 by 2304 pixels, at most 4 hours | Probe of the container header | About 1 hour of 1080p at 5 Mb/s |
| Placeholder | 64 bytes | Document change check | |
| Uploads in flight | 3 per user, at most 1 a video | Server | |

**Decompression bombs.** The rule is "know the size before allocating it": read the header, compute `w * h * 4`,
compare with the limits, and only then create a decoder with explicit `Limits`. The pixel cap makes the worst case
512 MiB per job whatever the compression ratio. A tiny file that declares 128 MP and compresses to almost nothing is
still accepted if within the limits, and then costs only its time budget. A PNG whose compressed stream expands
beyond the declared pixels is stopped because the decoder is given the output size. Video and font probes read
headers only, with caps on box depth, box count and table count.

**Isolation.** Even memory-safe crates can panic, loop or exhaust memory, and `zune-jpeg` has `unsafe` SIMD code.
Every derive job runs in a separate OS process (`everink-media-worker`) with memory and CPU limits, no network and,
where the platform allows, a syscall filter. A crash or kill is retried twice, then marks the blob
`Rejected(Internal)`. The server process never decodes pixels. The media worker is part of build step 4 (images),
not step 6. No media tool runs outside this sandbox: the optional external transcoder below is started by the worker
under the same limits, and no `ffmpeg`-class tool is ever run unsandboxed.

### The placeholder

The pasting client computes a ThumbHash from a 100-pixel downscale of its local preview and puts it in the document
with the object, so every user who receives the object can paint a blur at once, before any file is fetched. A wrong
placeholder cannot hurt anyone (at most a 32-by-32-pixel blur, attributable to its author). A video's placeholder is
the ThumbHash of its poster. Decoding one costs a fraction of a millisecond; the renderer does it once per object
and caches the result.

Everything larger is derived and stays out of the document: a 5 KB thumbnail per image would add 5 MB to the
document per thousand images and make every snapshot, sync and version bigger.

**When not to download at all.** This is a rule on the client, applied to objects that were sent. Section 03 sends
an object only if its longer side covers at least `eff_min_px` screen pixels (at least 1 by default; the server's
test is conservative to within a factor of two and section 06 culls exactly with the `eff_min_px` the server
reports). Among those objects, if an image or video is drawn smaller than 24 screen pixels on its long side, the
client draws only the placeholder and requests nothing. A far-zoomed-out board with thousands of pictures then costs
no image traffic.

### Derived versions: the tile pyramid

The media worker builds a pyramid after the upload is verified. Level 0 is the full image (orientation applied,
sRGB, 8 bits). Level *k* is the image reduced by 2^k in each direction (box filter in linear light, re-encoded to
sRGB). Every level is cut into 512 by 512 pixel tiles (edge tiles may be smaller). Levels continue until the whole
image fits in one tile; the top level is `K`.

```rust
pub const TILE: u32 = 512;

/// Pure arithmetic from the document fields; no manifest is fetched.
pub fn levels(px_w: u32, px_h: u32) -> u8 {            // K; 0 when the image fits in one tile
    let m = px_w.max(px_h).max(1);
    (m.div_ceil(TILE) as f64).log2().ceil() as u8
}
pub fn level_size(px_w: u32, px_h: u32, k: u8) -> (u32, u32) {   // ceil(w / 2^k), ceil(h / 2^k)
    (px_w.div_ceil(1 << k), px_h.div_ceil(1 << k))
}
pub fn tile_grid(px_w: u32, px_h: u32, k: u8) -> (u32, u32) {    // tiles across, tiles down
    let (w, h) = level_size(px_w, px_h, k);
    (w.div_ceil(TILE), h.div_ceil(TILE))
}
pub struct ImageTileKey { pub k: u8, pub x: u32, pub y: u32 }     // URL path: .../t/<k>/<x>_<y>
```

Because the layout is computed from `px_w` and `px_h`, a client needs no manifest before its first tile request.
The tile's format is not in the URL: the server sets `Content-Type` and the client decodes by that. (`levels` uses
floating point on a small integer; the implementation should use integer arithmetic so client and server cannot
differ. The tests cover this.)

**Tile format.** Lossless WebP (`image-webp`'s encoder is lossless only) when the source has alpha or came from PNG
or GIF (screenshots, diagrams, sharp text); JPEG (`jpeg-encoder`, quality 85, 4:4:4 chroma) otherwise. The rule is
a starting point to tune on a corpus. A photographic JPEG tile at quality 85 is roughly 1 bit per pixel, about 32 KB
for 512 by 512 (assumption from general experience; measure).

**Worked example: a 4000 by 3000 photo.** `K = ceil(log2(8)) = 3`. Level 0 is 8 by 6 = 48 tiles; level 1 (2000 by
1500) is 4 by 3 = 12; level 2 (1000 by 750) is 2 by 2 = 4; level 3 (500 by 375) is 1. Total 65 tiles, about 16
megapixels stored, 1.33 times the original. Fitted into a 1000-pixel view the client needs the 4 tiles of level 2,
about 130 KB, against 3 to 6 MB for the original.

**Order of production.** The worker writes the top level first and works down, updating the blob's status after each
level to `Partial { from_level }`. A client that finds only coarse levels ready draws the coarsest available
(blurry) and refines as `Partial` advances. When all levels exist the status is `Ready`.

**Choosing a level for a zoom.** Let `s` be screen pixels per source pixel for this image on this client now (from
the object transform and the camera; section 01 supplies it as an ordinary `f64` because it is a ratio of nearby
quantities). The level is the coarsest one that still has at least one tile pixel per screen pixel:

```
k = clamp(floor(log2(1 / s)), 0, K)          // s >= 1 (zoomed in) gives k = 0
```

At `s = 0.3` one screen pixel covers 3.3 source pixels, so `k = 1`; each tile pixel is 2 source pixels, giving 1.67
tile pixels per screen pixel: sharp enough and never more than 2 times oversampled. Section 06 may blend levels *k*
and *k+1* while the zoom changes; this section supplies both. A client requests only tiles that intersect the padded
view rectangle (design.md pads the view by half a screen), plus the level *k+1* tiles under them as a fallback while
the fine tiles load.

**Zooming far into an image.** For `s > 1` the level stays 0. What changes is the visible rectangle in source
pixels, `view_size_in_screen_px / s` wide. Section 01 provides it as an `f64` rectangle in image-local pixel
coordinates, computed from the exact positions, so it is small and precise at any zoom. Rules:

1. Clip the rectangle to `[0, px_w] x [0, px_h]`. If it is empty, the image is off screen and nothing is fetched.
2. If it is narrower than one source pixel, use the single level-0 tile that holds its center. The renderer fills
   the view with that pixel's color. No coordinate larger than the image's own pixel count is ever produced here, so
   nothing overflows at zoom 10^300.
3. For any zoom above 1, a view smaller than one tile touches at most 4 level-0 tiles, so the cost of zooming in is
   constant, not growing with the zoom.
4. Whether magnified pixels are drawn as sharp squares or smoothed is section 06's choice; design.md's "zooming far
   into an image shows its pixels" is the default.

A raster image has finite detail and nothing here pretends otherwise. Someone who needs detail at depth should draw
it as vector content.

**Client caches.** Decoded tiles in GPU memory: 256 MiB, which is 256 tiles of 512 by 512 RGBA at 1 MiB each, least
recently used first, tiles in the current view pinned. Encoded tiles on disk (native) or OPFS (web): 1 GiB, least
recently used. Both are starting values.

### The blob store

```rust
// TenantId is section 08's type: a lowercase text id of 1 to 32 bytes. This section only uses it.

pub enum Variant {
    Orig,                          // bytes as uploaded
    Tile(ImageTileKey),            // image tile
    Playback,                      // video: transcoded file, only if an operator configured a transcoder
    Manifest,                      // small CBOR: probe results for video and fonts, summary for images
}

pub trait BlobStore: Send + Sync {
    async fn stage(&self, up: UploadId) -> Result<StagedWriter>;              // multipart writer, 8 MiB parts
    async fn commit(&self, up: UploadId, t: TenantId, id: BlobId) -> Result<()>; // idempotent: same bytes, same key
    async fn get_range(&self, t: TenantId, id: BlobId, v: &Variant, r: Option<Range<u64>>) -> Result<GetResult>;
    async fn put_variant(&self, t: TenantId, id: BlobId, v: &Variant, bytes: Bytes) -> Result<()>;
    async fn delete_blob(&self, t: TenantId, id: BlobId) -> Result<()>;      // original and every variant
    async fn signed_read_url(&self, t: TenantId, id: BlobId, v: &Variant, ttl: Duration)
        -> Result<Option<Url>>;                                                // None: backend cannot sign
}
```

The implementation is a thin layer over the `object_store` crate (0.14.2, MIT OR Apache-2.0), which gives one async
API over the local file system, Amazon S3, Google Cloud Storage, Azure, an in-memory store (used in tests) and
generic HTTP. It has multipart writers (`put_multipart`, `WriteMultipart`) and a `Signer` trait for presigned URLs
implemented for S3, GCS and Azure. Backends offered: **local directory** (default; development and single machine)
and **S3-compatible**. GCS and Azure come free but are untested in v1. The local backend does not build for
WebAssembly, which is fine because only the server uses it.

**Key layout.**

```
staging/<upload-id>                              being received; deleted after 24 h idle
t/<tenant>/b/b3-<64 hex>/orig                     the original
t/<tenant>/b/b3-<64 hex>/m                        manifest
t/<tenant>/b/b3-<64 hex>/t/<k>/<x>_<y>            image tiles
t/<tenant>/b/b3-<64 hex>/playback                 transcoded video, if any
```

Ids are hex only, so a name cannot contain `..` or a separator. The tenant prefix is what limits sharing to one
tenant.

**The hash: BLAKE3, 256 bits.** Reasons: it is much faster than SHA-256 in software (the project's own comparison);
the same Rust crate has SIMD paths for x86, ARM and WebAssembly, so a browser can hash a 1 GB file incrementally in
a worker while reading it in chunks; the browser's built-in `crypto.subtle.digest` takes the whole buffer in one
call (to my knowledge; unverified here), which is impractical for a 2 GiB video; and the tree structure leaves room
for verified range reads later. SHA-256 would be the choice if a rule required a standardized algorithm; no owner
decision does. The id carries an algorithm prefix (`b3-`) so another can be added.

The hash is **not a secret and not a permission**. Knowing it does not allow a read; only a grant does.

**Deduplication scope is the tenant.** Two uploads of the same bytes in one tenant are stored once (the commit step
finds the key present and discards the staged copy). Across tenants they are stored separately. The cost is
duplicate storage across tenants; the gain is that an upload can never reveal that someone else holds a file
(design.md, security). The "already present" reply to an upload request is given only when the same surface already
references the blob; otherwise the bytes are uploaded and de-duplicated at commit. A later refinement could add a
proof of possession (hashes of random byte ranges) to skip the upload across surfaces of one tenant.

### The upload protocol

A strict subset of tus 1.0.0 (core plus the creation extension), so that off-the-shelf tus clients (tus-js-client,
Uppy) work for embedders. It runs over HTTP, not the WebSocket.

1. The client asks over the WebSocket for an **upload grant** with section 03's `UploadGrantRequest { req, blob,
   size, kind }`. The server checks role, limits and `size`. The grant is defined in section 08: it is bound to
   user, surface, blob and size, and is valid until the session ends and for at most 24 hours. Every upload request
   (`POST`, `PATCH` and `HEAD`) is checked against the revoked-session list and against the session's upload
   floor (section 08), so an upload is refused after a link is turned off, after the session ends for any reason,
   and after its role loses upload rights. A guest who is frozen becomes a viewer at once: grants issued before
   the freeze stay dead, and they keep read access to blobs.
2. `POST /v1/uploads` with `Tus-Resumable: 1.0.0`, `Upload-Length: <bytes>`, `Authorization: Bearer <grant>` and
   `Upload-Metadata` carrying `blob` (the `b3-` id) and `surface`. Reply `201` with `Location`, or `200` with
   `Everink-State: present` when the same surface already references the blob (nothing to upload).
3. `HEAD <location>` returns `Upload-Offset`. `PATCH <location>` with `Content-Type:
   application/offset+octet-stream` and `Upload-Offset` appends bytes and replies `204` with the new offset.
4. The server hashes bytes as they arrive (a BLAKE3 hasher kept in memory per upload) and streams them to the staged
   multipart writer. After the first 4 KiB it sniffs the type and, for images, reads the header, so a file that is
   refused (wrong type, too many pixels, wrong declared size) gets `415` or `413` at once, not after 1 GB.
5. After the last byte the server compares the hash with the claimed id. A mismatch gives `422` and the staged file
   is deleted. A match commits the blob (rename for the local backend, copy and delete for S3; both idempotent),
   moves it to `Processing`, and queues the media worker.
6. If the server restarts mid-upload it rebuilds the hasher from the staged bytes at the next `PATCH` (one read of
   the staged file; rare).

Chunking: start at 1 MiB per `PATCH`, double up to 8 MiB while each chunk takes under 5 s, halve on a timeout. A
small image is one `POST` and one `PATCH`. Concurrent uploads: see Limits.

**Retry.** A failed chunk is retried with exponential backoff, 0.5 s doubling to 30 s, with jitter, using `HEAD` to
learn the true offset first. After 10 consecutive failures of one chunk (about 5 minutes) the uploader's client
marks the upload `Failed(retryable)`, keeps the file in local storage, offers Retry, and retries on its own when the
connection returns, until the upload grant expires. The pending-upload queue is persisted with the offline changes
(design.md, offline), because the object already exists in the document and must eventually get its file.

**What other people see.** The server holds one status per blob and pushes it, throttled to 2 per second per blob,
to every client that has an object using that blob in its view. Status is not stored in the document (it would
churn it).

```rust
pub enum BlobStatus {
    Receiving { received: u64, total: u64 },   // others: "Uploading 43%"
    Stalled   { since_ms: u32 },               // no byte for 60 s: "Waiting for the uploader"
    Processing,                                // "Preparing"
    Partial   { from_level: u8 },              // images: coarse levels usable
    Ready,
    Abandoned,                                 // upload grant expired with the file incomplete
    Rejected  { reason: RejectReason },
}
pub enum RejectReason { TooLarge, UnsupportedType, TooManyPixels, Animated, Corrupt, HashMismatch, Mismatch,
                        FontRestricted, Internal }
```

| State | The uploader sees | Everyone else sees |
|---|---|---|
| Receiving | local preview, progress ring | ThumbHash blur with a thin progress bar |
| Stalled / Abandoned | preview, "Upload paused, retrying" / "Failed, Retry" | blur with a "not uploaded yet" badge |
| Processing / Partial | preview, then a seamless swap to tiles | blur, sharpening as levels appear |
| Ready | tiles | tiles |
| Rejected | message with the reason; the object stays until deleted | blur with a "file refused" badge |

The uploader keeps a local preview (the decoded image downscaled to 2048 pixels) until `Ready`, so it never
downloads its own picture.

### Download authorization and delivery

1. A client that needs a blob's files sends section 03's `BlobGrantRequest` over the WebSocket (the cap on ids per
   request is one number, in section 08's table). For each id the server checks that **the blob is referenced by
   the current state of the surface this connection is on, or by a snapshot or named version that the requester's
   role may open.** The reference index described under garbage collection, which never forgets a reference, is for
   garbage collection only and is never consulted here: a viewer asking for a blob that only a deleted object
   referenced is denied. A blob that does not pass is answered `Denied`, whether or not it exists.
2. The reply carries one grant per blob: `{ blob, exp, kid, tag }`. Section 08 owns the grant: `tag` is an
   HMAC-SHA-256 over (tenant, surface, blob hash, session id, expiry) under a server key. One grant covers every
   variant and tile of that blob.
3. The client builds the address exactly as section 08 specifies:
   `https://<blob origin>/b/<tenant>/<blob hash>/<file name>?s=<surface>&sid=<session id>&exp=<unix s>&k=<key id>&sig=<mac>`,
   where `<file name>` names the variant (tiles are `t/<k>/<x>_<y>`; the original is never fetched this way, see
   below). The media endpoint, which is part of `everink-server`, recomputes the MAC, checks `exp` and the key id,
   and checks the session id against the revoked-session list that `everink-server` keeps. It supports `Range`.
4. S3 backend: the endpoint answers `302` to a presigned URL valid for 60 seconds (through `Signer`), or streams the
   bytes itself ("proxy mode") for stores that cannot sign. Local backend: it streams. In presigned mode the
   redirect must carry the same response headers as section 08's table: response-header overrides on the presigned
   request, object metadata set at commit, and a bucket CORS policy. A store that cannot do all three runs in proxy
   mode. The 60 seconds of a presigned address are the **only** access that can outlive a revoked session; section
   08 accepts that and lists it in its threat table. Proxy mode has no such gap.

**Lifetimes.** Images, tiles, posters and fonts: 10 minutes; the client asks again at 50 percent of the lifetime.
Video: **2 hours**. Video needs the longer life because a browser keeps requesting
byte ranges from the same address while the video plays, and an address that expires after minutes would stop
playback midway. A client that still gets `403` mid-video asks for a new grant and reloads the element at the
current target position (a glitch of about a second, once per 2 hours).

**Revocation.** `everink-server` keeps a short list of revoked session ids for the longest grant lifetime (2 hours)
and checks every blob request against it. Turning a share link off, or removing a person, adds their session ids, so
blob access stops at once, apart from the up to 60 seconds of an already issued presigned address (above). If the
list cannot be read, video requests are refused (fail closed), as section 08 says.

**Response headers.** Section 08's table is the only list of headers a blob response carries (content type, type
sniffing, disposition, sandboxing policy, cache policy, cross-origin policy, referrer policy); this section repeats
none of them. Two consequences for this section: the media origin is a **separate site** from the app and the API,
so nothing it serves runs with their cookies or storage, and the `<video>` element is created with
`crossorigin="anonymous"` because WebGPU's `importExternalTexture` and WebGL2 texture uploads reject a cross-origin
video source that is not CORS-enabled.

**Images are never served as uploaded for display.** What clients display are the derived, re-encoded tiles, which
carry no metadata. An original (`Variant::Orig`) is download-only. It is fetched from a separate path, `/o/`, with
its own MAC context string (`everink-blob-orig-v1`), sent as an attachment, and the room issues such an address only
to members (never to guests), as section 08 specifies.

### Reference tracking and garbage collection

Counting references does not work here: the document is a CRDT with history, undo can bring a deleted object back,
snapshots and versions hold old states, and a library is another surface. Everink uses **mark and sweep with a grace
period** instead.

*The reference index.* For each surface the server keeps `refs(surface, blob) -> { first_seen, last_referenced }`,
updated when a change adds an object whose kind is image, video (its blob and poster) or text with a
`FontRef::Blob`. A reference is never removed when an object is deleted. This index serves garbage collection only;
download authorization uses current state and the states a role may open (above).

*Liveness.* A blob is **live** when it is referenced by (a) the current state of any surface, (b) any retained
snapshot, or (c) any named version. Section 02 owns the pure function `blobs_in_state(state: &Doc)` over one
document state; section 03 owns `blobs_in_store(store: &dyn RoomStore, state: StateRef)`, which loads a stored state
(current, snapshot or named version) and calls 02's. The sweep calls 03's function; the tenant comes from the
store's scope.

*Sweep,* once a day per tenant, in two phases:

1. Compute the live set as the union over all surfaces' current states, retained snapshots (default 30 days) and
   named versions. Every stored blob not in the set and older than 24 hours gets `condemned_at = now`. A blob that
   is live again has its mark cleared.
2. A blob condemned for 24 hours or more and still not live is checked once more and then deleted with all its
   variants. A new reference in between (a client pasting an old, de-duplicated picture) updates `last_referenced`
   and clears the mark. The 24-hour gap, longer than any sweep, closes the race.

Staged uploads idle for 24 hours are deleted. Blobs in state `Rejected` or `Abandoned`, and committed blobs that no
surface ever referenced, are deleted after 24 hours unless live. Retention agrees with sections 02 and 03: deleted
objects are purged after 30 days, the same period automatic snapshots are kept, so a blob stays live until both
are gone.

*Named versions.* A named version pins every blob in its state until the version is deleted, so restoring one finds
its blobs. Deleting a version, or snapshot expiry, only makes blobs sweepable at the next run.

*Overview pictures.* Section 06's overview pictures are ordinary blobs (lossless WebP or PNG) named by content hash.
06's table `PicTile { octave, tx, ty } -> blob` (06's key; 03 holds the table) records which blob is current for
which tile.
Those blobs are read with the same grants (the room treats a blob listed in that table as referenced by the surface)
and are live while the table, a retained snapshot's copy of the table, or a named version lists them; replaced
pictures follow the same condemn-then-delete rule. They are not part of `blobs_in_state`, which covers document
objects only, so the sweep unions the table in explicitly.

*Cross-surface use.* Placing a library template that contains an image copies the object, which references the same
blob id; deduplication inside the tenant makes that free. Libraries shared **across tenants** would need a
server-side copy of the blob into the receiving tenant at the time of use (an assumption on section 08, below).

### Video

**Accepted files.** The server accepts a video if a container probe finds a **direct-play profile**:

| Container | Video | Audio | Web support (MDN codec guide) | Native |
|---|---|---|---|---|
| MP4 | H.264 (AVC) | AAC-LC | Every major browser; Firefox uses the OS decoder | AVFoundation, Media Foundation, GStreamer plugin |
| WebM | VP9 | Opus | All major browsers | macOS and Windows support depends on version or extensions (unverified) |
| WebM | AV1 | Opus | Chrome 70+, Edge 121+, Firefox 67+; Safari 17+ on certain hardware only | Hardware dependent |
| MP4 / MOV | HEVC (H.265) | AAC | Partial: Safari yes, Chrome and Edge on supporting hardware, Firefox varies | Platform dependent; patent-encumbered |

The first two rows are the **baseline**: always accepted. AV1 is accepted with a warning that some viewers cannot
play it. HEVC is accepted only if the optional transcoder below is configured, because an HEVC clip is the default
format of recent iPhones and many viewers could not play it. Anything else (MKV, AVI, ProRes, 10-bit H.264) is
refused with a reason, or transcoded when the transcoder exists.

**Does the server transcode?** Not in the core. The server only **probes** a video (reads the container header with
a memory-safe demuxer; it decodes no frames), validates the numbers in `VideoFields`, and stores the file as is.
Reasons: transcoding needs an encoder (x264 is GPL; hardware encoders are platform bound; OpenH264 is BSD-2-Clause
but its patent coverage depends on using Cisco's own binaries, which I did not verify); it is heavy CPU; and it is a
service decision for the operator. An operator can configure an **external transcoder** command (typically
`ffmpeg`, run as a separate process so its licence does not reach Everink) that the media worker calls for files
outside the baseline. It produces H.264/AAC MP4, stored as `Variant::Playback`; the blob id stays the hash of what
was uploaded. If no transcoder is configured, files outside the baseline are refused. Codec licences are then the
operator's responsibility (this is not legal advice).

Container probe candidates: `mp4parse` (Mozilla) and `symphonia`, both believed MPL-2.0 (file-level copyleft;
acceptable as a dependency but needs a notice), or a small MP4 crate. None was verified here; the spike picks one.
Section 08's licence allow-list includes MPL-2.0 for this reason; if the spike picks a probe that is not MPL, that
entry is not needed.

**Streaming.** v1 is **progressive download with HTTP range requests** of the single stored file: the web shell
gives `<video>` the grant URL and the browser asks for the ranges it needs; native players do the same over HTTP. An
MP4 whose index (`moov`) is at the end still plays (the player reads the tail first), only with a slower start. HLS
and DASH are not used in v1: they need a transcoded ladder of segments; Safari plays HLS natively but other browsers
need JavaScript players on top of Media Source Extensions, iPhone Safari needs the separate `ManagedMediaSource`
(MDN: experimental, limited availability), and Rust shells have no equivalent. Revisit when files regularly exceed
500 MB or one video has more than ten simultaneous viewers; a CDN in front of the media origin is the first, cheaper
step.

**When a client fetches video bytes.** A video's data is requested only when the object is on screen at 160 pixels
wide or more, first with `preload="metadata"`; the poster fills in until the first frame is ready. Full buffering
starts when the shared state is `Playing` or the person interacts with it.

**Poster.** The pasting client draws the first frame (or a chosen frame) to a canvas, scales it to at most 2048
pixels on the long side, and uploads it as an ordinary image blob. It goes through the full image path (limits,
pyramid), so zoomed-out clients never fetch video bytes for a still view.

**Native decoding and licence.**

| Option | Licence implication for an MIT OR Apache-2.0 library | Patents | Verdict |
|---|---|---|---|
| FFmpeg bindings | LGPL 2.1+ only if built without `--enable-gpl` and `--enable-nonfree`; any GPL part (libx264 is named) makes the whole GPL. LGPL needs dynamic linking and source for the matching binaries, and that duty falls on whoever ships, which means every app that embeds Everink | Distributor's problem | Rejected as a dependency. May run as an external process under an operator |
| GStreamer through `gstreamer-rs` | Core is LGPL, and the project says applications may use any licence; loading it at run time keeps Everink's code permissive. Codecs with patent issues sit in separate "ugly" plugins the user installs | User or distribution | Accepted for Linux, optional feature |
| OS APIs: AVFoundation (macOS), Media Foundation (Windows) | No third-party licence; the OS vendor holds the codec licences | Vendor | Accepted; the primary native path |
| `rav1d` (AV1 only) | BSD-2-Clause, compatible | AV1 is royalty-free | Later fallback for AV1 gaps; I know of no H.264 decoder in safe Rust (unverified) |

Native video goes through a `PlatformVideoPlayer` supplied by the shell. This trait is the definition; section 07
refers to it and holds the rest of the platform API:

```rust
pub trait PlatformVideoPlayer {
    fn open(&mut self, url: &Url, probe: &VideoFields) -> Result<()>;      // async; progressive download
    fn set_rate(&mut self, milli: u16);               // effective rate: PlaybackState::rate_milli times 0.95 to 1.05 (nudge)
    fn seek(&mut self, to_us: u64);                   // accurate to the frame, not only to a keyframe
    fn play(&mut self); fn pause(&mut self);
    fn set_volume(&mut self, volume: f32, muted: bool);
    fn position_us(&self) -> Option<u64>;             // media time of the frame last presented
    fn next_frame(&mut self) -> Option<VideoFrameHandle>;  // CPU copy first; zero-copy surface later
    fn state(&self) -> PlayerState;                   // Idle, Loading, Ready, Stalled, Ended, Failed(e)
    fn output_latency_us(&self) -> Option<u64>;       // audio device latency, if the OS reports it
}
```

On the web the same trait is backed by a hidden `HTMLVideoElement` (`crossorigin="anonymous"`, `playsinline`,
`muted` at start). `requestVideoFrameCallback` delivers frames and its `mediaTime` gives the position (Baseline
since October 2024; the callback can fire one v-sync late and runs at the lower of the video's and the display's
rate, so `presentedFrames` is used to count missed frames). Nothing here assumes WebGPU is present; section 06 runs
`vello_gpu` on WebGPU, then on WebGL2, then `vello_cpu`, behind its `Backend` trait, and the frame path follows the
back end in use:

| Back end | How the current frame reaches the renderer |
|---|---|
| WebGPU | `importExternalTexture({ source: video })` samples the frame without a copy. The texture **expires as soon as it is used in a bind group**, so the renderer imports again every frame, just before building the bind group. If it is unavailable, `copyExternalImageToTexture` from the element, one copy per frame |
| WebGL2 | `texImage2D` (or `texSubImage2D` into a texture made once) from the `<video>` element each frame, run from the `requestVideoFrameCallback` callback |
| CPU (`vello_cpu`) | `drawImage` of the element onto a 2D canvas at the size it is drawn, then `getImageData` or the canvas used directly as the source; at most the displayed size, never the full frame, and at most 2 videos per client in this mode |

Browser support for WebGPU is not stated here: this section cites no dated source for it and does not depend on it.
WebCodecs (about 96 percent of global use; Firefox from 130, Safari fully from 26) is not used in v1: it gives exact
frames but no audio, demuxing (Mediabunny is MPL-2.0) or A/V sync, all of which the platform player already does. It
is the later path for frame-accurate features.

Native v1 uses a CPU copy of each frame (1080p NV12 is about 3 MB, so about 180 MB/s at 60 fps, within reach of
`queue.write_texture`); zero-copy IOSurface and D3D shared textures are a later optimization.

**A correction to the security text.** design.md says pasted media is "decoded by memory-safe decoders". That cannot
hold for video: browsers and operating systems decode with C and C++ code. Browsers sandbox it. On native,
Media Foundation and AVFoundation run in the app process. This design keeps the risk small by decoding only files
the server accepted and probed, and lists a sandboxed helper process for native decoding as hardening for later
(Questions for the owner, 6).

### Synchronized playback

**The shared state.** One register, written whole, so a play by one person and a seek by another never combine into
a mixture of the two:

```rust
pub struct PlaybackState {
    pub status: Status,            // Paused | Playing
    pub anchor_media_us: u64,      // media position at the anchor
    pub anchor_server_us: u64,     // server clock (microseconds since the Unix epoch) when the position was anchor_media_us
    pub rate_milli: u16,           // 1000 in the v1 UI; allowed 250..=2000
    pub looping: bool,
}
```

When `Paused`, the position is `anchor_media_us` and `anchor_server_us` is ignored. When `Playing`, the position at
server time *T* is

```
raw(T) = anchor_media_us + (T - anchor_server_us) * rate_milli / 1000
looping:      pos = raw mod duration              not looping:  pos = min(raw, duration), "Ended" once raw >= duration
```

"Ended" is computed locally by every client from the same formula; **nobody writes** a state at the end of a video,
which avoids a write storm. Pressing play at the end writes `Playing` with `anchor_media_us = 0`.

**Who writes the state.** The client that handles the click stamps `anchor_server_us` with its own estimate of the
server clock (below) and writes the register like any property (last writer wins by section 02's order). The server
validates before accepting: role editor or owner; `|anchor_server_us - server_now| <= 2 s` when `Playing`;
`anchor_media_us <= duration`; `rate` in range; at most 5 playback writes per second per user and 10 per second per
object (extra writes are dropped, not queued). Playback writes are **never queued offline**: a state with an old
anchor would claim the video has been playing for hours. With the connection down, the video controls are disabled,
and a write rejected for a stale anchor is discarded by the client, not retried. This needs a rule in section 02.

*Why the client stamps the time and not the server:* the server never rewrites a client's change (section 02); it
accepts or rejects a change whole. The cost is that the writer's clock error is baked into the
anchor, equal to its offset error below (a few milliseconds).

*Stale `Playing` at rest.* When the last client leaves a room, and when a room starts from storage, the server
replaces every `Playing` register with a `Paused` one at the position as of the last activity time (an ordinary
change by a server actor). A snapshot or named version opened read-only shows `Playing` as `Paused` at the position
as of the snapshot time.

**The server clock.** Server time is the real-time clock in microseconds since the Unix epoch, on a host kept in
time by NTP (an operator requirement). Rooms on different machines agree to the NTP error, a few milliseconds.

**Measuring the offset (an NTP-style exchange).** Section 03 owns the `Ping` and `Pong` messages on the WebSocket,
answered outside the ordered change queue; this section owns the estimate made from them. The fields it needs are:

```rust
pub enum ClockClientMsg { Ping { seq: u32, t0_us: u64 } }                         // t0: client monotonic clock
pub enum ClockServerMsg { Pong { seq: u32, t0_us: u64, t1_us: u64, t2_us: u64 } } // t1 receive, t2 send; server clock
```

The client reads its monotonic clock at `t3` when the pong arrives. Then:

```
delay  d = (t3 - t0) - (t2 - t1)              // network time both ways, excluding the server's holding time
offset o = ((t1 - t0) + (t2 - t3)) / 2         // server_clock = client_monotonic + o
```

If the outbound and return legs take equal time, `o` is exact. Otherwise the error is at most `d / 2`, the worst
case being all of the delay on one leg. Procedure, following the standard clock-filter idea: a burst of 8 pings 250
ms apart at connect, then one every 10 s; keep the last 8 samples; use the offset of the sample with the **smallest
delay** (least queued, so least likely to be asymmetric). The server stamps `t1` as it reads the frame and `t2` as
it writes the reply, so its own processing time is excluded. A change in `o` under 20 ms is smoothed in (new = old +
0.1 * (estimate - old) per sample); a larger one is a step, which also triggers a resync of every playing video.
Outside the opening burst, at most 1 ping per second per connection.

**Expected error (to be confirmed by measurement).** The bound is `d_min / 2`: at a 40 ms round trip, at most 20
ms, and usually a few milliseconds on a roughly symmetric path. The ping shares a TCP connection with ink and object
traffic, so a large burst can delay it; the minimum-delay filter discards those samples. Browsers coarsen
`performance.now()` (resolution varies with browser and privacy settings; exact values unverified): the client
estimates the resolution from sample spacing and, if it exceeds 5 ms, widens its dead band to match. Crystal drift
of 50 parts per million moves the offset 0.5 ms in the 10 s between samples (arithmetic), which is negligible.

**Following the target.** Each client, on every video frame callback (about 16 ms), for each video it is playing:

```
target = pos(client_monotonic + o) - output_latency      // output_latency from the platform if known, else 0
e      = position_us() - target                          // positive: this client is ahead. Looping: wrap into (-D/2, D/2]
if state is Stalled or a seek is in progress: return     // no decisions while the player cannot move
if |e| <= 10 ms:                      rate = 1.0          // and leave the nudging state
else if |e| <= 30 ms and not nudging: rate = 1.0          // dead band while not already correcting
else if |e| <= 400 ms:                rate = 1 - clamp(e / 2 s, -0.05, +0.05)   // nudge: at most 5 percent
else:                                 seek(target + L)    // L: this client's smoothed seek latency, initially 300 ms
```

Read from the top: under 10 ms, play at normal speed; between 10 and 30 ms, do nothing unless a nudge is already in
progress (a hysteresis band, so the rate does not flip back and forth); between 30 and 400 ms, change speed by up to
5 percent in the direction that closes the gap. The correction is proportional below 100 ms (`e / 2 s` reaches the 5
percent cap at 100 ms of error) and capped above it, so a 400 ms error takes about 6 s to fall to 100 ms and then
decays smoothly; above 400 ms, jump.
Browsers preserve pitch under `playbackRate` by default, so 5 percent is hard to hear (Syncplay's 0.95 slowdown is
the same practice). All constants live in one struct so measurements can tune them:

```rust
pub struct SyncTuning {
    pub dead_band_us: (u64, u64),   // (10_000 stop nudging below, 30_000 start nudging above)
    pub nudge_max_milli: u16,       // 50
    pub seek_threshold_us: u64,     // 400_000
    pub seek_cooldown_ms: u32,      // 2_000 between seeks of one video
    pub give_up_after: (u8, u32),   // (3 seeks, 30_000 ms) -> degraded mode
    pub min_decode_px: u32,         // 160 wide to decode
    pub placeholder_only_px: u32,   // 24: below this long side, draw the placeholder only (see the placeholder rule)
}
```

*Why 400 ms and not Syncplay's 4 s:* those thresholds suit people on separate players watching for an hour. Here the
goal is 100 ms, and a seek costs a visible and audible glitch plus a re-buffer from the previous keyframe (a typical
keyframe interval of 2 to 10 s is an assumption), so seeking is used only where nudging would take over 8 seconds.

*Error budget.* Per client: clock error up to `d_min / 2` (30 ms at a 60 ms round trip), dead band up to 30 ms,
frame presentation jitter up to one frame (16 ms at 60 Hz). Two clients in opposite directions add to `2 * (30 + 30
+ 16) = 152 ms` in the very worst case, which exceeds design.md's 100 ms target. Typical errors are far smaller and
not all in one direction. **The target is therefore restated as two viewers within
100 ms at the 95th percentile when round trips are under 60 ms**, and the test below measures it. If it fails, the
first adjustments are a smaller dead band (15 ms) and a longer clock filter.

**Falling behind and buffering.** A stall (`waiting`, or `readyState` below "have future data" on the web) pauses
the local player only. The shared register is untouched and nobody else waits. While `Stalled` the follower makes no
decisions. On recovery the error is large, so the client seeks to `target + L`, within the buffered range if it can.
Setting `L` to the measured duration of recent seeks keeps the client from landing behind again. After a seek, no
further seek for 2 s. If a client must seek three times within 30 seconds (the machine or network cannot keep up) it
enters **degraded mode**: audio muted, a badge reading "Cannot keep up", and it keeps following by the same rules.
It never asks others to wait.

**Late joiners and re-entering the view.** On receiving a video object, a client has the register but may not yet
have a clock estimate. It shows the poster, waits for the first three pongs (normally under a second), opens the
file, seeks to `target + L`, and starts playing muted. The same path runs when a video **scrolls back into view**. A
video out of view (or under 24 pixels) is paused locally and muted at once, with nothing written to the shared
state; its decoder is released after 30 seconds; on return the client recomputes the target and seeks to it. At most
**4 videos decode at once per client** (those closest to the view center); the rest show the poster or last frame
with a "paused here" badge, since hardware decoder counts are limited.

**Seek storms.** A person dragging a scrub bar previews locally (the local player moves; nothing is written) and
writes **one** register when the drag ends. If the product wants live scrubbing for others, it is limited to 4
writes per second. On receiving a new register, a follower seeks only if the new target differs from its position by
more than the threshold, and performs at most one seek per 200 ms, discarding intermediate ones (the newest wins).
The server's per-user and per-object limits above bound a hostile storm.

**Looping and the end.** The element's own `loop` is not used, because wrap handling must follow the shared formula.
With `looping`, the target wraps, the error is computed on the circle, and a follower that reaches the end seeks to
zero. A glitch of up to about 100 ms at the wrap is accepted. Without `looping`, a video at its end shows its last
frame; the register still says `Playing` and every client agrees it has ended.

**Audio.** Sound plays only for a video that is in view and at least 160 pixels wide. **Every video starts muted for
each person**; a person unmutes the ones they want (a click on the video's speaker button, which is a user
gesture), so pasting five videos does not produce five soundtracks. Volume and mute are per person and per video,
kept in local storage and never in the document, plus a master volume. Browsers block sound until the person has
interacted: the web shell starts `play()` muted (always allowed), and the unmute click handler calls `play()` with
sound inside the gesture. If `play()` rejects with `NotAllowedError`, the client stays muted and shows the unmute
prompt (Chrome's documented behavior; Safari's rules are stricter and were not verified here). After the first click
on the canvas, user activation lets later unmutes work. Bluetooth output adds 100 ms or more of audio delay; where
the platform reports `output_latency`, it is subtracted from `target`, so the sound, not the frame, is in step.

**Roles.** Only editor and owner roles write the register. Viewers see playback and control only their own volume.

### Fonts

**What a font is here.** A font is a blob like any other, parsed only by `read-fonts` (and `skrifa` for outlines),
the Rust parsers of Google's Fontations project, which is fuzzed with cargo-fuzz and reports coverage on OSS-Fuzz,
and is described by `ttf-parser`'s own README as the direction the Rust font ecosystem is moving. `ttf-parser`
forbids `unsafe` but is now in maintenance mode, so it is not chosen. Shaping and layout are section 06.

**Shipped set.** Compiled into the core as a table of `(family, style, build, blob id, scripts)`, so a document
names a font by family and build number and every client resolves the same bytes. The files are not embedded in the
wasm (size) but served from the server's static path under their blob ids, which are known at build time. Proposed
families (the selection is a product question; each licence is checked when added, as design.md requires):

| Role | Family | Licence | Status |
|---|---|---|---|
| Sans (default) | Inter (variable) | SIL OFL 1.1; "Inter" is a Reserved Font Name, so a modified copy must be renamed | verified |
| Serif, mono, hand-drawn style | to be chosen from OFL families | each read at addition | unverified |
| Fallback by script | Noto Sans per script, Noto Sans CJK, Noto Color Emoji | OFL (Noto CJK since 1.002; emoji font OFL, earlier versions Apache-2.0) | from secondary sources |

OFL permits bundling with software but not selling the font by itself, and the licence text must travel with it.
Everink ships the licence files next to the fonts and lists them in a notices file.

**Custom fonts.** A user uploads `.ttf` or `.otf`. TrueType collections, WOFF and WOFF2 are refused in v1 (WOFF2
needs a Brotli-based decoder whose Rust options I did not evaluate). The worker parses with `read-fonts`, requires
the tables a text renderer needs, applies the caps in Limits, reads the OS/2 `fsType` embedding flags, and records
family, weight, style and Unicode coverage in the manifest. If `fsType` says the font may not be embedded, the
upload is `Rejected(FontRestricted)` (a legal position to confirm, Questions for the owner, 5). Any client that
lays out text with the font downloads it as an ordinary blob through a grant.

**Subsetting.** An uploaded font is **not** subsetted: other editors need every glyph to keep typing, and a subset
would be a different file with a different hash. Delivery cost is controlled by (a) the 20 MiB cap plus HTTP
compression, and (b) for the very large *shipped* CJK fonts, build-time slicing into files that each cover a block
of code points (the Google Fonts approach, where each slice declares its Unicode range). An offline tool generates
the slices, and the builtin table lists them so a client downloads only the slices a text needs. Candidates for the
tool: `subsetter` (typst; MIT OR Apache-2.0; forbids `unsafe`) or Fontations' own subsetter (not confirmed).

**Fallback for missing glyphs.** The fallback chain is a fixed list in the builtin table, ordered by script and not
by what the operating system has: the run's font; then, for the missing code point's script, the first builtin
family that covers it; then Noto Color Emoji for emoji; then the font's `.notdef` glyph (the "tofu box"). Runs carry
a language tag where it matters (Han unification: one code point is drawn differently for Chinese, Japanese and
Korean), an input from section 06. While a needed font file is not downloaded, its text is laid out with the last
available font and laid out again when the font arrives, with the text box's stored size unchanged. A font is in one
of three states to the renderer: `Ready`, `Loading`, `Failed` (draw `.notdef`). Because the chain and the builtin
build number are in the document, two clients with all files present produce the same layout.

## Failure modes

| Failure | What happens |
|---|---|
| Upload interrupted | Client resumes from the `HEAD` offset; after 10 failures it shows Retry; the file stays in local storage; others see "Waiting for the uploader" after 60 s |
| Uploader closes the tab mid-upload | Status becomes `Stalled`, then `Abandoned` when the grant expires; on return the persisted queue resumes while the grant lives, else a fresh grant is requested |
| Hash mismatch (corruption or a lying client) | `422`, staged bytes deleted, the client retries once from zero, then shows an error |
| Declared dimensions or duration differ from the probe | `Rejected(Mismatch)`; the object stays until its author deletes it |
| Decompression bomb, huge dimensions | Refused from the header (`413` / `Rejected(TooManyPixels)`) before allocation; if a decoder still overshoots, the worker is killed at its memory or time limit |
| Decoder crash or panic | The worker dies, the job is retried twice, then `Rejected(Internal)`; the server process is unaffected |
| Corrupt file that passes the sniff | `Rejected(Corrupt)` with a category, not the decoder's text |
| Blob store down | Uploads return `503` and clients back off; cached tiles still display; new media shows the placeholder marked not yet uploaded (design.md) |
| Media worker backlog | Blobs wait in `Processing`; others keep seeing the blur; a queue-depth metric exists |
| Tile 404 or corrupt tile | Client re-requests once with a fresh grant, then keeps the coarser level it already has |
| Grant expires mid-video | Range request returns `403`; client gets a fresh grant and reloads the element at the target position |
| Poor clock estimate (round trip above 200 ms, coarse timer) | Dead band widens to `d_min / 2`; accuracy is reported in a debug panel; playback still follows, by seeks |
| Server clock steps (NTP correction, room moved to another host) | All clients re-measure within 10 s and resync; viewers see at most one seek |
| Two people press play and pause at once | One register wins on every client; the loser's button flips |
| A machine cannot decode in real time | Degraded mode (muted, badge); the others are unaffected |
| Browser blocks sound | Playback continues muted with an unmute prompt |
| Codec unsupported on this device (HEVC, AV1) | Poster plus "This device cannot play this video"; shared state unaffected |
| Font fails to parse or is restricted | `Rejected`; text keeps its fallback chain |
| Sweep deletes a blob that was about to be referenced | Closed by condemn-then-delete and the 24-hour grace; if it still happens, the pasting client re-uploads from its local copy, else the object shows "file missing" |

## How it is tested

| Rule | Test |
|---|---|
| A declared 65,000 by 65,000 PNG is refused before any large allocation | Corpus of bomb files; the worker runs under a 256 MiB limit and must return `TooManyPixels` without being killed |
| Every refused format is refused, by content and not by name | Files with wrong extensions and MIME types; animated GIF, APNG, animated WebP, SVG, HEIC and TIFF each give the right reason |
| Decoders survive hostile input | `cargo-fuzz` targets for `sniff`, the probes and each decoder wrapper, run continuously in CI; the corpus includes known crashers from the decoder crates' own repositories |
| Client and server agree on dimensions, orientation and tile geometry | Property tests over random sizes for `levels` and `tile_grid` (integer arithmetic only); 8 EXIF orientations through the client probe and the server probe |
| The level formula is sharp enough and not wasteful | Sweep `s` from 1e-6 to 1e6: tile pixels per screen pixel in [1, 2) whenever 0 < k < K |
| Zooming into an image costs constant tile requests | Simulated camera at `s` from 1 to 1e300: at most 4 level-0 tile requests, no non-finite number produced |
| A blob is stored under the hash the server computed, not the one claimed | Upload bytes with a wrong claimed id: `422`, nothing stored under either key |
| Resume works from any interruption | Cut the connection (and separately restart the server) after every N bytes for many N; the final blob is identical |
| A reader without a reference gets no grant | Grants for a blob in another surface of the same tenant, in another tenant, and nonexistent: all answered `Denied` identically |
| A viewer cannot read a blob only a deleted object referenced | Delete the only object that references a blob; a viewer's grant request is `Denied`; garbage collection still keeps the blob until the purge horizon |
| An upload after a link is turned off is refused | `POST`, `PATCH` and `HEAD` with the old upload grant after revocation: each refused |
| A frozen guest keeps read access and loses upload grants | Freeze a guest mid-upload: the next `PATCH` is refused, reads of blobs still succeed |
| Response headers are the same in presigned and proxy mode | Run 08's header test against both modes |
| Grants expire and are bound to one blob | Expired tag, tag of blob A used on blob B, tampered `exp`: each `403` |
| Identical files are shared inside a tenant and not across tenants | Two uploads, then a storage listing |
| Unreferenced blobs are collected and referenced ones never | Randomized document histories with deletes, undo, snapshots, named versions and restores; invariant: every blob in any live state exists after a sweep |
| The race between sweep and a new reference is closed | Paste an old picture between the two sweep phases |
| Playback state converges | Section 02's randomized convergence test extended with the `PlaybackState` register |
| The clock offset error is within its bound | Simulated network with asymmetric delay and bursts: error never exceeds `d_min / 2` of the chosen sample |
| Followers converge and do not oscillate | Simulated player with seek latency, rate limits and stalls: after any perturbation, `|e| < 30 ms` within 10 s with at most one seek, and no rate flip-flop |
| Two viewers stay within 100 ms (95th percentile) | Real clients over a network emulator at 40 ms and 60 ms round trip with 1 percent loss; compare frame `mediaTime` against wall time |
| Late join, scroll out and back, loop wrap, end of media and stall recovery behave as written | Scripted scenarios against a fake player |
| Nothing is written at the end of media or by followers | Assertion on the write log in those scenarios |
| Stale `Playing` is not resurrected | Room restart and snapshot-open tests |
| Playback writes are rate limited and rejected when stale | Negative tests with skewed anchors and write floods |
| Sound only in view; muted by default; autoplay block falls back to muted | Browser tests in Chrome and Safari |
| Hostile fonts do not crash the worker; `fsType` is honored; fallback is deterministic | Fuzz corpus; fixtures with restricted flags; the same text with the same chain on two platforms gives the same glyph ids |

## Dependencies

Versions were read from docs.rs or repositories on 2026-11-11 where stated; "unverified" means not read.

| Crate / component | Version seen | Licence | Maintenance / safety | Why |
|---|---|---|---|---|
| `image` | 0.25.10 | MIT OR Apache-2.0 | Active; `Limits` (`max_alloc` default 512 MiB, non-strict) | Decoder front end; its AVIF path calls C dav1d, so that feature stays off |
| `png` | unverified | MIT OR Apache-2.0 | README: no `unsafe`; fuzzed on OSS-Fuzz | PNG and APNG detection |
| `zune-jpeg` (zune-image project) | unverified | Project files: Apache-2.0, MIT, Zlib (per-crate not read) | Project states daily fuzzing of critical decoders in CI; only `unsafe` is SIMD | JPEG |
| `image-webp` | 0.2.3 seen | Apache-2.0 and MIT | README: no `unsafe` in it or its dependencies | WebP decode, lossless encode |
| `gif` | unverified | unverified | unverified | GIF, single frame |
| `jpeg-encoder` | unverified | unverified | unverified | JPEG tile encoding (named in the zune-image README) |
| `jxl-oxide` | 0.12.5 in README example | MIT and Apache-2.0 | Pure Rust; fuzz directory | JPEG XL, phase 2 |
| `rav1d` / `rav1d-safe` / `zenavif` | 1.1.0 / 0.5.7 / 0.1.7 | BSD-2-Clause / unverified / unverified | rav1d: port of dav1d, assembly optional; the others young | AVIF, phase 2 spike only |
| `moxcms` | 0.9.0 | BSD-3-Clause OR Apache-2.0 (take Apache-2.0) | Active, pure Rust | ICC to sRGB |
| `blake3` | unverified | CC0-1.0 or Apache-2.0 (or Apache-2.0 with LLVM exception) | Maintained by the BLAKE3 team; SIMD and WebAssembly paths | The blob hash |
| `thumbhash` or a port of the reference | unverified | MIT (reference) | Small algorithm | Placeholder |
| `object_store` | 0.14.2 (docs dated 2026-11-09) | MIT OR Apache-2.0 | Active | Backends, multipart, presigned URLs |
| `read-fonts`, `skrifa` | unverified | Apache-2.0 and MIT (repository level) | Fuzzed; Google Fonts | Font parsing |
| `subsetter` | unverified | MIT OR Apache-2.0 | `forbid(unsafe)` (from search summary) | Offline font slicing |
| `mp4parse` / `symphonia` / `mp4` | unverified | believed MPL-2.0 / MPL-2.0 / unverified | unverified | Container probe; choose one in the spike |
| `gstreamer-rs` | unverified | Bindings unverified; GStreamer core LGPL | Active | Linux native playback, optional, loaded at run time |
| `objc2`, `windows` | unverified | unverified | Active | AVFoundation and Media Foundation bindings |
| FFmpeg (operator-run process only) | n/a | LGPL 2.1+ or GPL, by build | n/a | Optional external transcoder, never linked |
| Not used: `timingsrc` | n/a | LGPL-3.0 | Group closed 2025 | Ideas only |
| Not used: `ttf-parser` | n/a | MIT OR Apache-2.0 | Maintenance mode; recommends Fontations | |

The chosen dependencies are MIT, Apache-2.0, BSD or CC0, so compatible with MIT OR Apache-2.0. MPL-2.0 (the
container probe candidates) is acceptable as a dependency but adds a notice duty. LGPL code is never linked
statically.

## What this assumes of other sections

- **01.** Treats `Position` as opaque to media. Gives media code, for an image object and a camera, the number `s`
  (screen pixels per source pixel) as an `f64` ratio, and the visible rectangle in image-local source pixels as a
  clipped `f64` rectangle that stays finite and precise at any depth, or says "off screen". These are section 01's
  `image_view`, which returns an `ImageView { screen_per_src, visible_src }`; the caller passes the image's
  rectangle in object units and its source pixel size. On-screen size uses 01's `pixel_log2` and `ExactBox`.
- **02.** Holds the schema of `ImageFields`, `VideoFields`, `PlaybackState`, `FontRef`, `BlobId` and `Placeholder`
  as I define them here, keeps `PlaybackState` as a register of its own, written whole and never queued offline (a
  write with a stale anchor is dropped, not retried), and allows server-authored changes, used for
  pause-at-rest. Provides `blobs_in_state(&Doc)`, `blobs_in_object` and `ObjectEffect::new_blobs`. Stores `size` and
  `crop` for images and videos, from which 06's `ExtentOracle` gives the box. Maps a `crop` in object units to
  source pixels by scaling with source pixel size over object size (02's rule); tile choice uses the cropped source
  region.
- **03.** Carries the blob messages with its own names and fields (`UploadGrantRequest { req, blob, size, kind }`,
  `BlobGrantRequest` and replies, `BlobStatus`, and the clock `Ping` and `Pong`, which it owns); pongs are answered
  outside the ordered change queue. Holds the blob records and the reference index; runs the daily sweep; supplies
  `blobs_in_store(store, state)` (which calls 02's `blobs_in_state`) over current state, snapshots and named
  versions; holds the table behind 06's `PicTile`; normalizes `Playing` on room start. Subscribes a client to
  `BlobStatus` for blobs of objects in its view.
- **04.** No dependency, except that presence and clock pings share the one WebSocket; section 03's pacing rule
  (unsent bytes kept to about 20 ms of the link's drain rate, between 4 KiB and 64 KiB, bulk chunks never over 16
  KiB) keeps a ping from waiting more than about 40 ms behind bulk data on the socket; blob transfers do not use the
  WebSocket at all. Both 04 and 05 use the same offset.
- **06.** Needs from media, and gets: decoded images as tiles at the mip levels above (tile size 512, levels
  `0..=K`), with the maximum texture size taken from the active back end (a tile never exceeds 512, so only back
  ends with a smaller limit are affected); font bytes for a per-script fallback set that includes an emoji font; and
  storage for overview-picture blobs (ordinary hash-named blobs, written by the headless renderer through the same
  `BlobStore` and listed in the `PicTile { octave, tx, ty }` table that 03 holds, read with the same grants, and
  kept live while that table or a retained copy of it lists them). It supplies `ExtentOracle` for images and videos
  from their stored `size`. It draws tiles, blends levels while zooming, magnifies pixels with its chosen filter,
  clips quads to the visible rectangle, draws video through the per-back-end path in Video (WebGPU, WebGL2, CPU),
  owns text layout and shaping, language-tagged runs, the *use* of the fallback chain defined here, and re-layout
  when a font arrives. It draws the blob-status badges.
- **07.** The platform API gains: an HTTP client with `Range`, streaming `PATCH` upload and cancellation
  (design.md's platform table lists only a WebSocket for Network); the `PlatformVideoPlayer` defined in this section
  (07 refers to it and keeps no sketch of its own); a persistent file store for pending uploads and the tile cache;
  user-activation awareness on the web; an unmute control in the shared controls. 07's package list includes
  `everink-media`.
- **08.** Holds the single limits table (adopting the member figures here, with stricter guest figures), who may
  upload, the upload-grant and blob-grant definitions (HMAC-SHA-256, the `/b/` and `/o/` paths) and key rotation,
  the upload grant (bound to user, surface, blob and size, valid until the session ends and for at most 24 hours,
  checked against the revoked-session list on every upload request), the 60-second presigned-address gap in its
  threat table, the cap on blob ids per grant request, the CORS origin list and the separate media site, the one
  list of response headers (presigned mode must set the same ones, else proxy mode), the rows "set by section 05"
  that mirror this section's server-enforced limits, MPL-2.0 on its licence allow-list for the container probes,
  freezing of guests (frozen guests keep read access and lose upload grants), whether libraries may be shared across
  tenants (assumed not, or by copy), and global rate limits. It requires, and this section states: images re-encoded
  and never served as uploaded, SVG refused, `nosniff`, attachment for originals, a sandboxing CSP, a cross-origin
  resource policy, server-computed hashes, unreferenced uploads cleaned after 24 hours, and no unsandboxed
  `ffmpeg`-class tool.

## Questions for the owner

1. **Animated pictures.** Refuse (this design), or convert animated GIF and WebP to a looping silent video at paste
   time? Conversion adds a transcoder and storage for something people paste often.
2. **SVG.** Never stored as an image (this design). Should a later release import SVG as editable Everink objects?
   It suits unlimited zoom but is a feature with its own bounds.
3. **Video that is not H.264/AAC or VP9/Opus** (iPhone HEVC, ProRes, MKV). Refuse, or allow an operator-run external
   `ffmpeg`? The second makes pasting from a phone work and puts codec licensing on the operator.
4. **Default limits** (50 MiB images, 128 megapixels, 2 GiB video, 20 MiB fonts) and whether **guests** may upload
   at all. Product and cost decisions; the figures live in section 08's table.
5. **Custom fonts.** Allow users to upload fonts at all, and honor the font's embedding flag (`fsType`) by refusing
   restricted ones? This is a legal position as well as a product one.
6. **Native video decoding in the app process.** Accept it for v1 (this design), or require a sandboxed helper
   process before release? Browsers already sandbox decoders.
7. **Originals.** Serve original files (with their metadata, including GPS) only to members, as attachments (this
   design), or to everyone who can see the surface?
8. **Default muted video** for every person until they unmute (this design). Confirm.
9. **Shipped font families** beyond Inter, and whether a hand-drawn style is wanted.
10. **CRDT library** (reserved for the owner). Media needs only a per-property last-writer-wins value that is
    replaced whole (so `PlaybackState` is one value) and a way for the server to act as an author. Both Loro and yrs
    have maps whose plain values should behave that way, but I did not verify either's API; the spike should check.
    No other preference follows from media.
11. **First app** (reserved). A video-heavy first app (a planning wall, a classroom board) raises the priority of
    HLS and a transcoder; a notebook or whiteboard does not.

## Sources

All read 2026-11-11 unless noted. "via search summary" means only a search result summary was read, not the page.

- [tldraw assets] https://tldraw.dev/sdk-features/assets and https://tldraw.dev/reference/tlschema/TLAssetStore (via search summary)
- [Excalidraw notes] https://deepwiki.com/zsviczian/excalidraw/7.3-local-data-persistence and
  https://git.biohazardvfx.com/Nicholai/excalidraw/raw/branch/nicholais-excalidraw/docs/architecture/brownfield-architecture.md
  (secondary sources about forks; upstream not read)
- [Figma images] https://developers.figma.com/docs/plugins/working-with-images and
  https://developers.figma.com/docs/plugins/api/Image (via search summary)
- [Miro formats] https://help.miro.com/hc/articles/360017731613 and
  https://community.miro.com/ideas/please-remove-the-32-mp-image-size-restriction-19220 (via search summary)
- [Deep Zoom] https://learn.microsoft.com/en-us/previous-versions/windows/silverlight/dotnet-windows-silverlight/cc645077(v=vs.95)
- [IIIF] https://iiif.io/api/image/3.0/
- [ThumbHash] https://github.com/evanw/thumbhash (BlurHash comparison taken from its README; BlurHash itself not fetched)
- [tus] https://tus.io/protocols/resumable-upload
- [S3 limits] https://docs.aws.amazon.com/AmazonS3/latest/userguide/qfacts.html
- [BLAKE3] https://github.com/BLAKE3-team/BLAKE3
- [Syncplay constants] https://raw.githubusercontent.com/Syncplay/syncplay/master/syncplay/constants.py
- [timing object] https://webtiming.github.io/timingobject/
- [timingsrc] https://github.com/webtiming/timingsrc (MediaSync algorithm details not read)
- `image` https://docs.rs/image/latest/image/ , https://docs.rs/image/latest/image/struct.Limits.html ,
  https://docs.rs/image/latest/image/codecs/index.html
- zune-image https://github.com/etemesi254/zune-image
- jxl-oxide https://github.com/tirr-c/jxl-oxide
- image-webp https://github.com/image-rs/image-webp
- png https://github.com/image-rs/image-png
- rav1d https://github.com/memorysafety/rav1d ; AVIF alternatives (search summary): https://docs.rs/crate/zenavif/latest ,
  https://docs.rs/crate/rav1d-safe/latest , https://docs.rs/crate/rav1d/latest
- moxcms https://lib.rs/crates/moxcms and https://docs.rs/crate/moxcms/0.9.0 (via search summary)
- object_store https://docs.rs/object_store/latest/object_store/ and
  https://docs.rs/object_store/latest/object_store/signer/trait.Signer.html
- Fontations https://github.com/googlefonts/fontations ; ttf-parser https://github.com/harfbuzz/ttf-parser
- subsetter https://github.com/typst/subsetter and fontcull https://lib.rs/crates/fontcull (via search summary)
- Inter https://github.com/rsms/inter
- Noto CJK licence history https://github.com/notofonts/noto-cjk/blob/main/Sans/README-third_party.md (via search summary)
- WebGPU `importExternalTexture` https://developer.mozilla.org/en-US/docs/Web/API/GPUDevice/importExternalTexture
- WebCodecs https://developer.mozilla.org/en-US/docs/Web/API/WebCodecs_API and https://caniuse.com/webcodecs
- `requestVideoFrameCallback` https://developer.mozilla.org/en-US/docs/Web/API/HTMLVideoElement/requestVideoFrameCallback
- `ManagedMediaSource` https://developer.mozilla.org/en-US/docs/Web/API/ManagedMediaSource
- Video codec support and licensing https://developer.mozilla.org/en-US/docs/Web/Media/Guides/Formats/Video_codecs
- Chrome autoplay policy https://developer.chrome.com/blog/autoplay
- Mediabunny https://github.com/Vanilagy/mediabunny
- FFmpeg licensing https://ffmpeg.org/legal.html
- GStreamer licensing https://gstreamer.freedesktop.org/documentation/frequently-asked-questions/licensing.html
- OpenH264 https://github.com/cisco/openh264 (BSD-2-Clause; Cisco's binary patent terms were not on the page)
- Rerun `re_video` https://github.com/rerun-io/rerun/tree/main/crates/utils/re_video (README only; not relied on)

Claims not verified, beyond those marked "unverified" in the text: ThumbHash's exact byte size; decoder speeds and
the 1 bit per pixel tile size; Safari autoplay and detached-video behavior; browser decoder-count limits; timer
resolution in Firefox; macOS and Windows VP9 and AV1 support; `crypto.subtle.digest` being one-shot; Cisco's
OpenH264 patent terms; Watch2Gether, Teleparty, FigJam and Miro video internals; EXIF orientation API availability
in `image`; the CRDT libraries' register semantics.
