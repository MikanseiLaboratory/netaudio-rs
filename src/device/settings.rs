//! Device configuration.

use super::Error;
use std::net::Ipv4Addr;
use std::time::Duration;

const MIN_LATENCY: Duration = Duration::from_nanos(4_000_000);
const ALLOWED_RATES: [u32; 4] = [44_100, 48_000, 88_200, 96_000];

/// PCM bit depth on the wire.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Bits {
    B16,
    B24,
    B32,
}

impl Bits {
    pub fn bits(self) -> u16 {
        match self {
            Bits::B16 => 16,
            Bits::B24 => 24,
            Bits::B32 => 32,
        }
    }

    pub fn bytes(self) -> usize {
        match self {
            Bits::B16 => 2,
            Bits::B24 => 3,
            Bits::B32 => 4,
        }
    }
}

/// Local IPv4 bind: a literal address or an interface name / Windows friendly name.
#[non_exhaustive]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Bind {
    Ip(Ipv4Addr),
    Interface(String),
}

/// Configuration snapshot. Not mutated after [`crate::Device::start`].
#[derive(Clone, Debug)]
pub struct Settings {
    pub name: String,
    pub bind: Bind,
    pub rx_channels: u16,
    pub tx_channels: u16,
    pub sample_rate: u32,
    pub bits: Bits,
    pub rx_latency: Duration,
    pub tx_latency: Duration,
    pub device_id: Option<[u8; 8]>,
    pub process_id: u16,
    pub alt_port: Option<u16>,
    pub rx_channel_names: Option<Vec<String>>,
    pub manufacturer: String,
    pub model: String,
    pub board: String,
    pub vendor_tag: [u8; 8],
    pub ptp_subdomain: [u8; 16],
}

impl Default for Settings {
    fn default() -> Self {
        let mut ptp_subdomain = [0u8; 16];
        ptp_subdomain[..5].copy_from_slice(b"_DFLT");
        Self {
            name: "netaudio".into(),
            bind: Bind::Ip(Ipv4Addr::LOCALHOST),
            rx_channels: 1,
            tx_channels: 0,
            sample_rate: 48_000,
            bits: Bits::B24,
            rx_latency: Duration::from_millis(10),
            tx_latency: Duration::from_millis(10),
            device_id: None,
            process_id: std::process::id() as u16,
            alt_port: None,
            rx_channel_names: None,
            manufacturer: "Mikansei".into(),
            model: "netaudio".into(),
            board: "netaudio-rs".into(),
            vendor_tag: *b"Audinate",
            ptp_subdomain,
        }
    }
}

impl Settings {
    pub fn validate(&self) -> Result<(), Error> {
        if self.rx_latency < MIN_LATENCY {
            return Err(Error::settings("rx_latency below 4 ms"));
        }
        if self.tx_latency < MIN_LATENCY {
            return Err(Error::settings("tx_latency below 4 ms"));
        }
        if self.tx_channels != 0 {
            return Err(Error::settings("tx_channels must be 0 in v1"));
        }
        if self.rx_channels < 1 {
            return Err(Error::settings("rx_channels must be >= 1"));
        }
        if !ALLOWED_RATES.contains(&self.sample_rate) {
            return Err(Error::settings(
                "sample_rate must be 44100/48000/88200/96000",
            ));
        }
        validate_name(&self.name)?;
        if let Some(names) = &self.rx_channel_names
            && names.len() != self.rx_channels as usize
        {
            return Err(Error::settings(
                "rx_channel_names length must match rx_channels",
            ));
        }
        match self.bind {
            Bind::Ip(ip) if ip.is_unspecified() || ip.is_broadcast() || ip.is_multicast() => {
                return Err(Error::UnspecifiedAddress);
            }
            _ => {}
        }
        Ok(())
    }
}

fn validate_name(name: &str) -> Result<(), Error> {
    if name.is_empty() || name.len() > 31 {
        return Err(Error::settings("name must be 1..=31 bytes"));
    }
    if !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
        return Err(Error::settings(
            "name must be a single DNS label [A-Za-z0-9-]",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
