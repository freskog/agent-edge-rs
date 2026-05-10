use crate::alsa_volume;
use crate::led_ring::{LedRing, RgbColor, NUM_LEDS};
use serde::{Deserialize, Serialize};
use std::time::Instant;
use tokio::sync::mpsc;

/// Tick rate for animated states (Init, Listening, Processing, Responding,
/// Error, Ack, TimerAlert). Each tick recomputes the frame.
const ANIMATED_TICK_MS: u64 = 33; // ~30 fps
/// Tick rate for static states (Idle, Volume). The engine still wakes
/// periodically to drain events and check for auto-transitions, but the
/// frame doesn't change so the I²C bus stays idle (frame-diffed writes).
const STATIC_TICK_MS: u64 = 100;
const VOLUME_OVERLAY_DURATION_MS: u64 = 2000;
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

/// Breathing brightness envelope for the Responding state. Lighting all 12
/// LEDs steady at full brightness pulls ~600-800 mA off the codec's supply
/// rail, which sags it enough to crosstalk audible noise into the speaker
/// during TTS. The PEAK here is set so that 12 LEDs at peak draw roughly the
/// same current as a single LED at full brightness (empirically verified
/// crackle-free). The valley dims to a faint glow so the breath is visible.
/// Raise PEAK if you want more glow; lower it if crackle creeps back.
const RESPONDING_BREATH_MIN: f32 = 0.05;
const RESPONDING_BREATH_PEAK: f32 = 0.15;

const ERROR_PULSE_PERIOD_MS: f64 = 1500.0;
const ERROR_BREATH_MIN: f32 = 0.10;
const ERROR_BREATH_PEAK: f32 = 1.0;

const INIT_BREATH_MIN: f32 = 0.10;
const INIT_BREATH_PEAK: f32 = 1.0;
const INIT_BREATH_PERIOD_MS: f64 = 4000.0;
const INIT_HUE_PERIOD_MS: f64 = 12_000.0;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum LedEvent {
    Init,
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
    Init,
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
    /// Last frame actually written over I²C. Used to skip redundant writes
    /// when a state's rendered frame is unchanged tick-to-tick — this is
    /// what keeps the bus silent during static states (Idle, Volume,
    /// Responding) and, critically, during TTS playback so I²C activity
    /// can't bleed into the I²S audio path.
    last_written: Option<[RgbColor; NUM_LEDS]>,
}

impl LedEngine {
    pub fn new(ring: LedRing, rx: mpsc::Receiver<LedEvent>) -> Self {
        Self {
            ring,
            state: LedState::Init,
            previous_state: LedState::Init,
            volume_leds: 6,
            state_entered_at: Instant::now(),
            rx,
            last_written: None,
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

        // Animated states need a fast tick to look smooth; static states
        // (Idle, Responding, Volume) keep rendering the same frame, so the
        // I²C bus stays idle thanks to frame-diffing and we can poll for
        // events less often.
        let interval = if Self::is_animated_state(self.state) {
            ANIMATED_TICK_MS
        } else {
            STATIC_TICK_MS
        };
        tokio::time::sleep(tokio::time::Duration::from_millis(interval)).await;
    }

    fn is_animated_state(state: LedState) -> bool {
        matches!(
            state,
            LedState::Init
                | LedState::Listening
                | LedState::Processing
                | LedState::Responding
                | LedState::Error
                | LedState::Ack
                | LedState::TimerAlert
        )
    }

    fn handle_event(&mut self, event: LedEvent) {
        match event {
            LedEvent::Init => self.transition(LedState::Init),
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
            LedState::Volume | LedState::Ack | LedState::TimerAlert => self.previous_state,
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
            LedState::Init => render_init(elapsed_ms),
            LedState::Listening => render_listening(elapsed_ms),
            LedState::Processing => render_processing(elapsed_ms),
            LedState::Responding => render_responding(elapsed_ms),
            LedState::Error => render_error(elapsed_ms),
            LedState::Ack => render_ack(elapsed_ms),
            LedState::Volume => render_volume(self.volume_leds),
            LedState::TimerAlert => render_timer_alert(elapsed_ms),
        };

        // Frame-diff: only touch the I²C bus when the on-screen frame
        // actually changes. For static states (Idle, Volume) this means a
        // single write on entry and then total bus silence; animated states
        // still write every tick. (I²C traffic is not the crackle source —
        // LED current draw is — but skipping redundant writes is still nice
        // for the bus.)
        if self.last_written.as_ref() == Some(&frame) {
            return;
        }
        if let Err(e) = self.ring.set_leds(&frame) {
            log::warn!("Failed to update LEDs: {}", e);
            return;
        }
        self.last_written = Some(frame);
    }
}

/// Slow rainbow breath for the Init state: every LED holds a different hue
/// around the color wheel, the whole wheel rotates slowly, and the overall
/// brightness breathes in and out on a long period.
fn render_init(elapsed_ms: u64) -> [RgbColor; NUM_LEDS] {
    let breath_phase = (elapsed_ms as f64 % INIT_BREATH_PERIOD_MS) / INIT_BREATH_PERIOD_MS;
    let breath = (breath_phase * std::f64::consts::TAU).sin() * 0.5 + 0.5;
    let v = INIT_BREATH_MIN + (INIT_BREATH_PEAK - INIT_BREATH_MIN) * breath as f32;

    let hue_offset = (elapsed_ms as f64 % INIT_HUE_PERIOD_MS) / INIT_HUE_PERIOD_MS * 360.0;
    let mut frame = [RgbColor::BLACK; NUM_LEDS];
    for (i, led) in frame.iter_mut().enumerate() {
        let hue = (hue_offset + (i as f64 / NUM_LEDS as f64) * 360.0) as f32;
        *led = RgbColor::from_hsv(hue, 1.0, v);
    }
    frame
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

/// Continuously looping slow red breath. Error is a persistent state, so this
/// must never settle to a fixed value — only an explicit event clears it.
fn render_error(elapsed_ms: u64) -> [RgbColor; NUM_LEDS] {
    let phase = (elapsed_ms as f64 % ERROR_PULSE_PERIOD_MS) / ERROR_PULSE_PERIOD_MS;
    let breath = (phase * std::f64::consts::TAU).sin() * 0.5 + 0.5;
    let v = ERROR_BREATH_MIN + (ERROR_BREATH_PEAK - ERROR_BREATH_MIN) * breath as f32;
    [COLOR_ERROR.scaled(v); NUM_LEDS]
}

/// Soft warm-yellow breathing on all 12 LEDs during TTS playback. The
/// brightness envelope is intentionally narrow and dim: peak brightness is
/// capped so 12 LEDs at peak draw roughly the same current as a single LED
/// at full, which keeps the codec's supply rail stable and avoids the
/// audible crosstalk we'd otherwise hear coupling into the speaker.
fn render_responding(elapsed_ms: u64) -> [RgbColor; NUM_LEDS] {
    let period_ms = 2000.0;
    let phase = (elapsed_ms as f64 % period_ms) / period_ms;
    let breath = (phase * std::f64::consts::TAU).sin() * 0.5 + 0.5; // 0..=1
    let span = RESPONDING_BREATH_PEAK - RESPONDING_BREATH_MIN;
    let brightness = RESPONDING_BREATH_MIN + span * breath as f32;
    [COLOR_RESPONDING.scaled(brightness); NUM_LEDS]
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

