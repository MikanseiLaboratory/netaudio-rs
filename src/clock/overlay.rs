//! Overlay clock over `std::time::Instant` with seqlock snapshot.

use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU32, AtomicU64, AtomicU8, Ordering};
use std::time::Instant;

pub type FineTime = u64;
pub type FineTimeDiff = i64;

const ALPHA: f64 = 0.125;
const MAX_PPM: f64 = 500e-6;
const OUTLIER_NS: i64 = 200_000_000;
const SLEW_ACQUIRE_NS: i64 = 1_000_000;
const SLEW_LOCK_NS: i64 = 50_000;
const LOCK_ERR_NS: i64 = 100_000;
const UNLOCK_ERR_NS: i64 = 1_000_000;
const MEDIA_DECIMATE_NS: u64 = 125_000_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClockStatus {
    Unlocked,
    MediaDriven,
    PtpLocked,
}

#[derive(Clone, Copy, Debug)]
pub enum Source {
    Ptp,
    Media,
}

const SRC_NONE: u8 = 0;
const SRC_MEDIA: u8 = 1;
const SRC_PTP: u8 = 2;

pub struct OverlayClock {
    origin: Instant,
    seq: AtomicU64,
    last_sync: AtomicU64,
    shift: AtomicI64,
    freq_scale_bits: AtomicU64,
    last_master: AtomicU64,
    lock_streak: AtomicU32,
    acquiring: AtomicBool,
    source: AtomicU8,
    last_media_t2: AtomicU64,
    initialized: AtomicBool,
}

impl OverlayClock {
    pub fn new(origin: Instant) -> Self {
        Self {
            origin,
            seq: AtomicU64::new(0),
            last_sync: AtomicU64::new(0),
            shift: AtomicI64::new(0),
            freq_scale_bits: AtomicU64::new(0f64.to_bits()),
            last_master: AtomicU64::new(0),
            lock_streak: AtomicU32::new(0),
            acquiring: AtomicBool::new(true),
            source: AtomicU8::new(SRC_NONE),
            last_media_t2: AtomicU64::new(0),
            initialized: AtomicBool::new(false),
        }
    }

    #[allow(dead_code)]
    pub fn origin(&self) -> Instant {
        self.origin
    }

    pub fn local_ns(&self) -> FineTime {
        self.origin.elapsed().as_nanos() as FineTime
    }

    #[allow(dead_code)]
    pub fn local_ns_at(&self, t: Instant) -> FineTime {
        t.saturating_duration_since(self.origin).as_nanos() as FineTime
    }

    fn freq_scale(&self) -> f64 {
        f64::from_bits(self.freq_scale_bits.load(Ordering::Relaxed))
    }

    fn overlay_at(
        &self,
        local: FineTime,
        last_sync: FineTime,
        shift: FineTimeDiff,
        freq: f64,
    ) -> FineTime {
        let elapsed = (local as FineTimeDiff).wrapping_sub(last_sync as FineTimeDiff);
        let corr = (elapsed as f64 * freq) as FineTimeDiff;
        (local as FineTimeDiff)
            .wrapping_add(shift)
            .wrapping_add(corr) as FineTime
    }

    pub fn now_ns(&self) -> FineTime {
        let local = self.local_ns();
        if !self.initialized.load(Ordering::Acquire) {
            return local;
        }
        loop {
            let s1 = self.seq.load(Ordering::Acquire);
            if s1 & 1 != 0 {
                continue;
            }
            let last_sync = self.last_sync.load(Ordering::Acquire);
            let shift = self.shift.load(Ordering::Acquire);
            let freq = f64::from_bits(self.freq_scale_bits.load(Ordering::Acquire));
            let s2 = self.seq.load(Ordering::Acquire);
            if s1 == s2 && s2 & 1 == 0 {
                return self.overlay_at(local, last_sync, shift, freq);
            }
        }
    }

    pub fn freq_scale_ppb(&self) -> i32 {
        (self.freq_scale() * 1_000_000_000.0) as i32
    }

    pub fn status(&self) -> ClockStatus {
        match self.source.load(Ordering::Acquire) {
            SRC_PTP if self.lock_streak.load(Ordering::Acquire) >= 4 => ClockStatus::PtpLocked,
            SRC_PTP | SRC_MEDIA => ClockStatus::MediaDriven,
            _ => ClockStatus::Unlocked,
        }
    }

