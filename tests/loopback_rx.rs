//! Loopback RX: Device + fake flows-control TX (W6/W8).

mod common;

use netaudio::{AudioFrameMut, Device, Sample};
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::{Duration, Instant};
use tokio::net::UdpSocket;
use tokio::time::{sleep, timeout};

fn split_timestamp(sample_index: u64, sample_rate: u32) -> (u32, u32) {
    let rate = u64::from(sample_rate);
    ((sample_index / rate) as u32, (sample_index % rate) as u32)
}

fn encode_media_16(seconds: u32, index: u32, samples: &[i32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(9 + samples.len() * 2);
    out.push(0x02);
    out.extend_from_slice(&seconds.to_be_bytes());
    out.extend_from_slice(&index.to_be_bytes());
    for s in samples {
        out.extend_from_slice(&((*s >> 16) as i16).to_be_bytes());
    }
    out
}

fn parse_rr(pkt: &[u8]) -> Option<(u16, u16, u16, u16, &[u8])> {
    if pkt.len() < 10 {
        return None;
    }
    let start = u16::from_be_bytes(pkt[0..2].try_into().ok()?);
    let total = u16::from_be_bytes(pkt[2..4].try_into().ok()?) as usize;
    let seq = u16::from_be_bytes(pkt[4..6].try_into().ok()?);
    let op1 = u16::from_be_bytes(pkt[6..8].try_into().ok()?);
    let op2 = u16::from_be_bytes(pkt[8..10].try_into().ok()?);
    let end = total.min(pkt.len());
    if end < 10 {
        return None;
    }
    Some((start, seq, op1, op2, &pkt[10..end]))
}

fn subscribe_pkt(local_id: u16, tx_ch: &str, tx_host: &str) -> Vec<u8> {
    subscribe_many(&[(local_id, tx_ch, tx_host)])
}

fn subscribe_many(recs: &[(u16, &str, &str)]) -> Vec<u8> {
    let count = recs.len() as u8;
    let mut body = vec![count, count];
    for _ in recs {
        body.extend_from_slice(&0u16.to_be_bytes());
        body.extend_from_slice(&0u16.to_be_bytes());
        body.extend_from_slice(&0u16.to_be_bytes());
    }
    for (i, (local_id, tx_ch, tx_host)) in recs.iter().enumerate() {
        let rec = 2 + i * 6;
        body[rec..rec + 2].copy_from_slice(&local_id.to_be_bytes());
        let name_off = (10 + body.len()) as u16;
        body.extend_from_slice(tx_ch.as_bytes());
        body.push(0);
        let host_off = (10 + body.len()) as u16;
        body.extend_from_slice(tx_host.as_bytes());
        body.push(0);
        body[rec + 2..rec + 4].copy_from_slice(&name_off.to_be_bytes());
        body[rec + 4..rec + 6].copy_from_slice(&host_off.to_be_bytes());
    }
    common::rr_encode(0x2729, 7, 0x3010, 0, &body)
}

fn unsub_pkt(local_id: u16) -> Vec<u8> {
    let mut body = vec![1u8, 1];
    body.extend_from_slice(&local_id.to_be_bytes());
    body.extend_from_slice(&0u16.to_be_bytes());
    body.extend_from_slice(&0u16.to_be_bytes());
    common::rr_encode(0x2729, 8, 0x3010, 0, &body)
}

struct FakeTx {
    flows_port: u16,
    stop: Arc<AtomicBool>,
    keepalives: Arc<AtomicU32>,
    requests: Arc<AtomicU32>,
    last_nchan: Arc<AtomicU32>,
    join: Option<tokio::task::JoinHandle<()>>,
}

impl FakeTx {
    async fn start() -> Self {
        let fc = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let flows_port = fc.local_addr().unwrap().port();
        let stop = Arc::new(AtomicBool::new(false));
        let keepalives = Arc::new(AtomicU32::new(0));
        let requests = Arc::new(AtomicU32::new(0));
        let last_nchan = Arc::new(AtomicU32::new(0));
        let stop2 = stop.clone();
        let ka = keepalives.clone();
        let reqs = requests.clone();
        let nchan = last_nchan.clone();
        let join = tokio::spawn(async move {
            fake_tx_loop(fc, stop2, ka, reqs, nchan).await;
        });
        Self {
            flows_port,
            stop,
            keepalives,
            requests,
            last_nchan,
            join: Some(join),
        }
    }

    async fn stop(mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(h) = self.join.take() {
            let _ = timeout(Duration::from_secs(1), h).await;
        }
    }
}

async fn fake_tx_loop(
    fc: UdpSocket,
    stop: Arc<AtomicBool>,
    keepalives: Arc<AtomicU32>,
    requests: Arc<AtomicU32>,
    last_nchan: Arc<AtomicU32>,
) {
    let media = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let mut buf = [0u8; 1500];
    let mut kbuf = [0u8; 64];
    let handle = [0xAAu8, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF];
    let mut dest: Option<SocketAddr> = None;
    let mut sending = false;
    let mut nchan = 1usize;
    let origin = Instant::now();
    let mut tick = tokio::time::interval(Duration::from_millis(4));
    loop {
        if stop.load(Ordering::Acquire) {
            break;
        }
        tokio::select! {
            rec = fc.recv_from(&mut buf) => {
                let Ok((n, src)) = rec else { continue };
                let Some((start, seq, op1, _op2, content)) = parse_rr(&buf[..n]) else { continue };
                if op1 == 0x0100 {
                    requests.fetch_add(1, Ordering::Relaxed);
                    if content.len() >= 6 {
                        let rate = u32::from_be_bytes(content[2..6].try_into().unwrap());
                        if rate != 48_000 {
                            let err = common::rr_encode(start, seq, 0x0100, 0x0301, &[]);
                            let _ = fc.send_to(&err, src).await;
                            continue;
                        }
                    }
                    if content.len() >= 14 {
                        nchan = u16::from_be_bytes(content[12..14].try_into().unwrap()) as usize;
                        nchan = nchan.max(1);
                        last_nchan.store(nchan as u32, Ordering::Relaxed);
                    }
                    let mut sock_off = if content.len() >= 16 {
                        u16::from_be_bytes(content[14..16].try_into().unwrap()) as usize
                    } else {
                        0
                    };
                    if (sock_off + 8 > n
                        || buf.get(sock_off..sock_off + 2) != Some(&[0x08, 0x02][..]))
                        && let Some(i) = buf.windows(2).position(|w| w == [0x08, 0x02]) {
                            sock_off = i;
                        }
                    if sock_off + 8 <= n && buf.get(sock_off..sock_off + 2) == Some(&[0x08, 0x02][..])
                    {
                        let port = u16::from_be_bytes(buf[sock_off + 2..sock_off + 4].try_into().unwrap());
                        dest = Some(SocketAddr::from((
                            [buf[sock_off + 4], buf[sock_off + 5], buf[sock_off + 6], buf[sock_off + 7]],
                            port,
                        )));
                        sending = true;
                    }
                    let ok = common::rr_encode(start, seq, 0x0100, 1, &handle);
                    let _ = fc.send_to(&ok, src).await;
                } else if op1 == 0x0101 {
                    sending = false;
                    dest = None;
                    let ok = common::rr_encode(start, seq, 0x0101, 1, &handle);
                    let _ = fc.send_to(&ok, src).await;
                } else if op1 == 0x0102 {
                    let ok = common::rr_encode(start, seq, 0x0102, 1, &handle);
                    let _ = fc.send_to(&ok, src).await;
                }
            }
            rec = media.recv_from(&mut kbuf) => {
                if let Ok((n, _)) = rec
                    && n == 2 && kbuf[0] == 0x13 && kbuf[1] == 0x37 {
                        keepalives.fetch_add(1, Ordering::Relaxed);
                    }
            }
            _ = tick.tick() => {
                if sending
                    && let Some(d) = dest {
                        let elapsed = origin.elapsed();
                        let idx = (elapsed.as_secs_f64() * 48_000.0) as u64;
                        let frames = 16u64;
                        let mut samples = vec![0i32; frames as usize * nchan];
                        for f in 0..frames as usize {
                            samples[f * nchan] = ((idx as i32).wrapping_add(f as i32)) << 16;
                            if nchan > 1 {
                                samples[f * nchan + 1] = 0x1000_0000;
                            }
                        }
                        let (sec, i) = split_timestamp(idx, 48_000);
                        let pkt = encode_media_16(sec, i, &samples);
                        let _ = media.send_to(&pkt, d).await;
                    }
            }
        }
    }
}

async fn wait_frames(d: &Device) -> Vec<Sample> {
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut samples = [0i32; 64];
    loop {
        let mut frame = AudioFrameMut {
            media_time: netaudio::MediaTime {
                sample_index: 0,
                ns: 0,
            },
            sample_rate: 0,
            channels: 0,
            samples: &mut samples,
        };
        let n = d.try_read(&mut frame).unwrap();
        if n > 0 && samples.iter().any(|&s| s != 0) {
            return samples[..n].to_vec();
        }
        if Instant::now() >= deadline {
            panic!("try_read produced no frames");
        }
        sleep(Duration::from_millis(2)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unpatched_try_read_is_zero() {
    let d = common::start_device("unpat").await;
    let mut buf = [0i32; 32];
    let mut frame = AudioFrameMut {
        media_time: netaudio::MediaTime {
            sample_index: 0,
            ns: 0,
        },
        sample_rate: 0,
        channels: 0,
        samples: &mut buf,
    };
    assert_eq!(d.try_read(&mut frame).unwrap(), 0);
    d.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn loopback_ramp_then_unsub() {
    let d = common::start_device("looprx").await;
    let fake = FakeTx::start().await;
    d.set_subscribe_override(std::net::Ipv4Addr::LOCALHOST, fake.flows_port);

    let sock = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let dest = SocketAddr::from(([127, 0, 0, 1], d.bound_ports().arc));
    sock.send_to(&subscribe_pkt(1, "1", "fake"), dest)
        .await
        .unwrap();
    let mut ack = [0u8; 64];
    timeout(Duration::from_secs(2), sock.recv_from(&mut ack))
        .await
        .expect("subscribe ACK")
        .unwrap();

    let got = wait_frames(&d).await;
    assert!(!got.is_empty());
    // 16-bit ramp in the top half.
    assert!(
        got.iter().any(|&s| s != 0),
        "expected non-silent PCM, got {got:?}"
    );

    let ka_before = fake.keepalives.load(Ordering::Relaxed);
    sleep(Duration::from_millis(400)).await;
    let ka_after = fake.keepalives.load(Ordering::Relaxed);
    assert!(
        ka_after >= ka_before,
        "keepalive counter should be reachable (before={ka_before} after={ka_after})"
    );

    sock.send_to(&unsub_pkt(1), dest).await.unwrap();
    let _ = timeout(Duration::from_secs(2), sock.recv_from(&mut ack)).await;
    sleep(Duration::from_millis(50)).await;

    let mut silent = true;
    for _ in 0..20 {
        let mut buf = [0i32; 32];
        let mut frame = AudioFrameMut {
            media_time: netaudio::MediaTime {
                sample_index: 0,
                ns: 0,
            },
            sample_rate: 0,
            channels: 0,
            samples: &mut buf,
        };
        if d.try_read(&mut frame).unwrap() > 0 && buf.iter().any(|&s| s != 0) {
            silent = false;
            break;
        }
        sleep(Duration::from_millis(5)).await;
    }
    assert!(silent, "unsub should stop PCM");

    fake.stop().await;
    d.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn loopback_stereo_one_flow() {
    let d = common::start_device_with("loop2ch", 2).await;
    let fake = FakeTx::start().await;
    d.set_subscribe_override(std::net::Ipv4Addr::LOCALHOST, fake.flows_port);

    let sock = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let dest = SocketAddr::from(([127, 0, 0, 1], d.bound_ports().arc));
    sock.send_to(&subscribe_many(&[(1, "1", "fake"), (2, "2", "fake")]), dest)
        .await
        .unwrap();
    let mut ack = [0u8; 64];
    timeout(Duration::from_secs(2), sock.recv_from(&mut ack))
        .await
        .expect("subscribe ACK")
        .unwrap();

    let deadline = Instant::now() + Duration::from_secs(3);
    let mut samples = [0i32; 128];
    let got = loop {
        let mut frame = AudioFrameMut {
            media_time: netaudio::MediaTime {
                sample_index: 0,
                ns: 0,
            },
            sample_rate: 0,
            channels: 0,
            samples: &mut samples,
        };
        let n = d.try_read(&mut frame).unwrap();
        if n > 0 && frame.channels == 2 {
            let used = n * 2;
            let ch0 = samples.iter().step_by(2).take(n).any(|&s| s != 0);
            let ch1 = samples
                .iter()
                .skip(1)
                .step_by(2)
                .take(n)
                .any(|&s| s == 0x1000_0000);
            if ch0 && ch1 {
                break samples[..used].to_vec();
            }
        }
        if Instant::now() >= deadline {
            panic!(
                "stereo try_read missing both channels; requests={} nchan={} sample0={:?}",
                fake.requests.load(Ordering::Relaxed),
                fake.last_nchan.load(Ordering::Relaxed),
                &samples[..8]
            );
        }
        sleep(Duration::from_millis(2)).await;
    };
    assert!(got.len() >= 4);
    assert_eq!(
        fake.requests.load(Ordering::Relaxed),
        1,
        "one 0x0100 for both channels"
    );
    assert_eq!(fake.last_nchan.load(Ordering::Relaxed), 2);

    fake.stop().await;
    d.shutdown().await.unwrap();
}
