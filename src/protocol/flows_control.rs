//! Flows-control client packets (UDP 4455).

use super::HEADER_RR;
use super::buf::BeWriter;
use super::req_resp;

pub const START_CODE: u16 = 0x1102;
pub const OP_REQUEST: u16 = 0x0100;
pub const OP_STOP: u16 = 0x0101;
pub const OP_UPDATE: u16 = 0x0102;

pub const ERR_EXPIRED: u16 = 0x0103;
pub const ERR_TOO_MANY: u16 = 0x0315;
pub const ERR_RATE: u16 = 0x0301;
/// Inferno TX: bad channel index or fpp above the device max.
pub const ERR_BAD_REQUEST: u16 = 0x0302;
/// TX rejected fpp / bit depth / channel list (observed on hardware 0x0100 replies).
pub const ERR_PARAMS: u16 = 0x0314;

/// Advertised in ARC 0x1000. Inferno chunks a TX into flows of this width.
pub const MAX_CHANNELS_IN_FLOW: usize = 8;
/// Audinate unicast flows are 4 channels (empty slots padded with 0). DVS TXT
/// `nchan=8` is not the unicast flow width.
pub const DANTE_UNICAST_CHANNELS: usize = 4;
/// Inferno `MAX_PAYLOAD_BYTES`: cap fpp so PCM fits one UDP datagram.
pub const MAX_PAYLOAD_BYTES: usize = 1400;

pub type FlowHandle = [u8; 6];

#[allow(clippy::too_many_arguments)]
pub fn encode_request_flow(
    seqnum: u16,
    sample_rate: u32,
    bits: u32,
    fpp: u16,
    tx_channel_ids: &[u16],
    rx_hostname: &str,
    rx_flow_name: &str,
    rx_port: u16,
    rx_ip: [u8; 4],
) -> Vec<u8> {
    let n = tx_channel_ids.len();
    let strings_off = 48 + 2 * n; // packet-relative
    let extra_off = 0x1C + 2 * n;
    let mut body = BeWriter::new();
    body.u16(strings_off as u16);
    body.u32(sample_rate);
    body.u32(bits);
    body.u16(1);
    body.u16(n as u16);
    let sock_slot = body.len();
    body.u16(0); // socket offset placeholder
    for id in tx_channel_ids {
        body.u16(*id);
    }
    body.u16(extra_off as u16);
    body.u16(0x0A00);
    body.u16(0x0002);
    body.u16(fpp);
    let name_slot = body.len();
    body.u16(0); // flow name offset
    body.zeros(12);
    debug_assert_eq!(body.len(), 38 + 2 * n);

    let hostname_off = HEADER_RR + body.len();
    body.cstr(rx_hostname);
    let flow_name_off = HEADER_RR + body.len();
    body.cstr(rx_flow_name);
    while !(HEADER_RR + body.len()).is_multiple_of(8) {
        body.u8(0);
    }
    let sock_off = HEADER_RR + body.len();
    body.u16(0x0802);
    body.u16(rx_port);
    body.bytes(&rx_ip);

    let mut raw = body.into_inner();
    raw[sock_slot..sock_slot + 2].copy_from_slice(&(sock_off as u16).to_be_bytes());
    raw[name_slot..name_slot + 2].copy_from_slice(&(flow_name_off as u16).to_be_bytes());
    let _ = hostname_off;
    req_resp::encode(START_CODE, seqnum, OP_REQUEST, 0, &raw)
}

pub fn encode_stop(seqnum: u16, handle: FlowHandle) -> Vec<u8> {
    req_resp::encode(START_CODE, seqnum, OP_STOP, 0, &handle)
}

pub fn encode_update(seqnum: u16, handle: FlowHandle, tx_channel_ids: &[u16]) -> Vec<u8> {
    let mut c = BeWriter::new();
    c.bytes(&handle);
    c.u16(tx_channel_ids.len() as u16);
    for id in tx_channel_ids {
        c.u16(*id);
    }
    req_resp::encode(START_CODE, seqnum, OP_UPDATE, 0, &c.into_inner())
}

pub fn parse_handle(content: &[u8]) -> Option<FlowHandle> {
    if content.len() < 6 {
        return None;
    }
    let mut h = [0u8; 6];
    h.copy_from_slice(&content[..6]);
    Some(h)
}

/// DVS 0x0100 ok is 14 bytes. Handle bytes 4..6 are often the TX media UDP port.
pub fn parse_tx_media_port(content: &[u8]) -> Option<u16> {
    let h = parse_handle(content)?;
    let p = u16::from_be_bytes([h[4], h[5]]);
    (p >= 1024).then_some(p)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::req_resp;

    #[test]
    fn request_flow_golden_n4() {
        // public capture: n=4, bits=24, fpp=16, ids 1,0,0,0, host A32-000001, flow 1
        let golden = "1102005000000100000000380000bb8000000018000100040048000100000000000000240a000002001000430000000000000000000000004133322d303030303031003100000000080238010afe4e0b";
        let pkt = encode_request_flow(
            0,
            48_000,
            24,
            16,
            &[1, 0, 0, 0],
            "A32-000001",
            "1",
            0x3801,
            [10, 254, 78, 11],
        );
        assert_eq!(hex::encode(&pkt), golden);
        let (h, _) = req_resp::decode(&pkt).unwrap();
        assert_eq!(h.start_code, START_CODE);
        assert_eq!(h.opcode1, OP_REQUEST);
        assert_eq!(h.total_length, 0x50);
    }

    #[test]
    fn request_flow_socket_matches_args() {
        let pkt = encode_request_flow(
            1,
            48_000,
            24,
            48,
            &[1, 0, 0, 0],
            "akizuki-test-rx",
            "1_42",
            60077,
            [192, 168, 3, 24],
        );
        let (_, content) = req_resp::decode(&pkt).unwrap();
        assert_eq!(u16::from_be_bytes(content[12..14].try_into().unwrap()), 4);
        let needle = {
            let mut v = vec![0x08, 0x02];
            v.extend_from_slice(&60077u16.to_be_bytes());
            v.extend_from_slice(&[192, 168, 3, 24]);
            v
        };
        assert!(
            pkt.windows(needle.len()).any(|w| w == needle),
            "missing 0x0802 socket in {}",
            hex::encode(&pkt)
        );
    }

    #[test]
    fn tx_media_port_from_dvs_ok() {
        let content = hex::decode("000100017c4f0001000100000000").unwrap();
        assert_eq!(parse_tx_media_port(&content), Some(0x7c4f));
        assert_eq!(parse_handle(&content).unwrap()[..6], [0, 1, 0, 1, 0x7c, 0x4f]);
    }
}
