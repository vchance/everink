//! Spike question 9: how fast is storage? One SQLite file per room, as section 03 of the design has it.
//!
//! The file holds a log with one row for each accepted change, and snapshots: the objects in chunks of 256 by
//! ordinal, each chunk compressed with LZ4 and stored under its BLAKE3 hash, and a manifest that lists the
//! hashes. A room is opened from the newest snapshot and the log rows after it.
//!
//! Of the design's tables only `log`, `chunk`, `snap` and `meta` are here; versions' names, blobs, pictures and
//! the audit trail are not.

use std::collections::BTreeSet;
use std::path::Path;
use std::time::Instant;

use everink_core::DepthCap;
use everink_sync::wire::{decode_change, encode_change};
use everink_sync::{Change, ClientId, Object, Outcome, Room, Seq, UserId};
use rusqlite::{Connection, OptionalExtension, params};

/// Objects in a chunk: the design's starting value.
pub const CHUNK: u32 = 256;

/// How hard a commit pushes its bytes to the disk.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Sync {
    /// `synchronous=FULL`: the design's setting. SQLite calls `fsync` at every commit.
    Full,
    /// `synchronous=FULL` with `fullfsync=ON`. On macOS `fsync` does not make the drive write its cache out;
    /// `F_FULLFSYNC` does, and this asks SQLite to use it. Elsewhere it is the same as `Full`.
    FullFsync,
    /// `synchronous=NORMAL`: in WAL mode no sync at a commit, so a commit can be lost on power failure. Here
    /// to show what the sync costs; the design rules it out.
    Normal,
}

#[derive(Debug)]
pub enum StoreError {
    Sql(rusqlite::Error),
    /// The file does not hold what it should; the text says what.
    Damaged(String),
}

impl From<rusqlite::Error> for StoreError {
    fn from(e: rusqlite::Error) -> Self {
        StoreError::Sql(e)
    }
}

fn damaged<T>(what: impl Into<String>) -> Result<T, StoreError> {
    Err(StoreError::Damaged(what.into()))
}

/// One accepted change, as the log keeps it: the change as the client sent it and who sent it.
pub struct LogRow<'a> {
    pub seq: Seq,
    pub time_ms: u64,
    pub user: UserId,
    pub client: ClientId,
    pub change: &'a Change,
}

/// What writing a snapshot did.
#[derive(Debug, Default, Clone, Copy)]
pub struct SnapshotStats {
    pub chunks_in_manifest: usize,
    pub chunks_written: usize,
    pub raw_bytes: usize,
    pub stored_bytes: usize,
    pub manifest_bytes: usize,
    /// Encoding and compressing the chunks, and hashing them.
    pub build_ms: f64,
    /// The transaction that writes them, to the return of the commit.
    pub commit_ms: f64,
}

/// Where the time of opening a room went, in milliseconds.
#[derive(Debug, Default, Clone, Copy)]
pub struct LoadStats {
    pub chunks: usize,
    pub objects: usize,
    pub log_rows: usize,
    /// Reading the manifest and the chunk rows from SQLite.
    pub read_ms: f64,
    /// Checking each chunk's hash and decompressing it.
    pub verify_ms: f64,
    /// Decoding and checking every object.
    pub decode_ms: f64,
    /// Building the room: the object table, the children index, the check of the tree.
    pub build_ms: f64,
    /// Reading the log rows after the snapshot, checking and applying them.
    pub replay_ms: f64,
}

pub struct Store {
    conn: Connection,
}

struct Manifest {
    head: u64,
    next_ordinal: u32,
    rng: u64,
    /// Chunk number and hash, ascending by number.
    chunks: Vec<(u32, [u8; 32])>,
}

