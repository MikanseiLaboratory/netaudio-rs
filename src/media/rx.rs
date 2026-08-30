//! Dedicated OS thread: flow UDP recv, keepalive, ring write.

use super::keepalive;
use crate::clock::overlay::Source;
use crate::device::Shared;
use crate::protocol::media as media_proto;
use crate::protocol::pcm;
use crate::protocol::ports;
use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, UdpSocket};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

pub enum MediaCommand {
    AddFlow {
        id: u16,
        sock: UdpSocket,
        map: Vec<Option<usize>>,
        nchan: usize,
        bytes_per_sample: usize,
        sample_rate: u32,
        /// Flows-control address of the TX (log / silence warning only).
        tx_hint: SocketAddrV4,
        /// TX media UDP from the 0x0100 handle, if present.
        tx_media_port: Option<u16>,
    },
    UpdateFlow {
        id: u16,
        map: Vec<Option<usize>>,
    },
    RemoveFlow {
        id: u16,
    },
    Shutdown,
}

struct Flow {
    id: u16,
    sock: UdpSocket,
    map: Vec<Option<usize>>,
    nchan: usize,
    bytes_per_sample: usize,
    sample_rate: u32,
    tx_hint: SocketAddrV4,
    tx_media_port: Option<u16>,
    last_source: Option<SocketAddr>,
    next_keepalive: Instant,
    saw_packet: bool,
    silent_until: Option<Instant>,
    logged_undecoded: bool,
}

pub struct MediaThread {
    join: Option<JoinHandle<()>>,
}

impl MediaThread {
    pub fn start(
        shared: Arc<Shared>,
        rx: Receiver<MediaCommand>,
    ) -> Result<Self, crate::device::Error> {
        let join = thread::Builder::new()
            .name("netaudio-media".into())
            .spawn(move || run(shared, rx))?;
        Ok(Self { join: Some(join) })
    }

    pub fn join(mut self) {
        if let Some(h) = self.join.take() {
            let _ = h.join();
        }
    }

    pub fn detach(mut self) {
        let _ = self.join.take();
    }
}

