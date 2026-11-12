//! An object: its fixed part, its registers and their stamps, and its canonical bytes.
//!
//! The prototype has two kinds, a stroke and a group, and six registers. A register's stamp is the `Seq` of the
//! change that last wrote it. Most registers are never written after the object is made, so an object stores a
//! stamp only for a register written later; every other register has the stamp `created`.

use everink_core::{DepthCap, Position};

use crate::bytes::{DecodeError, Reader, Writer};
use crate::ids::{ObjectId, Ordinal, Seq, UserId};
use crate::order::{MAX_ORDER_KEY_BYTES, OrderKey};
use crate::points::{MAX_STROKE_POINTS, PackedPoints};

pub const MAX_NAME_BYTES: usize = 256;
/// The identity, in 16.16 fixed point: `[a, b, c, d]` of the matrix `[[a, b], [c, d]]`.
pub const IDENTITY_LINEAR: [i32; 4] = [1 << 16, 0, 0, 1 << 16];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Stroke,
    Group,
}

/// The registers. The number is the field's byte in the canonical form.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FieldId {
    Alive = 0,
    Slot = 1,
    Xform = 2,
    Name = 3,
    Paint = 4,
    Clip = 5,
}

impl FieldId {
    pub const ALL: [FieldId; 6] = [
        FieldId::Alive,
        FieldId::Slot,
        FieldId::Xform,
        FieldId::Name,
        FieldId::Paint,
        FieldId::Clip,
    ];

    fn from_u8(b: u8) -> Option<FieldId> {
        FieldId::ALL.get(b as usize).copied()
    }

    /// Whether an object of this kind has the register.
    pub fn valid_for(self, kind: Kind) -> bool {
        match self {
            FieldId::Paint => kind == Kind::Stroke,
            FieldId::Clip => kind == Kind::Group,
            _ => true,
        }
    }
}

/// Where an object hangs in the tree and where it sits among its siblings. One register: a move changes both.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Slot {
    pub parent: Option<ObjectId>,
    pub order: OrderKey,
}

/// Where an object is and how it is turned and scaled. One register, as in the design, so a move and a rotation
/// made at the same time by two people leave one of the two.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Xform {
    pub anchor: Position,
    pub linear: [i32; 4],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Paint {
    pub rgba: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StrokeBrush {
    pub brush: u32,
    /// Width in 16.16 fixed point.
    pub base_width: i32,
    pub style_ver: u8,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Body {
    Stroke {
        brush: StrokeBrush,
        points: PackedPoints,
        paint: Paint,
    },
    Group {
        clip: bool,
    },
}

impl Body {
    pub fn kind(&self) -> Kind {
        match self {
            Body::Stroke { .. } => Kind::Stroke,
            Body::Group { .. } => Kind::Group,
        }
    }
}

/// A value for one register. The variant says which register.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Value {
    Alive(bool),
    Slot(Slot),
    Xform(Xform),
    Name(Option<Box<str>>),
    Paint(Paint),
    Clip(bool),
}

impl Value {
    pub fn field(&self) -> FieldId {
        match self {
            Value::Alive(_) => FieldId::Alive,
            Value::Slot(_) => FieldId::Slot,
            Value::Xform(_) => FieldId::Xform,
            Value::Name(_) => FieldId::Name,
            Value::Paint(_) => FieldId::Paint,
            Value::Clip(_) => FieldId::Clip,
        }
    }

    /// Checks the limits a value from outside must keep.
    pub fn check(&self, cap: &DepthCap) -> Result<(), &'static str> {
        match self {
            Value::Xform(x) => check_anchor(&x.anchor, cap),
            Value::Name(Some(n)) if n.len() > MAX_NAME_BYTES => Err("a name longer than its limit"),
            _ => Ok(()),
        }
    }
}