impl Manifest {
    fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(24 + self.chunks.len() * 36 + 4);
        out.extend_from_slice(&self.head.to_le_bytes());
        out.extend_from_slice(&self.next_ordinal.to_le_bytes());
        out.extend_from_slice(&self.rng.to_le_bytes());
        out.extend_from_slice(&(self.chunks.len() as u32).to_le_bytes());
        for (n, hash) in &self.chunks {
            out.extend_from_slice(&n.to_le_bytes());
            out.extend_from_slice(hash);
        }
        let crc = crc32c::crc32c(&out);
        out.extend_from_slice(&crc.to_le_bytes());
        out
    }

    fn decode(bytes: &[u8]) -> Result<Manifest, StoreError> {
        if bytes.len() < 28 {
            return damaged("a manifest that is too short");
        }
        let (body, crc) = bytes.split_at(bytes.len() - 4);
        if crc32c::crc32c(body).to_le_bytes() != crc {
            return damaged("a manifest whose checksum does not match");
        }
        let count = u32::from_le_bytes(body[20..24].try_into().unwrap()) as usize;
        if body.len() != 24 + count * 36 {
            return damaged("a manifest of the wrong length");
        }
        let chunks = body[24..]
            .chunks_exact(36)
            .map(|c| {
                (
                    u32::from_le_bytes(c[..4].try_into().unwrap()),
                    c[4..].try_into().unwrap(),
                )
            })
            .collect();
        Ok(Manifest {
            head: u64::from_le_bytes(body[..8].try_into().unwrap()),
            next_ordinal: u32::from_le_bytes(body[8..12].try_into().unwrap()),
            rng: u64::from_le_bytes(body[12..20].try_into().unwrap()),
            chunks,
        })
    }
}

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1e3
}

