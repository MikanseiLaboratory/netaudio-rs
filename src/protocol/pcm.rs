//! Left-justified integer PCM (not Q31 multiply).

use crate::Sample;

pub fn promote_16(b0: u8, b1: u8) -> Sample {
    i32::from(i16::from_be_bytes([b0, b1])) << 16
}

pub fn promote_24(b0: u8, b1: u8, b2: u8) -> Sample {
    ((b0 as i8 as i32) << 24) | ((b1 as i32) << 16) | ((b2 as i32) << 8)
}

pub fn promote_32(bytes: [u8; 4]) -> Sample {
    i32::from_be_bytes(bytes)
}

/// Interleaved BE PCM → left-justified i32. Drops a trailing partial frame.
pub fn promote_interleaved(bytes: &[u8], nchan: usize, bytes_per_sample: usize) -> Vec<Sample> {
    if nchan == 0 || bytes_per_sample == 0 {
        return Vec::new();
    }
    let stride = nchan * bytes_per_sample;
    if stride == 0 {
        return Vec::new();
    }
    let frames = bytes.len() / stride;
    let mut out = Vec::with_capacity(frames * nchan);
    for f in 0..frames {
        let base = f * stride;
        for ch in 0..nchan {
            let i = base + ch * bytes_per_sample;
            let s = match bytes_per_sample {
                2 => promote_16(bytes[i], bytes[i + 1]),
                3 => promote_24(bytes[i], bytes[i + 1], bytes[i + 2]),
                4 => promote_32([bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]]),
                _ => 0,
            };
            out.push(s);
        }
    }
    out
}

pub fn encode_interleaved(samples: &[Sample], nchan: usize, bytes_per_sample: usize) -> Vec<u8> {
    if nchan == 0 {
        return Vec::new();
    }
    let frames = samples.len() / nchan;
    let mut out = Vec::with_capacity(frames * nchan * bytes_per_sample);
    for f in 0..frames {
        for ch in 0..nchan {
            let s = samples[f * nchan + ch];
            match bytes_per_sample {
                2 => out.extend_from_slice(&((s >> 16) as i16).to_be_bytes()),
                3 => {
                    let b = s.to_be_bytes();
                    out.extend_from_slice(&b[..3]);
                }
                4 => out.extend_from_slice(&s.to_be_bytes()),
                _ => {}
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sixteen_bit() {
        assert_eq!(promote_16(0x7F, 0xFF), 0x7FFF_0000u32 as i32);
        assert_eq!(promote_16(0x80, 0x00), 0x8000_0000u32 as i32);
    }

    #[test]
    fn twenty_four_bit() {
        assert_eq!(promote_24(0x7F, 0xFF, 0xFF), 0x7FFF_FF00u32 as i32);
    }

    #[test]
    fn thirty_two_bit() {
        assert_eq!(promote_32([0x12, 0x34, 0x56, 0x78]), 0x1234_5678);
    }
}
