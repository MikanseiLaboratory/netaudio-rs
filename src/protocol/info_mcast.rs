//! Info multicast 32-byte header and payloads.

use super::HEADER_MCAST;
use super::buf::{BeWriter, write_ascii_padded};
use std::net::Ipv4Addr;

pub const START_INFO: u16 = 0xFFFF;
pub const START_HEARTBEAT: u16 = 0xFFFE;

pub const OPCODE_BOARD: [u8; 8] = [0x07, 0x2a, 0x00, 0x60, 0, 0, 0, 0];
pub const OPCODE_PRODUCT: [u8; 8] = [0x07, 0x2a, 0x00, 0xc0, 0, 0, 0, 0];
pub const OPCODE_CLOCK: [u8; 8] = [0x07, 0x2a, 0x00, 0x20, 0, 0, 0, 0];
pub const OPCODE_NET: [u8; 8] = [0x07, 0x2a, 0x00, 0x11, 0, 0, 0, 0];
pub const OPCODE_HEARTBEAT: [u8; 8] = [0x00, 0x08, 0x00, 0x01, 0x10, 0, 0, 0];
pub const OPCODE_CH_CHANGE: [u8; 8] = [0x07, 0x2a, 0x01, 0x02, 0, 0, 0, 0];

#[derive(Clone, Copy, Debug)]
pub struct McastHeader {
    pub start_code: u16,
    pub total_length: u16,
    pub seqnum: u16,
    pub process: u16,
    pub device_id: [u8; 8],
    pub vendor: [u8; 8],
    pub opcode: [u8; 8],
}

pub fn encode(
    start_code: u16,
    seqnum: u16,
    process: u16,
    device_id: [u8; 8],
    vendor: [u8; 8],
    opcode: [u8; 8],
    content: &[u8],
) -> Vec<u8> {
    let total = HEADER_MCAST + content.len();
    let mut w = BeWriter::with_capacity(total);
    w.u16(start_code);
    w.u16(total as u16);
    w.u16(seqnum);
    w.u16(process);
    w.bytes(&device_id);
    w.bytes(&vendor);
    w.bytes(&opcode);
    w.bytes(content);
    debug_assert_eq!(w.len(), total);
    w.into_inner()
}

pub fn decode(packet: &[u8]) -> Option<(McastHeader, &[u8])> {
    if packet.len() < HEADER_MCAST {
        return None;
    }
    let start_code = u16::from_be_bytes(packet[0..2].try_into().ok()?);
    let total_length = u16::from_be_bytes(packet[2..4].try_into().ok()?);
    let seqnum = u16::from_be_bytes(packet[4..6].try_into().ok()?);
    let process = u16::from_be_bytes(packet[6..8].try_into().ok()?);
    let mut device_id = [0u8; 8];
    device_id.copy_from_slice(&packet[8..16]);
    let mut vendor = [0u8; 8];
    vendor.copy_from_slice(&packet[16..24]);
    let mut opcode = [0u8; 8];
    opcode.copy_from_slice(&packet[24..32]);
    let end = (total_length as usize).min(packet.len());
    Some((
        McastHeader {
            start_code,
            total_length,
            seqnum,
            process,
            device_id,
            vendor,
            opcode,
        },
        &packet[HEADER_MCAST..end],
    ))
}

/// Request opcode byte 3 (`packet[27]`) → reply opcode.
pub fn reply_opcode(request: &[u8]) -> Option<[u8; 8]> {
    if request.len() < HEADER_MCAST {
        return None;
    }
    if request[24] != 0x07 {
        return None;
    }
    match request[27] {
        0x61 => Some(OPCODE_BOARD),
        0xC1 => Some(OPCODE_PRODUCT),
        0x21 => Some(OPCODE_CLOCK),
        0x13 => Some(OPCODE_NET),
        _ => None,
    }
}

pub fn board_payload(board: &str) -> Vec<u8> {
    let mut b = vec![0u8; 200];
    b[0..4].copy_from_slice(&[0x04, 0x01, 0x00, 0x06]);
    b[4..8].copy_from_slice(&[0x04, 0x01, 0x00, 0x03]);
    b[0x14..0x18].copy_from_slice(&[0x00, 0x00, 0x10, 0x00]);
    b[0x23] = 2;
    b[0x27] = 1;
    b[0x28..0x2C].copy_from_slice(&[0x01, 0x00, 0x00, 0x00]);
    b[0xBB] = 0x1F;
    write_ascii_padded(&mut b[12..20], board);
    write_ascii_padded(&mut b[0x38..0x48], board);
    b
}

pub fn product_payload(manufacturer: &str, board: &str, model: &str) -> Vec<u8> {
    let mut b = vec![0u8; 336];
    write_ascii_padded(&mut b[0..8], manufacturer);
    write_ascii_padded(&mut b[8..16], board);
    write_ascii_padded(&mut b[0x2C..0x3C], manufacturer);
    write_ascii_padded(&mut b[0xAC..0xBC], model);
    b[0x1C..0x20].copy_from_slice(&[0, 0, 0, 0]);
    b
}

pub fn heartbeat_clock_ppm(seq: u16, ppb: i32) -> Vec<u8> {
    let mut w = BeWriter::new();
    w.u16(16);
    w.u16(0x8001);
    w.u16(4);
    w.u16(4);
    w.u16(seq);
    w.u16(0);
    w.u32(ppb as u32);
    w.into_inner()
}

pub fn network_info(
    link_mbps: u16,
    mac: [u8; 6],
    ip: Ipv4Addr,
    mask: Ipv4Addr,
    gw: Ipv4Addr,
) -> Vec<u8> {
    let mut w = BeWriter::new();
    w.bytes(&[0x00, 0x01, 0x00, 0x00, 0x00, 0x00]);
    w.u16(link_mbps);
    w.u16(1);
    w.bytes(&mac);
    w.bytes(&ip.octets());
    w.bytes(&mask.octets());
    w.bytes(&gw.octets());
    w.zeros(4); // dns
    w.into_inner()
}

pub fn channel_change(indices: &[usize]) -> Vec<u8> {
    if indices.is_empty() {
        return vec![0x00, 0x01, 0x00];
    }
    let mut content = vec![0u8; 3];
    for &ch in indices {
        let byte = ch / 8;
        let bit = ch % 8;
        if byte + 2 >= content.len() {
            content.resize(byte + 3, 0);
        }
        content[byte + 2] |= 1 << bit;
    }
    let mask_len = (content.len() - 2) as u16;
    content[0..2].copy_from_slice(&mask_len.to_be_bytes());
    content
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_roundtrip() {
        let pkt = encode(
            START_INFO,
            1,
            0,
            [0; 8],
            *b"Audinate",
            OPCODE_BOARD,
            &[1, 2, 3],
        );
        let (h, c) = decode(&pkt).unwrap();
        assert_eq!(h.start_code, START_INFO);
        assert_eq!(h.vendor, *b"Audinate");
        assert_eq!(c, &[1, 2, 3]);
    }

    #[test]
    fn board_sets_flood_byte() {
        let b = board_payload("netaudio");
        assert_eq!(b.len(), 200);
        assert_eq!(b[0xBB], 0x1F);
    }
}
