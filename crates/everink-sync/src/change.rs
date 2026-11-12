//! Changes: what a client asks for, and what the server says it did.

use crate::ids::{ClientChangeId, ClientId, ObjectId, Seq};
use crate::object::{Body, Object, Value, Xform};
use crate::order::OrderKey;

pub const MAX_OPS_PER_CHANGE: usize = 1_000;

/// Where to put an object among its siblings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum At {
    /// Between two keys the client can see. `None` is "below everything" or "above everything". The server
    /// picks the key.
    Between(Option<OrderKey>, Option<OrderKey>),
    /// Above every sibling. The server resolves it against the siblings it has, which the client may not all see.
    Front,
    /// Below every sibling.
    Back,
}

/// A new object as the client describes it. The server adds what only it knows: the ordinal, the creator, the
/// `Seq`, and the order key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewObject {
    pub id: ObjectId,
    pub parent: Option<ObjectId>,
    pub at: At,
    pub xform: Xform,
    pub name: Option<Box<str>>,
    pub body: Body,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Op {
    Create(NewObject),
    /// Writes one register other than the slot. With `if_stamp`, only if the register still has that stamp
    /// (used by undo); without it the write is unconditional.
    Set {
        id: ObjectId,
        value: Value,
        if_stamp: Option<Seq>,
    },
    /// Moves an object in the tree or among its siblings.
    SetSlot {
        id: ObjectId,
        parent: Option<ObjectId>,
        at: At,
        if_stamp: Option<Seq>,
    },
}

/// One atomic change: the server applies every operation or none.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Change {
    pub id: ClientChangeId,
    /// The newest `Seq` the client had seen when it made the change.
    pub observed: Seq,
    pub ops: Vec<Op>,
}

/// One thing an accepted change did, with every choice the server made filled in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effect {
    /// The object as it is right after creation.
    Create(Object),
    /// A register write. Its stamp is the `Seq` of the change.
    Write { id: ObjectId, value: Value },
}

/// An accepted change as the server sends it to every copy.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Applied {
    pub seq: Seq,
    /// Who made the change. A client recognises its own change by this pair.
    pub client: ClientId,
    pub change: ClientChangeId,
    pub effects: Vec<Effect>,
}
