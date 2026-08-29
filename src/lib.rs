//! Unofficial Dante-compatible AoIP library.
//!
//! v1 is receive-first: appear as a Dante device, accept patches from
//! Dante Controller, and deliver PCM to the application. Clocking is
//! an in-process overlay. Windows, macOS, and Linux share the same API.
//! Protocol model follows Inferno.

#![cfg_attr(docsrs, feature(doc_auto_cfg))]

pub mod clock;
pub mod device;
pub mod media;
pub mod protocol;

/// Interleaved or planar PCM sample used at the crate boundary.
/// Dante payloads are 16/24/32-bit integer; internally promote to i32 (Q31).
pub type Sample = i32;
