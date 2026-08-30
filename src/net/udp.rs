//! socket2 bind helpers. Never bind 0.0.0.0.

use crate::device::Error;
use crate::protocol::ports as p;
use socket2::{Domain, Protocol, SockAddr, Socket, Type};
use std::net::{Ipv4Addr, SocketAddrV4, UdpSocket as StdUdp};

/// Resolve ARC / CMC / flows-control dest / info ports.
///
/// `None` tries the Dante defaults, then a free 4-port block.
/// `Some(base)` is exact (`base`..=`base+3`) and fails if any is busy.
pub fn pick_control_ports(
    ip: Ipv4Addr,
    requested: Option<u16>,
) -> Result<(u16, u16, u16, u16), Error> {
    if let Some(base) = requested {
        let block = port_block(base)?;
        reserve_block(ip, block)?;
        return Ok(block);
    }
    let defaults = (p::ARC, p::CMC, p::FLOWS_CONTROL, p::INFO_BIND);
    if reserve_block(ip, defaults).is_ok() {
        return Ok(defaults);
    }
    for base in control_base_candidates() {
        let Ok(block) = port_block(base) else {
            continue;
        };
        if reserve_block(ip, block).is_ok() {
            log::warn!(
                "default control ports unavailable; using ARC {} CMC {} info {}",
                block.0,
                block.1,
                block.3
            );
            return Ok(block);
        }
    }
    Err(Error::PortInUse {
        port: p::ARC,
        role: "arc",
    })
}

fn port_block(base: u16) -> Result<(u16, u16, u16, u16), Error> {
    let b1 = base.checked_add(1).ok_or(Error::PortInUse {
        port: base,
        role: "arc",
    })?;
    let b2 = base.checked_add(2).ok_or(Error::PortInUse {
        port: base,
        role: "arc",
    })?;
    let b3 = base.checked_add(3).ok_or(Error::PortInUse {
        port: base,
        role: "arc",
    })?;
    Ok((base, b1, b2, b3))
}

fn overlaps_fixed_or_media(base: u16) -> bool {
    let last = base.saturating_add(3);
    let hits = |p: u16| (base..=last).contains(&p);
    hits(p::MDNS)
        || hits(p::PTP_EVENT)
        || hits(p::PTP_GENERAL)
        || last >= p::MEDIA_PORT_START && base <= p::MEDIA_PORT_END_2
}

fn control_base_candidates() -> impl Iterator<Item = u16> {
    (10_000u16..=40_000)
        .step_by(4)
        .filter(|&base| !overlaps_fixed_or_media(base))
}

/// Bind all four then drop, so a later `bind_unicast` can take them.
fn reserve_block(ip: Ipv4Addr, block: (u16, u16, u16, u16)) -> Result<(), Error> {
    let a = bind_unicast(ip, block.0, "arc")?;
    let c = match bind_unicast(ip, block.1, "cmc") {
        Ok(s) => s,
        Err(e) => {
            drop(a);
            return Err(e);
        }
    };
    let f = match bind_unicast(ip, block.2, "flows") {
        Ok(s) => s,
        Err(e) => {
            drop((a, c));
            return Err(e);
        }
    };
    match bind_unicast(ip, block.3, "info") {
        Ok(i) => {
            drop((a, c, f, i));
            Ok(())
        }
        Err(e) => {
            drop((a, c, f));
            Err(e)
        }
    }
}

pub fn bind_unicast(ip: Ipv4Addr, port: u16, role: &'static str) -> Result<StdUdp, Error> {
    reject(ip)?;
    let sock = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
    #[cfg(windows)]
    crate::net::windows::set_exclusive_addr_use(&sock)?;
    let addr = SockAddr::from(SocketAddrV4::new(ip, port));
    sock.bind(&addr).map_err(|e| map_in_use(e, port, role))?;
    Ok(sock.into())
}

pub fn bind_in_range(ip: Ipv4Addr, start: u16, end: u16) -> Result<StdUdp, Error> {
    reject(ip)?;
    for port in start..=end {
        match bind_unicast(ip, port, "media") {
            Ok(s) => return Ok(s),
            Err(Error::PortInUse { .. }) => continue,
            Err(e) => return Err(e),
        }
    }
    bind_unicast(ip, 0, "media")
}

pub fn bind_mdns(ip: Ipv4Addr) -> Result<StdUdp, Error> {
    reject(ip)?;
    let sock = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
    sock.set_reuse_address(true)?;
    #[cfg(unix)]
    sock.set_reuse_port(true)?;
    let addr = SockAddr::from(SocketAddrV4::new(ip, p::MDNS));
    sock.bind(&addr).map_err(|_| Error::MdnsPortInUse)?;
    let std: StdUdp = sock.into();
    set_multicast_if_v4(&std, ip)?;
    join_multicast_v4(
        &std,
        Ipv4Addr::new(
            p::MDNS_GROUP[0],
            p::MDNS_GROUP[1],
            p::MDNS_GROUP[2],
            p::MDNS_GROUP[3],
        ),
        ip,
    )?;
    std.set_multicast_ttl_v4(255)?;
    std.set_multicast_loop_v4(true)?;
    Ok(std)
}

