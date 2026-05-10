use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Duration;

const IO_TIMEOUT: Duration = Duration::from_millis(500);
const MAX_EVENT_SKIP: usize = 32;
/// Default mpv IPC socket path, matching `deploy/systemd/mpv.service`'s
/// `--input-ipc-server=` flag. Used when `MPV_SOCKET_PATH` is unset so the
/// audio service works out of the box on the standard deployment.
const DEFAULT_SOCKET_PATH: &str = "/tmp/mpv-news.sock";

#[derive(Clone)]
pub struct MpvController {
    socket_path: String,
}

impl MpvController {
    pub fn new() -> Self {
        let (socket_path, from_env) = match std::env::var("MPV_SOCKET_PATH") {
            Ok(p) if !p.is_empty() => (p, true),
            _ => (DEFAULT_SOCKET_PATH.to_string(), false),
        };

        if from_env {
            log::info!("[mpv] MpvController initialized with socket: {}", socket_path);
        } else {
            log::info!(
                "[mpv] MPV_SOCKET_PATH unset, defaulting to {}",
                socket_path
            );
        }

        if !Path::new(&socket_path).exists() {
            log::warn!(
                "[mpv] socket {} does not exist at startup — is mpv running? \
                 Pause-on-wakeword will be a no-op until the socket appears.",
                socket_path
            );
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
        if !Path::new(&self.socket_path).exists() {
            log::warn!(
                "[mpv] socket {} missing — is mpv running? Skipping pause.",
                self.socket_path
            );
            return Ok(false);
        }

        log::info!("[mpv] Connecting to socket: {}", self.socket_path);
        let stream = match UnixStream::connect(&self.socket_path) {
            Ok(s) => {
                log::info!("[mpv] Connected to socket successfully");
                s
            }
            Err(e) => {
                log::warn!("[mpv] Failed to connect to socket {}: {}", self.socket_path, e);
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

        // Default to `false` (i.e. "not paused, try to pause it") if the
        // response is malformed or `data` is missing — matches the user
        // intent of "always try to pause on wakeword unless we know it's
        // already paused". The previous default of `true` silently
        // suppressed pauses on any unparseable response.
        let is_paused = get_resp
            .get("data")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        if is_paused {
            log::info!("[mpv] Already paused (data={}), nothing to do", is_paused);
            return Ok(false);
        }

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
