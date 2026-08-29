//! Application-facing types: settings, device lifecycle, errors.

mod arc;
mod cmc;
mod error;
mod info_mcast;
mod settings;
pub(crate) mod subscribe;

pub use error::Error;
pub use settings::{Bind, Bits, Settings};

use crate::clock::{ClockStatus as InnerClockStatus, OverlayClock};
use crate::media::ring::Ring;
use crate::media::rx::{MediaCommand, MediaThread};
use crate::net::iface::{self, IfaceInfo};
use crate::net::mdns::MdnsAnnouncer;
use crate::net::udp;
use crate::protocol::ports as proto_ports;
use std::net::Ipv4Addr;
use std::sync::atomic::{AtomicBool, AtomicU16, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tokio::sync::watch;

/// Ports actually bound by a running [`Device`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BoundPorts {
    pub arc: u16,
    pub cmc: u16,
    pub flows_control: Option<u16>,
    pub info: u16,
    pub mdns: Option<u16>,
    pub ptp_event: Option<u16>,
    pub ptp_general: Option<u16>,
    pub media: Vec<u16>,
}

/// Overlay / media time of the first sample of a block.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MediaTime {
    pub sample_index: u64,
    pub ns: u64,
}

/// Caller-owned interleaved PCM view filled by [`Device::try_read`].
pub struct AudioFrameMut<'a> {
    pub media_time: MediaTime,
    pub sample_rate: u32,
    pub channels: u16,
    pub samples: &'a mut [crate::Sample],
}

/// Clock lock state exposed to applications.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClockStatus {
    Unlocked,
    MediaDriven,
    PtpLocked,
}

#[allow(dead_code)]
pub(crate) struct Identity {
    pub ip: Ipv4Addr,
    pub iface: IfaceInfo,
    pub device_id: [u8; 8],
    pub process_id: u16,
    pub friendly_hostname: String,
    pub factory_hostname: String,
    pub rx_names: Vec<String>,
    pub arc_port: u16,
    pub cmc_port: u16,
    pub flows_control_port: u16,
    pub info_port: u16,
}

pub(crate) struct Shared {
    pub identity: Identity,
    pub settings: Settings,
    pub overlay: Arc<OverlayClock>,
    pub ring: Arc<Ring>,
    pub wakeup: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
    pub rx_names: Mutex<Vec<String>>,
    pub subs: Mutex<Vec<subscribe::RxSub>>,
    pub rx_flows: Mutex<Vec<subscribe::RxFlowInfo>>,
    pub stopped: AtomicBool,
    pub media_tx: std::sync::mpsc::Sender<MediaCommand>,
    pub next_flow_id: AtomicU16,
    pub flows_seq: AtomicU16,
    pub subscribe_override: Mutex<Option<(Ipv4Addr, u16)>>,
    pub info_tx: tokio::sync::mpsc::UnboundedSender<info_mcast::InfoEvent>,
}

/// Running receive device. `Send + Sync`. One instance per [`Device::start`].
pub struct Device {
    shared: Arc<Shared>,
    shutdown_tx: watch::Sender<bool>,
    mdns: Option<MdnsAnnouncer>,
    media: Option<MediaThread>,
    control: Option<tokio::task::JoinHandle<()>>,
    ptp: Option<std::thread::JoinHandle<()>>,
}

impl Device {
    /// Start control, media, and clock planes. Requires a tokio runtime.
    pub async fn start(settings: Settings) -> Result<Self, Error> {
        Self::start_inner(settings, true, true).await
    }

    /// Skip mDNS announcer and PTP binds (loopback / CI).
    pub async fn start_for_test(settings: Settings) -> Result<Self, Error> {
        Self::start_inner(settings, false, false).await
    }

    /// Skip mDNS when resolving a TX (loopback tests). `flows_port` is the fake TX.
    pub fn set_subscribe_override(&self, ipv4: Ipv4Addr, flows_port: u16) {
        *self
            .shared
            .subscribe_override
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Some((ipv4, flows_port));
    }

