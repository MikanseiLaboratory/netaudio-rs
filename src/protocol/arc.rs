//! ARC (UDP 4440) codecs.

use super::buf::{cstr_at, BeWriter};
use super::req_resp::{self, Header};
use super::{HEADER_RR, OPCODE2_FAIL, OPCODE2_MORE, OPCODE2_OK, OPCODE2_UNSUPPORTED};

pub const OP_COUNTS: u16 = 0x1000;
pub const OP_SET_NAME: u16 = 0x1001;
pub const OP_GET_NAME: u16 = 0x1002;
pub const OP_NAMES: u16 = 0x1003;
pub const OP_1100: u16 = 0x1100;
pub const OP_1102: u16 = 0x1102;
pub const OP_TX_CH: u16 = 0x2000;
pub const OP_TX_FRIENDLY: u16 = 0x2010;
pub const OP_RENAME_TX: u16 = 0x2013;
pub const OP_TX_FLOWS: u16 = 0x2200;
pub const OP_CREATE_MCAST: u16 = 0x2201;
pub const OP_DELETE_MCAST: u16 = 0x2202;
pub const OP_2320: u16 = 0x2320;
pub const OP_RX_CH: u16 = 0x3000;
pub const OP_RENAME_RX: u16 = 0x3001;
pub const OP_SUBSCRIBE: u16 = 0x3010;
pub const OP_UNSUB_ONE: u16 = 0x3014;
pub const OP_RX_FLOWS: u16 = 0x3200;
pub const OP_PORT_RANGES: u16 = 0x3300;

pub const STATUS_NONE: u32 = 0;
pub const STATUS_UNRESOLVED: u32 = 1;
pub const STATUS_ESTABLISHING: u32 = 8;
pub const STATUS_RX_UNICAST: u32 = 0x0101_0009;
pub const STATUS_RX_MCAST: u32 = 0x0101_000A;
pub const STATUS_CLOCK_MISMATCH: u32 = 0x0000_001B;

pub const PCM_TYPE: u16 = 0x000E;

#[derive(Clone, Debug)]
pub struct DeviceNames {
    pub board: String,
    pub revision: String,
    pub friendly: String,
    pub factory: String,
}

pub fn encode_counts(req: Header, tx: u16, rx: u16) -> Vec<u8> {
    let mut c = BeWriter::new();
    c.u8(0);
    c.u8(0); // flags2: no TX rename / multicast
    c.u16(tx);
    c.u16(rx);
    c.u16(4);
    c.u16(8); // max channels in flow
    c.u16(8);
    c.u16(32);
    c.u16(32);
    c.u16(tx.saturating_add(rx));
    c.u16(1);
    c.u16(1);
    c.zeros(14);
    req_resp::encode_ok(req, &c.into_inner())
}

pub fn encode_device_name(req: Header, name: &str) -> Vec<u8> {
    let mut c = BeWriter::new();
    c.cstr(name);
    req_resp::encode_ok(req, &c.into_inner())
}

pub fn encode_names(req: Header, n: &DeviceNames) -> Vec<u8> {
    let mut c = BeWriter::new();
    c.zeros(38);
    let mut put = |s: &str| -> u16 {
        let off = (HEADER_RR + c.len()) as u16;
        c.cstr(s);
        off
    };
    let board = put(&n.board);
    let rev = put(&n.revision);
    let friendly = put(&n.friendly);
    let factory = put(&n.factory);
    c.patch_u16(6, board);
    c.patch_u16(8, rev);
    c.patch_u16(12, friendly);
    c.patch_u16(14, factory);
    c.patch_u16(16, friendly);
    c.patch_u16(30, 0x2729);
    c.patch_u16(34, 0x1102);
    req_resp::encode_ok(req, &c.into_inner())
}

pub fn encode_zeros(req: Header, n: usize) -> Vec<u8> {
    req_resp::encode_ok(req, &vec![0u8; n])
}

