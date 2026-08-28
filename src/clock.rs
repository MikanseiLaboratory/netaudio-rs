//! In-process media clock.
//!
//! Overlay over a local monotonic clock. Never steers the OS system clock.
//! See the tracking issue for PTPv1 listen-only vs media-packet fallback.

#![allow(dead_code)]

/// Nanoseconds on the overlay timescale. May wrap.
pub type FineTime = u64;
