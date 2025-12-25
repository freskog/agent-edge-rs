use std::process::Command;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum SpotifyControlError {
    #[error("playerctl command failed: {0}")]
    PlayerctlError(String),
    #[error("No compatible media player found")]
    NoPlayerFound,
    #[error("Command execution failed: {0}")]
    ExecutionError(String),
}

#[derive(Clone)]
pub struct SpotifyController {
    preferred_player: Option<String>, // e.g., "spotify", "spotifyd"
}

impl SpotifyController {
    pub fn new() -> Self {
        Self {
            preferred_player: None,
        }
    }

    pub fn new_with_player(player_name: String) -> Self {
        Self {
            preferred_player: Some(player_name),
        }
    }

    fn get_player_args(&self) -> Vec<String> {
        if let Some(ref player) = self.preferred_player {
            vec!["--player".to_string(), player.clone()]
        } else {
            vec![]
        }
    }

    /// Pause music if currently playing. Returns true if music was paused, false if nothing was playing.
    pub fn pause_for_wakeword(&self) -> Result<bool, SpotifyControlError> {
        // First check if any player is actually playing
        if !self.is_playing()? {
            log::debug!("No music playing, skipping pause");
            return Ok(false);
        }

        let mut args = self.get_player_args();
        args.push("pause".to_string());

        let output = Command::new("playerctl")
            .args(&args)
            .output()
            .map_err(|e| SpotifyControlError::ExecutionError(e.to_string()))?;

        if !output.status.success() {
            let error_msg = String::from_utf8_lossy(&output.stderr);
            return Err(SpotifyControlError::PlayerctlError(error_msg.to_string()));
        }

        log::info!("🔇 Paused music for wakeword using playerctl");
        Ok(true)
    }

    fn is_playing(&self) -> Result<bool, SpotifyControlError> {
        let mut args = self.get_player_args();
        args.push("status".to_string());

        let output = Command::new("playerctl")
            .args(&args)
            .output()
            .map_err(|e| SpotifyControlError::ExecutionError(e.to_string()))?;

        if !output.status.success() {
            // No player found or other error
            return Ok(false);
        }

        let status = String::from_utf8_lossy(&output.stdout)
            .trim()
            .to_lowercase();
        Ok(status == "playing")
    }
}