pub fn bind_ptp(ip: Ipv4Addr, port: u16) -> Result<StdUdp, Error> {
    reject(ip)?;
    let sock = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
    #[cfg(windows)]
    crate::net::windows::set_exclusive_addr_use(&sock)?;
    let addr = SockAddr::from(SocketAddrV4::new(ip, port));
    sock.bind(&addr).map_err(|e| {
        if e.kind() == std::io::ErrorKind::PermissionDenied {
            Error::PtpBindDenied { port }
        } else {
            map_in_use(e, port, "ptp")
        }
    })?;
    let std: StdUdp = sock.into();
    set_multicast_if_v4(&std, ip)?;
    let g = Ipv4Addr::new(
        p::PTP_GROUP[0],
        p::PTP_GROUP[1],
        p::PTP_GROUP[2],
        p::PTP_GROUP[3],
    );
    join_multicast_v4(&std, g, ip)?;
    std.set_multicast_ttl_v4(1)?;
    Ok(std)
}

pub fn bind_querier(ip: Ipv4Addr) -> Result<StdUdp, Error> {
    reject(ip)?;
    let sock = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
    sock.bind(&SockAddr::from(SocketAddrV4::new(ip, 0)))?;
    let std: StdUdp = sock.into();
    set_multicast_if_v4(&std, ip)?;
    Ok(std)
}

pub fn set_multicast_if_v4(sock: &StdUdp, iface: Ipv4Addr) -> Result<(), Error> {
    socket2::SockRef::from(sock).set_multicast_if_v4(&iface)?;
    Ok(())
}

pub fn join_multicast_v4(sock: &StdUdp, group: Ipv4Addr, iface: Ipv4Addr) -> Result<(), Error> {
    sock.join_multicast_v4(&group, &iface)
        .map_err(|_| Error::MulticastJoinFailed { group })
}

pub fn std_to_tokio(sock: StdUdp) -> Result<tokio::net::UdpSocket, Error> {
    sock.set_nonblocking(true)?;
    #[cfg(windows)]
    crate::net::windows::disable_udp_connreset(&sock)?;
    Ok(tokio::net::UdpSocket::from_std(sock)?)
}

pub fn prepare_media(sock: &StdUdp) -> Result<(), Error> {
    sock.set_nonblocking(true)?;
    let _ = socket2::SockRef::from(sock).set_recv_buffer_size(2 * 1024 * 1024);
    #[cfg(windows)]
    crate::net::windows::disable_udp_connreset(sock)?;
    Ok(())
}

fn reject(ip: Ipv4Addr) -> Result<(), Error> {
    if ip.is_unspecified() || ip.is_multicast() || ip.is_broadcast() {
        Err(Error::UnspecifiedAddress)
    } else {
        Ok(())
    }
}

fn bind_denied(e: &std::io::Error) -> bool {
    e.kind() == std::io::ErrorKind::AddrInUse
        || e.kind() == std::io::ErrorKind::PermissionDenied
        || matches!(e.raw_os_error(), Some(10013 | 10048))
}

fn map_in_use(e: std::io::Error, port: u16, role: &'static str) -> Error {
    if bind_denied(&e) && (port == 319 || port == 320) {
        Error::PtpBindDenied { port }
    } else if bind_denied(&e) {
        Error::PortInUse { port, role }
    } else {
        Error::Io(e)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn access_denied_on_arc_is_port_in_use() {
        let e = std::io::Error::from_raw_os_error(10013);
        match map_in_use(e, 4440, "arc") {
            Error::PortInUse {
                port: 4440,
                role: "arc",
            } => {}
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn access_denied_on_ptp_is_ptp_bind_denied() {
        let e = std::io::Error::from_raw_os_error(10013);
        match map_in_use(e, 319, "ptp") {
            Error::PtpBindDenied { port: 319 } => {}
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn pick_defaults_on_loopback() {
        let (arc, cmc, flows, info) =
            pick_control_ports(Ipv4Addr::LOCALHOST, None).expect("loopback control ports");
        if arc == p::ARC {
            assert_eq!((cmc, flows, info), (p::CMC, p::FLOWS_CONTROL, p::INFO_BIND));
        } else {
            assert_eq!(cmc, arc + 1);
            assert_eq!(flows, arc + 2);
            assert_eq!(info, arc + 3);
            assert!(!overlaps_fixed_or_media(arc));
        }
    }

    #[test]
    fn pick_skips_held_arc() {
        let hold = bind_unicast(Ipv4Addr::LOCALHOST, p::ARC, "arc");
        let Ok(_hold) = hold else {
            return;
        };
        let (arc, _, _, _) =
            pick_control_ports(Ipv4Addr::LOCALHOST, None).expect("fallback control ports");
        assert_ne!(arc, p::ARC);
    }
}
