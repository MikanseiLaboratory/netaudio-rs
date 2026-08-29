//! Interface-bound UDP, multicast, mDNS I/O.

pub mod iface;
pub mod mdns;
pub mod udp;

#[cfg(windows)]
pub mod windows;
