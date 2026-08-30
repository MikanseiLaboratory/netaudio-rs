//! PTPv1 thread: receive Sync/Follow_Up, send Delay_Req.

use super::overlay::{OverlayClock, Source};
use crate::device::{Error, Shared};
use crate::net::udp;
use crate::protocol::ports;
use crate::protocol::ptp_v1::{self, CONTROL_FOLLOW_UP, CONTROL_SYNC};
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

struct PendingSync {
    uuid: [u8; 6],
    seq: u16,
    t2: u64,
}

pub fn start(shared: Arc<Shared>) -> Result<JoinHandle<()>, Error> {
    let ip = shared.identity.ip;
    let event = udp::bind_ptp(ip, ports::PTP_EVENT)?;
    let general = udp::bind_ptp(ip, ports::PTP_GENERAL)?;
    event
        .set_read_timeout(Some(Duration::from_millis(200)))
        .ok();
    general
        .set_read_timeout(Some(Duration::from_millis(200)))
        .ok();

    Ok(thread::Builder::new()
        .name("netaudio-ptp".into())
        .spawn(move || {
            let overlay = Arc::clone(&shared.overlay);
            let subdomain = shared.settings.ptp_subdomain;
            let id = shared.identity.device_id;
            let uuid = [id[2], id[3], id[4], id[5], id[6], id[7]];
            let group = SocketAddr::from((
                Ipv4Addr::new(
                    ports::PTP_GROUP[0],
                    ports::PTP_GROUP[1],
                    ports::PTP_GROUP[2],
                    ports::PTP_GROUP[3],
                ),
                ports::PTP_EVENT,
            ));
            let mut pending: Option<PendingSync> = None;
            let mut buf = [0u8; 1500];
            let mut logged_ptp = false;
            let mut saw_foreign = false;
            let mut warned_silence = false;
            let mut logged_subdomain = false;
            let mut delay_seq: u16 = 1;
            let mut template: Option<Vec<u8>> = None;
            let started = Instant::now();
            loop {
                if shared.stopped.load(std::sync::atomic::Ordering::Acquire) {
                    break;
                }
                if let Ok((n, src)) = event.recv_from(&mut buf) {
                    if !logged_ptp {
                        logged_ptp = true;
                        log::info!("PTP event {n} bytes from {src}");
                    }
                    let foreign = match src {
                        SocketAddr::V4(v) => *v.ip() != ip,
                        SocketAddr::V6(_) => true,
                    };
                    if foreign && !saw_foreign {
                        saw_foreign = true;
                        log::info!("PTP from network {src}");
                    }
                    if template.is_none() && n >= ptp_v1::HEADER_LEN {
                        template = Some(buf[..n.min(ptp_v1::DELAY_REQ_LEN)].to_vec());
                    }
                    if handle_event(
                        &overlay,
                        &subdomain,
                        &buf[..n],
                        &mut pending,
                        &mut logged_subdomain,
                    ) && foreign
                    {
                        let pkt = ptp_v1::encode_delay_req(
                            &subdomain,
                            uuid,
                            delay_seq,
                            template.as_deref(),
                        );
                        delay_seq = delay_seq.wrapping_add(1);
                        let _ = event.send_to(&pkt, src);
                        let _ = event.send_to(&pkt, group);
                    }
                }
                if let Ok((n, src)) = general.recv_from(&mut buf) {
                    if !logged_ptp {
                        logged_ptp = true;
                        log::info!("PTP general {n} bytes from {src}");
                    }
                    let foreign = match src {
                        SocketAddr::V4(v) => *v.ip() != ip,
                        SocketAddr::V6(_) => true,
                    };
                    if foreign && !saw_foreign {
                        saw_foreign = true;
                        log::info!("PTP from network {src}");
                    }
                    handle_general(
                        &overlay,
                        &subdomain,
                        &buf[..n],
                        &mut pending,
                        &mut logged_subdomain,
                    );
                }
                if !saw_foreign && !warned_silence && started.elapsed() >= Duration::from_secs(3) {
                    warned_silence = true;
                    log::warn!(
                        "no PTPv1 UDP on 319/320 from the LAN yet; DVS may not send media until this device is in the clock domain"
                    );
                }
            }
            let _ = SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0);
        })?)
}

fn handle_event(
    overlay: &OverlayClock,
    subdomain: &[u8; 16],
    pkt: &[u8],
    pending: &mut Option<PendingSync>,
    logged_subdomain: &mut bool,
) -> bool {
    let Some(h) = ptp_v1::decode_header(pkt) else {
        return false;
    };
    if !ptp_v1::subdomain_matches(&h.subdomain, subdomain) {
        if !*logged_subdomain {
            *logged_subdomain = true;
            log::warn!(
                "PTP subdomain mismatch got={} want={}",
                ptp_v1::subdomain_label(&h.subdomain),
                ptp_v1::subdomain_label(subdomain)
            );
        }
        return false;
    }
    if h.control != CONTROL_SYNC {
        return false;
    }
    let t2 = overlay.local_ns();
    if let Some(ts) = ptp_v1::origin_timestamp(pkt)
        && !ts.is_zero()
    {
        overlay.observe(ts.as_ns(), t2, Source::Ptp);
        *pending = None;
        return true;
    }
    *pending = Some(PendingSync {
        uuid: h.source_uuid,
        seq: h.sequence_id,
        t2,
    });
    true
}

fn handle_general(
    overlay: &OverlayClock,
    subdomain: &[u8; 16],
    pkt: &[u8],
    pending: &mut Option<PendingSync>,
    logged_subdomain: &mut bool,
) {
    let Some(h) = ptp_v1::decode_header(pkt) else {
        return;
    };
    if !ptp_v1::subdomain_matches(&h.subdomain, subdomain) {
        if !*logged_subdomain {
            *logged_subdomain = true;
            log::warn!(
                "PTP subdomain mismatch got={} want={}",
                ptp_v1::subdomain_label(&h.subdomain),
                ptp_v1::subdomain_label(subdomain)
            );
        }
        return;
    }
    if h.control != CONTROL_FOLLOW_UP {
        return;
    }
    let Some((assoc, ts)) = ptp_v1::follow_up(pkt) else {
        return;
    };
    let Some(p) = pending.take() else {
        return;
    };
    if p.uuid != h.source_uuid || p.seq != assoc {
        *pending = Some(p);
        return;
    }
    overlay.observe(ts.as_ns(), p.t2, Source::Ptp);
}
