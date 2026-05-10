use thiserror::Error;
use zbus::blocking::{fdo::DBusProxy, Connection, Proxy};

const MPRIS_PREFIX: &str = "org.mpris.MediaPlayer2.";
const MPRIS_PATH: &str = "/org/mpris/MediaPlayer2";
const MPRIS_PLAYER_IFACE: &str = "org.mpris.MediaPlayer2.Player";

#[derive(Error, Debug)]
pub enum SpotifyControlError {
    #[error("D-Bus error: {0}")]
    DbusError(String),
    #[error("No compatible media player found")]
    NoPlayerFound,
}

impl From<zbus::Error> for SpotifyControlError {
    fn from(e: zbus::Error) -> Self {
        SpotifyControlError::DbusError(e.to_string())
    }
}

impl From<zbus::fdo::Error> for SpotifyControlError {
    fn from(e: zbus::fdo::Error) -> Self {
        SpotifyControlError::DbusError(e.to_string())
    }
}

#[derive(Clone)]
pub struct SpotifyController;

impl SpotifyController {
    pub fn new() -> Self {
        let ctrl = Self;
        // Probe the session bus once at startup so a missing spotifyd is
        // visible in logs without having to wait for the first wakeword.
        match ctrl.find_mpris_player() {
            Ok((_, name)) => {
                log::info!("[spotify] MPRIS player available at startup: {}", name);
            }
            Err(SpotifyControlError::NoPlayerFound) => {
                log::warn!(
                    "[spotify] No MPRIS player on session bus at startup — \
                     is spotifyd running? Pause-on-wakeword will be a no-op \
                     until one appears."
                );
            }
            Err(e) => {
                log::warn!(
                    "[spotify] Could not probe session bus at startup: {} — \
                     pause-on-wakeword may fail.",
                    e
                );
            }
        }
        ctrl
    }

    /// Pause music if currently playing. Returns true if music was actually paused,
    /// false if nothing was playing or no media player was found.
    pub fn pause_for_wakeword(&self) -> bool {
        match self.try_pause() {
            Ok(was_paused) => {
                if was_paused {
                    log::info!("Paused music for wakeword via D-Bus MPRIS");
                }
                was_paused
            }
            Err(SpotifyControlError::NoPlayerFound) => {
                log::warn!(
                    "[spotify] No MPRIS player on session bus — \
                     is spotifyd running? Skipping pause."
                );
                false
            }
            Err(e) => {
                log::warn!("[spotify] Could not pause music: {}", e);
                false
            }
        }
    }

    fn try_pause(&self) -> Result<bool, SpotifyControlError> {
        let (conn, service) = self.find_mpris_player()?;

        let proxy = Proxy::new(&conn, service.as_str(), MPRIS_PATH, MPRIS_PLAYER_IFACE)?;

        let status: String = proxy
            .get_property("PlaybackStatus")
            .map_err(SpotifyControlError::from)?;

        if status != "Playing" {
            log::debug!(
                "[spotify] MPRIS player {} not playing (status={}), skipping pause",
                service,
                status
            );
            return Ok(false);
        }

        proxy
            .call_method("Pause", &())
            .map_err(SpotifyControlError::from)?;

        Ok(true)
    }

    /// Find an MPRIS player on the session bus.
    /// Connects fresh each time so restarts are handled automatically.
    fn find_mpris_player(&self) -> Result<(Connection, String), SpotifyControlError> {
        let conn = Connection::session()?;
        let dbus = DBusProxy::new(&conn)?;
        let names = dbus.list_names()?;

        let mut mpris_names: Vec<String> = names
            .into_iter()
            .filter_map(|n| {
                let s = n.as_str().to_string();
                if s.starts_with(MPRIS_PREFIX) {
                    Some(s)
                } else {
                    None
                }
            })
            .collect();

        if mpris_names.is_empty() {
            return Err(SpotifyControlError::NoPlayerFound);
        }

        // Priority order: spotifyd, spotify, then first available
        let priorities = ["spotifyd", "spotify"];
        for prio in &priorities {
            if let Some(pos) = mpris_names
                .iter()
                .position(|n| n[MPRIS_PREFIX.len()..].starts_with(prio))
            {
                let chosen = mpris_names.swap_remove(pos);
                log::debug!("Found MPRIS player: {}", chosen);
                return Ok((conn, chosen));
            }
        }

        let chosen = mpris_names.swap_remove(0);
        log::debug!("Using first MPRIS player: {}", chosen);
        Ok((conn, chosen))
    }
}
