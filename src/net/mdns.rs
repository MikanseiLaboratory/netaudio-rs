//! Custom mDNS announcer thread (source port 5353). Queries share that socket
//! so Dante `_netaudio-chan` answers (multicast to 5353) are visible.

use super::udp;
use crate::device::{Error, Shared};
use crate::protocol::mdns as mdns_proto;
use crate::protocol::ports;
use std::fmt::Write as _;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

pub struct MdnsAnnouncer {
    join: Option<JoinHandle<()>>,
    stop: Arc<std::sync::atomic::AtomicBool>,
}

pub struct MdnsQuery {
    pub names: Vec<String>,
    pub questions: Vec<(String, u16)>,
    pub timeout: Duration,
    pub reply: Sender<Option<mdns_proto::Message>>,
}

impl MdnsAnnouncer {
    pub fn start(shared: Arc<Shared>) -> Result<Self, Error> {
        let sock = udp::bind_mdns(shared.identity.ip)?;
        sock.set_read_timeout(Some(Duration::from_millis(50))).ok();
        let (tx, rx) = mpsc::channel::<MdnsQuery>();
        *shared.mdns_tx.lock().unwrap_or_else(|e| e.into_inner()) = Some(tx);
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let stop2 = stop.clone();
        let join = thread::Builder::new()
            .name("netaudio-mdns".into())
            .spawn(move || run(shared, sock, stop2, rx))?;
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

struct Pending {
    names: Vec<String>,
    deadline: Instant,
    acc: mdns_proto::Message,
    reply: Sender<Option<mdns_proto::Message>>,
}

fn run(
    shared: Arc<Shared>,
    sock: std::net::UdpSocket,
    stop: Arc<std::sync::atomic::AtomicBool>,
    cmds: Receiver<MdnsQuery>,
) {
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
    let mut pending: Vec<Pending> = Vec::new();
    loop {
        if stop.load(std::sync::atomic::Ordering::Acquire)
            || shared.stopped.load(std::sync::atomic::Ordering::Acquire)
        {
            break;
        }
        while let Ok(q) = cmds.try_recv() {
            let pairs: Vec<(&str, u16)> =
                q.questions.iter().map(|(n, t)| (n.as_str(), *t)).collect();
            let pkt = mdns_proto::build_query(&pairs).encode();
            let _ = sock.send_to(&pkt, dest);
            pending.push(Pending {
                names: q.names,
                deadline: Instant::now() + q.timeout,
                acc: mdns_proto::Message::default(),
                reply: q.reply,
            });
        }
        if last_announce.elapsed() > Duration::from_secs(60) {
            let _ = sock.send_to(&announce(), dest);
            last_announce = Instant::now();
        }
        match sock.recv_from(&mut buf) {
            Ok((n, src)) => {
                if let Some(msg) = mdns_proto::Message::decode(&buf[..n]) {
                    if should_answer(&msg, &shared.identity.friendly_hostname) {
                        let pkt = announce();
                        let _ = sock.send_to(&pkt, dest);
                        if src.port() != ports::MDNS {
                            let _ = sock.send_to(&pkt, src);
                        }
                    }
                    if msg.flags & 0x8000 != 0 {
                        for p in pending.iter_mut() {
                            if p.names.iter().any(|n| mdns_proto::records_mention(&msg, n)) {
                                mdns_proto::merge_records(&mut p.acc, &msg);
                            }
                        }
                    }
                }
            }
            Err(e)
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut => {}
            Err(_) => thread::sleep(Duration::from_millis(20)),
        }
        let now = Instant::now();
        let mut i = 0;
        while i < pending.len() {
            if now >= pending[i].deadline {
                let p = pending.remove(i);
                let out = if p.acc.answers.is_empty() && p.acc.additionals.is_empty() {
                    None
                } else {
                    Some(p.acc)
                };
                let _ = p.reply.send(out);
            } else {
                i += 1;
            }
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

/// Resolve via the 5353 announcer when present; otherwise a high-port send (tests).
pub fn query_names(
    shared: &Shared,
    names: &[&str],
    questions: &[(&str, u16)],
    timeout: Duration,
) -> Result<Option<mdns_proto::Message>, Error> {
    let tx = shared
        .mdns_tx
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    if let Some(tx) = tx {
        let (rtx, rrx) = mpsc::channel();
        tx.send(MdnsQuery {
            names: names.iter().map(|s| (*s).to_owned()).collect(),
            questions: questions
                .iter()
                .map(|(n, t)| ((*n).to_owned(), *t))
                .collect(),
            timeout,
            reply: rtx,
        })
        .map_err(|_| Error::Io(std::io::Error::other("mDNS announcer closed")))?;
        let wait = timeout + Duration::from_millis(250);
        return match rrx.recv_timeout(wait) {
            Ok(v) => Ok(v),
            Err(_) => Ok(None),
        };
    }
    query_legacy(shared.identity.ip, questions, names, timeout)
}

fn query_legacy(
    ip: Ipv4Addr,
    questions: &[(&str, u16)],
    names: &[&str],
    timeout: Duration,
) -> Result<Option<mdns_proto::Message>, Error> {
    let sock = udp::bind_querier(ip)?;
    sock.set_read_timeout(Some(Duration::from_millis(50)))?;
    let dest = SocketAddr::V4(SocketAddrV4::new(
        Ipv4Addr::new(
            ports::MDNS_GROUP[0],
            ports::MDNS_GROUP[1],
            ports::MDNS_GROUP[2],
            ports::MDNS_GROUP[3],
        ),
        ports::MDNS,
    ));
    sock.send_to(&mdns_proto::build_query(questions).encode(), dest)?;
    let mut buf = [0u8; 1500];
    let mut acc = mdns_proto::Message::default();
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        match sock.recv_from(&mut buf) {
            Ok((n, _)) => {
                if let Some(msg) = mdns_proto::Message::decode(&buf[..n])
                    && msg.flags & 0x8000 != 0
                    && names.iter().any(|n| mdns_proto::records_mention(&msg, n))
                {
                    mdns_proto::merge_records(&mut acc, &msg);
                }
            }
            Err(e)
                if e.kind() == std::io::ErrorKind::TimedOut
                    || e.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(e) => return Err(e.into()),
        }
    }
    if acc.answers.is_empty() && acc.additionals.is_empty() {
        Ok(None)
    } else {
        Ok(Some(acc))
    }
}
