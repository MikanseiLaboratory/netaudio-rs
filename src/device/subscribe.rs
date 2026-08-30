//! Subscribe state machine: ARC 0x3010 → mDNS → flows-control 0x0100 → media.
//!
//! Same-TX channels share one unicast flow (Inferno `ChannelsSubscriber`):
//! one `0x0100` with several TX ids, interleaved PCM, `0x0102` when membership changes.

use super::Shared;
use super::info_mcast::InfoEvent;
use crate::media::rx::MediaCommand;
use crate::net::udp;
use crate::protocol::arc::{self, SubscribeReq};
use crate::protocol::flows_control::{
    self, DANTE_UNICAST_CHANNELS, FlowHandle, MAX_CHANNELS_IN_FLOW,
};
use crate::protocol::mdns as mdns_proto;
use crate::protocol::ports;
use crate::protocol::req_resp;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};
use tokio::sync::watch;

const DEBOUNCE: Duration = Duration::from_millis(15);
const RESOLVE_RETRY: Duration = Duration::from_secs(9);
const MDNS_TIMEOUT: Duration = Duration::from_secs(3);

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
    /// Local 1-based RX ids per flow slot (`None` = unused).
    pub slots: Vec<Option<u16>>,
    pub tx_ids: Vec<u16>,
    pub port: u16,
    pub ip: [u8; 4],
    pub latency_ns: u32,
    pub handle: Option<FlowHandle>,
    pub tx_addr: SocketAddrV4,
}

impl RxFlowInfo {
    pub fn channels(&self) -> Vec<u16> {
        self.slots.iter().copied().flatten().collect()
    }

    fn media_map(&self) -> Vec<Option<usize>> {
        self.slots
            .iter()
            .map(|id| id.map(|n| n as usize - 1))
            .collect()
    }

    fn used(&self) -> bool {
        self.slots.iter().any(Option::is_some)
    }
}

pub fn apply(shared: &Arc<Shared>, recs: Vec<SubscribeReq>) {
    let mut changed = Vec::new();
    for rec in recs {
        if rec.local_id == 0 {
            continue;
        }
        let idx = rec.local_id as usize - 1;
        let unsub = rec.tx_channel.is_none() && rec.tx_host.is_none();
        {
            let mut subs = shared.subs.lock().unwrap_or_else(|e| e.into_inner());
            if idx >= subs.len() {
                continue;
            }
            if unsub {
                subs[idx] = RxSub::default();
            } else {
                let same = subs[idx].tx_channel == rec.tx_channel
                    && subs[idx].tx_host == rec.tx_host
                    && subs[idx].flow_id.is_some();
                if !same {
                    subs[idx].tx_channel = rec.tx_channel.clone();
                    subs[idx].tx_host = rec.tx_host.clone();
                    subs[idx].status = arc::STATUS_ESTABLISHING;
                    subs[idx].flow_id = None;
                    if let (Some(ch), Some(h)) = (&rec.tx_channel, &rec.tx_host) {
                        log::info!("subscribe RX{} <- {ch}@{h}", rec.local_id);
                    }
                }
            }
        }
        shared.ring.set_patched(idx, !unsub);
        changed.push(idx);
    }
    notify_channels(shared, changed);
    shared.sub_pending.store(true, Ordering::Release);
    shared.sub_wake.notify_one();
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
    let mut changed = Vec::new();
    {
        let mut subs = shared.subs.lock().unwrap_or_else(|e| e.into_inner());
        for (i, s) in subs.iter_mut().enumerate() {
            if s.flow_id == Some(flow_id) && s.status != arc::STATUS_RX_UNICAST {
                s.status = arc::STATUS_RX_UNICAST;
                changed.push(i);
            }
        }
    }
    notify_channels(shared, changed);
}

pub async fn run(shared: Arc<Shared>, mut shutdown: watch::Receiver<bool>) {
    let mut retry = tokio::time::interval(RESOLVE_RETRY);
    retry.tick().await;
    loop {
        if shared.stopped.load(Ordering::Acquire) || *shutdown.borrow() {
            break;
        }
        if shared.sub_pending.swap(false, Ordering::AcqRel) {
            tokio::time::sleep(DEBOUNCE).await;
            let _ = shared.sub_pending.swap(false, Ordering::AcqRel);
            reconcile(&shared).await;
            continue;
        }
        tokio::select! {
            _ = shutdown.changed() => break,
            _ = shared.sub_wake.notified() => {}
            _ = retry.tick() => {
                if has_unresolved(&shared) {
                    shared.sub_pending.store(true, Ordering::Release);
                }
            }
        }
    }
}

