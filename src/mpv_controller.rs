use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::time::Duration;

const IO_TIMEOUT: Duration = Duration::from_millis(500);

#[derive(Clone)]
pub struct MpvController {
    socket_path: Option<String>,
}

impl MpvController {
    pub fn new() -> Self {
        let socket_path = std::env::var("MPV_SOCKET_PATH").ok();
        if let Some(ref path) = socket_path {
            log::info!("MpvController initialized with socket: {}", path);
        } else {
            log::debug!("MPV_SOCKET_PATH not set, mpv pause disabled");
        }
        Self { socket_path }
    }

    /// Pause mpv if currently playing. Returns true if mpv was actually paused,
    /// false if nothing was playing, mpv isn't running, or socket doesn't exist.
    pub fn pause_for_wakeword(&self) -> bool {
        match self.try_pause() {
            Ok(was_paused) => {
                if was_paused {
                    log::info!("Paused mpv for wakeword via IPC socket");
                }
                was_paused
            }
            Err(e) => {
                log::debug!("Could not pause mpv: {}", e);
                false
            }
        }
    }

    fn try_pause(&self) -> Result<bool, Box<dyn std::error::Error>> {
        let path = match self.socket_path {
            Some(ref p) => p,
            None => return Ok(false),
        };

        let stream = UnixStream::connect_addr(
            &std::os::unix::net::SocketAddr::from_pathname(path)?,
        ).or_else(|_| UnixStream::connect(path))
        .map_err(|e| -> Box<dyn std::error::Error> {
            Box::new(e)
        })?;

        stream.set_read_timeout(Some(IO_TIMEOUT))?;
        stream.set_write_timeout(Some(IO_TIMEOUT))?;

        let mut writer = stream.try_clone()?;
        let mut reader = BufReader::new(stream);

        // Query current pause state
        writer.write_all(b"{\"command\": [\"get_property\", \"pause\"]}\n")?;
        writer.flush()?;

        let mut response = String::new();
        reader.read_line(&mut response)?;

        let resp: serde_json::Value = serde_json::from_str(&response)?;

        let is_paused = resp.get("data").and_then(|v| v.as_bool()).unwrap_or(true);

        if is_paused {
            return Ok(false);
        }

        // mpv is playing, pause it
        writer.write_all(b"{\"command\": [\"set_property\", \"pause\", true]}\n")?;
        writer.flush()?;

        // Read acknowledgment
        let mut ack = String::new();
        reader.read_line(&mut ack)?;

        Ok(true)
    }
}
