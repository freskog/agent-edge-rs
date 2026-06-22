//! Async D-Bus client for a running `spotifyd --use-mpris` instance.
//!
//! spotifyd exposes two interfaces (see <https://docs.spotifyd.rs/advanced/dbus.html>):
//!
//! * `rs.spotifyd.Controls` on bus name `rs.spotifyd.instance$PID`, available as
//!   soon as spotifyd connects to Spotify — even when it is NOT the active
//!   playback device. This is where `TransferPlayback` lives, which activates
//!   spotifyd as the Connect device without needing the phone/desktop app. This
//!   is the fix for "spotifyd not visible in available devices".
//! * `org.mpris.MediaPlayer2[.Player]` on bus name
//!   `org.mpris.MediaPlayer2.spotifyd.instance$PID`, present ONLY once spotifyd
//!   is the active device. This is where Play/Pause/Next/OpenUri/etc. live.

use std::collections::HashMap;
use std::time::Duration;

use serde::Serialize;
use thiserror::Error;
use zbus::zvariant::{OwnedValue, Value};
use zbus::{fdo::DBusProxy, Connection, Proxy};

const CONTROLS_PREFIX: &str = "rs.spotifyd.";
const MPRIS_SPOTIFYD_PREFIX: &str = "org.mpris.MediaPlayer2.spotifyd";

const CONTROLS_PATH: &str = "/rs/spotifyd/Controls";
const CONTROLS_IFACE: &str = "rs.spotifyd.Controls";

const MPRIS_PATH: &str = "/org/mpris/MediaPlayer2";
const MPRIS_PLAYER_IFACE: &str = "org.mpris.MediaPlayer2.Player";

/// How long to wait for the MPRIS interface to appear after a TransferPlayback.
const ACTIVATE_TIMEOUT: Duration = Duration::from_secs(5);
const ACTIVATE_POLL_INTERVAL: Duration = Duration::from_millis(300);

#[derive(Debug, Error)]
pub enum SpotifydError {
    /// spotifyd is not on the session bus at all (the `rs.spotifyd.*` name is
    /// absent). Surfaced as HTTP 503 by the server.
    #[error("spotifyd is not running / not on the session bus")]
    NotRunning,

    /// spotifyd is connected (Controls present) but has not become the active
    /// playback device, so the MPRIS player interface is unavailable. Callers
    /// can recover by issuing a transfer first.
    #[error("spotifyd is not the active playback device (MPRIS interface unavailable)")]
    NotActive,

    /// Timed out waiting for spotifyd to become active after a transfer.
    #[error("timed out waiting for spotifyd to become the active device")]
    ActivateTimeout,

    #[error("D-Bus error: {0}")]
    Dbus(#[from] zbus::Error),

    #[error("D-Bus error: {0}")]
    Fdo(#[from] zbus::fdo::Error),
}

/// Current player snapshot returned by `GET /api/spotify/status`.
#[derive(Debug, Serialize)]
pub struct Status {
    /// spotifyd is on the bus (Controls interface present).
    pub available: bool,
    /// spotifyd is the active playback device (MPRIS interface present).
    pub active: bool,
    /// MPRIS `PlaybackStatus` ("Playing"/"Paused"/"Stopped"), if active.
    pub playback_status: Option<String>,
    /// Best-effort "Artist - Title" from MPRIS metadata, if active.
    pub track: Option<String>,
    /// Volume in percent (0-100), if active.
    pub volume: Option<u8>,
}

/// Thin handle over a session-bus connection. Bus names are resolved per call
/// (cheap, and robust against spotifyd restarts which change the `$PID` suffix).
#[derive(Clone)]
pub struct Spotifyd {
    conn: Connection,
}

impl Spotifyd {
    pub async fn connect() -> Result<Self, SpotifydError> {
        let conn = Connection::session().await?;
        Ok(Self { conn })
    }

    async fn list_names(&self) -> Result<Vec<String>, SpotifydError> {
        let dbus = DBusProxy::new(&self.conn).await?;
        let names = dbus.list_names().await?;
        Ok(names.into_iter().map(|n| n.as_str().to_string()).collect())
    }

    /// The `rs.spotifyd.instance$PID` well-known name, if spotifyd is connected.
    async fn controls_name(&self) -> Result<String, SpotifydError> {
        self.list_names()
            .await?
            .into_iter()
            .find(|n| n.starts_with(CONTROLS_PREFIX))
            .ok_or(SpotifydError::NotRunning)
    }

    /// The `org.mpris.MediaPlayer2.spotifyd.instance$PID` name, if active.
    async fn mpris_name(&self) -> Result<Option<String>, SpotifydError> {
        Ok(self
            .list_names()
            .await?
            .into_iter()
            .find(|n| n.starts_with(MPRIS_SPOTIFYD_PREFIX)))
    }

    async fn controls_proxy(&self) -> Result<Proxy<'_>, SpotifydError> {
        let name = self.controls_name().await?;
        let proxy = Proxy::new(&self.conn, name, CONTROLS_PATH, CONTROLS_IFACE).await?;
        Ok(proxy)
    }

    async fn player_proxy(&self) -> Result<Proxy<'_>, SpotifydError> {
        let name = self.mpris_name().await?.ok_or(SpotifydError::NotActive)?;
        let proxy = Proxy::new(&self.conn, name, MPRIS_PATH, MPRIS_PLAYER_IFACE).await?;
        Ok(proxy)
    }

