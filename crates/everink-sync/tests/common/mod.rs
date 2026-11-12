//! Shared helpers: a world that drives a room with random changes from several clients and keeps the log of
//! what the room accepted.
#![allow(dead_code)]

use everink_core::{Axis, DepthCap, Level, Position};
use everink_sync::object::IDENTITY_LINEAR;
use everink_sync::rng::SplitMix64;
use everink_sync::{
    Applied, At, Body, Change, ClientChangeId, ClientId, Kind, NewObject, ObjectId, Op, OrderKey,
    Outcome, PackedPoints, Paint, Point, RejectCode, Room, Seq, StrokeBrush, UserId, Value, Xform,
};

pub const CAP: DepthCap = DepthCap::DEFAULT;

pub fn object_id(rng: &mut SplitMix64) -> ObjectId {
    let mut b = [0u8; 16];
    b[..8].copy_from_slice(&rng.next_u64().to_le_bytes());
    b[8..].copy_from_slice(&rng.next_u64().to_le_bytes());
    ObjectId(b)
}

pub fn client_id(n: u8) -> ClientId {
    ClientId([n; 16])
}

/// A position near the level-0 origin, on a level from -3 to 3, or now and then 100 levels down.
pub fn position(rng: &mut SplitMix64) -> Position {
    let level = if rng.below(10) == 0 {
        100
    } else {
        rng.below(7) as i32 - 3
    };
    let near = Position {
        level: Level(0),
        x: Axis::from_i128((rng.next_u64() >> 4) as i128 - (1 << 59)),
        y: Axis::from_i128((rng.next_u64() >> 4) as i128 - (1 << 59)),
    };
    if level >= 0 {
        let mut p = near.at_level(Level(level));
        p.x = p.x.add(&Axis::from_i128(rng.next_u64() as i128));
        p
    } else {
        near.at_coarser_floor(Level(level))
    }
}

pub fn xform(rng: &mut SplitMix64) -> Xform {
    let mut linear = IDENTITY_LINEAR;
    if rng.below(3) == 0 {
        linear[1] = rng.next_u64() as i32;
    }
    Xform {
        anchor: position(rng),
        linear,
    }
}

/// A pen stroke of `n` points: a random walk with small steps, as a hand makes.
pub fn stroke_points(rng: &mut SplitMix64, n: usize) -> PackedPoints {
    let (mut x, mut y) = (0i32, 0i32);
    let points: Vec<Point> = (0..n)
        .map(|_| {
            x += rng.below(4001) as i32 - 2000;
            y += rng.below(4001) as i32 - 2000;
            Point {
                x,
                y,
                pressure: rng.below(256) as u8,
            }
        })
        .collect();
    PackedPoints::pack(&points).expect("fewer points than the limit")
}

pub fn stroke_body(rng: &mut SplitMix64, n: usize) -> Body {
    Body::Stroke {
        brush: StrokeBrush {
            brush: rng.below(8) as u32,
            base_width: (1 + rng.below(20) as i32) << 16,
            style_ver: 1,
        },
        points: stroke_points(rng, n),
        paint: Paint {
            rgba: rng.next_u64() as u32,
        },
    }
}

pub struct Tally {
    pub accepted: usize,
    pub duplicates: usize,
    pub rejected: Vec<RejectCode>,
}

pub struct World {
    pub room: Room,
    pub rng: SplitMix64,
    /// Every accepted change, in the order the room accepted them.
    pub log: Vec<Applied>,
    /// Every id a client has tried to create, accepted or not.
    pub ids: Vec<ObjectId>,
    /// The ids among them that were meant to be groups.
    pub group_ids: Vec<ObjectId>,
    pub next_change: Vec<u64>,
    pub tally: Tally,
}

impl World {
    pub fn new(seed: u64, clients: usize) -> World {
        World {
            room: Room::new(CAP, seed ^ 0xabcd),
            rng: SplitMix64(seed),
            log: Vec::new(),
            ids: Vec::new(),
            group_ids: Vec::new(),
            next_change: vec![1; clients],
            tally: Tally {
                accepted: 0,
                duplicates: 0,
                rejected: Vec::new(),
            },
        }
    }

    fn pick_id(&mut self) -> ObjectId {
        if self.ids.is_empty() {
            return object_id(&mut self.rng);
        }
        self.ids[self.rng.below(self.ids.len() as u64) as usize]
    }

    fn pick_group(&mut self) -> ObjectId {
        if self.group_ids.is_empty() {
            return self.pick_id();
        }
        self.group_ids[self.rng.below(self.group_ids.len() as u64) as usize]
    }

    /// A parent: the top level, or a group (which may be deleted, or may never have been accepted), or now and
    /// then any object at all. The room must refuse the ones that cannot be a parent.
    fn pick_parent(&mut self) -> Option<ObjectId> {
        match self.rng.below(10) {
            0..5 => None,
            5..9 => Some(self.pick_group()),
            _ => Some(self.pick_id()),
        }
    }

