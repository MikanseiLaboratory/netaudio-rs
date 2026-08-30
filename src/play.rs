//! cpal playback adapter: `Device::try_read` → OS default output.

use crate::{AudioFrameMut, Device, MediaTime, Sample};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample, Stream, StreamConfig};
use std::sync::Arc;
use std::sync::atomic::{AtomicI32, AtomicU64, Ordering};

/// Full-scale divisor for left-justified [`Sample`] → `f32` (`2^31`).
pub const I32_FS: f32 = 2_147_483_648.0;

/// Left-justified `i32` to `[-1.0, 1.0)` float.
pub fn sample_to_f32(s: Sample) -> f32 {
    s as f32 / I32_FS
}

/// Map interleaved frames from `src_ch` to `dst_ch`.
///
/// Copies `min(src_ch, dst_ch)` channels. Mono source with `dst_ch >= 2`
/// is duplicated to L/R. Extra destination channels are silence.
pub fn map_channels(src: &[f32], src_ch: usize, dst: &mut [f32], dst_ch: usize) {
    if src_ch == 0 || dst_ch == 0 {
        dst.fill(0.0);
        return;
    }
    let src_frames = src.len() / src_ch;
    let dst_frames = dst.len() / dst_ch;
    let frames = src_frames.min(dst_frames);
    for f in 0..frames {
        if src_ch == 1 && dst_ch >= 2 {
            let s = src[f * src_ch];
            dst[f * dst_ch] = s;
            dst[f * dst_ch + 1] = s;
            for c in 2..dst_ch {
                dst[f * dst_ch + c] = 0.0;
            }
        } else {
            let n = src_ch.min(dst_ch);
            for c in 0..n {
                dst[f * dst_ch + c] = src[f * src_ch + c];
            }
            for c in n..dst_ch {
                dst[f * dst_ch + c] = 0.0;
            }
        }
    }
    let filled = frames * dst_ch;
    if filled < dst.len() {
        dst[filled..].fill(0.0);
    }
}

/// One-shot linear resample, same channel count. Rates equal → copy.
pub fn resample_linear(src: &[f32], nchan: usize, src_rate: u32, dst: &mut [f32], dst_rate: u32) {
    if nchan == 0 {
        dst.fill(0.0);
        return;
    }
    let src_frames = src.len() / nchan;
    let dst_frames = dst.len() / nchan;
    if src_frames == 0 {
        dst.fill(0.0);
        return;
    }
    if src_rate == dst_rate {
        let n = src.len().min(dst.len());
        dst[..n].copy_from_slice(&src[..n]);
        if n < dst.len() {
            dst[n..].fill(0.0);
        }
        return;
    }
    let step = f64::from(src_rate) / f64::from(dst_rate);
    let last = src_frames - 1;
    for i in 0..dst_frames {
        let pos = i as f64 * step;
        let i0 = (pos.floor() as usize).min(last);
        let i1 = (i0 + 1).min(last);
        let t = (pos - i0 as f64) as f32;
        for ch in 0..nchan {
            let a = src[i0 * nchan + ch];
            let b = src[i1 * nchan + ch];
            dst[i * nchan + ch] = a + (b - a) * t;
        }
    }
}

/// Failure opening or running the default output device.
#[derive(Debug, thiserror::Error)]
pub enum PlayError {
    #[error("no default output device")]
    NoDevice,
    #[error("audio device: {0}")]
    Device(String),
    #[error("unsupported sample format: {0}")]
    SampleFormat(String),
}

/// Handle for a running default-output stream. Dropping stops playback.
pub struct Output {
    _stream: Stream,
    device_name: String,
    sample_rate: u32,
    channels: u16,
    peaks: Arc<Vec<AtomicI32>>,
    frames: Arc<AtomicU64>,
}