    // --- rs.spotifyd.Controls (available even when inactive) ---

    /// Activate spotifyd as the Spotify Connect device. This is the local
    /// equivalent of "transfer playback to this device".
    ///
    /// `TransferPlayback` (librespot `spirc.activate()`) is asynchronous:
    /// spotifyd only exposes the MPRIS interface once it has actually become
    /// the active device. We therefore wait until that interface appears (or
    /// time out) so that a successful return means the device is ready for
    /// subsequent MPRIS calls (pause/next/volume/...). If spotifyd is already
    /// active this returns immediately.
    pub async fn transfer_playback(&self) -> Result<(), SpotifydError> {
        if self.mpris_name().await?.is_some() {
            return Ok(());
        }
        self.controls_proxy()
            .await?
            .call_method("TransferPlayback", &())
            .await?;
        self.wait_until_active().await?;
        Ok(())
    }

    #[allow(dead_code)] // part of the Controls surface; not yet wired to a route
    pub async fn volume_up(&self) -> Result<(), SpotifydError> {
        self.controls_proxy().await?.call_method("VolumeUp", &()).await?;
        Ok(())
    }

    #[allow(dead_code)] // part of the Controls surface; not yet wired to a route
    pub async fn volume_down(&self) -> Result<(), SpotifydError> {
        self.controls_proxy()
            .await?
            .call_method("VolumeDown", &())
            .await?;
        Ok(())
    }

    // --- MPRIS player (only available when active) ---

    #[allow(dead_code)] // unconditional pause; the pause route uses pause_if_playing
    pub async fn pause(&self) -> Result<(), SpotifydError> {
        self.player_proxy().await?.call_method("Pause", &()).await?;
        Ok(())
    }

    pub async fn resume(&self) -> Result<(), SpotifydError> {
        self.player_proxy().await?.call_method("Play", &()).await?;
        Ok(())
    }

    pub async fn next(&self) -> Result<(), SpotifydError> {
        self.player_proxy().await?.call_method("Next", &()).await?;
        Ok(())
    }

    pub async fn previous(&self) -> Result<(), SpotifydError> {
        self.player_proxy().await?.call_method("Previous", &()).await?;
        Ok(())
    }

    #[allow(dead_code)] // part of the MPRIS surface; not yet wired to a route
    pub async fn stop(&self) -> Result<(), SpotifydError> {
        self.player_proxy().await?.call_method("Stop", &()).await?;
        Ok(())
    }

    /// Pause only if currently playing. Returns whether a pause was issued.
    /// This is the endpoint the wakeword hot path calls; the "was playing"
    /// gate preserves the existing confirmation-beep semantics in the audio
    /// service (don't beep if music was already audibly paused).
    pub async fn pause_if_playing(&self) -> Result<bool, SpotifydError> {
        let proxy = self.player_proxy().await?;
        let status: String = proxy.get_property("PlaybackStatus").await?;
        if status != "Playing" {
            return Ok(false);
        }
        proxy.call_method("Pause", &()).await?;
        Ok(true)
    }

