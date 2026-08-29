//! Subscribe state machine: ARC 0x3010 → mDNS → flows-control 0x0100 → media.

use super::Shared;
use super::info_mcast::InfoEvent;
use crate::media::rx::MediaCommand;
use crate::net::udp;
use crate::protocol::arc::{self, SubscribeReq};
use crate::protocol::flows_control::{self, FlowHandle};
use crate::protocol::mdns as mdns_proto;
use crate::protocol::ports;
use crate::protocol::req_resp;
use std::net::{SocketAddr, SocketAddrV4};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

#[derive(Clone, Debug, Default)]
pub struct RxSub {
    pub tx_channel: Option<String>,
    pub tx_host: Option<String>,
    pub status: u32,
    pub flow_id: Option<u16>,
}

#[derive(Clone, Debug)]
pub struct RxFlowInfo {
    pub flow_id: u16,
    pub sample_rate: u32,
    pub bits: u16,
    pub channels: Vec<u16>,
    pub port: u16,
    pub ip: [u8; 4],
    pub latency_ns: u32,
    pub handle: Option<FlowHandle>,
    pub tx_addr: SocketAddrV4,
}

pub fn apply(shared: &Arc<Shared>, recs: Vec<SubscribeReq>) {
    for rec in recs {
        if rec.local_id == 0 {
            continue;
        }
        let idx = rec.local_id as usize - 1;
        let unsub = rec.tx_channel.is_none() && rec.tx_host.is_none();
        let old_flow = {
            let mut subs = shared.subs.lock().unwrap_or_else(|e| e.into_inner());
            if idx >= subs.len() {
                continue;
            }
            if unsub {
                let old = subs[idx].flow_id;
                subs[idx] = RxSub::default();
                old
            } else {
                subs[idx].tx_channel = rec.tx_channel.clone();
                subs[idx].tx_host = rec.tx_host.clone();
                subs[idx].status = arc::STATUS_UNRESOLVED;
                None
            }
        };
        if unsub {
            shared.ring.set_patched(idx, false);
            if let Some(fid) = old_flow {
                stop_flow(shared, fid);
            }
            continue;
        }
        shared.ring.set_patched(idx, true);
        let shared = Arc::clone(shared);
        tokio::spawn(async move {
            establish(shared, rec).await;
        });
    }
}

pub fn unsubscribe_one(shared: &Arc<Shared>, local_id: u16) {
    apply(
        shared,
        vec![SubscribeReq {
            local_id,
            tx_channel: None,
            tx_host: None,
        }],
    );
}

pub fn rename_rx(shared: &Shared, recs: Vec<arc::RenameReq>) {
    let mut changed = Vec::new();
    {
        let mut names = shared.rx_names.lock().unwrap_or_else(|e| e.into_inner());
        for r in recs {
            if r.local_id == 0 {
                continue;
            }
            let idx = r.local_id as usize - 1;
            if idx < names.len() {
                names[idx] = r.new_name;
                changed.push(idx);
            }
        }
    }
    if !changed.is_empty() {
        let _ = shared.info_tx.send(InfoEvent::ChannelChange(changed));
    }
}

pub fn mark_flow_receiving(shared: &Shared, flow_id: u16) {
    let mut subs = shared.subs.lock().unwrap_or_else(|e| e.into_inner());
    for s in subs.iter_mut() {
        if s.flow_id == Some(flow_id) {
            s.status = arc::STATUS_RX_UNICAST;
        }
    }
}

fn stop_flow(shared: &Shared, flow_id: u16) {
    let info = {
        let mut flows = shared.rx_flows.lock().unwrap_or_else(|e| e.into_inner());
        let pos = flows.iter().position(|f| f.flow_id == flow_id);
        pos.map(|i| flows.remove(i))
    };
    let _ = shared
        .media_tx
        .send(MediaCommand::RemoveFlow { id: flow_id });
    if let Some(f) = info
        && let Some(h) = f.handle
    {
        let seq = shared.flows_seq.fetch_add(1, Ordering::Relaxed);
        let pkt = flows_control::encode_stop(seq, h);
        let ip = shared.identity.ip;
        let addr = SocketAddr::V4(f.tx_addr);
        std::thread::spawn(move || {
            if let Ok(sock) = udp::bind_querier(ip) {
                let _ = sock.send_to(&pkt, addr);
            }
        });
    }
}