async fn reconcile(shared: &Arc<Shared>) {
    prune_stale_slots(shared).await;

    let pending: Vec<(usize, String, String)> = {
        let subs = shared.subs.lock().unwrap_or_else(|e| e.into_inner());
        subs.iter()
            .enumerate()
            .filter_map(|(idx, s)| {
                if s.flow_id.is_some() {
                    return None;
                }
                Some((idx, s.tx_channel.clone()?, s.tx_host.clone()?))
            })
            .collect()
    };
    if pending.is_empty() {
        return;
    }

    let mut by_host: BTreeMap<String, Vec<(usize, String)>> = BTreeMap::new();
    for (idx, tx_ch, tx_host) in pending {
        by_host.entry(tx_host).or_default().push((idx, tx_ch));
    }

    for (tx_host, chans) in by_host {
        attach_host(shared, &tx_host, chans).await;
    }
}

fn slot_belongs(subs: &[RxSub], flow_id: u16, local_id: u16) -> bool {
    let idx = local_id as usize - 1;
    match subs.get(idx) {
        Some(s) => s.flow_id == Some(flow_id) && s.tx_channel.is_some() && s.tx_host.is_some(),
        None => false,
    }
}

async fn prune_stale_slots(shared: &Arc<Shared>) {
    let mut stale: Vec<(u16, Vec<Option<u16>>, Vec<u16>)> = Vec::new();
    let mut stop: Vec<u16> = Vec::new();
    {
        let subs = shared.subs.lock().unwrap_or_else(|e| e.into_inner());
        let mut flows = shared.rx_flows.lock().unwrap_or_else(|e| e.into_inner());
        for f in flows.iter_mut() {
            let mut changed = false;
            for i in 0..f.slots.len() {
                if let Some(local_id) = f.slots[i]
                    && !slot_belongs(&subs, f.flow_id, local_id)
                {
                    f.slots[i] = None;
                    if i < f.tx_ids.len() {
                        f.tx_ids[i] = 0;
                    }
                    changed = true;
                }
            }
            if !f.used() {
                stop.push(f.flow_id);
            } else if changed {
                stale.push((f.flow_id, f.slots.clone(), f.tx_ids.clone()));
            }
        }
    }
    for fid in stop {
        stop_flow(shared, fid).await;
    }
    for (fid, slots, tx_ids) in stale {
        let map: Vec<Option<usize>> = slots.iter().map(|id| id.map(|n| n as usize - 1)).collect();
        let _ = shared
            .media_tx
            .send(MediaCommand::UpdateFlow { id: fid, map });
        if let Some(h) = flow_handle(shared, fid) {
            let addr = flow_addr(shared, fid);
            let seq = shared.flows_seq.fetch_add(1, Ordering::Relaxed);
            let pkt = flows_control::encode_update(seq, h, &tx_ids);
            if let Some(addr) = addr {
                let _ = send_opcode(shared, addr, seq, flows_control::OP_UPDATE, &pkt).await;
            }
        }
    }
}

async fn attach_host(shared: &Arc<Shared>, tx_host: &str, chans: Vec<(usize, String)>) {
    let mut resolved: Vec<(usize, ResolvedTx)> = Vec::new();
    resolved.extend(resolve_batch(shared, tx_host, chans).await);

    let extra: Vec<(usize, String)> = {
        let known: BTreeSet<usize> = resolved.iter().map(|(i, _)| *i).collect();
        let subs = shared.subs.lock().unwrap_or_else(|e| e.into_inner());
        subs.iter()
            .enumerate()
            .filter_map(|(idx, s)| {
                if known.contains(&idx) || s.flow_id.is_some() {
                    return None;
                }
                if s.tx_host.as_deref() != Some(tx_host) {
                    return None;
                }
                Some((idx, s.tx_channel.clone()?))
            })
            .collect()
    };
    if !extra.is_empty() {
        resolved.extend(resolve_batch(shared, tx_host, extra).await);
    }

    if resolved.is_empty() {
        return;
    }

    let mut leftover = Vec::new();
    for (idx, r) in resolved {
        if try_fill_existing(shared, idx, &r).await {
            continue;
        }
        leftover.push((idx, r));
    }
    leftover.sort_by_key(|(_, r)| r.flows_addr.to_string());

    let mut i = 0;
    while i < leftover.len() {
        let addr = leftover[i].1.flows_addr;
        let mut chunk = Vec::new();
        while i < leftover.len() && leftover[i].1.flows_addr == addr {
            chunk.push(leftover[i].clone());
            i += 1;
        }
        let rx_n = shared.settings.rx_channels as usize;
        let mut start = 0;
        while start < chunk.len() {
            let first = &chunk[start].1;
            let remaining = chunk.len() - start;
            let width = flow_width(rx_n, first.tx_nchan, remaining, first.unconstrained);
            let end = (start + width).min(chunk.len());
            let piece: Vec<(usize, ResolvedTx)> = chunk[start..end].to_vec();
            start = end;
            request_new_flow(shared, piece, width).await;
        }
    }
}