    async fn start_inner(settings: Settings, mdns: bool, ptp: bool) -> Result<Self, Error> {
        settings.validate()?;
        let iface = iface::resolve(&settings.bind)?;
        let ip = iface.ipv4;
        let process_id = if settings.process_id == 0 {
            std::process::id() as u16
        } else {
            settings.process_id
        };
        let device_id = settings.device_id.unwrap_or_else(|| {
            if iface.mac != [0; 6] {
                [
                    0x00,
                    0x00,
                    iface.mac[0],
                    iface.mac[1],
                    iface.mac[2],
                    iface.mac[3],
                    iface.mac[4],
                    iface.mac[5],
                ]
            } else {
                let o = ip.octets();
                [
                    0x00,
                    0x00,
                    o[0],
                    o[1],
                    o[2],
                    o[3],
                    (process_id >> 8) as u8,
                    process_id as u8,
                ]
            }
        });
        let (arc_port, cmc_port, flows_port, info_port) = match settings.alt_port {
            Some(a) => (
                a,
                a.saturating_add(1),
                a.saturating_add(2),
                a.saturating_add(3),
            ),
            None => (
                proto_ports::ARC,
                proto_ports::CMC,
                proto_ports::FLOWS_CONTROL,
                proto_ports::INFO_BIND,
            ),
        };
        let friendly = settings.name.clone();
        let hex_id = hex_id(&device_id);
        let mut factory = format!("netaudio-{hex_id}");
        if factory.len() > 31 {
            factory.truncate(31);
        }
        let rx_names = match &settings.rx_channel_names {
            Some(n) => n.clone(),
            None => (1..=settings.rx_channels)
                .map(|i| format!("Rx {i}"))
                .collect(),
        };

        let origin = Instant::now();
        let overlay = Arc::new(OverlayClock::new(origin));
        let latency_samples = latency_samples(settings.rx_latency, settings.sample_rate);
        let ring = Arc::new(Ring::new(settings.rx_channels as usize, latency_samples));

        let identity = Identity {
            ip,
            iface: iface.clone(),
            device_id,
            process_id,
            friendly_hostname: friendly,
            factory_hostname: factory,
            rx_names: rx_names.clone(),
            arc_port,
            cmc_port,
            flows_control_port: flows_port,
            info_port,
        };

        let (media_tx, media_rx) = std::sync::mpsc::channel();
        let (info_tx, info_rx) = tokio::sync::mpsc::unbounded_channel();
        let subs = vec![subscribe::RxSub::default(); settings.rx_channels as usize];
        let shared = Arc::new(Shared {
            identity,
            settings: settings.clone(),
            overlay: overlay.clone(),
            ring: ring.clone(),
            wakeup: Mutex::new(None),
            rx_names: Mutex::new(rx_names),
            subs: Mutex::new(subs),
            rx_flows: Mutex::new(Vec::new()),
            stopped: AtomicBool::new(false),
            media_tx: media_tx.clone(),
            next_flow_id: AtomicU16::new(1),
            flows_seq: AtomicU16::new(1),
            subscribe_override: Mutex::new(None),
            info_tx,
        });

        let (shutdown_tx, shutdown_rx) = watch::channel(false);

        let mdns_handle = if mdns {
            Some(MdnsAnnouncer::start(shared.clone())?)
        } else {
            None
        };

        let media = MediaThread::start(shared.clone(), media_rx)?;

        let ptp_handle = if ptp {
            match crate::clock::ptp::start(shared.clone()) {
                Ok(h) => Some(h),
                Err(Error::PtpBindDenied { port }) => {
                    log::warn!("PTP bind denied on port {port}; using media-driven clock");
                    None
                }
                Err(e) => return Err(e),
            }
        } else {
            None
        };

        let control_shared = shared.clone();
        let mut control_shutdown = shutdown_rx.clone();
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
        let control = tokio::spawn(async move {
            if let Err(e) =
                run_control(control_shared, info_rx, ready_tx, &mut control_shutdown).await
            {
                log::error!("control plane error: {e}");
            }
        });
        match ready_rx.await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => return Err(e),
            Err(_) => {
                return Err(Error::Io(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    "control plane task ended before bind",
                )));
            }
        }

        let _ = (shutdown_rx, media_tx);
        Ok(Device {
            shared,
            shutdown_tx,
            mdns: mdns_handle,
            media: Some(media),
            control: Some(control),
            ptp: ptp_handle,
        })
    }

    pub fn try_read(&self, dst: &mut AudioFrameMut<'_>) -> Result<usize, Error> {
        if self.shared.stopped.load(Ordering::Acquire) {
            return Err(Error::Stopped);
        }
        let rate = self.shared.settings.sample_rate;
        let nchan = self.shared.settings.rx_channels as usize;
        let latency = latency_samples(self.shared.settings.rx_latency, rate);
        let now_ns = self.shared.overlay.now_ns();
        let due = ns_to_samples(now_ns, rate);
        if due < latency {
            return Ok(0);
        }
        let read_pos = due.wrapping_sub(latency);
        let max_frames = dst.samples.len() / nchan;
        if max_frames == 0 {
            return Ok(0);
        }
        let (frames, first_index) = self
            .shared
            .ring
            .read(read_pos, nchan, max_frames, dst.samples);
        dst.sample_rate = rate;
        dst.channels = self.shared.settings.rx_channels;
        dst.media_time = MediaTime {
            sample_index: first_index,
            ns: samples_to_ns(first_index, rate),
        };
        Ok(frames)
    }

    pub fn set_rx_wakeup(&self, f: impl Fn() + Send + Sync + 'static) {
        *self.shared.wakeup.lock().unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(f));
    }

    pub fn bound_ports(&self) -> BoundPorts {
        let media = self.shared.ring.bound_media_ports();
        BoundPorts {
            arc: self.shared.identity.arc_port,
            cmc: self.shared.identity.cmc_port,
            flows_control: None,
            info: self.shared.identity.info_port,
            mdns: self.mdns.as_ref().map(|_| 5353),
            ptp_event: self.ptp.as_ref().map(|_| proto_ports::PTP_EVENT),
            ptp_general: self.ptp.as_ref().map(|_| proto_ports::PTP_GENERAL),
            media,
        }
    }

    pub fn clock_status(&self) -> ClockStatus {
        match self.shared.overlay.status() {
            InnerClockStatus::Unlocked => ClockStatus::Unlocked,
            InnerClockStatus::MediaDriven => ClockStatus::MediaDriven,
            InnerClockStatus::PtpLocked => ClockStatus::PtpLocked,
        }
    }

    pub async fn shutdown(mut self) -> Result<(), Error> {
        self.signal_stop();
        if let Some(h) = self.control.take() {
            let _ = h.await;
        }
        if let Some(m) = self.media.take() {
            m.join();
        }
        if let Some(mdns) = self.mdns.take() {
            mdns.stop();
        }
        if let Some(ptp) = self.ptp.take() {
            let _ = ptp.join();
        }
        Ok(())
    }

    fn signal_stop(&self) {
        self.shared.stopped.store(true, Ordering::Release);
        let _ = self.shutdown_tx.send(true);
        let _ = self.shared.media_tx.send(MediaCommand::Shutdown);
    }
}

