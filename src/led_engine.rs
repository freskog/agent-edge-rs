use crate::alsa_volume;
use crate::led_ring::{LedRing, RgbColor, NUM_LEDS};
use serde::{Deserialize, Serialize};
use std::time::Instant;
use tokio::sync::mpsc;

const FRAME_INTERVAL_MS: u64 = 33; // ~30 fps
const VOLUME_OVERLAY_DURATION_MS: u64 = 2000;
const ERROR_DURATION_MS: u64 = 2000;
const ACK_DURATION_MS: u64 = 1000;
const TIMER_ALERT_DURATION_MS: u64 = 4000;
const VOLUME_STEP: u8 = 1; // 1 LED per step

const MIXER_NAME: &str = "XVF3800 SoftMaster";

const COLOR_LISTENING: RgbColor = RgbColor::new(0, 80, 255);
const COLOR_PROCESSING: RgbColor = RgbColor::new(0, 200, 220);
const COLOR_RESPONDING: RgbColor = RgbColor::new(255, 180, 50);
const COLOR_ERROR: RgbColor = RgbColor::new(255, 0, 0);
const COLOR_ACK: RgbColor = RgbColor::new(0, 220, 80);
const COLOR_VOLUME: RgbColor = RgbColor::new(0, 200, 0);
const COLOR_TIMER_ALERT: RgbColor = RgbColor::new(255, 140, 0);

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum LedEvent {
    Wakeword,
    Processing,
    Responding,
    Error,
    Ack,
    Volume { level: u8 },
    VolumeUp,
    VolumeDown,
    Idle,
    TimerAlert,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LedState {
    Idle,
    Listening,
    Processing,
    Responding,
    Error,
    Ack,
    Volume,
    TimerAlert,
}

pub struct LedEngine {
    ring: LedRing,
    state: LedState,
    previous_state: LedState,
    volume_leds: u8,
    state_entered_at: Instant,
    rx: mpsc::Receiver<LedEvent>,
}

impl LedEngine {
    pub fn new(ring: LedRing, rx: mpsc::Receiver<LedEvent>) -> Self {
        Self {
            ring,
            state: LedState::Idle,
            previous_state: LedState::Idle,
            volume_leds: 6,
            state_entered_at: Instant::now(),
            rx,
        }
    }

    pub fn state(&self) -> LedState {
        self.state
    }

    pub fn volume_percent(&self) -> u8 {
        ((self.volume_leds as f32 / NUM_LEDS as f32) * 100.0).round() as u8
    }

    /// Process pending events, check auto-transitions, render one frame, then sleep.
    pub async fn run_tick(&mut self) {
        while let Ok(event) = self.rx.try_recv() {
            self.handle_event(event);
        }

        self.check_auto_transitions();
        self.render_frame();

        tokio::time::sleep(tokio::time::Duration::from_millis(FRAME_INTERVAL_MS)).await;
    }

    fn handle_event(&mut self, event: LedEvent) {
        match event {
            LedEvent::Wakeword => self.transition(LedState::Listening),
            LedEvent::Processing => self.transition(LedState::Processing),
            LedEvent::Responding => self.transition(LedState::Responding),
            LedEvent::Idle => self.transition(LedState::Idle),
            LedEvent::Error => {
                self.previous_state = self.base_state();
                self.transition(LedState::Error);
            }
            LedEvent::Ack => {
                self.previous_state = self.base_state();
                self.transition(LedState::Ack);
            }
            LedEvent::Volume { level } => {
                self.previous_state = self.base_state();
                self.volume_leds = (level.min(100) as f32 / 100.0 * NUM_LEDS as f32).round() as u8;
                alsa_volume::set_volume(MIXER_NAME, level.min(100));
                self.transition(LedState::Volume);
            }
            LedEvent::VolumeUp => {
                self.previous_state = self.base_state();
                self.volume_leds = (self.volume_leds + VOLUME_STEP).min(NUM_LEDS as u8);
                alsa_volume::set_volume(MIXER_NAME, self.volume_percent());
                self.transition(LedState::Volume);
            }
            LedEvent::VolumeDown => {
                self.previous_state = self.base_state();
                self.volume_leds = self.volume_leds.saturating_sub(VOLUME_STEP);
                alsa_volume::set_volume(MIXER_NAME, self.volume_percent());
                self.transition(LedState::Volume);
            }
            LedEvent::TimerAlert => {
                self.previous_state = self.base_state();
                self.transition(LedState::TimerAlert);
            }
        }
    }

    /// Returns the "base" state (ignoring temporary overlays) for restoration.
    fn base_state(&self) -> LedState {
        match self.state {
            LedState::Volume | LedState::Error | LedState::Ack | LedState::TimerAlert => {
                self.previous_state
            }
            other => other,
        }
    }

    fn transition(&mut self, new_state: LedState) {
        self.state = new_state;
        self.state_entered_at = Instant::now();
    }

    fn check_auto_transitions(&mut self) {
        let elapsed_ms = self.state_entered_at.elapsed().as_millis() as u64;

        match self.state {
            LedState::Error if elapsed_ms >= ERROR_DURATION_MS => {
                self.transition(LedState::Idle);
            }
            LedState::Ack if elapsed_ms >= ACK_DURATION_MS => {
                let restore = self.previous_state;
                self.transition(restore);
            }
            LedState::Volume if elapsed_ms >= VOLUME_OVERLAY_DURATION_MS => {
                let restore = self.previous_state;
                self.transition(restore);
            }
            LedState::TimerAlert if elapsed_ms >= TIMER_ALERT_DURATION_MS => {
                let restore = self.previous_state;
                self.transition(restore);
            }
            _ => {}
        }
    }

    fn render_frame(&mut self) {
        let elapsed_ms = self.state_entered_at.elapsed().as_millis() as u64;

        let frame = match self.state {
            LedState::Idle => [RgbColor::BLACK; NUM_LEDS],
            LedState::Listening => render_listening(elapsed_ms),
            LedState::Processing => render_processing(elapsed_ms),
            LedState::Responding => render_responding(elapsed_ms),
            LedState::Error => render_error(elapsed_ms),
            LedState::Ack => render_ack(elapsed_ms),
            LedState::Volume => render_volume(self.volume_leds),
            LedState::TimerAlert => render_timer_alert(elapsed_ms),
        };

        if let Err(e) = self.ring.set_leds(&frame) {
            log::warn!("Failed to update LEDs: {}", e);
        }
    }
}

/// Pulsing blue glow: all LEDs breathe between dim and bright.
fn render_listening(elapsed_ms: u64) -> [RgbColor; NUM_LEDS] {
    let period_ms = 1500.0;
    let phase = (elapsed_ms as f64 % period_ms) / period_ms;
    // Smooth sine wave between 0.15 and 1.0
    let brightness = 0.15 + 0.85 * ((phase * std::f64::consts::TAU).sin() * 0.5 + 0.5);

    [COLOR_LISTENING.scaled(brightness as f32); NUM_LEDS]
}

/// Spinning cyan dot chasing around the ring with a fading tail.
fn render_processing(elapsed_ms: u64) -> [RgbColor; NUM_LEDS] {
    let period_ms = 800.0;
    let pos = ((elapsed_ms as f64 % period_ms) / period_ms) * NUM_LEDS as f64;

    let mut frame = [RgbColor::BLACK; NUM_LEDS];
    for i in 0..NUM_LEDS {
        let dist = ((pos - i as f64).rem_euclid(NUM_LEDS as f64)).min(
            (i as f64 - pos).rem_euclid(NUM_LEDS as f64),
        );
        // Forward distance only (tail behind the head)
        let forward_dist = (i as f64 - pos).rem_euclid(NUM_LEDS as f64);
        let tail_len = 4.0;
        if forward_dist < 0.5 || dist < 0.5 {
            frame[i] = COLOR_PROCESSING;
        } else if forward_dist > (NUM_LEDS as f64 - tail_len) {
            let tail_pos = (NUM_LEDS as f64 - forward_dist) / tail_len;
            frame[i] = COLOR_PROCESSING.scaled((1.0 - tail_pos) as f32 * 0.6);
        }
    }
    frame
}

/// Flash red twice then fade out over the error duration.
fn render_error(elapsed_ms: u64) -> [RgbColor; NUM_LEDS] {
    let phase = elapsed_ms as f64 / ERROR_DURATION_MS as f64;

    let brightness = if phase < 0.2 {
        // First flash on
        1.0
    } else if phase < 0.3 {
        // First flash off
        0.0
    } else if phase < 0.5 {
        // Second flash on
        1.0
    } else if phase < 0.6 {
        // Second flash off
        0.0
    } else {
        // Fade out
        let fade_phase = (phase - 0.6) / 0.4;
        (1.0 - fade_phase).max(0.0)
    };

    [COLOR_ERROR.scaled(brightness as f32); NUM_LEDS]
}

/// Gentle pulsing warm white/yellow: slower and softer than listening.
fn render_responding(elapsed_ms: u64) -> [RgbColor; NUM_LEDS] {
    let period_ms = 2000.0;
    let phase = (elapsed_ms as f64 % period_ms) / period_ms;
    let brightness = 0.2 + 0.8 * ((phase * std::f64::consts::TAU).sin() * 0.5 + 0.5);

    [COLOR_RESPONDING.scaled(brightness as f32); NUM_LEDS]
}

/// Brief green flash then fade: quick visual confirmation.
fn render_ack(elapsed_ms: u64) -> [RgbColor; NUM_LEDS] {
    let phase = elapsed_ms as f64 / ACK_DURATION_MS as f64;
    let brightness = if phase < 0.3 {
        1.0
    } else {
        let fade = (phase - 0.3) / 0.7;
        (1.0 - fade).max(0.0)
    };
    [COLOR_ACK.scaled(brightness as f32); NUM_LEDS]
}

/// Light up N out of 12 LEDs in green, clockwise from LED 0.
fn render_volume(volume_leds: u8) -> [RgbColor; NUM_LEDS] {
    let mut frame = [RgbColor::BLACK; NUM_LEDS];
    let count = (volume_leds as usize).min(NUM_LEDS);
    for led in frame.iter_mut().take(count) {
        *led = COLOR_VOLUME;
    }
    frame
}

/// Rapid triple-flash amber strobe: 3 quick flashes, dark pause, repeat.
/// Distinct from error (double red) and ack (single green) in both color and rhythm.
fn render_timer_alert(elapsed_ms: u64) -> [RgbColor; NUM_LEDS] {
    let burst_period_ms = 700.0;
    let burst_phase = (elapsed_ms as f64 % burst_period_ms) / burst_period_ms;

    let on = match () {
        _ if burst_phase < 0.10 => true,
        _ if burst_phase < 0.17 => false,
        _ if burst_phase < 0.27 => true,
        _ if burst_phase < 0.34 => false,
        _ if burst_phase < 0.44 => true,
        _ => false,
    };

    if !on {
        return [RgbColor::BLACK; NUM_LEDS];
    }

    let overall = elapsed_ms as f64 / TIMER_ALERT_DURATION_MS as f64;
    let envelope = if overall < 0.8 {
        1.0
    } else {
        ((1.0 - overall) / 0.2).max(0.0)
    };

    [COLOR_TIMER_ALERT.scaled(envelope as f32); NUM_LEDS]
}

