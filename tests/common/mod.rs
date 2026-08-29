//! Shared helpers for integration tests.

use netaudio::{Bind, Bits, Device, Error, Settings};
use std::net::Ipv4Addr;
use std::time::Duration;

#[allow(dead_code)]
pub fn rr_encode(start: u16, seq: u16, op1: u16, op2: u16, content: &[u8]) -> Vec<u8> {
    let total = 10 + content.len();
    let mut w = Vec::with_capacity(total);
    w.extend_from_slice(&start.to_be_bytes());
    w.extend_from_slice(&(total as u16).to_be_bytes());
    w.extend_from_slice(&seq.to_be_bytes());
    w.extend_from_slice(&op1.to_be_bytes());
    w.extend_from_slice(&op2.to_be_bytes());
    w.extend_from_slice(content);
    w
}

pub fn settings_loopback(name: &str, alt: u16) -> Settings {
    Settings {
        name: name.into(),
        bind: Bind::Ip(Ipv4Addr::LOCALHOST),
        rx_channels: 1,
        bits: Bits::B16,
        rx_latency: Duration::from_millis(10),
        alt_port: Some(alt),
        ..Default::default()
    }
}

pub async fn start_device(name: &str) -> Device {
    let base = 20_000u16 + (std::process::id() as u16 % 2_000) * 4;
    for i in 0..40u16 {
        let alt = base.saturating_add(i.saturating_mul(4));
        if alt > 65_000 {
            break;
        }
        let s = settings_loopback(name, alt);
        match Device::start_for_test(s).await {
            Ok(d) => return d,
            Err(Error::PortInUse { .. }) => continue,
            Err(e) => panic!("start_for_test: {e}"),
        }
    }
    panic!("no free alt_port for test device");
}