impl Store {
    /// Opens the room's file, making it if it is not there.
    pub fn open(path: &Path, sync: Sync) -> Result<Store, StoreError> {
        let conn = Connection::open(path)?;
        let mode: String = conn.query_row("PRAGMA journal_mode=WAL", [], |r| r.get(0))?;
        if mode != "wal" {
            return damaged(format!("journal mode {mode}, not wal"));
        }
        conn.execute_batch(match sync {
            Sync::Full => "PRAGMA synchronous=FULL; PRAGMA fullfsync=OFF;",
            Sync::FullFsync => "PRAGMA synchronous=FULL; PRAGMA fullfsync=ON;",
            Sync::Normal => "PRAGMA synchronous=NORMAL; PRAGMA fullfsync=OFF;",
        })?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS log (seq INTEGER PRIMARY KEY, time_ms INTEGER, user BLOB, client BLOB,
                                             change_id INTEGER, crc INTEGER, blob BLOB);
             CREATE TABLE IF NOT EXISTS chunk (hash BLOB PRIMARY KEY, data BLOB) WITHOUT ROWID;
             CREATE TABLE IF NOT EXISTS snap (id INTEGER PRIMARY KEY, seq INTEGER, time_ms INTEGER, manifest BLOB);
             CREATE TABLE IF NOT EXISTS meta (k TEXT PRIMARY KEY, v BLOB);
             INSERT OR IGNORE INTO meta (k, v) VALUES ('format', x'01');",
        )?;
        Ok(Store { conn })
    }

    /// Appends accepted changes to the log in one transaction. When it returns they are as durable as the
    /// `Sync` the store was opened with makes them.
    pub fn append(&mut self, rows: &[LogRow<'_>]) -> Result<(), StoreError> {
        let tx = self.conn.transaction()?;
        {
            let mut insert = tx.prepare_cached(
                "INSERT INTO log (seq, time_ms, user, client, change_id, crc, blob) VALUES (?, ?, ?, ?, ?, ?, ?)",
            )?;
            for row in rows {
                let blob = encode_change(row.change);
                insert.execute(params![
                    row.seq.0 as i64,
                    row.time_ms as i64,
                    row.user.0.to_le_bytes(),
                    row.client.0,
                    row.change.id.0 as i64,
                    i64::from(crc32c::crc32c(&blob)),
                    blob,
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    fn newest_manifest(&self) -> Result<Option<Manifest>, StoreError> {
        let bytes: Option<Vec<u8>> = self
            .conn
            .query_row(
                "SELECT manifest FROM snap ORDER BY id DESC LIMIT 1",
                [],
                |r| r.get(0),
            )
            .optional()?;
        bytes.as_deref().map(Manifest::decode).transpose()
    }

    /// Writes a snapshot of the room as it is now. With `dirty` given, only those chunks are encoded and
    /// written and the others are taken from the newest snapshot's manifest; with `None`, all of them.
    pub fn snapshot(
        &mut self,
        room: &Room,
        dirty: Option<&BTreeSet<u32>>,
        time_ms: u64,
    ) -> Result<SnapshotStats, StoreError> {
        let mut stats = SnapshotStats::default();
        let t = Instant::now();
        let mut objects: Vec<&Object> = room.objects().collect();
        objects.sort_unstable_by_key(|o| o.ordinal());
        let previous = match dirty {
            Some(_) => self.newest_manifest()?,
            None => None,
        };
        let mut kept = previous
            .map(|m| m.chunks)
            .unwrap_or_default()
            .into_iter()
            .peekable();
        let mut chunks: Vec<(u32, [u8; 32])> = Vec::new();
        let mut fresh: Vec<([u8; 32], Vec<u8>)> = Vec::new();
        let mut raw = Vec::new();
        for group in objects.chunk_by(|a, b| a.ordinal().0 / CHUNK == b.ordinal().0 / CHUNK) {
            let number = group[0].ordinal().0 / CHUNK;
            while kept.peek().is_some_and(|(n, _)| *n < number) {
                kept.next();
            }
            let old = kept.peek().filter(|(n, _)| *n == number).map(|(_, h)| *h);
            if let (Some(hash), Some(set)) = (old, dirty)
                && !set.contains(&number)
            {
                chunks.push((number, hash));
                continue;
            }
            raw.clear();
            for object in group {
                let bytes = object.encode();
                raw.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
                raw.extend_from_slice(&bytes);
            }
            let data = lz4_flex::compress_prepend_size(&raw);
            let hash = *blake3::hash(&data).as_bytes();
            stats.raw_bytes += raw.len();
            stats.stored_bytes += data.len();
            chunks.push((number, hash));
            fresh.push((hash, data));
        }
        let manifest = Manifest {
            head: room.head().0,
            next_ordinal: room.next_ordinal(),
            rng: room.rng_state(),
            chunks,
        }
        .encode();
        stats.chunks_in_manifest = (manifest.len() - 28) / 36;
        stats.chunks_written = fresh.len();
        stats.manifest_bytes = manifest.len();
        stats.build_ms = ms(t);

        let t = Instant::now();
        let tx = self.conn.transaction()?;
        {
            let mut insert =
                tx.prepare_cached("INSERT OR IGNORE INTO chunk (hash, data) VALUES (?, ?)")?;
            for (hash, data) in &fresh {
                insert.execute(params![hash, data])?;
            }
            tx.execute(
                "INSERT INTO snap (seq, time_ms, manifest) VALUES (?, ?, ?)",
                params![room.head().0 as i64, time_ms as i64, manifest],
            )?;
        }
        tx.commit()?;
        stats.commit_ms = ms(t);
        Ok(stats)
    }

    /// Opens the room: the newest snapshot, then every log row after it, each checked and applied as the room
    /// applied it the first time. A file with no snapshot starts from an empty room with `seed`.
    pub fn load(&self, cap: DepthCap, seed: u64) -> Result<(Room, LoadStats), StoreError> {
        let mut stats = LoadStats::default();
        let t = Instant::now();
        let manifest = self.newest_manifest()?;
        let mut room = match manifest {
            None => Room::new(cap, seed),
            Some(manifest) => {
                let mut datas: Vec<Vec<u8>> = Vec::with_capacity(manifest.chunks.len());
                {
                    let mut get = self
                        .conn
                        .prepare_cached("SELECT data FROM chunk WHERE hash = ?")?;
                    for (number, hash) in &manifest.chunks {
                        let data: Option<Vec<u8>> =
                            get.query_row(params![hash], |r| r.get(0)).optional()?;
                        match data {
                            Some(d) => datas.push(d),
                            None => return damaged(format!("chunk {number} is missing")),
                        }
                    }
                }
                stats.chunks = datas.len();
                stats.read_ms = ms(t);

                let t = Instant::now();
                let mut raws: Vec<Vec<u8>> = Vec::with_capacity(datas.len());
                for ((number, hash), data) in manifest.chunks.iter().zip(&datas) {
                    if blake3::hash(data).as_bytes() != hash {
                        return damaged(format!("chunk {number} does not match its hash"));
                    }
                    match lz4_flex::decompress_size_prepended(data) {
                        Ok(raw) => raws.push(raw),
                        Err(_) => return damaged(format!("chunk {number} does not decompress")),
                    }
                }
                drop(datas);
                stats.verify_ms = ms(t);

                let t = Instant::now();
                let mut objects: Vec<Object> = Vec::new();
                for ((number, _), raw) in manifest.chunks.iter().zip(&raws) {
                    let mut rest = raw.as_slice();
                    while !rest.is_empty() {
                        if rest.len() < 4 {
                            return damaged(format!("chunk {number} ends inside a length"));
                        }
                        let len = u32::from_le_bytes(rest[..4].try_into().unwrap()) as usize;
                        if rest.len() < 4 + len {
                            return damaged(format!("chunk {number} ends inside an object"));
                        }
                        let object = Object::decode(&rest[4..4 + len], &cap)
                            .map_err(|e| StoreError::Damaged(format!("chunk {number}: {e:?}")))?;
                        if object.ordinal().0 / CHUNK != *number {
                            return damaged(format!("chunk {number} holds an object of another"));
                        }
                        objects.push(object);
                        rest = &rest[4 + len..];
                    }
                }
                drop(raws);
                stats.objects = objects.len();
                stats.decode_ms = ms(t);

                let t = Instant::now();
                let room = Room::from_objects(
                    Seq(manifest.head),
                    manifest.next_ordinal,
                    objects,
                    cap,
                    manifest.rng,
                )
                .map_err(|e| StoreError::Damaged(format!("the snapshot: {e:?}")))?;
                stats.build_ms = ms(t);
                room
            }
        };

        let t = Instant::now();
        let mut rows = self.conn.prepare_cached(
            "SELECT seq, user, client, crc, blob FROM log WHERE seq > ? ORDER BY seq",
        )?;
        let mut rows = rows.query(params![room.head().0 as i64])?;
        while let Some(row) = rows.next()? {
            let seq = row.get::<_, i64>(0)? as u64;
            let user: [u8; 8] = row.get(1)?;
            let client: [u8; 16] = row.get(2)?;
            let crc = row.get::<_, i64>(3)?;
            let blob = row.get_ref(4)?.as_blob().map_err(rusqlite::Error::from)?;
            if seq != room.head().0 + 1 {
                return damaged(format!("log row {seq} follows {}", room.head().0));
            }
            if i64::from(crc32c::crc32c(blob)) != crc {
                return damaged(format!("log row {seq} does not match its checksum"));
            }
            let change = decode_change(blob, &cap)
                .map_err(|e| StoreError::Damaged(format!("log row {seq}: {e:?}")))?;
            match room.submit(ClientId(client), UserId(u64::from_le_bytes(user)), &change) {
                Outcome::Accepted(applied) if applied.seq.0 == seq => {}
                other => {
                    return damaged(format!(
                        "log row {seq} was not accepted again: {}",
                        match other {
                            Outcome::Accepted(a) => format!("it got number {}", a.seq.0),
                            Outcome::Duplicate { .. } => "a duplicate".to_string(),
                            Outcome::Rejected(code) => format!("{code:?}"),
                        }
                    ));
                }
            }
            stats.log_rows += 1;
        }
        stats.replay_ms = ms(t);
        Ok((room, stats))
    }

    /// Deletes the log rows at or below the second-newest snapshot, as the design's compaction does.
    pub fn trim_log(&mut self) -> Result<usize, StoreError> {
        let seq: Option<i64> = self
            .conn
            .query_row(
                "SELECT seq FROM snap ORDER BY id DESC LIMIT 1 OFFSET 1",
                [],
                |r| r.get(0),
            )
            .optional()?;
        Ok(match seq {
            Some(seq) => self
                .conn
                .execute("DELETE FROM log WHERE seq <= ?", params![seq])?,
            None => 0,
        })
    }

    /// Moves the write-ahead log into the main file, so that the file's size can be read.
    pub fn checkpoint(&self) -> Result<(), StoreError> {
        self.conn
            .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))?;
        Ok(())
    }

    /// For tests: runs SQL on the file, to damage it.
    pub fn execute_for_test(&self, sql: &str) -> Result<usize, StoreError> {
        Ok(self.conn.execute(sql, [])?)
    }
}

/// The chunks that an accepted change touched, to pass to `Store::snapshot`.
pub fn dirty_chunks(room: &Room, applied: &everink_sync::Applied, into: &mut BTreeSet<u32>) {
    for effect in &applied.effects {
        let id = match effect {
            everink_sync::Effect::Create(object) => object.id(),
            everink_sync::Effect::Write { id, .. } => *id,
        };
        if let Some(object) = room.object(&id) {
            into.insert(object.ordinal().0 / CHUNK);
        }
    }
}

/// The strokes the trial stores: the same as the object map's timing run made (slice 2a).
pub mod make {
    use everink_core::{Axis, Level, Position};
    use everink_sync::object::IDENTITY_LINEAR;
    use everink_sync::rng::SplitMix64;
    use everink_sync::{
        At, Body, Change, ClientChangeId, NewObject, ObjectId, Op, PackedPoints, Paint, Point, Seq,
        StrokeBrush, Xform,
    };

    pub const POINTS_PER_STROKE: usize = 200;

    pub fn anchor(rng: &mut SplitMix64) -> Position {
        Position {
            level: Level(0),
            x: Axis::from_i128((rng.next_u64() >> 8) as i128),
            y: Axis::from_i128((rng.next_u64() >> 8) as i128),
        }
    }

    /// A pen stroke: a random walk of 200 points whose steps are a few hundred lattice units.
    pub fn points(rng: &mut SplitMix64) -> PackedPoints {
        let (mut x, mut y) = (0i32, 0i32);
        let points: Vec<Point> = (0..POINTS_PER_STROKE)
            .map(|_| {
                x += rng.below(801) as i32 - 400;
                y += rng.below(801) as i32 - 400;
                Point {
                    x,
                    y,
                    pressure: 1 + rng.below(255) as u8,
                }
            })
            .collect();
        PackedPoints::pack(&points).expect("under the point limit")
    }

    pub fn object_id(rng: &mut SplitMix64) -> ObjectId {
        let mut b = [0u8; 16];
        b[..8].copy_from_slice(&rng.next_u64().to_le_bytes());
        b[8..].copy_from_slice(&rng.next_u64().to_le_bytes());
        ObjectId(b)
    }

    /// The change that draws a stroke at `at`; it is the client's change number `number`.
    pub fn stroke(rng: &mut SplitMix64, number: u64, at: At) -> Change {
        Change {
            id: ClientChangeId(number),
            observed: Seq(0),
            ops: vec![Op::Create(NewObject {
                id: object_id(rng),
                parent: None,
                at,
                xform: Xform {
                    anchor: anchor(rng),
                    linear: IDENTITY_LINEAR,
                },
                name: None,
                body: Body::Stroke {
                    brush: StrokeBrush {
                        brush: 1,
                        base_width: 3 << 16,
                        style_ver: 1,
                    },
                    points: points(rng),
                    paint: Paint { rgba: 0x2020_20ff },
                },
            })],
        }
    }
}