async fn resolve_batch(
    shared: &Arc<Shared>,
    tx_host: &str,
    chans: Vec<(usize, String)>,
) -> Vec<(usize, ResolvedTx)> {
    let mut tasks = Vec::new();
    for (idx, tx_ch) in chans {
        let shared = shared.clone();
        let host = tx_host.to_string();
        tasks.push(tokio::spawn(async move {
            resolve_one(&shared, &host, idx, &tx_ch).await
        }));
    }
    let mut out = Vec::new();
    for task in tasks {
        if let Ok(Some(pair)) = task.await {
            out.push(pair);
        }
    }
    out
}

async fn resolve_one(
    shared: &Arc<Shared>,
    tx_host: &str,
    idx: usize,
    tx_ch: &str,
) -> Option<(usize, ResolvedTx)> {
    if !still_pending(shared, idx, tx_ch, tx_host) {
        return None;
    }
    set_status(shared, idx, arc::STATUS_ESTABLISHING);
    let mut last_err = None;
    for attempt in 0..3u32 {
        if !still_pending(shared, idx, tx_ch, tx_host) {
            return None;
        }
        match resolve_tx(shared, tx_host, tx_ch).await {
            Ok(r) => {
                if r.sample_rate != shared.settings.sample_rate {
                    log::warn!(
                        "subscribe rate mismatch: tx {} local {}",
                        r.sample_rate,
                        shared.settings.sample_rate
                    );
                    set_status(shared, idx, arc::STATUS_UNRESOLVED);
                    return None;
                }
                if !still_pending(shared, idx, tx_ch, tx_host) {
                    return None;
                }
                set_status(shared, idx, arc::STATUS_ESTABLISHING);
                return Some((idx, r));
            }
            Err(e) => {
                last_err = Some(e);
                if attempt + 1 < 3 {
                    tokio::time::sleep(Duration::from_millis(400)).await;
                }
            }
        }
    }
    if let Some(e) = last_err {
        log::warn!("subscribe resolve failed: {e}");
    }
    set_status(shared, idx, arc::STATUS_UNRESOLVED);
    None
}

struct FillUpdate {
    flow_id: u16,
    map: Vec<Option<usize>>,
    tx_ids: Vec<u16>,
    handle: FlowHandle,
    addr: SocketAddrV4,
}

async fn try_fill_existing(shared: &Shared, idx: usize, resolved: &ResolvedTx) -> bool {
    let local_id = (idx as u16) + 1;
    let found = {
        let mut flows = shared.rx_flows.lock().unwrap_or_else(|e| e.into_inner());
        let mut out = None;
        for f in flows.iter_mut() {
            if f.tx_addr != resolved.flows_addr {
                continue;
            }
            let Some(h) = f.handle else { continue };
            let Some(slot) = f.slots.iter().position(|s| s.is_none()) else {
                continue;
            };
            f.slots[slot] = Some(local_id);
            if slot < f.tx_ids.len() {
                f.tx_ids[slot] = resolved.tx_channel_id;
            }
            out = Some(FillUpdate {
                flow_id: f.flow_id,
                map: f.media_map(),
                tx_ids: f.tx_ids.clone(),
                handle: h,
                addr: f.tx_addr,
            });
            break;
        }
        out
    };
    let Some(u) = found else {
        return false;
    };
    {
        let mut subs = shared.subs.lock().unwrap_or_else(|e| e.into_inner());
        if idx < subs.len() {
            subs[idx].flow_id = Some(u.flow_id);
            subs[idx].status = arc::STATUS_ESTABLISHING;
        }
    }
    let _ = shared.media_tx.send(MediaCommand::UpdateFlow {
        id: u.flow_id,
        map: u.map,
    });
    let seq = shared.flows_seq.fetch_add(1, Ordering::Relaxed);
    let pkt = flows_control::encode_update(seq, u.handle, &u.tx_ids);
    log::info!("0x0102 {} ids={:?}", u.addr, u.tx_ids);
    match send_opcode(shared, u.addr, seq, flows_control::OP_UPDATE, &pkt).await {
        Ok(_) => log::info!("0x0102 ok ids={:?}", u.tx_ids),
        Err(e) => log::warn!("0x0102: {e}"),
    }
    true
}

