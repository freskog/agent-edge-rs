use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::time::Duration;

const IO_TIMEOUT: Duration = Duration::from_millis(500);
const MAX_EVENT_SKIP: usize = 32;

#[derive(Clone)]
pub struct MpvController {
    socket_path: Option<String>,
}

impl MpvController {
    pub fn new() -> Self {
        let socket_path = std::env::var("MPV_SOCKET_PATH").ok();
        if let Some(ref path) = socket_path {
            log::info!("[mpv] MpvController initialized with socket: {}", path);
        } else {
            log::warn!("[mpv] MPV_SOCKET_PATH not set, mpv pause disabled");
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
                log::warn!("[mpv] Could not pause mpv: {}", e);
                false
            }
        }
    }

    fn try_pause(&self) -> Result<bool, Box<dyn std::error::Error>> {
        let path = match self.socket_path {
            Some(ref p) => p,
            None => {
                log::warn!("[mpv] No socket path configured (set MPV_SOCKET_PATH), skipping");
                return Ok(false);
            }
        };

        log::info!("[mpv] Connecting to socket: {}", path);
        let stream = match UnixStream::connect(path) {
            Ok(s) => {
                log::info!("[mpv] Connected to socket successfully");
                s
            }
            Err(e) => {
                log::warn!("[mpv] Failed to connect to socket {}: {}", path, e);
                return Err(Box::new(e));
            }
        };
        stream.set_read_timeout(Some(IO_TIMEOUT))?;
        stream.set_write_timeout(Some(IO_TIMEOUT))?;

        let mut writer = stream.try_clone()?;
        let mut reader = BufReader::new(stream);

        // Query current pause state. Use request_id so we can skip
        // over any unsolicited mpv events on the socket.
        log::info!("[mpv] Querying pause state (get_property pause)");
        writer.write_all(b"{\"command\":[\"get_property\",\"pause\"],\"request_id\":1}\n")?;
        writer.flush()?;

        let get_resp = self.read_response(&mut reader, 1)?;
        log::info!("[mpv] get_property pause response: {}", get_resp);

        let is_paused = get_resp
            .get("data")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        if is_paused {
            log::info!("[mpv] Already paused (data={}), nothing to do", is_paused);
            return Ok(false);
        }

        // mpv is playing -- pause it
        log::info!("[mpv] Currently playing, sending pause command");
        writer.write_all(b"{\"command\":[\"set_property\",\"pause\",true],\"request_id\":2}\n")?;
        writer.flush()?;

        // Wait for ack so the pause is confirmed before we return
        match self.read_response(&mut reader, 2) {
            Ok(ack) => log::info!("[mpv] Pause ack received: {}", ack),
            Err(e) => log::warn!("[mpv] Pause ack failed (pause may still have taken effect): {}", e),
        }

        Ok(true)
    }

    /// Read lines from the mpv socket until we find the response matching
    /// `expected_id`, skipping over unsolicited event lines.
    fn read_response(
        &self,
        reader: &mut BufReader<UnixStream>,
        expected_id: u64,
    ) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
        for _ in 0..MAX_EVENT_SKIP {
            let mut line = String::new();
            reader.read_line(&mut line)?;
            if line.is_empty() {
                return Err("mpv socket closed".into());
            }
            let val: serde_json::Value = serde_json::from_str(line.trim())?;
            if val.get("request_id").and_then(|v| v.as_u64()) == Some(expected_id) {
                return Ok(val);
            }
            log::debug!("[mpv] Skipping event while waiting for request_id {}: {}", expected_id, line.trim());
        }
        Err(format!("exceeded {} lines without finding request_id {}", MAX_EVENT_SKIP, expected_id).into())
    }
}
