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
        Self
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
            Err(e) => {
                log::debug!("Could not pause music: {}", e);
                false
            }
        }
    }

    fn try_pause(&self) -> Result<bool, SpotifyControlError> {
        let (conn, service) = match self.find_mpris_player() {
            Ok(r) => r,
            Err(SpotifyControlError::NoPlayerFound) => return Ok(false),
            Err(e) => return Err(e),
        };

        let proxy = Proxy::new(&conn, service.as_str(), MPRIS_PATH, MPRIS_PLAYER_IFACE)?;

        let status: String = proxy
            .get_property("PlaybackStatus")
            .map_err(SpotifyControlError::from)?;

        if status != "Playing" {
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