impl std::fmt::Debug for Device {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Device")
            .field("name", &self.shared.settings.name)
            .field("ip", &self.shared.identity.ip)
            .finish()
    }
}

impl Drop for Device {
    fn drop(&mut self) {
        self.shared.stopped.store(true, Ordering::Release);
        let _ = self.shutdown_tx.send(true);
        let _ = self.shared.media_tx.send(MediaCommand::Shutdown);
        if let Some(m) = self.media.take() {
            m.detach();
        }
        if let Some(mdns) = self.mdns.take() {
            mdns.stop();
        }
        if let Some(h) = self.control.take() {
            h.abort();
        }
    }
}

async fn run_control(
    shared: Arc<Shared>,
    info_rx: tokio::sync::mpsc::UnboundedReceiver<info_mcast::InfoEvent>,
    ready: tokio::sync::oneshot::Sender<Result<(), Error>>,
    shutdown: &mut watch::Receiver<bool>,
) -> Result<(), Error> {
    let ip = shared.identity.ip;
    let bind = |port, role| match udp::bind_unicast(ip, port, role) {
        Ok(s) => udp::std_to_tokio(s),
        Err(e) => Err(e),
    };
    let arc_sock = match bind(shared.identity.arc_port, "arc") {
        Ok(s) => s,
        Err(e) => {
            let _ = ready.send(Err(e));
            return Ok(());
        }
    };
    let cmc_sock = match bind(shared.identity.cmc_port, "cmc") {
        Ok(s) => s,
        Err(e) => {
            let _ = ready.send(Err(e));
            return Ok(());
        }
    };
    let info_sock = match udp::bind_unicast(ip, shared.identity.info_port, "info") {
        Ok(s) => s,
        Err(e) => {
            let _ = ready.send(Err(e));
            return Ok(());
        }
    };
    if let Err(e) = udp::set_multicast_if_v4(&info_sock, ip) {
        let _ = ready.send(Err(e));
        return Ok(());
    }
    let _ = info_sock.set_multicast_ttl_v4(1);
    let info_sock = match udp::std_to_tokio(info_sock) {
        Ok(s) => s,
        Err(e) => {
            let _ = ready.send(Err(e));
            return Ok(());
        }
    };

    let arc_task = {
        let shared = shared.clone();
        let mut sd = shutdown.clone();
        tokio::spawn(async move { arc::run(shared, arc_sock, &mut sd).await })
    };
    let cmc_task = {
        let shared = shared.clone();
        let mut sd = shutdown.clone();
        tokio::spawn(async move { cmc::run(shared, cmc_sock, &mut sd).await })
    };
    let info_task = {
        let shared = shared.clone();
        let mut sd = shutdown.clone();
        tokio::spawn(async move { info_mcast::run(shared, info_sock, info_rx, &mut sd).await })
    };
    let _ = ready.send(Ok(()));

    let _ = shutdown.changed().await;
    arc_task.abort();
    cmc_task.abort();
    info_task.abort();
    Ok(())
}

fn latency_samples(d: std::time::Duration, rate: u32) -> u64 {
    let ns = d.as_nanos() as u64;
    ns.saturating_mul(rate as u64) / 1_000_000_000
}

pub(crate) fn ns_to_samples_pub(ns: u64, rate: u32) -> u64 {
    ns_to_samples(ns, rate)
}

fn ns_to_samples(ns: u64, rate: u32) -> u64 {
    ((ns as u128) * (rate as u128) / 1_000_000_000) as u64
}

fn samples_to_ns(s: u64, rate: u32) -> u64 {
    ((s as u128) * 1_000_000_000 / (rate as u128)) as u64
}

fn hex_id(id: &[u8; 8]) -> String {
    id.iter().map(|b| format!("{b:02x}")).collect()
}