async fn request_new_flow(shared: &Arc<Shared>, piece: Vec<(usize, ResolvedTx)>, width: usize) {
    if piece.is_empty() {
        return;
    }
    let first_addr = piece[0].1.flows_addr;
    let first_bits = sanitize_bits(piece[0].1.bits);
    let first_fpp_max = piece[0].1.fpp_max;
    let nchan = width;
    let wire_bytes = (first_bits / 8).max(1) as usize;
    let fpp = choose_fpp(nchan, wire_bytes, first_fpp_max);
    let latency_ns = piece[0]
        .1
        .latency_ns
        .max(shared.settings.rx_latency.as_nanos() as u32);

    let media = match udp::bind_media(shared.identity.ip) {
        Ok(s) => s,
        Err(e) => {
            log::warn!("media bind: {e}");
            for (idx, _) in &piece {
                set_status(shared, *idx, arc::STATUS_UNRESOLVED);
            }
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
            for (idx, _) in &piece {
                set_status(shared, *idx, arc::STATUS_UNRESOLVED);
            }
            return;
        }
    };

    let still: Vec<(usize, ResolvedTx)> = piece
        .into_iter()
        .filter(|(idx, r)| {
            let subs = shared.subs.lock().unwrap_or_else(|e| e.into_inner());
            match subs.get(*idx) {
                Some(s) => {
                    s.flow_id.is_none()
                        && s.tx_host.is_some()
                        && s.tx_channel.is_some()
                        && r.flows_addr == first_addr
                }
                None => false,
            }
        })
        .collect();
    if still.is_empty() {
        return;
    }

    let mut tx_ids = vec![0u16; width];
    for (i, (_, r)) in still.iter().enumerate() {
        if i >= width {
            break;
        }
        tx_ids[i] = r.tx_channel_id;
    }

    let flow_id = shared.next_flow_id.fetch_add(1, Ordering::Relaxed);
    let flow_name = format!("{}_{}", flow_id, shared.identity.process_id);
    let bytes = (first_bits / 8).max(1) as usize;
    let nchan = tx_ids.len();

    let mut live_slots = vec![None; nchan];
    let mut live_tx = tx_ids.clone();
    {
        let mut subs = shared.subs.lock().unwrap_or_else(|e| e.into_inner());
        for (i, (idx, r)) in still.iter().enumerate() {
            if i >= nchan {
                break;
            }
            if *idx < subs.len() && subs[*idx].flow_id.is_none() && subs[*idx].tx_channel.is_some()
            {
                subs[*idx].flow_id = Some(flow_id);
                subs[*idx].status = arc::STATUS_ESTABLISHING;
                live_slots[i] = Some((*idx as u16) + 1);
                live_tx[i] = r.tx_channel_id;
            }
        }
    }
    if live_slots.iter().all(Option::is_none) {
        return;
    }

    let map: Vec<Option<usize>> = live_slots
        .iter()
        .map(|id| id.map(|n| n as usize - 1))
        .collect();

    let seq = shared.flows_seq.fetch_add(1, Ordering::Relaxed);
    let pkt = flows_control::encode_request_flow(
        seq,
        shared.settings.sample_rate,
        first_bits,
        fpp,
        &tx_ids,
        &shared.identity.friendly_hostname,
        &flow_name,
        rx_port,
        shared.identity.ip.octets(),
    );
    log::info!(
        "0x0100 {} <- {}:{} ids={:?} rate={} bits={} fpp={} src=media pkt={}",
        first_addr,
        shared.identity.ip,
        rx_port,
        tx_ids,
        shared.settings.sample_rate,
        first_bits,
        fpp,
        to_hex(&pkt)
    );
    // Send 0x0100 from the media socket with std recv (no tokio/IOCP).
    // DVS then has a return path to this port; the media thread takes the
    // socket only after the reply so handshake bytes are not stolen.
    let (handle, tx_media_port) =
        match send_opcode_media(&media, first_addr, seq, flows_control::OP_REQUEST, &pkt).await {
            Ok(v) => v,
            Err(e) => {
                log::warn!("flows-control 0x0100: {e}");
                fail_new_flow(shared, flow_id, &still);
                return;
            }
        };
    log::info!(
        "0x0100 ok bits={first_bits} fpp={fpp} ids={tx_ids:?} src=media tx_media={tx_media_port:?}"
    );
    if let Err(e) = udp::prepare_media(&media) {
        log::warn!("media sockopt: {e}");
    }
    if shared
        .media_tx
        .send(MediaCommand::AddFlow {
            id: flow_id,
            sock: media,
            map,
            nchan,
            bytes_per_sample: bytes,
            sample_rate: shared.settings.sample_rate,
            tx_hint: first_addr,
            tx_media_port,
        })
        .is_err()
    {
        fail_new_flow(shared, flow_id, &still);
        return;
    }

    {
        let mut flows = shared.rx_flows.lock().unwrap_or_else(|e| e.into_inner());
        flows.push(RxFlowInfo {
            flow_id,
            sample_rate: shared.settings.sample_rate,
            bits: first_bits as u16,
            slots: live_slots,
            tx_ids: live_tx,
            port: rx_port,
            ip: shared.identity.ip.octets(),
            latency_ns,
            handle,
            tx_addr: first_addr,
        });
    }
}

