//! Planar timestamped ring. Public API converts to interleaved.

use crate::Sample;
use std::sync::Mutex;

pub struct Ring {
    nchan: usize,
    len: usize,
    mask: usize,
    inner: Mutex<Inner>,
}

struct Inner {
    lanes: Vec<Vec<Sample>>,
    filled: Vec<Vec<bool>>,
    patched: Vec<bool>,
    media_ports: Vec<u16>,
    last_packet: Option<std::time::Instant>,
}

impl Ring {
    pub fn new(nchan: usize, latency_samples: u64) -> Self {
        let min_len = 2048usize;
        let need = (latency_samples as usize).saturating_mul(4).max(min_len);
        let len = need.next_power_of_two().max(2);
        let inner = Inner {
            lanes: (0..nchan).map(|_| vec![0; len]).collect(),
            filled: (0..nchan).map(|_| vec![false; len]).collect(),
            patched: vec![false; nchan],
            media_ports: Vec::new(),
            last_packet: None,
        };
        Self {
            nchan,
            len,
            mask: len - 1,
            inner: Mutex::new(inner),
        }
    }

    pub fn set_patched(&self, ch: usize, patched: bool) {
        if let Ok(mut g) = self.inner.lock() {
            if ch < g.patched.len() {
                g.patched[ch] = patched;
            }
        }
    }

    #[allow(dead_code)]
    pub fn any_patched(&self) -> bool {
        self.inner
            .lock()
            .map(|g| g.patched.iter().any(|&p| p))
            .unwrap_or(false)
    }

    pub fn note_port(&self, port: u16) {
        if let Ok(mut g) = self.inner.lock() {
            if !g.media_ports.contains(&port) {
                g.media_ports.push(port);
            }
        }
    }

    pub fn bound_media_ports(&self) -> Vec<u16> {
        self.inner
            .lock()
            .map(|g| g.media_ports.clone())
            .unwrap_or_default()
    }

    pub fn write_packet(
        &self,
        sample_index: u64,
        read_pos: u64,
        flow_to_device: &[Option<usize>],
        interleaved: &[Sample],
        nchan_flow: usize,
    ) {
        if nchan_flow == 0 {
            return;
        }
        let frames = interleaved.len() / nchan_flow;
        let mut g = match self.inner.lock() {
            Ok(g) => g,
            Err(_) => return,
        };
        g.last_packet = Some(std::time::Instant::now());
        for k in 0..frames {
            let pos = sample_index.wrapping_add(k as u64);
            let ahead = pos.wrapping_sub(read_pos);
            if wrapped_diff(read_pos, pos) < 0 {
                continue; // late
            }
            if ahead as usize >= self.len {
                continue; // overrun
            }
            let slot = (pos as usize) & self.mask;
            for (slot_i, map) in flow_to_device.iter().enumerate() {
                let Some(dev) = *map else { continue };
                if dev >= self.nchan {
                    continue;
                }
                let s = interleaved[k * nchan_flow + slot_i];
                g.lanes[dev][slot] = s;
                g.filled[dev][slot] = true;
            }
        }
    }

    /// Copy interleaved frames. Returns (frames, first_sample_index).
    pub fn read(
        &self,
        read_pos: u64,
        nchan: usize,
        max_frames: usize,
        dst: &mut [Sample],
    ) -> (usize, u64) {
        let mut g = match self.inner.lock() {
            Ok(g) => g,
            Err(_) => return (0, read_pos),
        };
        if !g.patched.iter().any(|&p| p) {
            return (0, read_pos);
        }
        if let Some(t) = g.last_packet {
            if t.elapsed()
                > std::time::Duration::from_millis(50) + std::time::Duration::from_millis(4)
            {
                // stall after rx_latency+50ms is handled by caller via last packet;
                // keep returning zeros for a bit then 0 frames if very stale
                if t.elapsed() > std::time::Duration::from_millis(60) {
                    return (0, read_pos);
                }
            }
        } else {
            return (0, read_pos);
        }
        let nchan = nchan.min(self.nchan);
        let max_frames = max_frames.min(dst.len() / nchan.max(1));
        for f in 0..max_frames {
            let pos = read_pos.wrapping_add(f as u64);
            let slot = (pos as usize) & self.mask;
            for ch in 0..nchan {
                let s = if g.filled[ch][slot] {
                    g.filled[ch][slot] = false;
                    g.lanes[ch][slot]
                } else {
                    0
                };
                dst[f * nchan + ch] = s;
            }
        }
        (max_frames, read_pos)
    }
}

fn wrapped_diff(a: u64, b: u64) -> i64 {
    (b as i64).wrapping_sub(a as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unpatched_returns_zero() {
        let r = Ring::new(2, 192);
        let mut buf = [0; 16];
        let (n, _) = r.read(0, 2, 8, &mut buf);
        assert_eq!(n, 0);
    }

    #[test]
    fn reorder_and_late() {
        let r = Ring::new(1, 192);
        r.set_patched(0, true);
        r.write_packet(10, 0, &[Some(0)], &[100, 101], 1);
        r.write_packet(8, 0, &[Some(0)], &[80, 81], 1);
        let mut buf = [0; 8];
        let (n, idx) = r.read(8, 1, 4, &mut buf);
        assert_eq!(n, 4);
        assert_eq!(idx, 8);
        assert_eq!(&buf[..4], &[80, 81, 100, 101]);
        // late packet dropped
        r.write_packet(8, 12, &[Some(0)], &[1], 1);
        let mut buf2 = [0; 1];
        let _ = r.read(12, 1, 1, &mut buf2);
        assert_eq!(buf2[0], 0);
    }
}