async fn establish(shared: Arc<Shared>, rec: SubscribeReq) {
    let Some(tx_ch) = rec.tx_channel.clone() else {
        return;
    };
    let Some(tx_host) = rec.tx_host.clone() else {
        return;
    };
    let local_id = rec.local_id;
    let idx = local_id as usize - 1;

    let resolved = match resolve_tx(&shared, &tx_host, &tx_ch).await {
        Ok(r) => r,
        Err(e) => {
            log::warn!("subscribe resolve failed: {e}");
            set_status(&shared, idx, arc::STATUS_UNRESOLVED);
            return;
        }
    };
    if resolved.sample_rate != shared.settings.sample_rate {
        log::warn!(
            "subscribe rate mismatch: tx {} local {}",
            resolved.sample_rate,
            shared.settings.sample_rate
        );
        set_status(&shared, idx, arc::STATUS_UNRESOLVED);
        return;
    }

    set_status(&shared, idx, arc::STATUS_ESTABLISHING);

    let media = match udp::bind_in_range(
        shared.identity.ip,
        ports::MEDIA_PORT_START,
        ports::MEDIA_PORT_END,
    ) {
        Ok(s) => s,
        Err(e) => {
            log::warn!("media bind: {e}");
            set_status(&shared, idx, arc::STATUS_UNRESOLVED);
            return;
        }
    };
    if let Err(e) = udp::prepare_media(&media) {
        log::warn!("media sockopt: {e}");
    }
    let rx_port = match media.local_addr() {
        Ok(a) => a.port(),
        Err(e) => {
            log::warn!("media local_addr: {e}");
            set_status(&shared, idx, arc::STATUS_UNRESOLVED);
            return;
        }
    };

    let flow_id = shared.next_flow_id.fetch_add(1, Ordering::Relaxed);
    let nchan = 1usize;
    let fpp = choose_fpp(
        shared.settings.sample_rate,
        shared.settings.rx_latency,
        nchan,
        shared.settings.bits.bytes(),
        resolved.fpp_max,
        resolved.fpp_min,
    );
    let seq = shared.flows_seq.fetch_add(1, Ordering::Relaxed);
    let flow_name = format!("{flow_id}");
    let pkt = flows_control::encode_request_flow(
        seq,
        shared.settings.sample_rate,
        u32::from(shared.settings.bits.bits()),
        fpp,
        &[resolved.tx_channel_id],
        &shared.identity.friendly_hostname,
        &flow_name,
        rx_port,
        shared.identity.ip.octets(),
    );

    let handle = match send_flows_request(&shared, resolved.flows_addr, seq, &pkt).await {
        Ok(h) => h,
        Err(e) => {
            log::warn!("flows-control 0x0100: {e}");
            set_status(&shared, idx, arc::STATUS_UNRESOLVED);
            return;
        }
    };

    {
        let mut flows = shared.rx_flows.lock().unwrap_or_else(|e| e.into_inner());
        flows.push(RxFlowInfo {
            flow_id,
            sample_rate: shared.settings.sample_rate,
            bits: shared.settings.bits.bits(),
            channels: vec![local_id],
            port: rx_port,
            ip: shared.identity.ip.octets(),
            latency_ns: shared.settings.rx_latency.as_nanos() as u32,
            handle,
            tx_addr: resolved.flows_addr,
        });
    }
    {
        let mut subs = shared.subs.lock().unwrap_or_else(|e| e.into_inner());
        if idx < subs.len() {
            subs[idx].flow_id = Some(flow_id);
            subs[idx].status = arc::STATUS_ESTABLISHING;
        }
    }

    let map = vec![Some(idx)];
    if shared
        .media_tx
        .send(MediaCommand::AddFlow {
            id: flow_id,
            sock: media,
            map,
            nchan,
            bytes_per_sample: shared.settings.bits.bytes(),
            sample_rate: shared.settings.sample_rate,
        })
        .is_err()
    {
        set_status(&shared, idx, arc::STATUS_UNRESOLVED);
    }
}

struct ResolvedTx {
    flows_addr: SocketAddrV4,
    sample_rate: u32,
    tx_channel_id: u16,
    fpp_max: u16,
    fpp_min: u16,
}

