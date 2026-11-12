//! A client's copy of a surface.
//!
//! The copy may be partial: it holds the objects the server has sent it. It has two layers.
//!
//! - **Confirmed** state is what the server has said. It is merged register by register, the higher stamp
//!   winning, so messages may arrive in any order, more than once, or as whole-object snapshots that overlap the
//!   stream of changes.
//! - **Pending** changes are the client's own, not yet answered. `shown` lays them over the confirmed state, so
//!   a person sees their own change at once and a remote write to the same register does not flicker through
//!   underneath. When the server's answer arrives the pending change is dropped: accepted, its effects are in
//!   the confirmed state with their real stamps; rejected, the confirmed state shows through again.

use std::collections::HashMap;

use crate::change::{Applied, Change, Effect, Op};
use crate::ids::{ClientChangeId, ClientId, ObjectId, Ordinal, Seq, UserId};
use crate::object::{Object, Slot, Value};
use crate::order::OrderKey;
use crate::room::encode_objects;

pub struct Replica {
    client: ClientId,
    user: UserId,
    objects: HashMap<ObjectId, Object>,
    pending: Vec<Change>,
    /// The newest `Seq` seen in an applied change.
    head: Seq,
}

impl Replica {
    pub fn new(client: ClientId, user: UserId) -> Replica {
        Replica {
            client,
            user,
            objects: HashMap::new(),
            pending: Vec::new(),
            head: Seq(0),
        }
    }

    pub fn client(&self) -> ClientId {
        self.client
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

    /// The confirmed state of an object.
    pub fn object(&self, id: &ObjectId) -> Option<&Object> {
        self.objects.get(id)
    }

    /// Merges an accepted change. A write to an object this copy does not hold is dropped: the copy is partial,
    /// and when the server later sends that object it sends it whole, with the write in it.
    pub fn apply(&mut self, applied: &Applied) {
        for effect in &applied.effects {
            match effect {
                Effect::Create(object) => self.enter(object),
                Effect::Write { id, value } => {
                    if let Some(object) = self.objects.get_mut(id) {
                        object.write(value, applied.seq);
                    }
                }
            }
        }
        self.head = self.head.max(applied.seq);
        if applied.client == self.client {
            self.pending.retain(|c| c.id != applied.change);
        }
    }

    /// Merges a whole object the server sent (it came into view, or a snapshot).
    pub fn enter(&mut self, object: &Object) {
        match self.objects.get_mut(&object.id()) {
            Some(mine) => {
                mine.merge(object);
            }
            None => {
                self.objects.insert(object.id(), object.clone());
            }
        }
    }

    /// Drops an object from the copy (it left the view).
    pub fn exit(&mut self, id: &ObjectId) {
        self.objects.remove(id);
    }

    /// Records a change this client made and has sent, or will send, to the server.
    pub fn local(&mut self, change: Change) {
        self.pending.push(change);
    }

    /// The server refused a change: it is dropped and the confirmed state shows again.
    pub fn rejected(&mut self, change: ClientChangeId) {
        self.pending.retain(|c| c.id != change);
    }

    pub fn pending(&self) -> &[Change] {
        &self.pending
    }

    /// The object as the person sees it: the confirmed state with this client's pending changes laid over it.
    ///
    /// For a pending move, the parent is shown and the order key is a guess (the old key, or a middle key for a
    /// new object) until the server answers. For a pending new object, the ordinal and `Seq` are placeholders.
    pub fn shown(&self, id: &ObjectId) -> Option<Object> {
        let mut object = self.objects.get(id).cloned();
        for change in &self.pending {
            for op in &change.ops {
                match op {
                    Op::Create(new) if new.id == *id && object.is_none() => {
                        object = Some(Object::new(
                            new.id,
                            Ordinal(u32::MAX),
                            self.user,
                            Seq(u64::MAX),
                            Slot {
                                parent: new.parent,
                                order: OrderKey::new(&[0x80]).expect("a valid key"),
                            },
                            new.xform.clone(),
                            new.name.clone(),
                            new.body.clone(),
                        ));
                    }
                    Op::Set {
                        id: target, value, ..
                    } if target == id => {
                        if let Some(o) = &mut object
                            && value.field().valid_for(o.kind())
                        {
                            o.put(value);
                        }
                    }
                    Op::SetSlot {
                        id: target, parent, ..
                    } if target == id => {
                        if let Some(o) = &mut object {
                            let order = o.slot().order.clone();
                            o.put(&Value::Slot(Slot {
                                parent: *parent,
                                order,
                            }));
                        }
                    }
                    _ => {}
                }
            }
        }
        object
    }

    /// The confirmed objects, in id order. Equal bytes mean equal objects.
    pub fn encode_objects(&self) -> Vec<u8> {
        encode_objects(self.objects.values())
    }
}