fn fail_new_flow(shared: &Shared, flow_id: u16, still: &[(usize, ResolvedTx)]) {
    for (idx, _) in still {
        set_status(shared, *idx, arc::STATUS_UNRESOLVED);
    }
    let mut subs = shared.subs.lock().unwrap_or_else(|e| e.into_inner());
    for (idx, _) in still {
        if *idx < subs.len() && subs[*idx].flow_id == Some(flow_id) {
            subs[*idx].flow_id = None;
        }
    }
}

async fn stop_flow(shared: &Shared, flow_id: u16) {
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
        let _ = send_opcode(shared, f.tx_addr, seq, flows_control::OP_STOP, &pkt).await;
    }
}

fn flow_handle(shared: &Shared, flow_id: u16) -> Option<FlowHandle> {
    shared
        .rx_flows
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .find(|f| f.flow_id == flow_id)
        .and_then(|f| f.handle)
}

fn flow_addr(shared: &Shared, flow_id: u16) -> Option<SocketAddrV4> {
    shared
        .rx_flows
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .find(|f| f.flow_id == flow_id)
        .map(|f| f.tx_addr)
}

#[derive(Clone, Debug)]
struct ResolvedTx {
    flows_addr: SocketAddrV4,
    sample_rate: u32,
    tx_channel_id: u16,
    fpp_max: u16,
    bits: u32,
    tx_nchan: usize,
    latency_ns: u32,
    unconstrained: bool,
}