async fn resolve_tx(shared: &Shared, tx_host: &str, tx_ch: &str) -> Result<ResolvedTx, String> {
    if let Some((ip, port)) = *shared
        .subscribe_override
        .lock()
        .unwrap_or_else(|e| e.into_inner())
    {
        let id = tx_ch.parse::<u16>().unwrap_or(1);
        return Ok(ResolvedTx {
            flows_addr: SocketAddrV4::new(ip, port),
            sample_rate: shared.settings.sample_rate,
            tx_channel_id: id,
            fpp_max: 16,
            fpp_min: 2,
        });
    }

    let ip = shared.identity.ip;
    let chan_name = mdns_proto::chan_instance(tx_ch, tx_host);
    let arc_name = mdns_proto::service_instance(tx_host, mdns_proto::ARC_SERVICE);
    let timeout = Duration::from_millis(800);

    let chan_msg = tokio::task::spawn_blocking({
        let chan_name = chan_name.clone();
        move || crate::net::mdns::query(ip, &chan_name, mdns_proto::QTYPE_ANY, timeout)
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e.to_string())?;

    let arc_msg = tokio::task::spawn_blocking({
        let arc_name = arc_name.clone();
        move || crate::net::mdns::query(ip, &arc_name, mdns_proto::QTYPE_ANY, timeout)
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e.to_string())?;

    let mut flows_port = ports::FLOWS_CONTROL;
    let mut sample_rate = shared.settings.sample_rate;
    let mut fpp_max = 16u16;
    let mut fpp_min = 2u16;
    let mut ipv4 = None;
    let mut tx_channel_id = tx_ch.parse::<u16>().unwrap_or(1);

    let mut consider = |msg: &mdns_proto::Message| {
        for r in msg.answers.iter().chain(msg.additionals.iter()) {
            match &r.data {
                mdns_proto::RecordData::Srv { port, .. } => {
                    if mdns_proto::names_match(&r.name, &chan_name) {
                        flows_port = *port;
                    }
                }
                mdns_proto::RecordData::A(a) => {
                    ipv4 = Some(*a);
                }
                mdns_proto::RecordData::Txt(strs) => {
                    if let Some(v) = mdns_proto::txt_get(strs, "rate")
                        && let Ok(r) = v.parse()
                    {
                        sample_rate = r;
                    }
                    if let Some(v) = mdns_proto::txt_get(strs, "fpp") {
                        let (mx, mn) = mdns_proto::parse_fpp(v);
                        fpp_max = mx;
                        fpp_min = mn;
                    }
                    if let Some(v) = mdns_proto::txt_get(strs, "id")
                        && let Ok(n) = v.parse()
                    {
                        tx_channel_id = n;
                    }
                }
                _ => {}
            }
        }
    };
    if let Some(m) = chan_msg.as_ref() {
        consider(m);
    }
    if let Some(m) = arc_msg.as_ref() {
        consider(m);
    }
    let ipv4 = ipv4.ok_or_else(|| format!("no A record for {tx_host}"))?;
    Ok(ResolvedTx {
        flows_addr: SocketAddrV4::new(ipv4, flows_port),
        sample_rate,
        tx_channel_id,
        fpp_max,
        fpp_min,
    })
}

fn choose_fpp(
    rate: u32,
    latency: Duration,
    nchan: usize,
    bytes: usize,
    advertised_max: u16,
    advertised_min: u16,
) -> u16 {
    let stride = nchan.saturating_mul(bytes).max(1);
    let mtu = ((1400 - 9) / stride) as u16;
    let lat = (u128::from(rate) * latency.as_nanos() / 4 / 1_000_000_000) as u16;
    let mut v = advertised_max.min(mtu).min(lat).min(32);
    let min = advertised_min.max(2);
    if v < min {
        v = min;
    }
    v.max(2)
}

async fn send_flows_request(
    shared: &Shared,
    dest: SocketAddrV4,
    seq: u16,
    pkt: &[u8],
) -> Result<Option<FlowHandle>, String> {
    let sock = udp::std_to_tokio(udp::bind_querier(shared.identity.ip).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    sock.send_to(pkt, SocketAddr::V4(dest))
        .await
        .map_err(|e| e.to_string())?;
    let mut buf = [0u8; 1500];
    let rec = tokio::time::timeout(Duration::from_secs(3), sock.recv_from(&mut buf))
        .await
        .map_err(|_| "flows-control timeout".to_string())?
        .map_err(|e| e.to_string())?;
    let (h, content) = req_resp::decode(&buf[..rec.0]).ok_or("bad flows-control reply")?;
    if h.seqnum != seq || h.opcode1 != flows_control::OP_REQUEST {
        return Err("flows-control reply mismatch".into());
    }
    if h.opcode2 == flows_control::ERR_RATE
        || h.opcode2 == flows_control::ERR_TOO_MANY
        || h.opcode2 == flows_control::ERR_EXPIRED
    {
        return Err(format!("flows-control error {:#06x}", h.opcode2));
    }
    if h.opcode2 != 1 && h.opcode2 != 0 {
        return Err(format!("flows-control opcode2 {:#06x}", h.opcode2));
    }
    Ok(flows_control::parse_handle(content))
}

fn set_status(shared: &Shared, idx: usize, status: u32) {
    let mut subs = shared.subs.lock().unwrap_or_else(|e| e.into_inner());
    if idx < subs.len() {
        subs[idx].status = status;
    }
}
