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
    let mut s = Settings::default();
    s.rx_latency = Duration::from_nanos(3_900_000);
    assert!(matches!(s.validate(), Err(Error::InvalidSettings { .. })));
}

#[test]
fn accepts_4ms() {
    let mut s = Settings::default();
    s.rx_latency = Duration::from_millis(4);
    s.validate().unwrap();
}

#[test]
fn rejects_long_name() {
    let mut s = Settings::default();
    s.name = "a".repeat(32);
    assert!(s.validate().is_err());
}

#[test]
fn rejects_name_with_dot_or_space() {
    let mut s = Settings::default();
    s.name = "foo.bar".into();
    assert!(s.validate().is_err());
    s.name = "foo bar".into();
    assert!(s.validate().is_err());
}

#[test]
fn rejects_bad_rate() {
    let mut s = Settings::default();
    s.sample_rate = 32_000;
    assert!(s.validate().is_err());
}

#[test]
fn rejects_tx_channels() {
    let mut s = Settings::default();
    s.tx_channels = 2;
    assert!(s.validate().is_err());
}

#[test]
fn rejects_unspecified_bind() {
    let mut s = Settings::default();
    s.bind = Bind::Ip(Ipv4Addr::UNSPECIFIED);
    assert!(matches!(s.validate(), Err(Error::UnspecifiedAddress)));
}
