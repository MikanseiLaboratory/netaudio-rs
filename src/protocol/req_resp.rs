//! 10-byte request/response header (ARC, CMC, flows-control).

use super::HEADER_RR;
use super::buf::{BeSlice, BeWriter};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Header {
    pub start_code: u16,
    pub total_length: u16,
    pub seqnum: u16,
    pub opcode1: u16,
    pub opcode2: u16,
}

pub fn decode(packet: &[u8]) -> Option<(Header, &[u8])> {
    if packet.len() < HEADER_RR {
        return None;
    }
    let mut r = BeSlice::new(packet);
    let h = Header {
        start_code: r.u16()?,
        total_length: r.u16()?,
        seqnum: r.u16()?,
        opcode1: r.u16()?,
        opcode2: r.u16()?,
    };
    let content_end = (h.total_length as usize).min(packet.len());
    if content_end < HEADER_RR {
        return None;
    }
    Some((h, &packet[HEADER_RR..content_end]))
}

pub fn encode(start_code: u16, seqnum: u16, opcode1: u16, opcode2: u16, content: &[u8]) -> Vec<u8> {
    let total = HEADER_RR + content.len();
    debug_assert!(total <= u16::MAX as usize);
    let mut w = BeWriter::with_capacity(total);
    w.u16(start_code);
    w.u16(total as u16);
    w.u16(seqnum);
    w.u16(opcode1);
    w.u16(opcode2);
    w.bytes(content);
    w.into_inner()
}

pub fn encode_ok(req: Header, content: &[u8]) -> Vec<u8> {
    encode(
        req.start_code,
        req.seqnum,
        req.opcode1,
        super::OPCODE2_OK,
        content,
    )
}

pub fn encode_code(req: Header, opcode2: u16, content: &[u8]) -> Vec<u8> {
    encode(req.start_code, req.seqnum, req.opcode1, opcode2, content)
}

/// 1-based start index from a paginated request. `None` if invalid.
pub fn pagination_start(content: &[u8]) -> Option<usize> {
    if content.len() < 4 {
        return None;
    }
    let idx = u16::from_be_bytes([content[2], content[3]]);
    if idx == 0 {
        return None;
    }
    Some((idx - 1) as usize)
}

pub fn empty_page() -> Vec<u8> {
    vec![0, 0]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let pkt = encode(0x2729, 0x033c, 0x3300, 0, &[]);
        let (h, c) = decode(&pkt).unwrap();
        assert_eq!(h.start_code, 0x2729);
        assert_eq!(h.total_length, 10);
        assert_eq!(h.seqnum, 0x033c);
        assert_eq!(h.opcode1, 0x3300);
        assert_eq!(h.opcode2, 0);
        assert!(c.is_empty());
    }
}