async fn resolve_tx(
    shared: &Arc<Shared>,
    tx_host: &str,
    tx_ch: &str,
) -> Result<ResolvedTx, String> {
    if let Some((ip, port)) = *shared
        .subscribe_override
        .lock()
        .unwrap_or_else(|e| e.into_inner())
    {
        let id = mdns_proto::parse_tx_id(tx_ch);
        return Ok(ResolvedTx {
            flows_addr: SocketAddrV4::new(ip, port),
            sample_rate: shared.settings.sample_rate,
            tx_channel_id: id,
            fpp_max: 16,
            bits: u32::from(shared.settings.bits.bits()),
            tx_nchan: 0,
            latency_ns: shared.settings.rx_latency.as_nanos() as u32,
            unconstrained: true,
        });
    }

    let variants = mdns_proto::chan_name_variants(tx_ch, tx_host);
    let host_local = format!("{tx_host}.local");
    let msg = tokio::task::spawn_blocking({
        let shared = shared.clone();
        let variants = variants.clone();
        let host_local = host_local.clone();
        move || {
            let mut names: Vec<&str> = variants.iter().map(String::as_str).collect();
            names.push(host_local.as_str());
            let mut questions: Vec<(&str, u16)> = Vec::new();
            for v in &variants {
                questions.push((v.as_str(), mdns_proto::QTYPE_SRV));
                questions.push((v.as_str(), mdns_proto::QTYPE_TXT));
            }
            questions.push((host_local.as_str(), mdns_proto::QTYPE_A));
            crate::net::mdns::query_names(&shared, &names, &questions, MDNS_TIMEOUT)
        }
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e.to_string())?;

    let mut flows_port = ports::FLOWS_CONTROL;
    let mut sample_rate = shared.settings.sample_rate;
    let mut fpp_max = 4u16;
    let mut fpp_min = 2u16;
    let mut ipv4 = None;
    let mut srv_target: Option<String> = None;
    let mut tx_channel_id = mdns_proto::parse_tx_id(tx_ch);
    let mut bits = u32::from(shared.settings.bits.bits());
    let mut tx_nchan = 0usize;
    let mut dbcp1 = 0u16;
    let mut latency_ns = 0u32;
    let mut have_chan = false;
    let mut got_fpp = false;
    let mut got_id = false;
    let mut got_enc = false;
    let mut got_nchan = false;
    let mut got_dbcp = false;
    let mut got_latency = false;
    let chan_ok = |name: &str| variants.iter().any(|v| mdns_proto::names_match(name, v));

    if let Some(msg) = msg.as_ref() {
        let dump: Vec<String> = msg
            .answers
            .iter()
            .chain(msg.additionals.iter())
            .map(|r| format!("{}#{}", r.name, r.rtype))
            .collect();
        log::info!("mDNS replies {dump:?}");
        for r in msg.answers.iter().chain(msg.additionals.iter()) {
            match &r.data {
                mdns_proto::RecordData::Srv { port, target, .. } => {
                    if chan_ok(&r.name) {
                        have_chan = true;
                        flows_port = *port;
                        srv_target = Some(target.clone());
                    }
                }
                mdns_proto::RecordData::A(a) => {
                    log::info!("mDNS A {} {a}", r.name);
                    let host_ok = mdns_proto::names_match(&r.name, &host_local);
                    let srv_ok = srv_target
                        .as_ref()
                        .map(|t| mdns_proto::names_match(&r.name, t))
                        .unwrap_or(false);
                    if host_ok || srv_ok {
                        ipv4 = Some(*a);
                    }
                }
                mdns_proto::RecordData::Txt(strs) => {
                    if !chan_ok(&r.name) {
                        continue;
                    }
                    have_chan = true;
                    if let Some(v) = mdns_proto::txt_get(strs, "rate")
                        && let Some(r) = mdns_proto::parse_txt_u32(v)
                    {
                        sample_rate = r;
                    }
                    if let Some(v) = mdns_proto::txt_get(strs, "fpp") {
                        match mdns_proto::parse_fpp(v) {
                            Some((mx, mn)) => {
                                fpp_max = mx;
                                fpp_min = mn;
                                got_fpp = true;
                            }
                            None => log::warn!("chan TXT fpp={v} ignored"),
                        }
                    }
                    if let Some(v) = mdns_proto::txt_get(strs, "id")
                        && let Some(n) = mdns_proto::parse_txt_u32(v)
                    {
                        tx_channel_id = n as u16;
                        got_id = true;
                    }
                    if let Some(v) = mdns_proto::txt_get(strs, "nchan")
                        && let Some(n) = mdns_proto::parse_txt_u32(v)
                    {
                        tx_nchan = n as usize;
                        got_nchan = true;
                    }
                    if let Some(v) = mdns_proto::txt_get(strs, "dbcp1")
                        && let Some(n) = mdns_proto::parse_txt_u32(v)
                    {
                        dbcp1 = n as u16;
                        got_dbcp = true;
                    }
                    if let Some(v) = mdns_proto::txt_get(strs, "latency_ns")
                        && let Some(n) = mdns_proto::parse_txt_u32(v)
                    {
                        latency_ns = n;
                        got_latency = true;
                    }
                    if let Some(v) =
                        mdns_proto::txt_get(strs, "enc").or_else(|| mdns_proto::txt_get(strs, "en"))
                    {
                        if let Some(n) = mdns_proto::parse_wire_bits(v) {
                            bits = n;
                            got_enc = true;
                        } else {
                            log::warn!("chan TXT enc={v} ignored (not 16/24/32)");
                        }
                    }
                    let dump: Vec<String> = strs
                        .iter()
                        .filter_map(|s| std::str::from_utf8(s).ok().map(str::to_owned))
                        .collect();
                    log::info!("mDNS {} TXT {dump:?}", r.name);
                }
                _ => {}
            }
        }
    }

    if ipv4.is_none() {
        let a_name = srv_target.clone().unwrap_or_else(|| host_local.clone());
        if let Some(msg) = msg.as_ref() {
            ipv4 = take_a(msg, &[a_name.as_str(), host_local.as_str()]);
        }
        if ipv4.is_none() {
            let follow = tokio::task::spawn_blocking({
                let shared = shared.clone();
                let a_name = a_name.clone();
                move || {
                    crate::net::mdns::query_names(
                        &shared,
                        &[&a_name],
                        &[(&a_name, mdns_proto::QTYPE_A)],
                        MDNS_TIMEOUT,
                    )
                }
            })
            .await
            .map_err(|e| e.to_string())?
            .map_err(|e| e.to_string())?;
            if let Some(m) = follow.as_ref() {
                ipv4 = take_a(m, &[a_name.as_str(), host_local.as_str()]);
            }
        }
    }

    let ipv4 = ipv4.ok_or_else(|| format!("mDNS miss A {tx_ch}@{tx_host}"))?;
    if !have_chan {
        return Err(format!(
            "mDNS chan TXT/SRV miss {tx_ch}@{tx_host} (host A only is not a Dante channel)"
        ));
    }
    if !got_fpp || !got_id || !got_enc || !got_nchan || tx_nchan == 0 || !got_dbcp || !got_latency {
        return Err(format!(
            "mDNS chan TXT incomplete {tx_ch}@{tx_host} fpp={got_fpp} id={got_id} enc={got_enc} nchan={got_nchan}/{tx_nchan} dbcp1={got_dbcp} latency_ns={got_latency}"
        ));
    }
    log::info!(
        "resolved {tx_ch}@{tx_host} → {ipv4}:{flows_port} id={tx_channel_id} bits={bits} fpp={fpp_max},{fpp_min} nchan={tx_nchan} dbcp1={dbcp1:#06x} latency_ns={latency_ns}"
    );
    Ok(ResolvedTx {
        flows_addr: SocketAddrV4::new(ipv4, flows_port),
        sample_rate,
        tx_channel_id,
        fpp_max,
        bits,
        tx_nchan,
        latency_ns,
        unconstrained: false,
    })
}

fn take_a(msg: &mdns_proto::Message, names: &[&str]) -> Option<Ipv4Addr> {
    for r in msg.answers.iter().chain(msg.additionals.iter()) {
        if let mdns_proto::RecordData::A(a) = &r.data
            && names.iter().any(|n| mdns_proto::names_match(&r.name, n))
        {
            return Some(*a);
        }
    }
    None
}

/// Dante unicast flows are 4 channels with unused slots 0 (Audinate, golden
/// capture). Inferno uses `nchan.min(rx)` which becomes 2 for a 2ch RX and DVS
/// then accepts 0x0100 without sending media. `needed` is loopback override only.
fn flow_width(_rx_channels: usize, tx_nchan: usize, needed: usize, unconstrained: bool) -> usize {
    if needed == 0 {
        return 1;
    }
    if unconstrained {
        return needed.min(MAX_CHANNELS_IN_FLOW);
    }
    DANTE_UNICAST_CHANNELS
        .min(tx_nchan.max(1))
        .clamp(1, MAX_CHANNELS_IN_FLOW)
}

fn choose_fpp(nchan: usize, bytes: usize, advertised_max: u16) -> u16 {
    let stride = nchan.saturating_mul(bytes).max(1);
    let mtu = (flows_control::MAX_PAYLOAD_BYTES / stride).min(usize::from(u16::MAX)) as u16;
    advertised_max.min(mtu).max(1)
}

fn sanitize_bits(n: u32) -> u32 {
    match n {
        16 | 24 | 32 => n,
        2..=4 => n * 8,
        _ => 24,
    }
}

enum FcErr {
    Code(u16),
    Msg(String),
}

impl std::fmt::Display for FcErr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FcErr::Code(c) => write!(f, "flows-control error {c:#06x}"),
            FcErr::Msg(s) => f.write_str(s),
        }
    }
}

