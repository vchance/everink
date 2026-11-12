//! Everink core, spike slice 1: positions and the camera.
//!
//! The design is `docs/design/01-positions.md`. A position is a level plus one exact fixed-point integer per
//! axis, in steps of 2^-54 of a level unit. Levels are 1024 apart. Every comparison and difference is exact;
//! only a difference near the camera is turned into an `f64`.

pub mod axis;
pub mod camera;
pub mod cut;
pub mod outline;
pub mod position;
pub mod wire;

pub use axis::Axis;
pub use camera::{Camera, CameraError};
pub use position::{Delta, Level, Position, Rel};
pub use wire::{DecodeError, DepthCap};

/// Bits between two levels: one level is 1024 = 2^10 times finer than the one above.
pub const LEVEL_SHIFT: u32 = 10;
/// Bits of offset inside a cell.
pub const FRAC_BITS: u32 = 64;
/// Steps per level unit are 2^54 (`FRAC_BITS - LEVEL_SHIFT`).
pub const UNIT_BITS: u32 = 54;
