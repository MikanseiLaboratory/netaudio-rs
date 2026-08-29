//! Custom mDNS announcer thread (source port 5353).

use super::udp;
use crate::device::{Error, Shared};
use crate::protocol::mdns as mdns_proto;
use crate::protocol::ports;
use std::fmt::Write as _;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

pub struct MdnsAnnouncer {
    join: Option<JoinHandle<()>>,
    stop: Arc<std::sync::atomic::AtomicBool>,
}

impl MdnsAnnouncer {
    pub fn start(shared: Arc<Shared>) -> Result<Self, Error> {
        let sock = udp::bind_mdns(shared.identity.ip)?;
        sock.set_read_timeout(Some(Duration::from_millis(50))).ok();
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let stop2 = stop.clone();
        let join = thread::Builder::new()
            .name("netaudio-mdns".into())
            .spawn(move || run(shared, sock, stop2))?;
        Ok(Self {
            join: Some(join),
            stop,
        })
    }

    pub fn stop(mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::Release);
        if let Some(h) = self.join.take() {
            let _ = h.join();
        }
    }
}

fn run(shared: Arc<Shared>, sock: std::net::UdpSocket, stop: Arc<std::sync::atomic::AtomicBool>) {
    let dest = SocketAddr::V4(SocketAddrV4::new(
        Ipv4Addr::new(
            ports::MDNS_GROUP[0],
            ports::MDNS_GROUP[1],
            ports::MDNS_GROUP[2],
            ports::MDNS_GROUP[3],
        ),
        ports::MDNS,
    ));
    let id = hex_id(&shared.identity.device_id);
    let announce = || {
        mdns_proto::build_announcement(
            &shared.identity.friendly_hostname,
            shared.identity.ip,
            shared.identity.arc_port,
            shared.identity.cmc_port,
            &id,
            shared.identity.process_id,
            &shared.settings.board,
            &shared.settings.manufacturer,
            &shared.settings.model,
        )
        .encode()
    };
    let probe = mdns_proto::build_probe(&shared.identity.friendly_hostname).encode();

    let jitter_ms = (std::process::id() % 250) as u64;
    thread::sleep(Duration::from_millis(jitter_ms));
    for _ in 0..3 {
        if stop.load(std::sync::atomic::Ordering::Acquire) {
            return;
        }
        let _ = sock.send_to(&probe, dest);
        thread::sleep(Duration::from_millis(250));
    }
    for _ in 0..2 {
        if stop.load(std::sync::atomic::Ordering::Acquire) {
            return;
        }
        let _ = sock.send_to(&announce(), dest);
        thread::sleep(Duration::from_secs(1));
    }

    let mut buf = [0u8; 1500];
    let mut last_announce = Instant::now();
    loop {
        if stop.load(std::sync::atomic::Ordering::Acquire)
            || shared.stopped.load(std::sync::atomic::Ordering::Acquire)
        {
            break;
        }
        if last_announce.elapsed() > Duration::from_secs(60) {
            let _ = sock.send_to(&announce(), dest);
            last_announce = Instant::now();
        }
        match sock.recv_from(&mut buf) {
            Ok((n, src)) => {
                if let Some(msg) = mdns_proto::Message::decode(&buf[..n])
                    && should_answer(&msg, &shared.identity.friendly_hostname)
                {
                    let pkt = announce();
                    let _ = sock.send_to(&pkt, dest);
                    if src.port() != ports::MDNS {
                        let _ = sock.send_to(&pkt, src);
                    }
                }
            }
            Err(e)
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut => {}
            Err(_) => thread::sleep(Duration::from_millis(20)),
        }
    }
}

fn should_answer(msg: &mdns_proto::Message, hostname: &str) -> bool {
    if msg.flags & 0x8000 != 0 {
        return false;
    }
    let arc = mdns_proto::service_instance(hostname, mdns_proto::ARC_SERVICE);
    let cmc = mdns_proto::service_instance(hostname, mdns_proto::CMC_SERVICE);
    let host = format!("{hostname}.local");
    msg.questions.iter().any(|q| {
        mdns_proto::names_match(&q.name, mdns_proto::ARC_SERVICE)
            || mdns_proto::names_match(&q.name, mdns_proto::CMC_SERVICE)
            || mdns_proto::names_match(&q.name, &arc)
            || mdns_proto::names_match(&q.name, &cmc)
            || mdns_proto::names_match(&q.name, &host)
    })
}

fn hex_id(id: &[u8; 8]) -> String {
    let mut s = String::with_capacity(16);
    for b in id {
        let _ = write!(s, "{b:02x}");
    }
    s
}

/// Legacy unicast mDNS query (high port → 5353).
pub fn query(
    ip: Ipv4Addr,
    name: &str,
    qtype: u16,
    timeout: Duration,
) -> Result<Option<mdns_proto::Message>, Error> {
    let sock = udp::bind_querier(ip)?;
    sock.set_read_timeout(Some(timeout))?;
    let q = mdns_proto::Message {
        id: 0,
        flags: mdns_proto::FLAGS_QUERY,
        questions: vec![mdns_proto::Question {
            name: name.into(),
            qtype,
        }],
        answers: Vec::new(),
        additionals: Vec::new(),
    };
    let dest = SocketAddr::V4(SocketAddrV4::new(
        Ipv4Addr::new(
            ports::MDNS_GROUP[0],
            ports::MDNS_GROUP[1],
            ports::MDNS_GROUP[2],
            ports::MDNS_GROUP[3],
        ),
        ports::MDNS,
    ));
    sock.send_to(&q.encode(), dest)?;
    let mut buf = [0u8; 1500];
    match sock.recv_from(&mut buf) {
        Ok((n, _)) => Ok(mdns_proto::Message::decode(&buf[..n])),
        Err(e)
            if e.kind() == std::io::ErrorKind::TimedOut
                || e.kind() == std::io::ErrorKind::WouldBlock =>
        {
            Ok(None)
        }
        Err(e) => Err(e.into()),
    }
}
