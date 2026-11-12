//! Everink sync, spike slice 2a: the server-ordered object map.
//!
//! The design is `docs/design/02-document-model.md`. A surface is a map from object id to object. Every field
//! that can change is a register: a value and the number (`Seq`) of the change that wrote it. The server puts
//! every change in one order by giving it the next `Seq`; a write with a higher number replaces the value and a
//! lower one is ignored, so copies that hold the same writes hold the same state whatever order they arrived in.
//!
//! - [`room::Room`] is the server's copy: it checks a change, numbers it, and says what it did.
//! - [`replica::Replica`] is a client's copy: it merges what the server sends, in any order, and shows its own
//!   changes before the server has answered.
//!
//! What this prototype leaves out is listed in `docs/spike/results-2a-object-map.md`.

pub mod bytes;
pub mod change;
pub mod ids;
pub mod link_model;
pub mod object;
pub mod order;
pub mod pacing;
pub mod points;
pub mod replica;
pub mod rng;
pub mod room;
pub mod wire;

pub use bytes::DecodeError;
pub use change::{Applied, At, Change, Effect, NewObject, Op};
pub use ids::{ClientChangeId, ClientId, ObjectId, Ordinal, Seq, UserId};
pub use object::{Body, FieldId, Kind, Object, Paint, Slot, StrokeBrush, Value, Xform};
pub use order::OrderKey;
pub use points::{PackedPoints, Point};
pub use replica::Replica;
pub use room::{Outcome, RejectCode, Room};