impl Output {
    /// Open the host default output and pull PCM from `device` in the callback.
    pub fn start_default(device: Arc<Device>) -> Result<Self, PlayError> {
        let host = cpal::default_host();
        let cpal_dev = host.default_output_device().ok_or(PlayError::NoDevice)?;
        let device_name = cpal_dev.to_string();
        let src_rate = device.sample_rate();
        let src_ch = device.rx_channels() as usize;
        let (config, sample_format) = pick_config(&cpal_dev, src_rate)?;
        let dst_rate = config.sample_rate;
        let dst_ch = config.channels as usize;
        let peaks = Arc::new((0..src_ch).map(|_| AtomicI32::new(0)).collect::<Vec<_>>());
        let frames = Arc::new(AtomicU64::new(0));
        let writer = Writer {
            device,
            src_ch,
            src_rate,
            dst_ch,
            dst_rate,
            peaks: Arc::clone(&peaks),
            frames: Arc::clone(&frames),
            pcm: Vec::new(),
            f32_src: Vec::new(),
            f32_rs: Vec::new(),
        };
        let stream = match sample_format {
            SampleFormat::F32 => build_stream::<f32>(&cpal_dev, config, writer)?,
            SampleFormat::F64 => build_stream::<f64>(&cpal_dev, config, writer)?,
            SampleFormat::I16 => build_stream::<i16>(&cpal_dev, config, writer)?,
            SampleFormat::I32 => build_stream::<i32>(&cpal_dev, config, writer)?,
            SampleFormat::U16 => build_stream::<u16>(&cpal_dev, config, writer)?,
            other => {
                return Err(PlayError::SampleFormat(format!("{other}")));
            }
        };
        stream
            .play()
            .map_err(|e| PlayError::Device(e.to_string()))?;
        Ok(Self {
            _stream: stream,
            device_name,
            sample_rate: dst_rate,
            channels: config.channels,
            peaks,
            frames,
        })
    }

    pub fn device_name(&self) -> &str {
        &self.device_name
    }

    /// Output device sample rate (Hz).
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Output device channel count.
    pub fn channels(&self) -> u16 {
        self.channels
    }

    /// Swap out accumulated Dante-channel peaks and frame count since last call.
    pub fn snapshot_meters(&self) -> (u64, Vec<Sample>) {
        let n = self.frames.swap(0, Ordering::AcqRel);
        let peaks = self
            .peaks
            .iter()
            .map(|p| p.swap(0, Ordering::AcqRel))
            .collect();
        (n, peaks)
    }
}

struct Writer {
    device: Arc<Device>,
    src_ch: usize,
    src_rate: u32,
    dst_ch: usize,
    dst_rate: u32,
    peaks: Arc<Vec<AtomicI32>>,
    frames: Arc<AtomicU64>,
    pcm: Vec<Sample>,
    f32_src: Vec<f32>,
    f32_rs: Vec<f32>,
}

impl Writer {
    fn fill_f32(&mut self, out: &mut [f32]) {
        out.fill(0.0);
        if self.dst_ch == 0 || self.src_ch == 0 {
            return;
        }
        let dst_frames = out.len() / self.dst_ch;
        if dst_frames == 0 {
            return;
        }
        let src_needed = src_frames_for(dst_frames, self.src_rate, self.dst_rate);
        self.pcm.resize(src_needed * self.src_ch, 0);
        let mut frame = AudioFrameMut {
            media_time: MediaTime {
                sample_index: 0,
                ns: 0,
            },
            sample_rate: 0,
            channels: 0,
            samples: &mut self.pcm,
        };
        let n = self.device.try_read(&mut frame).unwrap_or_default();
        if n == 0 {
            return;
        }
        self.frames.fetch_add(n as u64, Ordering::Relaxed);
        for frame_i in 0..n {
            for ch in 0..self.src_ch {
                let s = self.pcm[frame_i * self.src_ch + ch];
                let abs = s.saturating_abs();
                if let Some(slot) = self.peaks.get(ch) {
                    slot.fetch_max(abs, Ordering::Relaxed);
                }
            }
        }
        self.f32_src.resize(n * self.src_ch, 0.0);
        for (i, s) in self.pcm[..n * self.src_ch].iter().enumerate() {
            self.f32_src[i] = sample_to_f32(*s);
        }
        if self.src_rate == self.dst_rate {
            map_channels(&self.f32_src, self.src_ch, out, self.dst_ch);
            return;
        }
        self.f32_rs.resize(dst_frames * self.src_ch, 0.0);
        resample_linear(
            &self.f32_src,
            self.src_ch,
            self.src_rate,
            &mut self.f32_rs,
            self.dst_rate,
        );
        map_channels(&self.f32_rs, self.src_ch, out, self.dst_ch);
    }
}

fn src_frames_for(dst_frames: usize, src_rate: u32, dst_rate: u32) -> usize {
    if dst_rate == 0 || src_rate == dst_rate {
        return dst_frames;
    }
    let n = (dst_frames as u64)
        .saturating_mul(u64::from(src_rate))
        .div_ceil(u64::from(dst_rate))
        .saturating_add(1);
    n.max(1) as usize
}