pub fn encode_empty_page(req: Header) -> Vec<u8> {
    req_resp::encode_ok(req, &req_resp::empty_page())
}

pub fn encode_unsupported(req: Header) -> Vec<u8> {
    req_resp::encode_code(req, OPCODE2_UNSUPPORTED, &[])
}

pub fn encode_fail(req: Header) -> Vec<u8> {
    req_resp::encode_code(req, OPCODE2_FAIL, &[])
}

pub fn encode_port_ranges(req: Header) -> Vec<u8> {
    let mut c = BeWriter::new();
    c.u16(0x3800);
    c.u16(0x397F);
    c.u16(0x3980);
    c.u16(0x39FF);
    req_resp::encode_ok(req, &c.into_inner())
}

#[derive(Clone, Debug)]
pub struct RxChannel {
    pub id: u16,
    pub friendly: String,
    pub tx_channel: Option<String>,
    pub tx_host: Option<String>,
    pub status: u32,
}

pub fn encode_rx_channels(
    req: Header,
    start: usize,
    sample_rate: u32,
    bits: u16,
    channels: &[RxChannel],
) -> Vec<u8> {
    const ITEM: usize = 20;
    const PCM: usize = 16;
    let slice = if start >= channels.len() {
        &[][..]
    } else {
        &channels[start..]
    };
    let space = (slice.len().min(16)) as u8;
    let mut c = BeWriter::new();
    c.u8(space);
    c.u8(0); // count placeholder
    if space == 0 {
        return req_resp::encode_ok(req, &c.into_inner());
    }
    c.zeros(space as usize * ITEM);
    let pcm_off = (HEADER_RR + c.len()) as u16;
    c.u32(sample_rate);
    c.u8(1);
    c.u8(1);
    c.u16(bits);
    c.u16(0x0400);
    c.u16(bits);
    c.u16(bits);
    c.u16(PCM_TYPE);
    let mut count = 0u8;
    let mut have_more = false;
    for (i, ch) in slice.iter().enumerate() {
        if i >= space as usize {
            have_more = true;
            break;
        }
        if c.len() >= 800 {
            have_more = true;
            break;
        }
        let friendly_off = (HEADER_RR + c.len()) as u16;
        c.cstr(&ch.friendly);
        let (tx_name_off, tx_host_off) = match (&ch.tx_channel, &ch.tx_host) {
            (Some(n), Some(h)) => {
                let no = (HEADER_RR + c.len()) as u16;
                c.cstr(n);
                let ho = (HEADER_RR + c.len()) as u16;
                c.cstr(h);
                (no, ho)
            }
            _ => (0, 0),
        };
        let item_at = 2 + i * ITEM;
        let mut item = BeWriter::new();
        item.u16(ch.id);
        item.u16(0x0006);
        item.u16(pcm_off);
        item.u16(tx_name_off);
        item.u16(tx_host_off);
        item.u16(friendly_off);
        item.u32(ch.status);
        item.u32(0);
        let bytes = item.into_inner();
        c.as_slice(); // keep writer
                      // patch item into reserved slots via a temp copy
        let mut raw = c.into_inner();
        raw[item_at..item_at + ITEM].copy_from_slice(&bytes);
        c = BeWriter::new();
        c.bytes(&raw);
        count += 1;
        let _ = PCM;
    }
    let mut raw = c.into_inner();
    raw[1] = count;
    let code = if have_more { OPCODE2_MORE } else { OPCODE2_OK };
    req_resp::encode(req.start_code, req.seqnum, req.opcode1, code, &raw)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SubscribeReq {
    pub local_id: u16,
    pub tx_channel: Option<String>,
    pub tx_host: Option<String>,
}

/// Parse 0x3010. Honor `count` (byte 1), not `space`.
pub fn parse_subscribe(packet: &[u8], content: &[u8]) -> Vec<SubscribeReq> {
    if content.len() < 2 {
        return Vec::new();
    }
    let count = content[1] as usize;
    let mut out = Vec::new();
    let mut off = 2usize;
    for _ in 0..count {
        if off + 6 > content.len() {
            break;
        }
        let local_id = u16::from_be_bytes([content[off], content[off + 1]]);
        let name_off = u16::from_be_bytes([content[off + 2], content[off + 3]]);
        let host_off = u16::from_be_bytes([content[off + 4], content[off + 5]]);
        off += 6;
        if local_id == 0 {
            continue;
        }
        out.push(SubscribeReq {
            local_id,
            tx_channel: cstr_at(packet, name_off).map(str::to_owned),
            tx_host: cstr_at(packet, host_off).map(str::to_owned),
        });
    }
    out
}

#[derive(Clone, Debug)]
pub struct RenameReq {
    pub local_id: u16,
    pub new_name: String,
}

pub fn parse_rename_rx(packet: &[u8], content: &[u8]) -> Vec<RenameReq> {
    if content.len() < 2 {
        return Vec::new();
    }
    let count = content[1] as usize;
    let mut out = Vec::new();
    let mut off = 2usize;
    for _ in 0..count {
        if off + 4 > content.len() {
            break;
        }
        let id = u16::from_be_bytes([content[off], content[off + 1]]);
        let name_off = u16::from_be_bytes([content[off + 2], content[off + 3]]);
        off += 4;
        if id == 0 {
            continue;
        }
        if let Some(n) = cstr_at(packet, name_off) {
            out.push(RenameReq {
                local_id: id,
                new_name: n.to_owned(),
            });
        }
    }
    out
}

/// NAC 0x3014: channel id at content [4..6).
pub fn parse_unsub_one(content: &[u8]) -> Option<u16> {
    if content.len() < 6 {
        return None;
    }
    let id = u16::from_be_bytes([content[4], content[5]]);
    if id == 0 {
        None
    } else {
        Some(id)
    }
}

#[derive(Clone, Debug)]
pub struct RxFlowView {
    pub flow_id: u16,
    pub sample_rate: u32,
    pub bits: u16,
    pub channels: Vec<u16>, // 1-based local RX ids in this flow
    pub port: u16,
    pub ip: [u8; 4],
    pub latency_ns: u32,
}

pub fn encode_rx_flows(req: Header, start: usize, flows: &[RxFlowView]) -> Vec<u8> {
    let slice = if start >= flows.len() {
        &[][..]
    } else {
        &flows[start..]
    };
    let space = (slice.len().min(16)) as u8;
    let mut c = BeWriter::new();
    c.u8(space);
    c.u8(0);
    if space == 0 {
        return req_resp::encode_ok(req, &c.into_inner());
    }
    c.zeros(space as usize * 2); // u16 offsets
    let mut count = 0u8;
    let mut have_more = false;
    for (i, f) in slice.iter().enumerate() {
        if i >= space as usize {
            have_more = true;
            break;
        }
        while c.len() % 4 != 0 {
            c.u8(0);
        }
        let desc_off = (HEADER_RR + c.len()) as u16;
        let nchan = f.channels.len() as u16;
        c.u16(f.flow_id);
        c.u16(1);
        c.u32(f.sample_rate);
        c.u16(0);
        c.u16(f.bits);
        c.u16(1);
        c.u16(nchan);
        c.u16(1); // words_per_bitmask
        let sock_slot = c.len();
        c.u16(0); // socket offset placeholder
        for _ in 0..nchan {
            c.u16(0); // bitmask offset placeholder
        }
        let foot_slot = c.len();
        c.u16(0); // descriptor2 offset
        while c.len() % 4 != 0 {
            c.u8(0);
        }
        let sock_off = (HEADER_RR + c.len()) as u16;
        c.u16(0x0802);
        c.u16(f.port);
        c.bytes(&f.ip);
        let mut mask = 0u16;
        for id in &f.channels {
            if *id > 0 {
                let bit = (*id - 1) as u32;
                if bit < 16 {
                    mask |= 1 << bit;
                }
            }
        }
        let mask_off = (HEADER_RR + c.len()) as u16;
        c.u16(mask);
        let d2_off = (HEADER_RR + c.len()) as u16;
        c.u16(9);
        c.u16(1);
        c.u16(0x0800);
        c.u16(0);
        c.u32(f.latency_ns);
        c.u32(0);
        let mut raw = c.into_inner();
        raw[2 + i * 2..2 + i * 2 + 2].copy_from_slice(&desc_off.to_be_bytes());
        // header is 18 bytes then socket offset at content relative...
        // FlowDescriptorHeader 18 bytes starts at desc_off - HEADER_RR
        let header_at = desc_off as usize - HEADER_RR;
        let sock_field = header_at + 16;
        raw[sock_field..sock_field + 2].copy_from_slice(&sock_off.to_be_bytes());
        let mut bit_field = header_at + 18;
        for _ in 0..nchan {
            raw[bit_field..bit_field + 2].copy_from_slice(&mask_off.to_be_bytes());
            bit_field += 2;
        }
        let _ = (sock_slot, foot_slot);
        raw[bit_field..bit_field + 2].copy_from_slice(&d2_off.to_be_bytes());
        c = BeWriter::new();
        c.bytes(&raw);
        count += 1;
    }
    let mut raw = c.into_inner();
    raw[1] = count;
    let code = if have_more { OPCODE2_MORE } else { OPCODE2_OK };
    req_resp::encode(req.start_code, req.seqnum, req.opcode1, code, &raw)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::req_resp::{self, Header};

    fn hdr(op: u16, seq: u16) -> Header {
        Header {
            start_code: 0x2729,
            total_length: 10,
            seqnum: seq,
            opcode1: op,
            opcode2: 0,
        }
    }

    #[test]
    fn port_ranges_golden() {
        let req_hex = "2729000a033c33000000";
        let resp_hex = "27290012033c330000013800397f398039ff";
        let req = hex::decode(req_hex).unwrap();
        let (h, c) = req_resp::decode(&req).unwrap();
        assert_eq!(h.opcode1, OP_PORT_RANGES);
        assert!(c.is_empty());
        let out = encode_port_ranges(h);
        assert_eq!(hex::encode(&out), resp_hex);
    }

    #[test]
    fn counts_36_bytes() {
        let pkt = encode_counts(hdr(OP_COUNTS, 1), 0, 8);
        let (_, c) = req_resp::decode(&pkt).unwrap();
        assert_eq!(c.len(), 36);
        assert_eq!(&c[2..6], &[0, 0, 0, 8]);
        assert_eq!(&c[8..10], &[0, 8]);
    }

    #[test]
    fn encode_rx_one_channel() {
        let pkt = encode_rx_channels(
            hdr(OP_RX_CH, 2),
            0,
            48_000,
            24,
            &[RxChannel {
                id: 1,
                friendly: "Rx 1".into(),
                tx_channel: None,
                tx_host: None,
                status: STATUS_NONE,
            }],
        );
        let (h, c) = req_resp::decode(&pkt).unwrap();
        assert_eq!(h.opcode2, crate::protocol::OPCODE2_OK);
        assert_eq!(c[0], 1);
        assert_eq!(c[1], 1);
        assert_eq!(&c[2..4], &[0, 1]); // id
    }

    #[test]
    fn subscribe_honors_count() {
        // space=2, count=1, one record + extra zeros
        let mut content = vec![2, 1, 0, 1, 0, 0, 0, 0];
        content.extend_from_slice(&[0u8; 6]);
        let pkt = req_resp::encode(0x2729, 1, OP_SUBSCRIBE, 0, &content);
        let recs = parse_subscribe(&pkt, &req_resp::decode(&pkt).unwrap().1);
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0].local_id, 1);
        assert!(recs[0].tx_channel.is_none());
    }
}
