//! W3: live ARC/CMC against a loopback Device.

mod common;

use netaudio::ClockStatus;
use std::net::SocketAddr;
use std::time::Duration;
use tokio::net::UdpSocket;
use tokio::time::timeout;

#[tokio::test]
async fn arc_counts_and_port_ranges() {
    let d = common::start_device("arctest").await;
    let ports = d.bound_ports();
    let sock = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let dest = SocketAddr::from(([127, 0, 0, 1], ports.arc));

    let counts = common::rr_encode(0x2729, 1, 0x1000, 0, &[]);
    sock.send_to(&counts, dest).await.unwrap();
    let mut buf = [0u8; 512];
    let (n, _) = timeout(Duration::from_secs(2), sock.recv_from(&mut buf))
        .await
        .expect("arc 0x1000 timeout")
        .unwrap();
    assert!(n >= 10 + 36);
    assert_eq!(&buf[6..8], &[0x10, 0x00]);
    assert_eq!(&buf[8..10], &[0x00, 0x01]);
    assert_eq!(&buf[10 + 2..10 + 6], &[0, 0, 0, 1]); // tx=0 rx=1
    assert_eq!(&buf[10 + 8..10 + 10], &[0, 8]); // max ch in flow

    let req3300 = hex::decode("2729000a033c33000000").unwrap();
    sock.send_to(&req3300, dest).await.unwrap();
    let (n, _) = timeout(Duration::from_secs(2), sock.recv_from(&mut buf))
        .await
        .expect("arc 0x3300 timeout")
        .unwrap();
    assert_eq!(
        hex::encode(&buf[..n]),
        "27290012033c330000013800397f398039ff"
    );

    let empty = common::rr_encode(0x2729, 3, 0x2000, 0, &[0, 0, 0, 1]);
    sock.send_to(&empty, dest).await.unwrap();
    let (_n, _) = timeout(Duration::from_secs(2), sock.recv_from(&mut buf))
        .await
        .expect("arc 0x2000 timeout")
        .unwrap();
    assert_eq!(&buf[6..8], &[0x20, 0x00]);
    assert_eq!(&buf[8..10], &[0x00, 0x01]);
    assert_eq!(buf[10], 0); // empty page

    assert_eq!(d.clock_status(), ClockStatus::Unlocked);
    d.shutdown().await.unwrap();
}

#[tokio::test]
async fn cmc_advertisement() {
    let d = common::start_device("cmctest").await;
    let ports = d.bound_ports();
    let sock = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let dest = SocketAddr::from(([127, 0, 0, 1], ports.cmc));
    let req = common::rr_encode(0x1200, 1, 0x1001, 0, &[]);
    sock.send_to(&req, dest).await.unwrap();
    let mut buf = [0u8; 512];
    let (n, _) = timeout(Duration::from_secs(2), sock.recv_from(&mut buf))
        .await
        .expect("cmc timeout")
        .unwrap();
    assert_eq!(n, 10 + 22);
    assert_eq!(&buf[6..8], &[0x10, 0x01]);
    assert_eq!(&buf[8..10], &[0x00, 0x01]);
    let info = u16::from_be_bytes([buf[10 + 18], buf[10 + 19]]);
    assert_eq!(info, ports.info);
    d.shutdown().await.unwrap();
}