fn pick_config(
    dev: &cpal::Device,
    want_rate: u32,
) -> Result<(StreamConfig, SampleFormat), PlayError> {
    let ranges = match dev.supported_output_configs() {
        Ok(r) => r,
        Err(e) => {
            return fallback_default(dev, e.to_string());
        }
    };
    let mut matched_other: Option<cpal::SupportedStreamConfigRange> = None;
    for range in ranges {
        if !range.contains_rate(want_rate) {
            continue;
        }
        if range.sample_format() == SampleFormat::F32 {
            if let Some(cfg) = range.try_with_sample_rate(want_rate) {
                let fmt = cfg.sample_format();
                return Ok((cfg.into(), fmt));
            }
        } else if matched_other.is_none() {
            matched_other = Some(range);
        }
    }
    if let Some(range) = matched_other
        && let Some(cfg) = range.try_with_sample_rate(want_rate)
    {
        let fmt = cfg.sample_format();
        return Ok((cfg.into(), fmt));
    }
    fallback_default(dev, "no matching config".into())
}

fn fallback_default(
    dev: &cpal::Device,
    why: String,
) -> Result<(StreamConfig, SampleFormat), PlayError> {
    let default = dev
        .default_output_config()
        .map_err(|e| PlayError::Device(format!("{why}; default: {e}")))?;
    let fmt = default.sample_format();
    Ok((default.into(), fmt))
}

fn build_stream<T>(
    cpal_dev: &cpal::Device,
    config: StreamConfig,
    mut writer: Writer,
) -> Result<Stream, PlayError>
where
    T: SizedSample + FromSample<f32>,
{
    let dst_ch = writer.dst_ch;
    let mut mix = Vec::<f32>::new();
    cpal_dev
        .build_output_stream(
            config,
            move |data: &mut [T], _| {
                let n = data.len();
                mix.resize(n, 0.0);
                if dst_ch > 0 {
                    writer.fill_f32(&mut mix);
                }
                for (o, s) in data.iter_mut().zip(mix.iter()) {
                    *o = T::from_sample(*s);
                }
            },
            |err| log::error!("cpal output: {err}"),
            None,
        )
        .map_err(|e| PlayError::Device(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sample_to_f32_full_scale() {
        assert!((sample_to_f32(0) - 0.0).abs() < 1e-9);
        let pos = sample_to_f32(0x7FFF_0000u32 as i32);
        assert!((pos - (0x7FFF_0000u32 as i32 as f32 / I32_FS)).abs() < 1e-9);
        assert!(sample_to_f32(i32::MIN) <= -1.0);
    }

    #[test]
    fn map_stereo_copy() {
        let src = [0.1, 0.2, 0.3, 0.4];
        let mut dst = [0.0; 4];
        map_channels(&src, 2, &mut dst, 2);
        assert_eq!(dst, src);
    }

    #[test]
    fn map_mono_to_stereo_duplicates() {
        let src = [0.5, -0.25];
        let mut dst = [0.0; 4];
        map_channels(&src, 1, &mut dst, 2);
        assert_eq!(dst, [0.5, 0.5, -0.25, -0.25]);
    }

    #[test]
    fn map_many_to_few_truncates() {
        let src = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0];
        let mut dst = [0.0; 4];
        map_channels(&src, 3, &mut dst, 2);
        assert_eq!(dst, [1.0, 2.0, 4.0, 5.0]);
    }

    #[test]
    fn map_few_to_many_pads_zero() {
        let src = [0.1, 0.2, 0.3, 0.4];
        let mut dst = [9.0; 6];
        map_channels(&src, 2, &mut dst, 3);
        assert_eq!(dst, [0.1, 0.2, 0.0, 0.3, 0.4, 0.0]);
    }

    #[test]
    fn resample_same_rate_copies() {
        let src = [0.0, 1.0, 0.5, -0.5];
        let mut dst = [0.0; 4];
        resample_linear(&src, 2, 48_000, &mut dst, 48_000);
        assert_eq!(dst, src);
    }

    #[test]
    fn resample_linear_halves_rate() {
        // 4 frames @ 2ch → 2 frames. step = 2, so dest[0]=src[0], dest[1]=src[2]
        let src = [0.0, 10.0, 1.0, 11.0, 2.0, 12.0, 3.0, 13.0];
        let mut dst = [0.0; 4];
        resample_linear(&src, 2, 48_000, &mut dst, 24_000);
        assert!((dst[0] - 0.0).abs() < 1e-5);
        assert!((dst[1] - 10.0).abs() < 1e-5);
        assert!((dst[2] - 2.0).abs() < 1e-5);
        assert!((dst[3] - 12.0).abs() < 1e-5);
    }

    #[test]
    fn src_frames_ceil() {
        assert_eq!(src_frames_for(256, 48_000, 48_000), 256);
        assert_eq!(src_frames_for(256, 48_000, 44_100), 280);
    }
}
