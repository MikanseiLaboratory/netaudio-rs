//! WinSock extras: SO_EXCLUSIVEADDRUSE, SIO_UDP_CONNRESET.

#![cfg(windows)]

use crate::device::Error;
use socket2::Socket;
use std::os::windows::io::{AsRawSocket, RawSocket};

pub fn set_exclusive_addr_use(sock: &Socket) -> Result<(), Error> {
    use windows_sys::Win32::Networking::WinSock::{
        SO_EXCLUSIVEADDRUSE, SOCKET, SOL_SOCKET, setsockopt,
    };
    let raw: RawSocket = sock.as_raw_socket();
    let enable: i32 = 1;
    let rc = unsafe {
        setsockopt(
            raw as SOCKET,
            SOL_SOCKET,
            SO_EXCLUSIVEADDRUSE,
            &enable as *const i32 as *const u8,
            std::mem::size_of::<i32>() as i32,
        )
    };
    if rc != 0 {
        return Err(Error::Io(std::io::Error::last_os_error()));
    }
    Ok(())
}

pub fn disable_udp_connreset(sock: &std::net::UdpSocket) -> Result<(), Error> {
    use std::os::windows::io::AsRawSocket;
    use windows_sys::Win32::Networking::WinSock::{SOCKET, SOCKET_ERROR, WSAIoctl};
    const SIO_UDP_CONNRESET: u32 = 0x9800_000C;
    let raw = sock.as_raw_socket();
    let mut enable: u32 = 0;
    let mut ret = 0u32;
    let rc = unsafe {
        WSAIoctl(
            raw as SOCKET,
            SIO_UDP_CONNRESET,
            &mut enable as *mut u32 as *mut _,
            std::mem::size_of::<u32>() as u32,
            std::ptr::null_mut(),
            0,
            &mut ret,
            std::ptr::null_mut(),
            None,
        )
    };
    if rc == SOCKET_ERROR {
        return Err(Error::Io(std::io::Error::last_os_error()));
    }
    Ok(())
}

/// Best-effort inbound allow for this process. DVS installer does this;
/// without it Windows drops unicast media from TX source ports we cannot know.
pub fn try_allow_inbound_udp() {
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let program = format!("program={}", exe.display());
    match std::process::Command::new("netsh")
        .args([
            "advfirewall",
            "firewall",
            "add",
            "rule",
            "name=netaudio-rs",
            "dir=in",
            "action=allow",
            program.as_str(),
            "enable=yes",
            "profile=any",
        ])
        .output()
    {
        Ok(o) if o.status.success() => {
            eprintln!("firewall inbound UDP allow for {}", exe.display());
            log::info!("Windows Firewall inbound allow for {}", exe.display());
        }
        Ok(o) => {
            let msg = String::from_utf8_lossy(&o.stderr);
            let msg = msg.trim();
            if msg.is_empty() {
                eprintln!("firewall rule netaudio-rs already present");
                log::info!("Windows Firewall rule netaudio-rs already present");
            } else {
                eprintln!("firewall {msg}; allow inbound UDP for {}", exe.display());
                log::warn!(
                    "Windows Firewall: {msg}; allow inbound UDP for {}",
                    exe.display()
                );
            }
        }
        Err(e) => {
            eprintln!("firewall netsh: {e}");
            log::warn!("Windows Firewall netsh: {e}");
        }
    }
    try_allow_udp_ports("netaudio-rs-ptp", "319,320");
    try_allow_udp_ports("netaudio-rs-media", "14336-14591");
    try_allow_udp_ports_remote("netaudio-rs-ptp-mcast", "319,320", "224.0.1.129");
}

fn try_allow_udp_ports(name: &str, localport: &str) {
    match std::process::Command::new("netsh")
        .args([
            "advfirewall",
            "firewall",
            "add",
            "rule",
            &format!("name={name}"),
            "dir=in",
            "action=allow",
            "protocol=UDP",
            &format!("localport={localport}"),
            "enable=yes",
            "profile=any",
        ])
        .output()
    {
        Ok(o) if o.status.success() => {
            log::info!("Windows Firewall inbound UDP {localport}");
        }
        Ok(_) => {}
        Err(e) => log::warn!("Windows Firewall {name}: {e}"),
    }
}

fn try_allow_udp_ports_remote(name: &str, localport: &str, remoteip: &str) {
    match std::process::Command::new("netsh")
        .args([
            "advfirewall",
            "firewall",
            "add",
            "rule",
            &format!("name={name}"),
            "dir=in",
            "action=allow",
            "protocol=UDP",
            &format!("localport={localport}"),
            &format!("remoteip={remoteip}"),
            "enable=yes",
            "profile=any",
        ])
        .output()
    {
        Ok(o) if o.status.success() => {
            log::info!("Windows Firewall inbound UDP {localport} from {remoteip}");
        }
        Ok(_) => {}
        Err(e) => log::warn!("Windows Firewall {name}: {e}"),
    }
}
