//! In-process overlay clock.

pub mod overlay;
pub mod ptp;

pub use overlay::{ClockStatus, OverlayClock};