    /// Set volume as a percentage (0-100) via the MPRIS `Volume` property
    /// (0.0-1.0). Requires spotifyd to be active.
    pub async fn set_volume_percent(&self, percent: u8) -> Result<(), SpotifydError> {
        let v = (percent.min(100) as f64) / 100.0;
        self.player_proxy().await?.set_property("Volume", v).await?;
        Ok(())
    }

    /// Ensure spotifyd is the active device (transferring if necessary), then
    /// start playback of the given `spotify:` URI.
    pub async fn play_uri(&self, uri: &str) -> Result<(), SpotifydError> {
        if self.mpris_name().await?.is_none() {
            log::info!("[spotify] not active yet, issuing TransferPlayback before OpenUri");
            // transfer_playback waits until the MPRIS interface is up.
            self.transfer_playback().await?;
        }
        self.player_proxy()
            .await?
            .call_method("OpenUri", &(uri,))
            .await?;
        Ok(())
    }

    /// Poll the bus until the MPRIS interface for spotifyd appears.
    async fn wait_until_active(&self) -> Result<(), SpotifydError> {
        let deadline = std::time::Instant::now() + ACTIVATE_TIMEOUT;
        loop {
            if self.mpris_name().await?.is_some() {
                return Ok(());
            }
            if std::time::Instant::now() >= deadline {
                return Err(SpotifydError::ActivateTimeout);
            }
            tokio::time::sleep(ACTIVATE_POLL_INTERVAL).await;
        }
    }

    pub async fn status(&self) -> Result<Status, SpotifydError> {
        // `available` requires the Controls name; if absent, spotifyd is down.
        let available = match self.controls_name().await {
            Ok(_) => true,
            Err(SpotifydError::NotRunning) => false,
            Err(e) => return Err(e),
        };

        if !available {
            return Ok(Status {
                available: false,
                active: false,
                playback_status: None,
                track: None,
                volume: None,
            });
        }

        let Some(mpris) = self.mpris_name().await? else {
            return Ok(Status {
                available: true,
                active: false,
                playback_status: None,
                track: None,
                volume: None,
            });
        };

        let proxy = Proxy::new(&self.conn, mpris, MPRIS_PATH, MPRIS_PLAYER_IFACE).await?;
        let playback_status: Option<String> = proxy.get_property("PlaybackStatus").await.ok();
        let volume: Option<u8> = proxy
            .get_property::<f64>("Volume")
            .await
            .ok()
            .map(|v| (v * 100.0).round().clamp(0.0, 100.0) as u8);
        let track = proxy
            .get_property::<HashMap<String, OwnedValue>>("Metadata")
            .await
            .ok()
            .and_then(|m| format_track(&m));

        Ok(Status {
            available: true,
            active: true,
            playback_status,
            track,
            volume,
        })
    }
}

/// Best-effort "Artist - Title" from MPRIS xesam metadata.
fn format_track(meta: &HashMap<String, OwnedValue>) -> Option<String> {
    let title = meta.get("xesam:title").and_then(value_to_string);
    let artist = meta.get("xesam:artist").and_then(value_to_string);
    match (artist, title) {
        (Some(a), Some(t)) => Some(format!("{} - {}", a, t)),
        (None, Some(t)) => Some(t),
        _ => None,
    }
}

/// Extract a string from a metadata value that may be a `Str` or an array of
/// `Str` (xesam:artist is an array).
fn value_to_string(v: &OwnedValue) -> Option<String> {
    match &**v {
        Value::Str(s) => Some(s.to_string()),
        Value::Array(arr) => {
            let parts: Vec<String> = arr
                .iter()
                .filter_map(|e| match e {
                    Value::Str(s) => Some(s.to_string()),
                    _ => None,
                })
                .collect();
            if parts.is_empty() {
                None
            } else {
                Some(parts.join(", "))
            }
        }
        _ => None,
    }
}
