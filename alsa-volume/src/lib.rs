use std::process::Command;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum VolumeError {
    #[error("Failed to run amixer: {0}")]
    CommandFailed(String),
    #[error("Could not parse volume from amixer output")]
    ParseError,
}

/// Read the current playback volume percentage for the given ALSA mixer element.
/// Parses the `[XX%]` token from `amixer get <mixer>` output.
pub fn get_volume(mixer: &str) -> Result<u8, VolumeError> {
    let output = Command::new("amixer")
        .args(["get", mixer])
        .output()
        .map_err(|e| VolumeError::CommandFailed(e.to_string()))?;

    if !output.status.success() {
        return Err(VolumeError::CommandFailed(
            String::from_utf8_lossy(&output.stderr).to_string(),
        ));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_volume_percent(&stdout).ok_or(VolumeError::ParseError)
}

/// Set the playback volume percentage for the given ALSA mixer element.
pub fn set_volume(mixer: &str, percent: u8) {
    let arg = format!("{}%", percent.min(100));
    match Command::new("amixer")
        .args(["sset", mixer, &arg])
        .output()
    {
        Ok(output) if !output.status.success() => {
            log::warn!(
                "amixer exited with {}: {}",
                output.status,
                String::from_utf8_lossy(&output.stderr)
            );
        }
        Err(e) => log::warn!("Failed to run amixer: {}", e),
        _ => {}
    }
}

/// Extract the first `[NN%]` value from amixer output.
fn parse_volume_percent(output: &str) -> Option<u8> {
    for line in output.lines() {
        if let Some(start) = line.find('[') {
            let rest = &line[start + 1..];
            if let Some(pct_pos) = rest.find("%]") {
                if let Ok(val) = rest[..pct_pos].trim().parse::<u8>() {
                    return Some(val);
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_volume_percent() {
        let output = r#"Simple mixer control 'XVF3800 SoftMaster',0
  Capabilities: volume
  Playback channels: Mono
  Capture channels: Mono
  Limits: 0 - 100
  Mono: 75 [75%]"#;
        assert_eq!(parse_volume_percent(output), Some(75));
    }

    #[test]
    fn test_parse_volume_percent_stereo() {
        let output = r#"Simple mixer control 'Master',0
  Capabilities: pvolume pswitch pswitch-joined
  Playback channels: Front Left - Front Right
  Limits: Playback 0 - 65536
  Front Left: Playback 32768 [50%] [on]
  Front Right: Playback 32768 [50%] [on]"#;
        assert_eq!(parse_volume_percent(output), Some(50));
    }

    #[test]
    fn test_parse_volume_percent_zero() {
        let output = "  Mono: 0 [0%]";
        assert_eq!(parse_volume_percent(output), Some(0));
    }

    #[test]
    fn test_parse_volume_percent_no_match() {
        assert_eq!(parse_volume_percent("no volume info here"), None);
    }
}
