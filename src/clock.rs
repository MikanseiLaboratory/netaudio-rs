//! In-process media clock.
//!
//! Overlay over `std::time::Instant` (QPC on Windows): shift + freq_scale.
//! Sources: PTPv1 listen-only, then media-packet timestamps. See docs/issues/003.

#![allow(dead_code)]

/// Nanoseconds on the overlay timescale. May wrap.
pub type FineTime = u64;