    pub fn observe(&self, t1: FineTime, t2: FineTime, source: Source) {
        if matches!(source, Source::Media) {
            let last = self.last_media_t2.load(Ordering::Relaxed);
            if t2.wrapping_sub(last) < MEDIA_DECIMATE_NS && last != 0 {
                return;
            }
            self.last_media_t2.store(t2, Ordering::Relaxed);
            if self.status() == ClockStatus::PtpLocked {
                return;
            }
        }

        if !self.initialized.load(Ordering::Acquire) {
            self.publish(
                t2,
                (t1 as FineTimeDiff).wrapping_sub(t2 as FineTimeDiff),
                0.0,
            );
            self.last_master.store(t1, Ordering::Release);
            self.initialized.store(true, Ordering::Release);
            self.acquiring.store(true, Ordering::Release);
            self.set_source(source);
            return;
        }

        let last_sync = self.last_sync.load(Ordering::Acquire);
        let last_master = self.last_master.load(Ordering::Acquire);
        let local_dt = (t2 as FineTimeDiff).wrapping_sub(last_sync as FineTimeDiff);
        let master_dt = (t1 as FineTimeDiff).wrapping_sub(last_master as FineTimeDiff);
        if local_dt <= 0 || master_dt <= 0 {
            return;
        }
        if !(200_000_000..=4_000_000_000).contains(&local_dt) {
            return;
        }
        if (master_dt - local_dt).abs() > 50_000_000 {
            return;
        }

        let mut raw = (master_dt as f64 / local_dt as f64) - 1.0;
        raw = raw.clamp(-MAX_PPM, MAX_PPM);
        let freq = (1.0 - ALPHA) * self.freq_scale() + ALPHA * raw;

        let shift = self.shift.load(Ordering::Acquire);
        let pred = self.overlay_at(t2, last_sync, shift, self.freq_scale());
        let error = (t1 as FineTimeDiff).wrapping_sub(pred as FineTimeDiff);

        if error.abs() > OUTLIER_NS {
            return;
        }

        let acquiring = self.acquiring.load(Ordering::Acquire);
        let max_step = if acquiring {
            SLEW_ACQUIRE_NS
        } else {
            SLEW_LOCK_NS
        };
        let applied = error.clamp(-max_step, max_step);
        let new_at_t2 = (pred as FineTimeDiff).wrapping_add(applied) as FineTime;
        let new_shift = (new_at_t2 as FineTimeDiff).wrapping_sub(t2 as FineTimeDiff);

        self.publish(t2, new_shift, freq);
        self.last_master.store(t1, Ordering::Release);

        if error.abs() < LOCK_ERR_NS {
            let n = self.lock_streak.fetch_add(1, Ordering::AcqRel) + 1;
            if n >= 4 {
                self.acquiring.store(false, Ordering::Release);
            }
        } else if error.abs() > UNLOCK_ERR_NS {
            self.lock_streak.store(0, Ordering::Release);
            self.acquiring.store(true, Ordering::Release);
        }
        self.set_source(source);
    }

    fn set_source(&self, source: Source) {
        let v = match source {
            Source::Ptp => SRC_PTP,
            Source::Media => SRC_MEDIA,
        };
        self.source.store(v, Ordering::Release);
    }

    fn publish(&self, last_sync: FineTime, shift: FineTimeDiff, freq: f64) {
        self.seq.fetch_add(1, Ordering::Release);
        self.last_sync.store(last_sync, Ordering::Release);
        self.shift.store(shift, Ordering::Release);
        self.freq_scale_bits
            .store(freq.to_bits(), Ordering::Release);
        self.seq.fetch_add(1, Ordering::Release);
    }

    pub fn mark_unlocked_if_stale(&self, idle_ns: u64) {
        if self.status() == ClockStatus::PtpLocked {
            return;
        }
        let last = self.last_media_t2.load(Ordering::Acquire);
        if last != 0 && self.local_ns().wrapping_sub(last) > idle_ns {
            self.source.store(SRC_NONE, Ordering::Release);
            self.lock_streak.store(0, Ordering::Release);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn wrapping_overlay() {
        let c = OverlayClock::new(Instant::now());
        c.publish(u64::MAX - 10, 100, 0.0);
        c.initialized.store(true, Ordering::Release);
        let v = c.overlay_at(5, u64::MAX - 10, 100, 0.0);
        assert_eq!(v, 5u64.wrapping_add(100));
    }

    #[test]
    fn acquire_then_slew() {
        let origin = Instant::now();
        let c = OverlayClock::new(origin);
        c.observe(1_000_000_000, 0, Source::Ptp);
        assert!(c.initialized.load(Ordering::Acquire));
        std::thread::sleep(Duration::from_millis(5));
        let t2 = 250_000_000;
        c.last_sync.store(0, Ordering::Release);
        c.observe(1_250_000_000, t2, Source::Ptp);
        let _ = c.now_ns();
    }

    #[test]
    fn ema_100ppm() {
        let origin = Instant::now();
        let c = OverlayClock::new(origin);
        let mut t1 = 0u64;
        let mut t2 = 0u64;
        c.observe(t1, t2, Source::Ptp);
        for _ in 0..40 {
            t1 += 1_000_100_000; // 100 ppm fast master
            t2 += 1_000_000_000;
            c.observe(t1, t2, Source::Ptp);
        }
        let ppm = c.freq_scale() * 1e6;
        assert!(ppm > 50.0 && ppm < 150.0, "ppm={ppm}");
    }

    #[test]
    fn outlier_dropped() {
        let c = OverlayClock::new(Instant::now());
        c.observe(0, 0, Source::Ptp);
        c.last_sync.store(0, Ordering::Release);
        c.last_master.store(0, Ordering::Release);
        // 1s later, t1 jumps by 1 second extra (would be 1s error) — actually
        // master_dt vs local_dt gate of 50ms would drop a 1s jump.
        c.observe(2_000_000_000, 1_000_000_000, Source::Ptp);
        assert_eq!(c.last_master.load(Ordering::Acquire), 0);
    }
}
