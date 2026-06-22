//! Short wake-word confirmation beep.
//!
//! Plays a brief, quiet sine tone the instant a wake word is detected *when no
//! media was paused* (if Spotify/mpv were paused, that pause is already obvious
//! feedback). The tone is synthesized in-process and played through a
//! short-lived cpal output stream on the default output device — the same
//! device the TTS sink uses in the standard deployment.
//!
//! Everything here is best-effort: playback runs on a detached thread and any
//! failure (e.g. the output device being momentarily busy with TTS) is logged
//! at debug level and otherwise ignored, so it can never disrupt detection.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SizedSample};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// Tone frequency in Hz. ~880 Hz (A5) is clearly audible but unobtrusive.
const FREQ_HZ: f32 = 880.0;
/// Total tone duration.
const DURATION_MS: u64 = 120;
/// Peak amplitude (0.0..1.0). Kept low so the beep is quiet, per the UX goal.
const AMPLITUDE: f32 = 0.18;
/// Linear fade in/out length to avoid click/pop transients at the edges.
const FADE_MS: f32 = 8.0;

/// Fire the confirmation beep without blocking the caller. Safe to call from
/// the wake-word detection hot path.
pub fn play_confirmation() {
    std::thread::spawn(|| {
        if let Err(e) = play_tone() {
            log::debug!("[beep] confirmation tone failed: {}", e);
        }
    });
}

fn play_tone() -> Result<(), Box<dyn std::error::Error>> {
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or("no default output device")?;
    let supported = device.default_output_config()?;
    let sample_format = supported.sample_format();
    let config: cpal::StreamConfig = supported.into();

    match sample_format {
        cpal::SampleFormat::F32 => run_tone::<f32>(&device, &config),
        cpal::SampleFormat::I16 => run_tone::<i16>(&device, &config),
        cpal::SampleFormat::U16 => run_tone::<u16>(&device, &config),
        other => Err(format!("unsupported output sample format: {:?}", other).into()),
    }
}

fn run_tone<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
) -> Result<(), Box<dyn std::error::Error>>
where
    T: SizedSample + FromSample<f32>,
{
    let sample_rate = config.sample_rate.0 as f32;
    let channels = config.channels as usize;
    let total_frames = (sample_rate * DURATION_MS as f32 / 1000.0) as usize;
    let fade_frames = (sample_rate * FADE_MS / 1000.0).max(1.0) as usize;

    // Frame counter shared with the audio callback. After `total_frames` the
    // callback emits silence so the stream can be torn down cleanly.
    let frame_idx = Arc::new(AtomicUsize::new(0));
    let cb_frame_idx = Arc::clone(&frame_idx);

    let stream = device.build_output_stream(
        config,
        move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
            for frame in data.chunks_mut(channels) {
                let i = cb_frame_idx.fetch_add(1, Ordering::Relaxed);
                let value = T::from_sample(tone_sample(i, total_frames, fade_frames, sample_rate));
                for sample in frame.iter_mut() {
                    *sample = value;
                }
            }
        },
        |e| log::debug!("[beep] output stream error: {}", e),
        None,
    )?;

    stream.play()?;

    // Hold the stream alive until the tone has fully played out, plus a small
    // tail so the device buffer drains before we drop it.
    std::thread::sleep(Duration::from_millis(DURATION_MS + 60));
    drop(stream);
    Ok(())
}

/// Compute one mono sample of the faded sine tone at frame `i`.
fn tone_sample(i: usize, total_frames: usize, fade_frames: usize, sample_rate: f32) -> f32 {
    if i >= total_frames {
        return 0.0;
    }
    let envelope = if i < fade_frames {
        i as f32 / fade_frames as f32
    } else if i >= total_frames.saturating_sub(fade_frames) {
        (total_frames - i) as f32 / fade_frames as f32
    } else {
        1.0
    };
    let t = i as f32 / sample_rate;
    (2.0 * std::f32::consts::PI * FREQ_HZ * t).sin() * AMPLITUDE * envelope
}
