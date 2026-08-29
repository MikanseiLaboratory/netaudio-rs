//! Unofficial Dante-compatible AoIP library.
//!
//! v1 is receive-first: appear as a Dante device, accept patches from
//! Dante Controller, and deliver PCM to the application. Clocking is
//! an in-process overlay. Windows, macOS, and Linux share the same API.

#![cfg_attr(docsrs, feature(doc_auto_cfg))]

pub(crate) mod clock;
pub(crate) mod device;
pub(crate) mod media;
pub(crate) mod net;
pub(crate) mod protocol;

/// Left-justified PCM sample at the crate boundary.
///
/// Dante payloads are 16/24/32-bit big-endian integer. Unused LSBs are zero
/// (not a Q31 multiply). See `docs/IMPLEMENTATION-PLAN.md` §11.4.
pub type Sample = i32;

pub use device::{
    AudioFrameMut, Bind, Bits, BoundPorts, ClockStatus, Device, Error, MediaTime, Settings,
};
