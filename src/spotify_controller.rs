use std::process::Command;
use std::thread;
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
    /// Create a new controller that auto-detects any available music player
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

    fn get_player_args(&self) -> Result<Vec<String>, SpotifyControlError> {
        if let Some(ref player) = self.preferred_player {
            // If player name contains a wildcard pattern or is just a prefix like "spotifyd",
            // find the actual instance
            let actual_player = if player.contains('*') || !player.contains('.') {
                self.find_player_instance(player)?
            } else {
                player.clone()
            };
            Ok(vec!["--player".to_string(), actual_player])
        } else {
            // No preferred player - auto-detect any available music player
            match self.find_any_music_player() {
                Ok(player) => Ok(vec!["--player".to_string(), player]),
                Err(_) => Ok(vec![]), // No player found, use default (playerctl will handle it)
            }
        }
    }

    /// Auto-detect any available music player (spotifyd, spotify, etc.)
    fn find_any_music_player(&self) -> Result<String, SpotifyControlError> {
        log::debug!("Auto-detecting available music players");

        let output = Command::new("playerctl")
            .arg("--list-all")
            .output()
            .map_err(|e| {
                log::debug!("Failed to run 'playerctl --list-all': {}", e);
                SpotifyControlError::ExecutionError(e.to_string())
            })?;

        if !output.status.success() {
            return Err(SpotifyControlError::NoPlayerFound);
        }

        let players = String::from_utf8_lossy(&output.stdout);

        // Priority order: spotifyd, spotify, then any other player
        let priorities = ["spotifyd", "spotify"];

        for priority in &priorities {
            for line in players.lines() {
                let player = line.trim();
                if player.starts_with(priority) {
                    log::info!("✅ Auto-detected music player: {}", player);
                    return Ok(player.to_string());
                }
            }
        }

        // If no priority player found, use first available
        if let Some(first_player) = players.lines().next() {
            let player = first_player.trim().to_string();
            if !player.is_empty() {
                log::info!("✅ Auto-detected music player: {}", player);
                return Ok(player);
            }
        }

        log::debug!("No music players found");
        Err(SpotifyControlError::NoPlayerFound)
    }

    /// Find actual player instance matching a pattern (e.g., "spotifyd" finds "spotifyd.instance12345")
    fn find_player_instance(&self, pattern: &str) -> Result<String, SpotifyControlError> {
        log::debug!("Looking for player matching pattern: {}", pattern);

        let output = Command::new("playerctl")
            .arg("--list-all")
            .output()
            .map_err(|e| {
                log::error!("Failed to run 'playerctl --list-all': {}", e);
                SpotifyControlError::ExecutionError(e.to_string())
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            log::error!("playerctl --list-all failed: {}", stderr);
            return Err(SpotifyControlError::NoPlayerFound);
        }

        let players = String::from_utf8_lossy(&output.stdout);
        log::debug!("Available players:\n{}", players);

        // Find first player matching the pattern
        for line in players.lines() {
            let player = line.trim();
            if player.starts_with(pattern) {
                log::info!(
                    "✅ Found player instance: {} (pattern: {})",
                    player,
                    pattern
                );
                return Ok(player.to_string());
            }
        }

        log::warn!(
            "⚠️  No player found matching pattern '{}'. Available: {}",
            pattern,
            players.trim()
        );
        Err(SpotifyControlError::NoPlayerFound)
    }

    /// Pause music if currently playing. Returns true if music was paused, false if nothing was playing.
    /// This is FAST - runs in background thread to avoid blocking wakeword detection.
    pub fn pause_for_wakeword(&self) -> Result<bool, SpotifyControlError> {
        // Clone self for background thread
        let controller = self.clone();

        // Spawn background thread for pause operation (don't block detection)
        thread::spawn(move || match controller.pause_blocking() {
            Ok(was_paused) => {
                if was_paused {
                    log::info!("🔇 Paused music for wakeword using playerctl");
                } else {
                    log::debug!("No music playing, skipping pause");
                }
            }
            Err(e) => {
                log::warn!(
                    "⚠️ Failed to pause Spotify: {} (playerctl may not be working)",
                    e
                );
            }
        });

        // Return immediately - actual pause happens in background
        Ok(true)
    }

    /// Blocking pause operation - called in background thread
    fn pause_blocking(&self) -> Result<bool, SpotifyControlError> {
        // First check if any player is actually playing
        if !self.is_playing()? {
            return Ok(false);
        }

        let mut args = self.get_player_args()?;
        args.push("pause".to_string());

        let output = Command::new("playerctl")
            .args(&args)
            .output()
            .map_err(|e| SpotifyControlError::ExecutionError(e.to_string()))?;

        if !output.status.success() {
            let error_msg = String::from_utf8_lossy(&output.stderr);
            return Err(SpotifyControlError::PlayerctlError(error_msg.to_string()));
        }

        Ok(true)
    }

    fn is_playing(&self) -> Result<bool, SpotifyControlError> {
        let mut args = match self.get_player_args() {
            Ok(args) => args,
            Err(e) => {
                log::debug!("No player found for status check: {}", e);
                return Ok(false);
            }
        };
        args.push("status".to_string());

        let output = Command::new("playerctl")
            .args(&args)
            .output()
            .map_err(|e| {
                log::error!("Failed to run playerctl status: {}", e);
                SpotifyControlError::ExecutionError(e.to_string())
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            log::debug!(
                "playerctl status failed (no player or not playing): {}",
                stderr
            );
            return Ok(false);
        }

        let status = String::from_utf8_lossy(&output.stdout)
            .trim()
            .to_lowercase();

        log::debug!("playerctl status: {}", status);
        Ok(status == "playing")
    }
}
