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
