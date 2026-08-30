//! Public error type.

use std::io;
use std::net::Ipv4Addr;

#[non_exhaustive]
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid settings: {reason}")]
    InvalidSettings { reason: String },
    #[error("refusing to bind unspecified / multicast / broadcast address")]
    UnspecifiedAddress,
    #[error("interface not found: {name}")]
    InterfaceNotFound { name: String },
    #[error("interface {name} has no usable IPv4 address")]
    InterfaceHasNoIpv4 { name: String },
    #[error("UDP port {port} in use or reserved ({role})")]
    PortInUse { port: u16, role: &'static str },
    #[error("mDNS UDP 5353 is in use or not shareable on this host")]
    MdnsPortInUse,
    #[error("PTP bind denied on UDP {port} (need CAP_NET_BIND_SERVICE on Unix)")]
    PtpBindDenied { port: u16 },
    #[error("device has been stopped")]
    Stopped,
    #[error("multicast join failed for {group}")]
    MulticastJoinFailed { group: Ipv4Addr },
    #[error(transparent)]
    Io(#[from] io::Error),
}

impl Error {
    pub(crate) fn settings(reason: impl Into<String>) -> Self {
        Error::InvalidSettings {
            reason: reason.into(),
        }
    }
}
