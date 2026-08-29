//! CMC (UDP 8800).

use super::buf::BeWriter;
use super::req_resp::{self, Header};

pub const OP_ADVERTISEMENT: u16 = 0x1001;

pub fn encode_advertisement(
    req: Header,
    process_id: u16,
    device_id: [u8; 8],
    ipv4: [u8; 4],
    info_port: u16,
) -> Vec<u8> {
    let mut c = BeWriter::new();
    c.u16(process_id);
    c.bytes(&device_id);
    c.u16(1);
    c.u16(0);
    c.bytes(&ipv4);
    c.u16(info_port);
    c.u16(0);
    debug_assert_eq!(c.len(), 22);
    req_resp::encode_ok(req, &c.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::req_resp::Header;

    #[test]
    fn advertisement_22_bytes() {
        let req = Header {
            start_code: 0x1200,
            total_length: 10,
            seqnum: 1,
            opcode1: OP_ADVERTISEMENT,
            opcode2: 0,
        };
        let pkt = encode_advertisement(req, 42, [1, 2, 3, 4, 5, 6, 7, 8], [192, 168, 1, 10], 8700);
        let (_, c) = crate::protocol::req_resp::decode(&pkt).unwrap();
        assert_eq!(c.len(), 22);
        assert_eq!(&c[0..2], &[0, 42]);
        assert_eq!(&c[18..20], &[0x21, 0xFC]); // 8700
    }
}
