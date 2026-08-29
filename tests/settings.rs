//! W0: public Settings validation.

use netaudio::{Bind, Error, Settings};
use std::net::Ipv4Addr;
use std::time::Duration;

#[test]
fn default_is_valid() {
    Settings::default().validate().unwrap();
}

#[test]
fn rejects_latency_below_4ms() {
    let s = Settings {
        rx_latency: Duration::from_nanos(3_900_000),
        ..Default::default()
    };
    assert!(matches!(s.validate(), Err(Error::InvalidSettings { .. })));
}

#[test]
fn accepts_4ms() {
    let s = Settings {
        rx_latency: Duration::from_millis(4),
        ..Default::default()
    };
    s.validate().unwrap();
}

#[test]
fn rejects_long_name() {
    let s = Settings {
        name: "a".repeat(32),
        ..Default::default()
    };
    assert!(s.validate().is_err());
}

#[test]
fn rejects_name_with_dot_or_space() {
    let s = Settings {
        name: "foo.bar".into(),
        ..Default::default()
    };
    assert!(s.validate().is_err());
    let s = Settings {
        name: "foo bar".into(),
        ..Default::default()
    };
    assert!(s.validate().is_err());
}

#[test]
fn rejects_bad_rate() {
    let s = Settings {
        sample_rate: 32_000,
        ..Default::default()
    };
    assert!(s.validate().is_err());
}

#[test]
fn rejects_tx_channels() {
    let s = Settings {
        tx_channels: 2,
        ..Default::default()
    };
    assert!(s.validate().is_err());
}

#[test]
fn rejects_unspecified_bind() {
    let s = Settings {
        bind: Bind::Ip(Ipv4Addr::UNSPECIFIED),
        ..Default::default()
    };
    assert!(matches!(s.validate(), Err(Error::UnspecifiedAddress)));
}
