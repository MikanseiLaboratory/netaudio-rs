//! IEEE 1588-2002 (PTPv1) header parse. Listen-only.

pub const HEADER_LEN: usize = 40;
pub const CONTROL_SYNC: u8 = 0;
pub const CONTROL_FOLLOW_UP: u8 = 2;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PtpHeader {
    pub version_ptp: u16,
    pub subdomain: [u8; 16],
    pub source_uuid: [u8; 6],
    pub sequence_id: u16,
    pub control: u8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Timestamp {
    pub seconds: u32,
    pub nanoseconds: i32,
}

impl Timestamp {
    pub fn as_ns(self) -> u64 {
        (self.seconds as u64)
            .wrapping_mul(1_000_000_000)
            .wrapping_add(self.nanoseconds as u32 as u64)
    }

    pub fn is_zero(self) -> bool {
        self.seconds == 0 && self.nanoseconds == 0
    }
}

pub fn decode_header(packet: &[u8]) -> Option<PtpHeader> {
    if packet.len() < HEADER_LEN {
        return None;
    }
    let version_ptp = u16::from_be_bytes(packet[0..2].try_into().ok()?);
    if version_ptp != 1 {
        return None;
    }
    let mut subdomain = [0u8; 16];
    subdomain.copy_from_slice(&packet[4..20]);
    let mut source_uuid = [0u8; 6];
    source_uuid.copy_from_slice(&packet[22..28]);
    Some(PtpHeader {
        version_ptp,
        subdomain,
        source_uuid,
        sequence_id: u16::from_be_bytes(packet[30..32].try_into().ok()?),
        control: packet[32],
    })
}

pub fn origin_timestamp(packet: &[u8]) -> Option<Timestamp> {
    if packet.len() < HEADER_LEN + 8 {
        return None;
    }
    Some(Timestamp {
        seconds: u32::from_be_bytes(packet[40..44].try_into().ok()?),
        nanoseconds: i32::from_be_bytes(packet[44..48].try_into().ok()?),
    })
}

/// Follow_Up: associatedSequenceId at 40, preciseOriginTimestamp at 44.
pub fn follow_up(packet: &[u8]) -> Option<(u16, Timestamp)> {
    if packet.len() < HEADER_LEN + 12 {
        return None;
    }
    let assoc = u16::from_be_bytes(packet[40..42].try_into().ok()?);
    let ts = Timestamp {
        seconds: u32::from_be_bytes(packet[44..48].try_into().ok()?),
        nanoseconds: i32::from_be_bytes(packet[48..52].try_into().ok()?),
    };
    Some((assoc, ts))
}

pub fn subdomain_matches(got: &[u8; 16], want: &[u8; 16]) -> bool {
    got == want
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_wrong_version() {
        let mut p = vec![0u8; 48];
        p[1] = 2;
        assert!(decode_header(&p).is_none());
    }

    #[test]
    fn sync_origin() {
        let mut p = vec![0u8; 48];
        p[1] = 1; // versionPTP
        p[3] = 1;
        p[32] = CONTROL_SYNC;
        p[40..44].copy_from_slice(&1u32.to_be_bytes());
        p[44..48].copy_from_slice(&500_000_000i32.to_be_bytes());
        let h = decode_header(&p).unwrap();
        assert_eq!(h.control, CONTROL_SYNC);
        let ts = origin_timestamp(&p).unwrap();
        assert_eq!(ts.as_ns(), 1_500_000_000);
    }
}
