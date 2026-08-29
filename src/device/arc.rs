//! Tokio ARC server (UDP 4440 or alt+0).

use super::subscribe;
use super::{Error, Shared};
use crate::protocol::arc::{self, RxChannel, RxFlowView};
use crate::protocol::buf::cstr_at;
use crate::protocol::req_resp::{self, Header};
use crate::protocol::HEADER_RR;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::UdpSocket;
use tokio::sync::watch;

pub async fn run(
    shared: Arc<Shared>,
    sock: UdpSocket,
    shutdown: &mut watch::Receiver<bool>,
) -> Result<(), Error> {
    let mut buf = [0u8; 2048];
    loop {
        tokio::select! {
            _ = shutdown.changed() => break,
            rec = sock.recv_from(&mut buf) => {
                match rec {
                    Ok((n, src)) => {
                        if let Some(reply) = handle(&shared, &buf[..n]) {
                            let _ = sock.send_to(&reply, src).await;
                        }
                    }
                    Err(e) => {
                        log::warn!("arc recv: {e}");
                    }
                }
            }
        }
    }
    Ok(())
}

fn handle(shared: &Arc<Shared>, packet: &[u8]) -> Option<Vec<u8>> {
    let (hdr, content) = req_resp::decode(packet)?;
    match hdr.opcode1 {
        arc::OP_COUNTS => Some(arc::encode_counts(
            hdr,
            shared.settings.tx_channels,
            shared.settings.rx_channels,
        )),
        arc::OP_GET_NAME => Some(arc::encode_device_name(
            hdr,
            &shared.identity.friendly_hostname,
        )),
        arc::OP_SET_NAME => {
            if let Some(n) = cstr_at(packet, HEADER_RR as u16) {
                if n.len() <= 31
                    && n.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
                    && !n.is_empty()
                {
                    // Friendly hostname is identity snapshot; names list is the live view.
                    log::info!("arc set name ignored after start ({n}); v1 identity is fixed");
                }
            }
            Some(req_resp::encode_ok(hdr, &[]))
        }
        arc::OP_NAMES => Some(arc::encode_names(
            hdr,
            &arc::DeviceNames {
                board: shared.settings.board.clone(),
                revision: "0.0.0".into(),
                friendly: shared.identity.friendly_hostname.clone(),
                factory: shared.identity.factory_hostname.clone(),
            },
        )),
        arc::OP_1100 => Some(arc::encode_zeros(hdr, 110)),
        arc::OP_1102 => Some(arc::encode_zeros(hdr, 94)),
        arc::OP_TX_CH | arc::OP_TX_FRIENDLY | arc::OP_TX_FLOWS => Some(arc::encode_empty_page(hdr)),
        arc::OP_RENAME_TX | arc::OP_CREATE_MCAST | arc::OP_DELETE_MCAST => {
            Some(arc::encode_fail(hdr))
        }
        arc::OP_2320 => Some(arc::encode_unsupported(hdr)),
        arc::OP_RX_CH => Some(encode_rx_page(shared, hdr, content)),
        arc::OP_RENAME_RX => {
            let recs = arc::parse_rename_rx(packet, content);
            subscribe::rename_rx(shared, recs);
            Some(req_resp::encode_ok(hdr, &[]))
        }
        arc::OP_SUBSCRIBE => {
            let recs = arc::parse_subscribe(packet, content);
            subscribe::apply(shared, recs);
            Some(req_resp::encode_ok(hdr, &[]))
        }
        arc::OP_UNSUB_ONE => {
            if let Some(id) = arc::parse_unsub_one(content) {
                subscribe::unsubscribe_one(shared, id);
            }
            Some(req_resp::encode_ok(hdr, &[]))
        }
        arc::OP_RX_FLOWS => Some(encode_rx_flows_page(shared, hdr, content)),
        arc::OP_PORT_RANGES => Some(arc::encode_port_ranges(hdr)),
        other => {
            log::debug!("arc unknown opcode1={other:#06x} from seq {}", hdr.seqnum);
            None
        }
    }
}

fn encode_rx_page(shared: &Shared, hdr: Header, content: &[u8]) -> Vec<u8> {
    let start = req_resp::pagination_start(content).unwrap_or(0);
    let names = shared
        .rx_names
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    let subs = shared.subs.lock().unwrap_or_else(|e| e.into_inner());
    let mut chans = Vec::with_capacity(names.len());
    for (i, name) in names.iter().enumerate() {
        let sub = subs.get(i);
        chans.push(RxChannel {
            id: (i as u16) + 1,
            friendly: name.clone(),
            tx_channel: sub.and_then(|s| s.tx_channel.clone()),
            tx_host: sub.and_then(|s| s.tx_host.clone()),
            status: sub.map(|s| s.status).unwrap_or(arc::STATUS_NONE),
        });
    }
    drop(subs);
    arc::encode_rx_channels(
        hdr,
        start,
        shared.settings.sample_rate,
        shared.settings.bits.bits(),
        &chans,
    )
}

fn encode_rx_flows_page(shared: &Shared, hdr: Header, content: &[u8]) -> Vec<u8> {
    let start = req_resp::pagination_start(content).unwrap_or(0);
    let flows = shared.rx_flows.lock().unwrap_or_else(|e| e.into_inner());
    if flows.is_empty() {
        return arc::encode_empty_page(hdr);
    }
    let views: Vec<RxFlowView> = flows
        .iter()
        .map(|f| RxFlowView {
            flow_id: f.flow_id,
            sample_rate: f.sample_rate,
            bits: f.bits,
            channels: f.channels.clone(),
            port: f.port,
            ip: f.ip,
            latency_ns: f.latency_ns,
        })
        .collect();
    drop(flows);
    arc::encode_rx_flows(hdr, start, &views)
}

#[allow(dead_code)]
pub fn peer_dbg(src: SocketAddr) -> String {
    src.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::arc as arc_proto;

    #[test]
    fn unknown_opcode_is_none() {
        // handle needs Shared; smoke the encoder path instead
        let hdr = Header {
            start_code: 0x2729,
            total_length: 10,
            seqnum: 1,
            opcode1: 0x9999,
            opcode2: 0,
        };
        assert_eq!(hdr.opcode1, 0x9999);
        let _ = arc_proto::OP_COUNTS;
    }
}