fn run(shared: Arc<Shared>, rx: Receiver<MediaCommand>) {
    let mut flows: HashMap<u16, Flow> = HashMap::new();
    let mut buf = [0u8; 2048];
    loop {
        if shared.stopped.load(std::sync::atomic::Ordering::Acquire) {
            break;
        }
        match rx.recv_timeout(Duration::from_millis(1)) {
            Ok(MediaCommand::Shutdown) | Err(RecvTimeoutError::Disconnected) => break,
            Ok(MediaCommand::AddFlow {
                id,
                sock,
                map,
                nchan,
                bytes_per_sample,
                sample_rate,
                tx_hint,
                tx_media_port,
            }) => {
                let _ = crate::net::udp::prepare_media(&sock);
                if let Ok(p) = sock.local_addr() {
                    shared.ring.note_port(p.port());
                    log::info!(
                        "media bind {p} nchan={nchan} bps={bytes_per_sample} tx={tx_hint} tx_media={tx_media_port:?}"
                    );
                    if let Some(port) = tx_media_port {
                        log::info!("keepalive will probe {}:{}", tx_hint.ip(), port);
                    }
                }
                punch_dvs_tx_media(&sock, *tx_hint.ip());
                flows.insert(
                    id,
                    Flow {
                        id,
                        sock,
                        map,
                        nchan,
                        bytes_per_sample,
                        sample_rate,
                        tx_hint,
                        tx_media_port,
                        last_source: None,
                        next_keepalive: Instant::now(),
                        saw_packet: false,
                        silent_until: Some(Instant::now() + Duration::from_secs(2)),
                        logged_undecoded: false,
                    },
                );
            }
            Ok(MediaCommand::UpdateFlow { id, map }) => {
                if let Some(flow) = flows.get_mut(&id) {
                    flow.map = map;
                }
            }
            Ok(MediaCommand::RemoveFlow { id }) => {
                flows.remove(&id);
            }
            Err(RecvTimeoutError::Timeout) => {}
        }

        let now = Instant::now();
        let rate = shared.settings.sample_rate;
        let latency = (shared.settings.rx_latency.as_nanos() as u64).saturating_mul(rate as u64)
            / 1_000_000_000;

        for flow in flows.values_mut() {
            loop {
                match flow.sock.recv_from(&mut buf) {
                    Ok((n, src)) => {
                        flow.last_source = Some(src);
                        flow.silent_until = None;
                        if n == 2 && buf[0] == 0x13 && buf[1] == 0x37 {
                            continue;
                        }
                        let Some((hdr, pcm_bytes)) = media_proto::decode(&buf[..n]) else {
                            if !flow.logged_undecoded {
                                flow.logged_undecoded = true;
                                log::warn!("media undecodable {n} bytes from {src}");
                            }
                            continue;
                        };
                        if !flow.saw_packet {
                            log::info!("media first packet {n} bytes from {src}");
                        }
                        let t1 = hdr.t1_ns(flow.sample_rate);
                        shared
                            .overlay
                            .observe(t1, shared.overlay.local_ns(), Source::Media);
                        let overlay_now = shared.overlay.now_ns();
                        let read_pos = crate::device::ns_to_samples_pub(overlay_now, rate)
                            .wrapping_sub(latency);
                        let ts = hdr.sample_index(flow.sample_rate);
                        let samples =
                            pcm::promote_interleaved(pcm_bytes, flow.nchan, flow.bytes_per_sample);
                        shared
                            .ring
                            .write_packet(ts, read_pos, &flow.map, &samples, flow.nchan);
                        if !flow.saw_packet {
                            flow.saw_packet = true;
                            crate::device::subscribe::mark_flow_receiving(&shared, flow.id);
                        }
                        if let Ok(guard) = shared.wakeup.lock()
                            && let Some(cb) = guard.as_ref()
                        {
                            let cb = Arc::clone(cb);
                            drop(guard);
                            cb();
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                    Err(_) => break,
                }
            }
            if now >= flow.next_keepalive {
                if let Some(src) = flow.last_source {
                    let _ = flow.sock.send_to(&keepalive::KEEPALIVE, src);
                } else {
                    punch_media_path(&flow.sock, flow.tx_hint, flow.tx_media_port);
                }
                flow.next_keepalive = now + Duration::from_millis(keepalive::INTERVAL_MS);
            }
            if let Some(until) = flow.silent_until
                && now >= until
            {
                flow.silent_until = None;
                if let Ok(local) = flow.sock.local_addr() {
                    log::warn!(
                        "no media UDP on {local} from {} yet; allow inbound UDP, confirm PTP is received, and check Dante Controller shows Receiving not Pending",
                        flow.tx_hint.ip()
                    );
                }
            }
        }
        shared.overlay.mark_unlocked_if_stale(1_000_000_000);
    }
}

fn punch_media_path(sock: &UdpSocket, tx_hint: SocketAddrV4, tx_media_port: Option<u16>) {
    let ip = *tx_hint.ip();
    if let Some(p) = tx_media_port {
        let _ = sock.send_to(&keepalive::KEEPALIVE, SocketAddr::from((ip, p)));
    }
    let _ = sock.send_to(&keepalive::KEEPALIVE, SocketAddr::V4(tx_hint));
    let _ = sock.send_to(
        &keepalive::KEEPALIVE,
        SocketAddr::from((ip, ports::MEDIA_PORT_START)),
    );
    if let Ok(local) = sock.local_addr() {
        let _ = sock.send_to(&keepalive::KEEPALIVE, SocketAddr::from((ip, local.port())));
    }
}

/// DVS software TX binds ephemeral UDP around 60880 (apec3/ptp), not 31823.
fn punch_dvs_tx_media(sock: &UdpSocket, ip: Ipv4Addr) {
    log::info!("keepalive probe {ip}:60880..=60920");
    for p in 60880u16..=60920 {
        let _ = sock.send_to(&keepalive::KEEPALIVE, SocketAddr::from((ip, p)));
    }
}

impl Drop for MediaThread {
    fn drop(&mut self) {
        let _ = self.join.take();
    }
}
