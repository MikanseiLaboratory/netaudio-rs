//! Media 9-byte header.

use super::buf::BeSlice;

pub const HEADER_LEN: usize = 9;
pub const KEEPALIVE: [u8; 2] = [0x13, 0x37];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MediaHeader {
    pub seconds: u32,
    pub index: u32,
}

impl MediaHeader {
    pub fn sample_index(self, sample_rate: u32) -> u64 {
        (self.seconds as u64)
            .wrapping_mul(sample_rate as u64)
            .wrapping_add(self.index as u64)
    }

    pub fn t1_ns(self, sample_rate: u32) -> u64 {
        (self.seconds as u64)
            .wrapping_mul(1_000_000_000)
            .wrapping_add(((self.index as u128) * 1_000_000_000 / (sample_rate as u128)) as u64)
    }
}

pub fn decode(packet: &[u8]) -> Option<(MediaHeader, &[u8])> {
    if packet.len() < HEADER_LEN {
        return None;
    }
    let mut r = BeSlice::new(packet);
    let _byte0 = r.u8()?;
    let h = MediaHeader {
        seconds: r.u32()?,
        index: r.u32()?,
    };
    Some((h, &packet[HEADER_LEN..]))
}

pub fn encode(seconds: u32, index: u32, pcm: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(HEADER_LEN + pcm.len());
    out.push(0x02);
    out.extend_from_slice(&seconds.to_be_bytes());
    out.extend_from_slice(&index.to_be_bytes());
    out.extend_from_slice(pcm);
    out
}

pub fn split_timestamp(sample_index: u64, sample_rate: u32) -> (u32, u32) {
    let rate = sample_rate as u64;
    ((sample_index / rate) as u32, (sample_index % rate) as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_ignores_byte0() {
        let mut pkt = encode(1, 240, &[0, 1, 2]);
        pkt[0] = 0x00;
        let (h, pcm) = decode(&pkt).unwrap();
        assert_eq!(h.seconds, 1);
        assert_eq!(h.index, 240);
        assert_eq!(pcm, &[0, 1, 2]);
        assert_eq!(h.sample_index(48_000), 48_000 + 240);
    }

    #[test]
    fn keepalive_payload() {
        assert_eq!(KEEPALIVE, [0x13, 0x37]);
    }
}
