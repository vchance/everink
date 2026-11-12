//! The bytes of a change on the wire: what a client sends (`Change`) and what the server sends back to every
//! copy (`Applied`). One value has one encoding, and the decoders refuse anything else, so bytes from outside
//! are checked before they become a value.
//!
//! ```text
//! change  := varint(id) varint(observed) varint(n) op x n                 n <= 1,000
//! op      := 0 id[16] parent at xform name kind:u8 body                    create
//!          | 1 id[16] value stamp                                          set one register
//!          | 2 id[16] parent at stamp                                      move
//! parent  := 0 | 1 id[16]
//! at      := 0 (front) | 1 (back) | 2 key key (between)       key := 0 | 1 blob
//! stamp   := 0 | 1 varint(seq)                                             the condition, if any
//! value   := field:u8 then the field's value
//! applied := varint(seq) client[16] varint(change) varint(n) effect x n
//! effect  := 0 blob(object) | 1 id[16] value
//! ```
//!
//! The transform, name and body are written as in an object's canonical bytes (`object.rs`).

use everink_core::{DepthCap, Position};

use crate::bytes::{DecodeError, Reader, Writer};
use crate::change::{Applied, At, Change, Effect, MAX_OPS_PER_CHANGE, NewObject, Op};
use crate::ids::{ClientChangeId, ClientId, ObjectId, Seq};
use crate::object::{
    Body, FieldId, MAX_NAME_BYTES, Object, Paint, Slot, StrokeBrush, Value, Xform,
};
use crate::order::{MAX_ORDER_KEY_BYTES, OrderKey};
use crate::points::{MAX_STROKE_POINTS, PackedPoints};

/// The most bytes one object may take inside an `Applied`. The largest stroke is under a quarter of this.
const MAX_OBJECT_BYTES: usize = 1 << 20;

pub fn encode_change(change: &Change) -> Vec<u8> {
    let mut w = Writer::new();
    w.varint(change.id.0);
    w.varint(change.observed.0);
    w.varint(change.ops.len() as u64);
    for op in &change.ops {
        match op {
            Op::Create(new) => {
                w.u8(0);
                w.raw(&new.id.0);
                put_parent(&mut w, &new.parent);
                put_at(&mut w, &new.at);
                put_xform(&mut w, &new.xform);
                put_name(&mut w, &new.name);
                put_body(&mut w, &new.body);
            }
            Op::Set {
                id,
                value,
                if_stamp,
            } => {
                w.u8(1);
                w.raw(&id.0);
                put_value(&mut w, value);
                put_stamp(&mut w, if_stamp);
            }
            Op::SetSlot {
                id,
                parent,
                at,
                if_stamp,
            } => {
                w.u8(2);
                w.raw(&id.0);
                put_parent(&mut w, parent);
                put_at(&mut w, at);
                put_stamp(&mut w, if_stamp);
            }
        }
    }
    w.out
}

/// Decodes exactly one change; anything after it is an error.
pub fn decode_change(bytes: &[u8], cap: &DepthCap) -> Result<Change, DecodeError> {
    let mut r = Reader::new(bytes);
    let id = ClientChangeId(r.varint()?);
    let observed = Seq(r.varint()?);
    let n = count(&mut r)?;
    // Each operation is at least 18 bytes, so no more are set aside for than the bytes left could hold.
    let mut ops = Vec::with_capacity(n.min(r.left() / 18));
    for _ in 0..n {
        ops.push(match r.u8()? {
            0 => {
                let id = ObjectId(r.array()?);
                let parent = get_parent(&mut r)?;
                let at = get_at(&mut r)?;
                let xform = get_xform(&mut r, cap)?;
                let name = get_name(&mut r)?;
                let body = get_body(&mut r)?;
                Op::Create(NewObject {
                    id,
                    parent,
                    at,
                    xform,
                    name,
                    body,
                })
            }
            1 => Op::Set {
                id: ObjectId(r.array()?),
                value: get_value(&mut r, cap)?,
                if_stamp: get_stamp(&mut r)?,
            },
            2 => Op::SetSlot {
                id: ObjectId(r.array()?),
                parent: get_parent(&mut r)?,
                at: get_at(&mut r)?,
                if_stamp: get_stamp(&mut r)?,
            },
            _ => return Err(DecodeError::Invalid("an unknown operation")),
        });
    }
    r.finish()?;
    Ok(Change { id, observed, ops })
}