impl From<&str> for FcErr {
    fn from(s: &str) -> Self {
        FcErr::Msg(s.to_owned())
    }
}

impl From<String> for FcErr {
    fn from(s: String) -> Self {
        FcErr::Msg(s)
    }
}

async fn send_opcode_media(
    sock: &std::net::UdpSocket,
    dest: SocketAddrV4,
    seq: u16,
    opcode1: u16,
    pkt: &[u8],
) -> Result<(Option<FlowHandle>, Option<u16>), FcErr> {
    let sock = sock.try_clone().map_err(|e| e.to_string())?;
    let pkt = pkt.to_vec();
    tokio::task::spawn_blocking(move || send_opcode_std(sock, dest, seq, opcode1, &pkt))
        .await
        .map_err(|e| e.to_string())?
}

fn send_opcode_std(
    sock: std::net::UdpSocket,
    dest: SocketAddrV4,
    seq: u16,
    opcode1: u16,
    pkt: &[u8],
) -> Result<(Option<FlowHandle>, Option<u16>), FcErr> {
    let _ = sock.set_nonblocking(false);
    let _ = sock.set_read_timeout(Some(Duration::from_millis(250)));
    sock.send_to(pkt, SocketAddr::V4(dest))
        .map_err(|e| e.to_string())?;
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut buf = [0u8; 1500];
    loop {
        if Instant::now() >= deadline {
            return Err("flows-control timeout".into());
        }
        match sock.recv_from(&mut buf) {
            Ok((n, _)) => {
                let Some((h, content)) = req_resp::decode(&buf[..n]) else {
                    continue;
                };
                if h.seqnum != seq || h.opcode1 != opcode1 {
                    continue;
                }
                if h.opcode2 != 1 {
                    log::warn!(
                        "flows-control error op={opcode1:#06x} code={:#06x} reply={}",
                        h.opcode2,
                        to_hex(&buf[..n])
                    );
                    return Err(FcErr::Code(h.opcode2));
                }
                log::info!(
                    "flows-control ok op={opcode1:#06x} reply={}",
                    to_hex(&buf[..n])
                );
                return Ok((
                    flows_control::parse_handle(content),
                    flows_control::parse_tx_media_port(content),
                ));
            }
            Err(e)
                if e.kind() == std::io::ErrorKind::TimedOut
                    || e.kind() == std::io::ErrorKind::WouldBlock =>
            {
                continue;
            }
            Err(e) => return Err(e.to_string().into()),
        }
    }
}

