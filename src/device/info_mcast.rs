//! Info multicast: board/product at start, 1 Hz heartbeat, unicast replies.

use super::{Error, Shared};
use crate::protocol::info_mcast as im;
use crate::protocol::ports;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::Arc;
use tokio::net::UdpSocket;
use tokio::sync::watch;
use tokio::time::{interval, Duration};

pub enum InfoEvent {
    ChannelChange(Vec<usize>),
}

pub async fn run(
    shared: Arc<Shared>,
    sock: UdpSocket,
    mut events: tokio::sync::mpsc::UnboundedReceiver<InfoEvent>,
    shutdown: &mut watch::Receiver<bool>,
) -> Result<(), Error> {
    let seq = AtomicU16::new(1);
    send_board(&shared, &sock, &seq).await;
    send_product(&shared, &sock, &seq).await;

    let mut hb = interval(Duration::from_secs(1));
    let mut buf = [0u8; 2048];
    loop {
        tokio::select! {
            _ = shutdown.changed() => break,
            _ = hb.tick() => {
                send_heartbeat(&shared, &sock, &seq).await;
            }
            ev = events.recv() => {
                match ev {
                    Some(InfoEvent::ChannelChange(idx)) => {
                        send_channel_change(&shared, &sock, &seq, &idx).await;
                    }
                    None => {}
                }
            }
            rec = sock.recv_from(&mut buf) => {
                match rec {
                    Ok((n, src)) => {
                        if let Some(pkt) = reply_request(&shared, &buf[..n], &seq) {
                            let _ = sock.send_to(&pkt, src).await;
                            let dest = info_dest(&shared);
                            let _ = sock.send_to(&pkt, dest).await;
                        }
                    }
                    Err(e) => log::warn!("info recv: {e}"),
                }
            }
        }
    }
    Ok(())
}

fn next_seq(seq: &AtomicU16) -> u16 {
    seq.fetch_add(1, Ordering::Relaxed)
}

fn info_dest(_shared: &Shared) -> SocketAddr {
    SocketAddr::V4(SocketAddrV4::new(
        Ipv4Addr::new(
            ports::INFO_GROUP[0],
            ports::INFO_GROUP[1],
            ports::INFO_GROUP[2],
            ports::INFO_GROUP[3],
        ),
        ports::INFO_DEST_PORT,
    ))
}

fn hb_dest() -> SocketAddr {
    SocketAddr::V4(SocketAddrV4::new(
        Ipv4Addr::new(
            ports::HEARTBEAT_GROUP[0],
            ports::HEARTBEAT_GROUP[1],
            ports::HEARTBEAT_GROUP[2],
            ports::HEARTBEAT_GROUP[3],
        ),
        ports::HEARTBEAT_DEST_PORT,
    ))
}

fn wrap(shared: &Shared, seq: u16, start: u16, opcode: [u8; 8], content: &[u8]) -> Vec<u8> {
    im::encode(
        start,
        seq,
        shared.identity.process_id,
        shared.identity.device_id,
        shared.settings.vendor_tag,
        opcode,
        content,
    )
}

async fn send_board(shared: &Shared, sock: &UdpSocket, seq: &AtomicU16) {
    let pkt = wrap(
        shared,
        next_seq(seq),
        im::START_INFO,
        im::OPCODE_BOARD,
        &im::board_payload(&shared.settings.board),
    );
    let _ = sock.send_to(&pkt, info_dest(shared)).await;
}

async fn send_product(shared: &Shared, sock: &UdpSocket, seq: &AtomicU16) {
    let pkt = wrap(
        shared,
        next_seq(seq),
        im::START_INFO,
        im::OPCODE_PRODUCT,
        &im::product_payload(
            &shared.settings.manufacturer,
            &shared.settings.board,
            &shared.settings.model,
        ),
    );
    let _ = sock.send_to(&pkt, info_dest(shared)).await;
}

async fn send_heartbeat(shared: &Shared, sock: &UdpSocket, seq: &AtomicU16) {
    let mut content = Vec::new();
    if shared.overlay.status() != crate::clock::ClockStatus::Unlocked {
        content.extend_from_slice(&im::heartbeat_clock_ppm(
            next_seq(seq),
            shared.overlay.freq_scale_ppb(),
        ));
    }
    let pkt = wrap(
        shared,
        next_seq(seq),
        im::START_HEARTBEAT,
        im::OPCODE_HEARTBEAT,
        &content,
    );
    let _ = sock.send_to(&pkt, hb_dest()).await;
}

async fn send_channel_change(shared: &Shared, sock: &UdpSocket, seq: &AtomicU16, idx: &[usize]) {
    let pkt = wrap(
        shared,
        next_seq(seq),
        im::START_INFO,
        im::OPCODE_CH_CHANGE,
        &im::channel_change(idx),
    );
    let _ = sock.send_to(&pkt, info_dest(shared)).await;
}

fn reply_request(shared: &Shared, packet: &[u8], seq: &AtomicU16) -> Option<Vec<u8>> {
    let opcode = im::reply_opcode(packet)?;
    let content = if opcode == im::OPCODE_BOARD {
        im::board_payload(&shared.settings.board)
    } else if opcode == im::OPCODE_PRODUCT {
        im::product_payload(
            &shared.settings.manufacturer,
            &shared.settings.board,
            &shared.settings.model,
        )
    } else if opcode == im::OPCODE_NET {
        im::network_info(
            shared.identity.iface.link_mbps,
            shared.identity.iface.mac,
            shared.identity.ip,
            shared.identity.iface.netmask,
            shared.identity.iface.gateway,
        )
    } else if opcode == im::OPCODE_CLOCK {
        im::heartbeat_clock_ppm(next_seq(seq), shared.overlay.freq_scale_ppb())
    } else {
        return None;
    };
    Some(wrap(
        shared,
        next_seq(seq),
        im::START_INFO,
        opcode,
        &content,
    ))
}