pub fn encode_applied(applied: &Applied) -> Vec<u8> {
    let mut w = Writer::new();
    w.varint(applied.seq.0);
    w.raw(&applied.client.0);
    w.varint(applied.change.0);
    w.varint(applied.effects.len() as u64);
    for effect in &applied.effects {
        match effect {
            Effect::Create(object) => {
                w.u8(0);
                w.blob(&object.encode());
            }
            Effect::Write { id, value } => {
                w.u8(1);
                w.raw(&id.0);
                put_value(&mut w, value);
            }
        }
    }
    w.out
}

/// Decodes exactly one accepted change; anything after it is an error.
pub fn decode_applied(bytes: &[u8], cap: &DepthCap) -> Result<Applied, DecodeError> {
    let mut r = Reader::new(bytes);
    let seq = Seq(r.varint()?);
    let client = ClientId(r.array()?);
    let change = ClientChangeId(r.varint()?);
    let n = count(&mut r)?;
    // Each effect is at least 18 bytes.
    let mut effects = Vec::with_capacity(n.min(r.left() / 18));
    for _ in 0..n {
        effects.push(match r.u8()? {
            0 => Effect::Create(Object::decode(r.blob(MAX_OBJECT_BYTES)?, cap)?),
            1 => Effect::Write {
                id: ObjectId(r.array()?),
                value: get_value(&mut r, cap)?,
            },
            _ => return Err(DecodeError::Invalid("an unknown effect")),
        });
    }
    r.finish()?;
    Ok(Applied {
        seq,
        client,
        change,
        effects,
    })
}

/// A number of operations or effects, checked against the limit before anything is allocated for them.
fn count(r: &mut Reader<'_>) -> Result<usize, DecodeError> {
    let n = r.varint()?;
    if n > MAX_OPS_PER_CHANGE as u64 {
        return Err(DecodeError::Invalid(
            "more operations than a change may hold",
        ));
    }
    Ok(n as usize)
}

fn put_parent(w: &mut Writer, parent: &Option<ObjectId>) {
    match parent {
        None => w.bool(false),
        Some(p) => {
            w.bool(true);
            w.raw(&p.0);
        }
    }
}

fn get_parent(r: &mut Reader<'_>) -> Result<Option<ObjectId>, DecodeError> {
    Ok(if r.bool()? {
        Some(ObjectId(r.array()?))
    } else {
        None
    })
}

fn put_key(w: &mut Writer, key: &Option<OrderKey>) {
    match key {
        None => w.bool(false),
        Some(k) => {
            w.bool(true);
            w.blob(k.as_bytes());
        }
    }
}

fn get_key(r: &mut Reader<'_>) -> Result<Option<OrderKey>, DecodeError> {
    Ok(if r.bool()? { Some(order_key(r)?) } else { None })
}

fn order_key(r: &mut Reader<'_>) -> Result<OrderKey, DecodeError> {
    OrderKey::new(r.blob(MAX_ORDER_KEY_BYTES)?)
        .map_err(|_| DecodeError::Invalid("an order key that is not valid"))
}

fn put_at(w: &mut Writer, at: &At) {
    match at {
        At::Front => w.u8(0),
        At::Back => w.u8(1),
        At::Between(a, b) => {
            w.u8(2);
            put_key(w, a);
            put_key(w, b);
        }
    }
}

