//! The server's copy of one surface.
//!
//! `submit` is the only way in. It checks a whole change against the full state, and either refuses it, leaving
//! the room exactly as it was, or accepts it, gives it the next `Seq`, and returns what it did. The server never
//! rewrites an operation; the only things it adds are the ones a client cannot know: the `Seq`, a new object's
//! ordinal, and the order key for "front", "back" and "between".

use std::collections::{BTreeSet, HashMap, HashSet};

use everink_core::DepthCap;

use crate::bytes::{DecodeError, Reader, Writer};
use crate::change::{Applied, At, Change, Effect, MAX_OPS_PER_CHANGE, NewObject, Op};
use crate::ids::{ClientChangeId, ClientId, ObjectId, Ordinal, Seq, UserId};
use crate::object::{FieldId, Kind, Object, Slot, Value, check_anchor};
use crate::order::OrderKey;
use crate::rng::SplitMix64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RejectCode {
    /// A change with no operations.
    Empty,
    TooManyOps,
    /// The change says it saw a `Seq` the room has not reached.
    ObservedAhead,
    /// Two operations of one change write the same register.
    DuplicateRegister,
    /// A `Create` for an id that already exists.
    IdExists,
    UnknownObject,
    /// A value that breaks a limit or does not fit the object; the text says which.
    Invalid(&'static str),
    /// The parent is unknown, is not a group, or is deleted.
    BadParent,
    /// The move would make an object its own ancestor.
    Cycle,
    /// An `if_stamp` condition did not hold: the register was written since.
    Conflict,
    /// `At::Between(a, b)` with `a` not below `b`.
    BadOrder,
    /// No order key of the allowed length fits at that place.
    OrderKeyFull,
    /// The room has used every `Seq` or every ordinal. Not reachable by counting; a state from outside can
    /// claim to be there.
    RoomFull,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    Accepted(Applied),
    /// The client has sent this change, or a later one, before. The pair is the newest change of that client the
    /// room accepted. Nothing was applied.
    Duplicate {
        highest: ClientChangeId,
        seq: Seq,
    },
    /// Nothing was applied.
    Rejected(RejectCode),
}

type Siblings = BTreeSet<(OrderKey, ObjectId)>;

pub struct Room {
    objects: HashMap<ObjectId, Object>,
    /// For each parent (`None` is the top level), its children in order. Built from the objects' slots.
    children: HashMap<Option<ObjectId>, Siblings>,
    head: Seq,
    next_ordinal: u32,
    /// The newest accepted change of each client, so a change sent twice is applied once.
    dedupe: HashMap<ClientId, (ClientChangeId, Seq)>,
    rng: SplitMix64,
    cap: DepthCap,
}

/// What a change would do, worked out without touching the room.
struct Plan {
    touched: HashMap<ObjectId, Object>,
    effects: Vec<Effect>,
    created: u32,
    rng: SplitMix64,
}

impl Room {
    pub fn new(cap: DepthCap, seed: u64) -> Room {
        Room {
            objects: HashMap::new(),
            children: HashMap::new(),
            head: Seq(0),
            next_ordinal: 0,
            dedupe: HashMap::new(),
            rng: SplitMix64(seed),
            cap,
        }
    }

    pub fn head(&self) -> Seq {
        self.head
    }

    pub fn len(&self) -> usize {
        self.objects.len()
    }

    pub fn is_empty(&self) -> bool {
        self.objects.is_empty()
    }

    pub fn object(&self, id: &ObjectId) -> Option<&Object> {
        self.objects.get(id)
    }

    pub fn objects(&self) -> impl Iterator<Item = &Object> {
        self.objects.values()
    }

    /// The children of a parent (`None` is the top level), from back to front.
    pub fn children(&self, parent: Option<ObjectId>) -> impl Iterator<Item = ObjectId> + '_ {
        self.children
            .get(&parent)
            .into_iter()
            .flat_map(|s| s.iter().map(|(_, id)| *id))
    }

    pub fn submit(&mut self, client: ClientId, user: UserId, change: &Change) -> Outcome {
        if let Some(&(highest, seq)) = self.dedupe.get(&client)
            && change.id <= highest
        {
            return Outcome::Duplicate { highest, seq };
        }
        let Some(next) = self.head.0.checked_add(1) else {
            return Outcome::Rejected(RejectCode::RoomFull);
        };
        let seq = Seq(next);
        let plan = match self.plan(user, seq, change) {
            Ok(plan) => plan,
            Err(code) => return Outcome::Rejected(code),
        };
        for (id, object) in plan.touched {
            self.store(id, object);
        }
        self.head = seq;
        self.next_ordinal += plan.created;
        self.rng = plan.rng;
        self.dedupe.insert(client, (change.id, seq));
        Outcome::Accepted(Applied {
            seq,
            client,
            change: change.id,
            effects: plan.effects,
        })
    }

    /// Puts an object in the map and keeps the children index in step.
    fn store(&mut self, id: ObjectId, object: Object) {
        let new_slot = object.slot.clone();
        let old_slot = self.objects.insert(id, object).map(|o| o.slot);
        if old_slot.as_ref() == Some(&new_slot) {
            return;
        }
        if let Some(old) = old_slot
            && let Some(set) = self.children.get_mut(&old.parent)
        {
            set.remove(&(old.order, id));
            if set.is_empty() {
                self.children.remove(&old.parent);
            }
        }
        self.children
            .entry(new_slot.parent)
            .or_default()
            .insert((new_slot.order, id));
    }

    fn plan(&self, user: UserId, seq: Seq, change: &Change) -> Result<Plan, RejectCode> {
        if change.ops.is_empty() {
            return Err(RejectCode::Empty);
        }
        if change.ops.len() > MAX_OPS_PER_CHANGE {
            return Err(RejectCode::TooManyOps);
        }
        if change.observed > self.head {
            return Err(RejectCode::ObservedAhead);
        }
        let mut plan = Plan {
            touched: HashMap::new(),
            effects: Vec::with_capacity(change.ops.len()),
            created: 0,
            rng: self.rng.clone(),
        };
        let mut written: HashSet<(ObjectId, FieldId)> = HashSet::new();
        for op in &change.ops {
            match op {
                Op::Create(new) => {
                    if self.get(&plan, &new.id).is_some() {
                        return Err(RejectCode::IdExists);
                    }
                    self.check_new(new)?;
                    // The ordinal this object gets, and the one after it, must both exist.
                    let ordinal = self
                        .next_ordinal
                        .checked_add(plan.created)
                        .filter(|o| *o < u32::MAX)
                        .ok_or(RejectCode::RoomFull)?;
                    let order = self.resolve(&mut plan, new.parent, &new.at, new.id)?;
                    // A new object's registers all carry this change's number, so no later operation of the
                    // same change may write one.
                    for field in FieldId::ALL {
                        written.insert((new.id, field));
                    }
                    let object = Object::new(
                        new.id,
                        Ordinal(ordinal),
                        user,
                        seq,
                        Slot {
                            parent: new.parent,
                            order,
                        },
                        new.xform.clone(),
                        new.name.clone(),
                        new.body.clone(),
                    );
                    plan.created += 1;
                    plan.effects.push(Effect::Create(object.clone()));
                    plan.touched.insert(new.id, object);
                }
                Op::Set {
                    id,
                    value,
                    if_stamp,
                } => {
                    let field = value.field();
                    if field == FieldId::Slot {
                        return Err(RejectCode::Invalid("the slot is written with SetSlot"));
                    }
                    let mut object = self
                        .get(&plan, id)
                        .ok_or(RejectCode::UnknownObject)?
                        .clone();
                    if !field.valid_for(object.kind()) {
                        return Err(RejectCode::Invalid(
                            "the object does not have that register",
                        ));
                    }
                    value.check(&self.cap).map_err(RejectCode::Invalid)?;
                    if !written.insert((*id, field)) {
                        return Err(RejectCode::DuplicateRegister);
                    }
                    if if_stamp.is_some_and(|s| s != object.stamp(field)) {
                        return Err(RejectCode::Conflict);
                    }
                    object.write(value, seq);
                    plan.effects.push(Effect::Write {
                        id: *id,
                        value: value.clone(),
                    });
                    plan.touched.insert(*id, object);
                }
                Op::SetSlot {
                    id,
                    parent,
                    at,
                    if_stamp,
                } => {
                    let mut object = self
                        .get(&plan, id)
                        .ok_or(RejectCode::UnknownObject)?
                        .clone();
                    if !written.insert((*id, FieldId::Slot)) {
                        return Err(RejectCode::DuplicateRegister);
                    }
                    if if_stamp.is_some_and(|s| s != object.stamp(FieldId::Slot)) {
                        return Err(RejectCode::Conflict);
                    }
                    // Walk up from the new parent. Reaching the object itself would close a loop.
                    let mut up = *parent;
                    while let Some(p) = up {
                        if p == *id {
                            return Err(RejectCode::Cycle);
                        }
                        up = self.get(&plan, &p).and_then(|o| o.slot.parent);
                    }
                    let order = self.resolve(&mut plan, *parent, at, *id)?;
                    let value = Value::Slot(Slot {
                        parent: *parent,
                        order,
                    });
                    object.write(&value, seq);
                    plan.effects.push(Effect::Write { id: *id, value });
                    plan.touched.insert(*id, object);
                }
            }
        }
        Ok(plan)
    }

    /// The object as the change so far has left it.
    fn get<'a>(&'a self, plan: &'a Plan, id: &ObjectId) -> Option<&'a Object> {
        plan.touched.get(id).or_else(|| self.objects.get(id))
    }

    fn check_new(&self, new: &NewObject) -> Result<(), RejectCode> {
        check_anchor(&new.xform.anchor, &self.cap).map_err(RejectCode::Invalid)?;
        Value::Name(new.name.clone())
            .check(&self.cap)
            .map_err(RejectCode::Invalid)
    }

    /// Checks the parent and picks the order key.
    fn resolve(
        &self,
        plan: &mut Plan,
        parent: Option<ObjectId>,
        at: &At,
        moving: ObjectId,
    ) -> Result<OrderKey, RejectCode> {
        if let Some(p) = parent {
            let ok = self
                .get(plan, &p)
                .is_some_and(|o| o.kind() == Kind::Group && o.alive());
            if !ok {
                return Err(RejectCode::BadParent);
            }
        }
        let key = match at {
            At::Between(a, b) => OrderKey::between(a.as_ref(), b.as_ref(), &mut plan.rng),
            At::Front => {
                let top = self.edge(plan, parent, true, moving);
                OrderKey::after(top.as_ref(), &mut plan.rng)
            }
            At::Back => {
                let bottom = self.edge(plan, parent, false, moving);
                OrderKey::before(bottom.as_ref(), &mut plan.rng)
            }
        };
        key.map_err(|e| match e {
            crate::order::OrderError::NotAscending => RejectCode::BadOrder,
            _ => RejectCode::OrderKeyFull,
        })
    }

    /// The highest (`front`) or lowest key among a parent's children as the change so far has left them, not
    /// counting the object being placed.
    fn edge(
        &self,
        plan: &Plan,
        parent: Option<ObjectId>,
        front: bool,
        moving: ObjectId,
    ) -> Option<OrderKey> {
        let untouched =
            |(_, id): &&(OrderKey, ObjectId)| !plan.touched.contains_key(id) && *id != moving;
        let committed = self.children.get(&parent).and_then(|set| {
            if front {
                set.iter().rev().find(untouched)
            } else {
                set.iter().find(untouched)
            }
            .map(|(k, _)| k)
        });
        let in_change = plan
            .touched
            .values()
            .filter(|o| o.slot.parent == parent && o.id != moving)
            .map(|o| &o.slot.order);
        let all = committed.into_iter().chain(in_change);
        if front { all.max() } else { all.min() }.cloned()
    }

    // -----------------------------------------------------------------------------------------------------------
    // Canonical bytes of the whole state
    //
    //   state   := varint(head) varint(next ordinal) objects
    //   objects := varint(count) object x count        ascending by id
    // -----------------------------------------------------------------------------------------------------------

    /// The objects alone, in id order. Two copies hold the same objects exactly when these bytes are equal.
    pub fn encode_objects(&self) -> Vec<u8> {
        encode_objects(self.objects.values())
    }

    pub fn encode_all(&self) -> Vec<u8> {
        let mut w = Writer::new();
        w.varint(self.head.0);
        w.varint(u64::from(self.next_ordinal));
        write_objects(&mut w, self.objects.values());
        w.out
    }

    /// Opens a room from `encode_all` bytes: decodes and checks every object, then rebuilds the children index
    /// and checks the tree. The list of clients' newest changes is not part of these bytes.
    pub fn decode_all(bytes: &[u8], cap: DepthCap, seed: u64) -> Result<Room, DecodeError> {
        let mut r = Reader::new(bytes);
        let head = Seq(r.varint()?);
        let next_ordinal = r.varint_u32()?;
        let count = r.varint()?;
        let mut room = Room::new(cap, seed);
        room.head = head;
        room.next_ordinal = next_ordinal;
        let mut last: Option<ObjectId> = None;
        let mut ordinals: HashSet<u32> = HashSet::new();
        for _ in 0..count {
            let object = Object::decode_from(&mut r, &cap)?;
            if last.is_some_and(|l| l >= object.id) {
                return Err(DecodeError::Invalid("objects out of order"));
            }
            last = Some(object.id);
            if object.created.0 == 0 {
                return Err(DecodeError::Invalid(
                    "an object created before the first change",
                ));
            }
            if !ordinals.insert(object.ordinal.0) {
                return Err(DecodeError::Invalid("an ordinal used twice"));
            }
            if object.rev() > head || object.ordinal.0 >= next_ordinal {
                return Err(DecodeError::Invalid(
                    "an object newer than the state says it is",
                ));
            }
            room.store(object.id, object);
        }
        r.finish()?;
        room.check_tree()
            .map_err(|_| DecodeError::Invalid("a tree that is not a tree"))?;
        Ok(room)
    }

    /// Checks what must always hold: the children index matches the slots, every parent exists and is a group,
    /// and following parents from any object ends at the top level.
    pub fn check_tree(&self) -> Result<(), &'static str> {
        let indexed: usize = self.children.values().map(BTreeSet::len).sum();
        if indexed != self.objects.len() {
            return Err("the children index does not hold every object once");
        }
        // 0 = not yet known, 1 = on the path being walked, 2 = known to reach the top level.
        let mut state: HashMap<ObjectId, u8> = HashMap::with_capacity(self.objects.len());
        let mut path = Vec::new();
        for object in self.objects.values() {
            let in_index = self
                .children
                .get(&object.slot.parent)
                .is_some_and(|s| s.contains(&(object.slot.order.clone(), object.id)));
            if !in_index {
                return Err("an object is missing from its parent's children");
            }
            path.clear();
            let mut at = Some(object.id);
            while let Some(id) = at {
                match state.get(&id) {
                    Some(2) => break,
                    Some(_) => return Err("a cycle"),
                    None => {}
                }
                let Some(o) = self.objects.get(&id) else {
                    return Err("a parent that does not exist");
                };
                if id != object.id && o.kind() != Kind::Group {
                    return Err("a parent that is not a group");
                }
                state.insert(id, 1);
                path.push(id);
                at = o.slot.parent;
            }
            for id in &path {
                state.insert(*id, 2);
            }
        }
        Ok(())
    }

    /// An estimate of the bytes the room holds: the object map, what the objects hold outside it, and the
    /// children index. Returned as `(objects, index)`.
    pub fn memory_estimate(&self) -> (usize, usize) {
        // One slot per unit of capacity, plus one control byte each (how `HashMap` is laid out).
        let slot = std::mem::size_of::<(ObjectId, Object)>() + 1;
        let objects = self.objects.capacity() * slot
            + self.objects.values().map(Object::heap_bytes).sum::<usize>();
        // A B-tree node holds up to 11 entries; nodes are on average about two thirds full.
        let entry = std::mem::size_of::<(OrderKey, ObjectId)>();
        let index = self.children.capacity()
            * (std::mem::size_of::<(Option<ObjectId>, Siblings)>() + 1)
            + self
                .children
                .values()
                .map(|s| s.len() * entry * 3 / 2)
                .sum::<usize>();
        (objects, index)
    }
}

pub(crate) fn encode_objects<'a>(objects: impl Iterator<Item = &'a Object>) -> Vec<u8> {
    let mut w = Writer::new();
    write_objects(&mut w, objects);
    w.out
}

fn write_objects<'a>(w: &mut Writer, objects: impl Iterator<Item = &'a Object>) {
    let mut sorted: Vec<&Object> = objects.collect();
    sorted.sort_unstable_by_key(|o| o.id);
    w.varint(sorted.len() as u64);
    for object in sorted {
        object.encode_into(w);
    }
}
