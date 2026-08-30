//! socket2 bind helpers. Never bind 0.0.0.0.

use crate::device::Error;
use crate::protocol::ports as p;
use socket2::{Domain, Protocol, SockAddr, Socket, Type};
use std::net::{Ipv4Addr, SocketAddrV4, UdpSocket as StdUdp};

/// Bound ARC / CMC / info sockets plus the four logical control ports.
pub struct ControlSockets {
    pub arc: StdUdp,
    pub cmc: StdUdp,
    pub info: StdUdp,
    pub ports: (u16, u16, u16, u16),
}

/// Bind ARC / CMC / info. `None` tries Dante defaults, then a free 4-port block.
/// `Some(base)` is exact (`base`, `base+1`, `base+3`) and fails if any is busy.
pub fn bind_control_ports(ip: Ipv4Addr, requested: Option<u16>) -> Result<ControlSockets, Error> {
    if let Some(base) = requested {
        return bind_triple(ip, port_block(base)?);
    }
    let defaults = (p::ARC, p::CMC, p::FLOWS_CONTROL, p::INFO_BIND);
    if let Ok(bound) = bind_triple(ip, defaults) {
        return Ok(bound);
    }
    for base in control_base_candidates() {
        let Ok(block) = port_block(base) else {
            continue;
        };
        if let Ok(bound) = bind_triple(ip, block) {
            log::warn!(
                "default control ports unavailable; using ARC {} CMC {} info {}",
                bound.ports.0,
                bound.ports.1,
                bound.ports.3
            );
            return Ok(bound);
        }
    }
    Err(Error::PortInUse {
        port: p::ARC,
        role: "arc",
    })
}

/// Numbers only (sockets dropped). Used by tests.
#[cfg(test)]
fn pick_control_ports(ip: Ipv4Addr, requested: Option<u16>) -> Result<(u16, u16, u16, u16), Error> {
    Ok(bind_control_ports(ip, requested)?.ports)
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
        || last >= p::MEDIA_VIA_START && base <= p::MEDIA_VIA_END
}

fn control_base_candidates() -> impl Iterator<Item = u16> {
    (10_000u16..=40_000)
        .step_by(4)
        .filter(|&base| !overlaps_fixed_or_media(base))
}

fn bind_triple(ip: Ipv4Addr, block: (u16, u16, u16, u16)) -> Result<ControlSockets, Error> {
    let arc = bind_unicast(ip, block.0, "arc")?;
    let cmc = match bind_unicast(ip, block.1, "cmc") {
        Ok(s) => s,
        Err(e) => {
            drop(arc);
            return Err(e);
        }
    };
    match bind_unicast(ip, block.3, "info") {
        Ok(info) => Ok(ControlSockets {
            arc,
            cmc,
            info,
            ports: block,
        }),
        Err(e) => {
            drop((arc, cmc));
            Err(e)
        }
    }
}

pub fn bind_unicast(ip: Ipv4Addr, port: u16, role: &'static str) -> Result<StdUdp, Error> {
    match bind_unicast_inner(ip, port, role, true) {
        Ok(s) => Ok(s),
        Err(Error::PortInUse { port, role }) => {
            #[cfg(windows)]
            {
                bind_unicast_inner(ip, port, role, false)
            }
            #[cfg(not(windows))]
            {
                Err(Error::PortInUse { port, role })
            }
        }
        Err(e) => Err(e),
    }
}

fn bind_unicast_inner(
    ip: Ipv4Addr,
    port: u16,
    role: &'static str,
    #[cfg_attr(not(windows), allow(unused_variables))] exclusive: bool,
) -> Result<StdUdp, Error> {
    reject(ip)?;
    let sock = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
    #[cfg(windows)]
    if exclusive {
        crate::net::windows::set_exclusive_addr_use(&sock)?;
    }
    let addr = SockAddr::from(SocketAddrV4::new(ip, port));
    sock.bind(&addr).map_err(|e| map_in_use(e, port, role))?;
    Ok(sock.into())
}

