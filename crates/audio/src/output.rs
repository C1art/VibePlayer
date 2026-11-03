use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SizedSample};
use crossbeam::queue::ArrayQueue;
use cut::{SampleRate, Ticks, Timebase};

use crate::{AudioError, Volume};

/// Seconds of audio the queue can hold ahead of the device.
const QUEUE_SECONDS: u32 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OutputFormat {
    pub sample_rate: SampleRate,
    pub channels: u16,
}

/// State shared between the device callback and everyone else. All lock-free.
struct Shared {
    queue: ArrayQueue<f32>,
    channels: usize,
    played_frames: AtomicU64,
    paused: AtomicBool,
    gain_bits: AtomicU32,
    latency_micros: AtomicU64,
    device_lost: AtomicBool,
}

/// An open output device. Owns the stream; keep it on the thread that created it.
pub struct AudioOutput {
    _stream: cpal::Stream,
    shared: Arc<Shared>,
    format: OutputFormat,
}

/// A thread-safe handle for pausing, volume and clock reads.
#[derive(Clone)]
pub struct AudioControl {
    shared: Arc<Shared>,
    format: OutputFormat,
}

impl AudioOutput {
    /// Opens the system's default output device in its preferred format, starting paused.
    pub fn open_default(volume: Volume) -> Result<AudioOutput, AudioError> {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or(AudioError::NoOutputDevice)?;
        let supported = device
            .default_output_config()
            .map_err(|e| AudioError::Device(e.to_string()))?;
        let config = supported.config();
        let format = OutputFormat {
            sample_rate: SampleRate(config.sample_rate),
            channels: config.channels.max(1),
        };
        let capacity =
            (format.sample_rate.0 * QUEUE_SECONDS) as usize * usize::from(format.channels);
        let shared = Arc::new(Shared {
            queue: ArrayQueue::new(capacity.max(1)),
            channels: usize::from(format.channels),
            played_frames: AtomicU64::new(0),
            paused: AtomicBool::new(true),
            gain_bits: AtomicU32::new(volume.gain().to_bits()),
            latency_micros: AtomicU64::new(0),
            device_lost: AtomicBool::new(false),
        });
        use cpal::SampleFormat as F;
        let stream = match supported.sample_format() {
            F::F32 => build::<f32>(&device, &config, &shared),
            F::I16 => build::<i16>(&device, &config, &shared),
            F::U16 => build::<u16>(&device, &config, &shared),
            F::I32 => build::<i32>(&device, &config, &shared),
            other => Err(AudioError::UnsupportedSampleFormat(other.to_string())),
        }?;
        stream
            .play()
            .map_err(|e| AudioError::Device(e.to_string()))?;
        Ok(AudioOutput {
            _stream: stream,
            shared,
            format,
        })
    }

    pub fn format(&self) -> OutputFormat {
        self.format
    }

    pub fn control(&self) -> AudioControl {
        AudioControl {
            shared: Arc::clone(&self.shared),
            format: self.format,
        }
    }

    /// Queues interleaved samples; returns how many fit. Never blocks.
    pub fn push(&self, samples: &[f32]) -> usize {
        samples
            .iter()
            .take_while(|&&sample| self.shared.queue.push(sample).is_ok())
            .count()
    }

    pub fn free_samples(&self) -> usize {
        self.shared.queue.capacity() - self.shared.queue.len()
    }
}

impl AudioControl {
    pub fn format(&self) -> OutputFormat {
        self.format
    }

    pub fn set_paused(&self, paused: bool) {
        self.shared.paused.store(paused, Ordering::Release);
    }

    pub fn set_volume(&self, volume: Volume, muted: bool) {
        let gain = if muted { 0.0 } else { volume.gain() };
        self.shared
            .gain_bits
            .store(gain.to_bits(), Ordering::Relaxed);
    }

    /// Drops queued samples, e.g. after a seek.
    pub fn clear(&self) {
        while self.shared.queue.pop().is_some() {}
    }

    /// Sample frames the device has consumed since the stream opened.
    pub fn played_frames(&self) -> u64 {
        self.shared.played_frames.load(Ordering::Acquire)
    }

    pub fn queued_frames(&self) -> usize {
        self.shared.queue.len() / self.shared.channels
    }

    /// How long after the callback a sample is actually heard, in microseconds.
    pub fn latency(&self) -> Ticks {
        Ticks(self.shared.latency_micros.load(Ordering::Relaxed) as i64)
    }

    /// Microseconds a number of frames lasts at the device rate.
    pub fn frames_to_micros(&self, frames: u64) -> Ticks {
        let timebase = self
            .format
            .sample_rate
            .timebase()
            .unwrap_or(Timebase::MICROS);
        Ticks(i64::try_from(frames).unwrap_or(i64::MAX)).rescale(timebase, Timebase::MICROS)
    }

    pub fn is_device_lost(&self) -> bool {
        self.shared.device_lost.load(Ordering::Relaxed)
    }
}

fn build<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    shared: &Arc<Shared>,
) -> Result<cpal::Stream, AudioError>
where
    T: SizedSample + FromSample<f32>,
{
    let callback_shared = Arc::clone(shared);
    let error_shared = Arc::clone(shared);
    device
        .build_output_stream::<T, _, _>(
            *config,
            move |data: &mut [T], info: &cpal::OutputCallbackInfo| {
                fill(&callback_shared, data);
                let timestamp = info.timestamp();
                let latency = timestamp.playback.duration_since(timestamp.callback);
                callback_shared
                    .latency_micros
                    .store(latency.as_micros() as u64, Ordering::Relaxed);
            },
            move |error: cpal::Error| {
                // Underruns and reroutes leave the stream playing; anything else ends it.
                let recoverable = matches!(
                    error.kind(),
                    cpal::ErrorKind::Xrun
                        | cpal::ErrorKind::DeviceChanged
                        | cpal::ErrorKind::RealtimeDenied
                );
                if !recoverable {
                    error_shared.device_lost.store(true, Ordering::Relaxed);
                }
            },
            None,
        )
        .map_err(|e| AudioError::Device(e.to_string()))
}

/// The device callback: no allocation, locks or blocking.
fn fill<T: SizedSample + FromSample<f32>>(shared: &Shared, data: &mut [T]) {
    let silence = T::from_sample(0.0f32);
    if shared.paused.load(Ordering::Acquire) {
        data.fill(silence);
        return;
    }
    let gain = f32::from_bits(shared.gain_bits.load(Ordering::Relaxed));
    let mut played = 0u64;
    for frame in data.chunks_mut(shared.channels) {
        // The callback is the only consumer, so a full frame is still there after this check.
        if shared.queue.len() < frame.len() {
            frame.fill(silence);
            continue;
        }
        for sample in frame.iter_mut() {
            let value = shared.queue.pop().unwrap_or(0.0);
            *sample = T::from_sample(value * gain);
        }
        played += 1;
    }
    shared.played_frames.fetch_add(played, Ordering::AcqRel);
}
