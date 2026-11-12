//! The identifiers of the document model (design section 02, "Identifiers").

/// An object's id: 128 random bits chosen by the client that creates it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ObjectId(pub [u8; 16]);

/// The number the server gives an accepted change. One counter per surface, starting at 1.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Seq(pub u64);

/// The number the room gives an object when it accepts its creation. Increasing, never reused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Ordinal(pub u32);

/// One device's copy of one surface.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ClientId(pub [u8; 16]);

/// A counter per client device. With the `ClientId` it names a change, so a change sent twice is applied once.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ClientChangeId(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct UserId(pub u64);
