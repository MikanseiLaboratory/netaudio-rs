//! PTPv1 listen-only thread.

use super::overlay::{OverlayClock, Source};
use crate::device::{Error, Shared};
use crate::net::udp;
use crate::protocol::ports;
use crate::protocol::ptp_v1::{self, CONTROL_FOLLOW_UP, CONTROL_SYNC};
use std::net::{Ipv4Addr, SocketAddrV4};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

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
            let mut pending: Option<PendingSync> = None;
            let mut buf = [0u8; 1500];
            let mut logged_ptp = false;
            loop {
                if shared.stopped.load(std::sync::atomic::Ordering::Acquire) {
                    break;
                }
                if let Ok((n, src)) = event.recv_from(&mut buf) {
                    if !logged_ptp {
                        logged_ptp = true;
                        log::info!("PTP event {n} bytes from {src}");
                    }
                    handle_event(&overlay, &subdomain, &buf[..n], &mut pending);
                }
                if let Ok((n, src)) = general.recv_from(&mut buf) {
                    if !logged_ptp {
                        logged_ptp = true;
                        log::info!("PTP general {n} bytes from {src}");
                    }
                    handle_general(&overlay, &subdomain, &buf[..n], &mut pending);
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
) {
    let Some(h) = ptp_v1::decode_header(pkt) else {
        return;
    };
    if !ptp_v1::subdomain_matches(&h.subdomain, subdomain) {
        return;
    }
    if h.control != CONTROL_SYNC {
        return;
    }
    let t2 = overlay.local_ns();
    if let Some(ts) = ptp_v1::origin_timestamp(pkt)
        && !ts.is_zero()
    {
        overlay.observe(ts.as_ns(), t2, Source::Ptp);
        *pending = None;
        return;
    }
    *pending = Some(PendingSync {
        uuid: h.source_uuid,
        seq: h.sequence_id,
        t2,
    });
}

fn handle_general(
    overlay: &OverlayClock,
    subdomain: &[u8; 16],
    pkt: &[u8],
    pending: &mut Option<PendingSync>,
) {
    let Some(h) = ptp_v1::decode_header(pkt) else {
        return;
    };
    if !ptp_v1::subdomain_matches(&h.subdomain, subdomain) {
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
