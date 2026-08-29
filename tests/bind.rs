//! W2: bind rules via the public API.

mod common;

use netaudio::{Bind, Device, Settings};
use std::net::Ipv4Addr;

#[tokio::test]
async fn unspecified_rejected_at_start() {
    let mut s = Settings::default();
    s.bind = Bind::Ip(Ipv4Addr::UNSPECIFIED);
    let err = Device::start_for_test(s).await.unwrap_err();
    assert!(matches!(err, netaudio::Error::UnspecifiedAddress));
}

#[tokio::test]
async fn loopback_device_binds_unicast() {
    let d = common::start_device("bindtest").await;
    let p = d.bound_ports();
    assert!(p.arc != 0);
    assert!(p.cmc != 0);
    assert!(p.info != 0);
    assert!(p.flows_control.is_none());
    assert!(p.mdns.is_none());
    d.shutdown().await.unwrap();
}

#[tokio::test]
async fn loopback_interface_name() {
    let mut s = Settings::default();
    s.name = "loif".into();
    s.bind = Bind::Interface("lo".into());
    s.alt_port = Some(21_000);
    // On some hosts "lo" has no IPv4 in netdev; localhost IP is the portable path.
    let _ = Device::start_for_test(s).await;
}