async fn send_opcode(
    shared: &Shared,
    dest: SocketAddrV4,
    seq: u16,
    opcode1: u16,
    pkt: &[u8],
) -> Result<Option<FlowHandle>, FcErr> {
    let sock = udp::std_to_tokio(udp::bind_querier(shared.identity.ip).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    sock.connect(SocketAddr::V4(dest))
        .await
        .map_err(|e| e.to_string())?;
    sock.send(pkt).await.map_err(|e| e.to_string())?;
    let mut buf = [0u8; 1500];
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    loop {
        let now = tokio::time::Instant::now();
        if now >= deadline {
            return Err("flows-control timeout".into());
        }
        let n = tokio::time::timeout(deadline - now, sock.recv(&mut buf))
            .await
            .map_err(|_| "flows-control timeout".to_string())?
            .map_err(|e| e.to_string())?;
        let Some((h, content)) = req_resp::decode(&buf[..n]) else {
            continue;
        };
        if h.seqnum != seq || h.opcode1 != opcode1 {
            log::warn!(
                "spurious flows-control packet op={:#06x} seq={}",
                h.opcode1,
                h.seqnum
            );
            continue;
        }
        if h.opcode2 != 1 {
            log::warn!(
                "flows-control error op={opcode1:#06x} code={:#06x} reply={}",
                h.opcode2,
                to_hex(&buf[..n])
            );
            return Err(FcErr::Code(h.opcode2));
        }
        log::info!(
            "flows-control ok op={opcode1:#06x} reply={}",
            to_hex(&buf[..n])
        );
        return Ok(flows_control::parse_handle(content));
    }
}

fn to_hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(s, "{b:02x}");
    }
    s
}

fn still_pending(shared: &Shared, idx: usize, tx_ch: &str, tx_host: &str) -> bool {
    let subs = shared.subs.lock().unwrap_or_else(|e| e.into_inner());
    match subs.get(idx) {
        Some(s) => {
            s.flow_id.is_none()
                && s.tx_channel.as_deref() == Some(tx_ch)
                && s.tx_host.as_deref() == Some(tx_host)
        }
        None => false,
    }
}

fn set_status(shared: &Shared, idx: usize, status: u32) {
    {
        let mut subs = shared.subs.lock().unwrap_or_else(|e| e.into_inner());
        if idx < subs.len() {
            subs[idx].status = status;
        }
    }
    notify_channels(shared, [idx]);
}

fn has_unresolved(shared: &Shared) -> bool {
    shared
        .subs
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .any(|s| s.tx_host.is_some() && s.flow_id.is_none())
}

fn notify_channels(shared: &Shared, idxs: impl IntoIterator<Item = usize>) {
    let v: Vec<usize> = idxs.into_iter().collect();
    if !v.is_empty() {
        let _ = shared.info_tx.send(InfoEvent::ChannelChange(v));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn width_override_is_needed_count() {
        assert_eq!(flow_width(8, 64, 2, true), 2);
        assert_eq!(flow_width(8, 0, 1, true), 1);
    }

    #[test]
    fn width_is_dante_unicast_four() {
        assert_eq!(flow_width(2, 8, 1, false), 4);
        assert_eq!(flow_width(2, 64, 1, false), 4);
        assert_eq!(flow_width(2, 64, 2, false), 4);
        assert_eq!(flow_width(2, 2, 1, false), 2);
        assert_eq!(flow_width(8, 8, 2, false), 4);
        assert_eq!(flow_width(8, 2, 2, false), 2);
        assert_eq!(flow_width(8, 8, 8, false), 4);
    }

    #[test]
    fn fpp_is_advertised_max_capped_by_mtu() {
        assert_eq!(choose_fpp(2, 3, 4), 4);
        assert_eq!(choose_fpp(2, 3, 32), 32);
        assert_eq!(sanitize_bits(3), 24);
        assert_eq!(sanitize_bits(24), 24);
    }
}