fn get_at(r: &mut Reader<'_>) -> Result<At, DecodeError> {
    Ok(match r.u8()? {
        0 => At::Front,
        1 => At::Back,
        2 => At::Between(get_key(r)?, get_key(r)?),
        _ => return Err(DecodeError::Invalid("an unknown place")),
    })
}

fn put_stamp(w: &mut Writer, stamp: &Option<Seq>) {
    match stamp {
        None => w.bool(false),
        Some(s) => {
            w.bool(true);
            w.varint(s.0);
        }
    }
}

fn get_stamp(r: &mut Reader<'_>) -> Result<Option<Seq>, DecodeError> {
    Ok(if r.bool()? {
        Some(Seq(r.varint()?))
    } else {
        None
    })
}

fn put_xform(w: &mut Writer, xform: &Xform) {
    w.blob(&xform.anchor.encode());
    for v in xform.linear {
        w.i32(v);
    }
}

fn get_xform(r: &mut Reader<'_>, cap: &DepthCap) -> Result<Xform, DecodeError> {
    let anchor = Position::decode(r.blob(1 << 16)?, cap).map_err(DecodeError::Position)?;
    Ok(Xform {
        anchor,
        linear: [r.i32()?, r.i32()?, r.i32()?, r.i32()?],
    })
}

fn put_name(w: &mut Writer, name: &Option<Box<str>>) {
    match name {
        None => w.bool(false),
        Some(n) => {
            w.bool(true);
            w.blob(n.as_bytes());
        }
    }
}

fn get_name(r: &mut Reader<'_>) -> Result<Option<Box<str>>, DecodeError> {
    Ok(if r.bool()? {
        let s = std::str::from_utf8(r.blob(MAX_NAME_BYTES)?)
            .map_err(|_| DecodeError::Invalid("a name that is not UTF-8"))?;
        Some(s.into())
    } else {
        None
    })
}

fn put_body(w: &mut Writer, body: &Body) {
    match body {
        Body::Stroke {
            brush,
            points,
            paint,
        } => {
            w.u8(0);
            w.varint(u64::from(brush.brush));
            w.i32(brush.base_width);
            w.u8(brush.style_ver);
            w.varint(u64::from(points.count()));
            w.blob(points.bytes());
            w.u32(paint.rgba);
        }
        Body::Group { clip } => {
            w.u8(1);
            w.bool(*clip);
        }
    }
}

fn get_body(r: &mut Reader<'_>) -> Result<Body, DecodeError> {
    Ok(match r.u8()? {
        0 => {
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
        1 => Body::Group { clip: r.bool()? },
        _ => return Err(DecodeError::Invalid("an unknown kind")),
    })
}

fn put_value(w: &mut Writer, value: &Value) {
    w.u8(value.field() as u8);
    match value {
        Value::Alive(v) | Value::Clip(v) => w.bool(*v),
        Value::Slot(slot) => {
            put_parent(w, &slot.parent);
            w.blob(slot.order.as_bytes());
        }
        Value::Xform(x) => put_xform(w, x),
        Value::Name(n) => put_name(w, n),
        Value::Paint(p) => w.u32(p.rgba),
    }
}

fn get_value(r: &mut Reader<'_>, cap: &DepthCap) -> Result<Value, DecodeError> {
    let field = FieldId::ALL
        .get(r.u8()? as usize)
        .copied()
        .ok_or(DecodeError::Invalid("an unknown register"))?;
    Ok(match field {
        FieldId::Alive => Value::Alive(r.bool()?),
        FieldId::Slot => Value::Slot(Slot {
            parent: get_parent(r)?,
            order: order_key(r)?,
        }),
        FieldId::Xform => Value::Xform(get_xform(r, cap)?),
        FieldId::Name => Value::Name(get_name(r)?),
        FieldId::Paint => Value::Paint(Paint { rgba: r.u32()? }),
        FieldId::Clip => Value::Clip(r.bool()?),
    })
}
