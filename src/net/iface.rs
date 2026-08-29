//! NIC resolution via `netdev`.

use crate::device::{Bind, Error};
use std::net::Ipv4Addr;

#[derive(Clone, Debug)]
pub struct IfaceInfo {
    #[allow(dead_code)]
    pub name: String,
    pub ipv4: Ipv4Addr,
    pub netmask: Ipv4Addr,
    pub gateway: Ipv4Addr,
    pub mac: [u8; 6],
    pub link_mbps: u16,
}

pub fn resolve(bind: &Bind) -> Result<IfaceInfo, Error> {
    match bind {
        Bind::Ip(ip) => {
            reject_bad_ip(*ip)?;
            find_by_ip(*ip)
        }
        Bind::Interface(name) => find_by_name(name),
    }
}

fn reject_bad_ip(ip: Ipv4Addr) -> Result<(), Error> {
    if ip.is_unspecified() || ip.is_multicast() || ip.is_broadcast() {
        return Err(Error::UnspecifiedAddress);
    }
    Ok(())
}

fn find_by_ip(ip: Ipv4Addr) -> Result<IfaceInfo, Error> {
    if ip.is_loopback() {
        return Ok(loopback(ip));
    }
    for iface in netdev::get_interfaces() {
        for net in &iface.ipv4 {
            if net.addr() == ip {
                return Ok(from_netdev(&iface, ip, net.netmask()));
            }
        }
    }
    // Still allow binding if the OS will accept it (e.g. extra address).
    Ok(IfaceInfo {
        name: ip.to_string(),
        ipv4: ip,
        netmask: Ipv4Addr::new(255, 255, 255, 0),
        gateway: Ipv4Addr::UNSPECIFIED,
        mac: [0; 6],
        link_mbps: 0,
    })
}

fn find_by_name(name: &str) -> Result<IfaceInfo, Error> {
    if let Ok(ip) = name.parse::<Ipv4Addr>() {
        return find_by_ip(ip);
    }
    let want = name.to_ascii_lowercase();
    for iface in netdev::get_interfaces() {
        let names = [
            iface.name.clone(),
            iface.friendly_name.clone().unwrap_or_default(),
            iface.description.clone().unwrap_or_default(),
        ];
        if names
            .iter()
            .any(|n| !n.is_empty() && n.to_ascii_lowercase() == want)
        {
            let net = iface
                .ipv4
                .iter()
                .find(|n| !n.addr().is_unspecified())
                .ok_or_else(|| Error::InterfaceHasNoIpv4 {
                    name: name.to_owned(),
                })?;
            reject_bad_ip(net.addr())?;
            return Ok(from_netdev(&iface, net.addr(), net.netmask()));
        }
    }
    Err(Error::InterfaceNotFound {
        name: name.to_owned(),
    })
}

fn from_netdev(iface: &netdev::Interface, ipv4: Ipv4Addr, netmask: Ipv4Addr) -> IfaceInfo {
    let mut mac = [0u8; 6];
    if let Some(m) = iface.mac_addr {
        let oct = m.octets();
        mac.copy_from_slice(&oct[..6.min(oct.len())]);
    }
    let mut gateway = Ipv4Addr::UNSPECIFIED;
    if let Some(gws) = &iface.gateway
        && let Some(g) = gws.ipv4.first()
    {
        gateway = *g;
    }
    let speed = [
        iface.transmit_speed.unwrap_or(0),
        iface.receive_speed.unwrap_or(0),
    ]
    .into_iter()
    .max()
    .unwrap_or(0)
        / 1_000_000;
    IfaceInfo {
        name: iface.name.clone(),
        ipv4,
        netmask,
        gateway,
        mac,
        link_mbps: speed.clamp(0, 10_000) as u16,
    }
}

fn loopback(ip: Ipv4Addr) -> IfaceInfo {
    IfaceInfo {
        name: "lo".into(),
        ipv4: ip,
        netmask: Ipv4Addr::new(255, 0, 0, 0),
        gateway: Ipv4Addr::UNSPECIFIED,
        mac: [0; 6],
        link_mbps: 0,
    }
}