pub(crate) fn check_anchor(anchor: &Position, cap: &DepthCap) -> Result<(), &'static str> {
    everink_core::wire::check_cap_bytes(&anchor.encode(), cap)
        .map_err(|_| "a position beyond the depth cap")
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Object {
    pub(crate) id: ObjectId,
    pub(crate) ordinal: Ordinal,
    pub(crate) creator: UserId,
    pub(crate) created: Seq,
    pub(crate) alive: bool,
    pub(crate) slot: Slot,
    pub(crate) xform: Xform,
    pub(crate) name: Option<Box<str>>,
    pub(crate) body: Body,
    /// Stamps of the registers written after creation, ascending by field. Each is above `created`.
    pub(crate) stamps: Box<[(FieldId, Seq)]>,
}

impl Object {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        id: ObjectId,
        ordinal: Ordinal,
        creator: UserId,
        created: Seq,
        slot: Slot,
        xform: Xform,
        name: Option<Box<str>>,
        body: Body,
    ) -> Object {
        Object {
            id,
            ordinal,
            creator,
            created,
            alive: true,
            slot,
            xform,
            name,
            body,
            stamps: Box::new([]),
        }
    }

    pub fn id(&self) -> ObjectId {
        self.id
    }

    pub fn ordinal(&self) -> Ordinal {
        self.ordinal
    }

    pub fn creator(&self) -> UserId {
        self.creator
    }

    pub fn created(&self) -> Seq {
        self.created
    }

    pub fn kind(&self) -> Kind {
        self.body.kind()
    }

    /// False for a deleted object. A deleted object is kept whole, so a restore brings everything back.
    pub fn alive(&self) -> bool {
        self.alive
    }

    pub fn slot(&self) -> &Slot {
        &self.slot
    }

    pub fn xform(&self) -> &Xform {
        &self.xform
    }

    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    pub fn body(&self) -> &Body {
        &self.body
    }

    /// The stamp of a register: the `Seq` of the change that last wrote it.
    pub fn stamp(&self, field: FieldId) -> Seq {
        self.stamps
            .iter()
            .find(|(f, _)| *f == field)
            .map_or(self.created, |(_, s)| *s)
    }

    /// The registers written after creation, with their stamps.
    pub fn later_stamps(&self) -> &[(FieldId, Seq)] {
        &self.stamps
    }

    /// The newest stamp on the object: the `Seq` of the last change that touched it.
    pub fn rev(&self) -> Seq {
        self.stamps
            .iter()
            .map(|(_, s)| *s)
            .max()
            .unwrap_or(self.created)
    }

    /// The value of a register, or `None` when this kind of object does not have it.
    pub fn value(&self, field: FieldId) -> Option<Value> {
        Some(match (field, &self.body) {
            (FieldId::Alive, _) => Value::Alive(self.alive),
            (FieldId::Slot, _) => Value::Slot(self.slot.clone()),
            (FieldId::Xform, _) => Value::Xform(self.xform.clone()),
            (FieldId::Name, _) => Value::Name(self.name.clone()),
            (FieldId::Paint, Body::Stroke { paint, .. }) => Value::Paint(*paint),
            (FieldId::Clip, Body::Group { clip }) => Value::Clip(*clip),
            _ => return None,
        })
    }

    /// The whole merge rule. The write takes effect when its stamp is above the register's stamp, and is ignored
    /// otherwise. Returns whether it took effect, or `None` when this kind of object does not have the register.
    pub fn write(&mut self, value: &Value, stamp: Seq) -> Option<bool> {
        let field = value.field();
        if !field.valid_for(self.kind()) {
            return None;
        }
        if stamp <= self.stamp(field) {
            return Some(false);
        }
        self.put(value);
        self.set_stamp(field, stamp);
        Some(true)
    }

    /// Stores a value without looking at stamps. The caller has checked that the object has the register.
    pub(crate) fn put(&mut self, value: &Value) {
        match (value, &mut self.body) {
            (Value::Alive(v), _) => self.alive = *v,
            (Value::Slot(v), _) => self.slot = v.clone(),
            (Value::Xform(v), _) => self.xform = v.clone(),
            (Value::Name(v), _) => self.name = v.clone(),
            (Value::Paint(v), Body::Stroke { paint, .. }) => *paint = *v,
            (Value::Clip(v), Body::Group { clip }) => *clip = *v,
            _ => unreachable!("the register was checked against the kind"),
        }
    }

    fn set_stamp(&mut self, field: FieldId, stamp: Seq) {
        debug_assert!(stamp > self.created);
        let mut v = std::mem::take(&mut self.stamps).into_vec();
        match v.binary_search_by_key(&field, |(f, _)| *f) {
            Ok(i) => v[i].1 = stamp,
            Err(i) => v.insert(i, (field, stamp)),
        }
        self.stamps = v.into_boxed_slice();
    }

    /// Merges another copy of the same object into this one, register by register: the higher stamp wins. The
    /// order in which copies are merged does not matter, and merging a copy twice changes nothing. Returns false,
    /// and changes nothing, when `other` is not the same object (its fixed part differs).
    pub fn merge(&mut self, other: &Object) -> bool {
        let same = self.id == other.id
            && self.ordinal == other.ordinal
            && self.creator == other.creator
            && self.created == other.created
            && self.kind() == other.kind()
            && self.fixed_body_eq(other);
        if !same {
            return false;
        }
        for &(field, stamp) in other.stamps.iter() {
            if let Some(value) = other.value(field) {
                self.write(&value, stamp);
            }
        }
        true
    }

    fn fixed_body_eq(&self, other: &Object) -> bool {
        match (&self.body, &other.body) {
            (
                Body::Stroke { brush, points, .. },
                Body::Stroke {
                    brush: b2,
                    points: p2,
                    ..
                },
            ) => brush == b2 && points == p2,
            (Body::Group { .. }, Body::Group { .. }) => true,
            _ => false,
        }
    }

    /// Bytes this object holds outside its own struct: points, a long name, long numbers, later stamps.
    pub fn heap_bytes(&self) -> usize {
        let axis = |a: &everink_core::Axis| {
            if a.is_small() {
                0
            } else {
                a.to_signed_bytes_be().len()
            }
        };
        let points = match &self.body {
            // The bytes and the two counters of the shared pointer.
            Body::Stroke { points, .. } => points.bytes().len() + 16,
            Body::Group { .. } => 0,
        };
        points
            + self.name.as_ref().map_or(0, |n| n.len())
            + self.slot.order.heap_bytes()
            + axis(&self.xform.anchor.x)
            + axis(&self.xform.anchor.y)
            + std::mem::size_of_val(&*self.stamps)
    }

    // -----------------------------------------------------------------------------------------------------------
    // Canonical bytes
    //
    //   object := id[16] varint(ordinal) varint(creator) varint(created) kind:u8
    //             alive:u8  has_parent:u8 [parent[16]]  blob(order key)
    //             blob(anchor position)  linear[4 x i32 le]
    //             has_name:u8 [blob(name)]
    //             body
    //             varint(n) { field:u8 varint(stamp - created) } x n      ascending by field, each above zero
    //   stroke body := varint(brush) i32le(base width) style_ver:u8 varint(count) blob(points) u32le(paint)
    //   group body  := clip:u8
    // -----------------------------------------------------------------------------------------------------------

    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        self.encode_into(&mut w);
        w.out
    }

    pub(crate) fn encode_into(&self, w: &mut Writer) {
        w.raw(&self.id.0);
        w.varint(u64::from(self.ordinal.0));
        w.varint(self.creator.0);
        w.varint(self.created.0);
        w.u8(match self.kind() {
            Kind::Stroke => 0,
            Kind::Group => 1,
        });
        w.bool(self.alive);
        match &self.slot.parent {
            None => w.bool(false),
            Some(p) => {
                w.bool(true);
                w.raw(&p.0);
            }
        }
        w.blob(self.slot.order.as_bytes());
        w.blob(&self.xform.anchor.encode());
        for v in self.xform.linear {
            w.i32(v);
        }
        match &self.name {
            None => w.bool(false),
            Some(n) => {
                w.bool(true);
                w.blob(n.as_bytes());
            }
        }
        match &self.body {
            Body::Stroke {
                brush,
                points,
                paint,
            } => {
                w.varint(u64::from(brush.brush));
                w.i32(brush.base_width);
                w.u8(brush.style_ver);
                w.varint(u64::from(points.count()));
                w.blob(points.bytes());
                w.u32(paint.rgba);
            }
            Body::Group { clip } => w.bool(*clip),
        }
        w.varint(self.stamps.len() as u64);
        for (field, stamp) in self.stamps.iter() {
            w.u8(*field as u8);
            w.varint(stamp.0 - self.created.0);
        }
    }

    /// Decodes exactly one object; anything after it is an error.
    pub fn decode(bytes: &[u8], cap: &DepthCap) -> Result<Object, DecodeError> {
        let mut r = Reader::new(bytes);
        let o = Object::decode_from(&mut r, cap)?;
        r.finish()?;
        Ok(o)
    }

    pub(crate) fn decode_from(r: &mut Reader<'_>, cap: &DepthCap) -> Result<Object, DecodeError> {
        let id = ObjectId(r.array()?);
        let ordinal = Ordinal(r.varint_u32()?);
        let creator = UserId(r.varint()?);
        let created = Seq(r.varint()?);
        let kind = match r.u8()? {
            0 => Kind::Stroke,
            1 => Kind::Group,
            _ => return Err(DecodeError::Invalid("an unknown kind")),
        };
        let alive = r.bool()?;
        let parent = if r.bool()? {
            Some(ObjectId(r.array()?))
        } else {
            None
        };
        let order = OrderKey::new(r.blob(MAX_ORDER_KEY_BYTES)?)
            .map_err(|_| DecodeError::Invalid("an order key that is not valid"))?;
        // The longest position the default cap allows is 3,094 bytes; the cap itself is checked by the decoder.
        let anchor = Position::decode(r.blob(1 << 16)?, cap).map_err(DecodeError::Position)?;
        let linear = [r.i32()?, r.i32()?, r.i32()?, r.i32()?];
        let name = if r.bool()? {
            let s = std::str::from_utf8(r.blob(MAX_NAME_BYTES)?)
                .map_err(|_| DecodeError::Invalid("a name that is not UTF-8"))?;
            Some(Box::<str>::from(s))
        } else {
            None
        };
        let body = match kind {
            Kind::Stroke => {
                let brush = StrokeBrush {
                    brush: r.varint_u32()?,
                    base_width: r.i32()?,
                    style_ver: r.u8()?,
                };
                let count = r.varint_u32()?;
                let bytes = r.blob(MAX_STROKE_POINTS as usize * 11)?;
                let points = PackedPoints::from_bytes(count, bytes)
                    .map_err(|_| DecodeError::Invalid("stroke points that do not decode"))?;
                Body::Stroke {
                    brush,
                    points,
                    paint: Paint { rgba: r.u32()? },
                }
            }
            Kind::Group => Body::Group { clip: r.bool()? },
        };
        let n = r.varint()?;
        if n > FieldId::ALL.len() as u64 {
            return Err(DecodeError::Invalid("more stamps than registers"));
        }
        let mut stamps = Vec::with_capacity(n as usize);
        let mut last: Option<FieldId> = None;
        for _ in 0..n {
            let field = FieldId::from_u8(r.u8()?)
                .filter(|f| f.valid_for(kind))
                .ok_or(DecodeError::Invalid(
                    "a stamp for a register the object does not have",
                ))?;
            if last.is_some_and(|l| l >= field) {
                return Err(DecodeError::Invalid("stamps out of order"));
            }
            last = Some(field);
            let above = r.varint()?;
            if above == 0 {
                return Err(DecodeError::Invalid("a later stamp that is not later"));
            }
            let stamp = created
                .0
                .checked_add(above)
                .ok_or(DecodeError::Invalid("a stamp beyond 64 bits"))?;
            stamps.push((field, Seq(stamp)));
        }
        Ok(Object {
            id,
            ordinal,
            creator,
            created,
            alive,
            slot: Slot { parent, order },
            xform: Xform { anchor, linear },
            name,
            body,
            stamps: stamps.into_boxed_slice(),
        })
    }
}