    fn pick_at(&mut self, parent: Option<ObjectId>) -> At {
        match self.rng.below(4) {
            0 => At::Front,
            1 => At::Back,
            _ => {
                let keys: Vec<OrderKey> = self
                    .room
                    .children(parent)
                    .filter_map(|id| self.room.object(&id).map(|o| o.slot().order.clone()))
                    .collect();
                if keys.is_empty() {
                    return At::Between(None, None);
                }
                // A gap between two neighbours, or below the first, or above the last.
                let gap = self.rng.below(keys.len() as u64 + 1) as usize;
                At::Between(
                    gap.checked_sub(1).map(|i| keys[i].clone()),
                    keys.get(gap).cloned(),
                )
            }
        }
    }

    pub fn random_op(&mut self) -> Op {
        match self.rng.below(100) {
            0..35 => {
                let id = object_id(&mut self.rng);
                self.ids.push(id);
                let parent = self.pick_parent();
                let n = 2 + self.rng.below(30) as usize;
                Op::Create(NewObject {
                    id,
                    parent,
                    at: self.pick_at(parent),
                    xform: xform(&mut self.rng),
                    name: None,
                    body: stroke_body(&mut self.rng, n),
                })
            }
            35..45 => {
                let id = object_id(&mut self.rng);
                self.ids.push(id);
                self.group_ids.push(id);
                let parent = self.pick_parent();
                Op::Create(NewObject {
                    id,
                    parent,
                    at: self.pick_at(parent),
                    xform: xform(&mut self.rng),
                    name: Some("group".into()),
                    body: Body::Group { clip: false },
                })
            }
            45..80 => {
                let id = self.pick_id();
                let value = match self.rng.below(6) {
                    0 => Value::Alive(self.rng.below(3) != 0),
                    1 | 2 => Value::Xform(xform(&mut self.rng)),
                    3 => Value::Name(if self.rng.below(2) == 0 {
                        None
                    } else {
                        Some(format!("n{}", self.rng.below(1000)).into())
                    }),
                    4 => Value::Paint(Paint {
                        rgba: self.rng.next_u64() as u32,
                    }),
                    _ => Value::Clip(self.rng.below(2) == 0),
                };
                // Now and then a conditional write, as undo makes: on the stamp the room has, or on a stale one.
                let if_stamp = match self.rng.below(10) {
                    0 => self.room.object(&id).map(|o| o.stamp(value.field())),
                    1 => Some(Seq(1)),
                    _ => None,
                };
                Op::Set {
                    id,
                    value,
                    if_stamp,
                }
            }
            _ => {
                // Half the moves move a group, which is what can close a loop.
                let id = if self.rng.below(2) == 0 {
                    self.pick_group()
                } else {
                    self.pick_id()
                };
                // Now and then the move aims below the object itself: under one of its children, or under itself.
                let parent = if self.rng.below(5) == 0 {
                    Some(self.room.children(Some(id)).next().unwrap_or(id))
                } else {
                    self.pick_parent()
                };
                Op::SetSlot {
                    id,
                    parent,
                    at: self.pick_at(parent),
                    if_stamp: None,
                }
            }
        }
    }

    pub fn random_change(&mut self, client: usize) -> Change {
        let n = if self.rng.below(4) == 0 {
            2 + self.rng.below(3) as usize
        } else {
            1
        };
        let ops = (0..n).map(|_| self.random_op()).collect();
        let id = ClientChangeId(self.next_change[client]);
        self.next_change[client] += 1;
        Change {
            id,
            observed: self.room.head(),
            ops,
        }
    }

    pub fn submit(&mut self, client: usize, change: &Change) -> Outcome {
        let outcome = self
            .room
            .submit(client_id(client as u8), UserId(client as u64), change);
        match &outcome {
            Outcome::Accepted(applied) => {
                self.tally.accepted += 1;
                self.log.push(applied.clone());
            }
            Outcome::Duplicate { .. } => self.tally.duplicates += 1,
            Outcome::Rejected(code) => self.tally.rejected.push(*code),
        }
        outcome
    }

    /// Runs `n` random changes from random clients.
    pub fn run(&mut self, n: usize) {
        for _ in 0..n {
            let client = self.rng.below(self.next_change.len() as u64) as usize;
            let change = self.random_change(client);
            self.submit(client, &change);
        }
    }

    pub fn groups(&self) -> Vec<ObjectId> {
        self.room
            .objects()
            .filter(|o| o.kind() == Kind::Group)
            .map(|o| o.id())
            .collect()
    }
}

pub fn shuffle<T>(rng: &mut SplitMix64, v: &mut [T]) {
    for i in (1..v.len()).rev() {
        v.swap(i, rng.below(i as u64 + 1) as usize);
    }
}