/// Hardware / DVS unicast audio is UDP 14336..=14591 (`0x3800..=0x38FF`).
/// Via uses 34336..=34600. `bind(0)` on Windows often returns a Hyper-V
/// excluded ephemeral port (bind succeeds, inbound never arrives).
/// Do not wrap this socket in tokio (IOCP then `into_std` drops recv_from).
pub fn bind_media(ip: Ipv4Addr) -> Result<StdUdp, Error> {
    for (start, end) in [
        (p::MEDIA_PORT_START, p::MEDIA_PORT_END),
        (p::MEDIA_PORT_START_2, p::MEDIA_PORT_END_2),
        (p::MEDIA_VIA_START, p::MEDIA_VIA_END),
        (20_000, 20_255),
    ] {
        match bind_in_range(ip, start, end) {
            Ok(s) => return Ok(s),
            Err(Error::PortInUse { .. }) => continue,
            Err(e) => return Err(e),
        }
    }
    bind_unicast(ip, 0, "media")
}

fn bind_media_port(ip: Ipv4Addr, port: u16) -> Result<StdUdp, Error> {
    #[cfg(windows)]
    {
        bind_unicast_inner(ip, port, "media", false)
    }
    #[cfg(not(windows))]
    {
        bind_unicast(ip, port, "media")
    }
}

fn bind_in_range(ip: Ipv4Addr, start: u16, end: u16) -> Result<StdUdp, Error> {
    reject(ip)?;
    let mut last = Error::PortInUse {
        port: start,
        role: "media",
    };
    let mut consecutive_dead = 0u8;
    for port in start..=end {
        match bind_media_port(ip, port) {
            Ok(s) => {
                if media_port_receives(&s) {
                    return Ok(s);
                }
                log::warn!("media UDP {ip}:{port} bound but self-echo failed (excluded/WFP)");
                consecutive_dead = consecutive_dead.saturating_add(1);
                if consecutive_dead >= 8 {
                    break;
                }
            }
            Err(e @ Error::PortInUse { .. }) => {
                consecutive_dead = 0;
                last = e;
            }
            Err(e) => return Err(e),
        }
    }
    Err(last)
}

/// True if a datagram sent to the bound address comes back. Hyper-V excluded
/// ports bind successfully then drop inbound, including this probe.
fn media_port_receives(sock: &StdUdp) -> bool {
    #[cfg(windows)]
    let _ = crate::net::windows::disable_udp_connreset(sock);
    let Ok(addr) = sock.local_addr() else {
        return false;
    };
    let _ = sock.set_nonblocking(true);
    let mut buf = [0u8; 64];
    while sock.recv_from(&mut buf).is_ok() {}
    if sock.send_to(&[0x13, 0x37], addr).is_err() {
        return false;
    }
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(20);
    while std::time::Instant::now() < deadline {
        match sock.recv_from(&mut buf) {
            Ok((n, _)) if n >= 2 && buf[0] == 0x13 && buf[1] == 0x37 => return true,
            Ok(_) => continue,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            Err(_) => return false,
        }
    }
    false
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
    fn bind_media_uses_dante_unicast_range() {
        let s = bind_media(Ipv4Addr::LOCALHOST).expect("media");
        let port = s.local_addr().unwrap().port();
        assert!(
            (p::MEDIA_PORT_START..=p::MEDIA_PORT_END).contains(&port)
                || (p::MEDIA_PORT_START_2..=p::MEDIA_PORT_END_2).contains(&port)
                || (p::MEDIA_VIA_START..=p::MEDIA_VIA_END).contains(&port)
                || (20_000..=20_255).contains(&port),
            "unexpected media port {port}"
        );
        assert!(
            media_port_receives(&s),
            "bound media port {port} failed self-echo"
        );
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
