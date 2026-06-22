//! Wake-word pause integration with the `spotify-control` service.
//!
//! Spotify playback is owned by `spotifyd` and controlled out-of-process by the
//! `spotify-control` REST service (which talks D-Bus/MPRIS to spotifyd). The
//! audio service only needs to *pause on wake word*, so this is a tiny HTTP
//! client that POSTs to that service.
//!
//! Hard requirement: wake-word detection must never fail or stall if spotifyd
//! or the `spotify-control` service is down. Every call uses short, bounded
//! connect/write/read timeouts and any error degrades to "nothing was paused"
//! (the same graceful no-op as when no player is found). The HTTP request is
//! hand-rolled over a raw `TcpStream` (like `led_notify`) so the audio crate
//! needs no HTTP-client or D-Bus dependency.

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

const CONNECT_TIMEOUT: Duration = Duration::from_millis(300);
const WRITE_TIMEOUT: Duration = Duration::from_millis(300);
const READ_TIMEOUT: Duration = Duration::from_millis(300);

#[derive(Clone)]
pub struct SpotifyController {
    /// `host:port` of the spotify-control service.
    endpoint: String,
}

impl SpotifyController {
    pub fn new(endpoint: String) -> Self {
        Self { endpoint }
    }

    /// Pause music if currently playing. Returns true if music was actually
    /// paused, false if nothing was playing or the service was unreachable.
    ///
    /// Never blocks longer than the bounded timeouts above, so a down/hung
    /// spotify-control service can't stall wake-word detection.
    pub fn pause_for_wakeword(&self) -> bool {
        match self.post_pause() {
            Ok(paused) => {
                if paused {
                    log::info!("Paused music for wakeword via spotify-control");
                }
                paused
            }
            Err(e) => {
                log::debug!(
                    "[spotify] pause request to {} failed ({}); skipping pause",
                    self.endpoint,
                    e
                );
                false
            }
        }
    }

    /// POST /api/spotify/pause and return the `paused` flag from the response.
    fn post_pause(&self) -> std::io::Result<bool> {
        let addr = self
            .endpoint
            .to_socket_addrs()?
            .next()
            .ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::AddrNotAvailable,
                    format!("could not resolve spotify endpoint '{}'", self.endpoint),
                )
            })?;

        let request = format!(
            "POST /api/spotify/pause HTTP/1.1\r\n\
             Host: {host}\r\n\
             Content-Type: application/json\r\n\
             Content-Length: 0\r\n\
             Connection: close\r\n\
             \r\n",
            host = self.endpoint,
        );

        let mut stream = TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT)?;
        stream.set_write_timeout(Some(WRITE_TIMEOUT))?;
        stream.set_read_timeout(Some(READ_TIMEOUT))?;
        stream.write_all(request.as_bytes())?;
        stream.flush()?;

        // Read the (small) response. We only need to know whether the service
        // reported `"paused": true`. Cap the read so a misbehaving peer can't
        // make us read forever within the timeout window.
        let mut buf = Vec::with_capacity(512);
        let mut chunk = [0u8; 512];
        loop {
            match stream.read(&mut chunk) {
                Ok(0) => break,
                Ok(n) => {
                    buf.extend_from_slice(&chunk[..n]);
                    if buf.len() >= 8192 {
                        break;
                    }
                }
                Err(e)
                    if e.kind() == std::io::ErrorKind::WouldBlock
                        || e.kind() == std::io::ErrorKind::TimedOut =>
                {
                    // Read timeout: stop with whatever we have.
                    break;
                }
                Err(e) => return Err(e),
            }
        }

        let body = String::from_utf8_lossy(&buf);
        Ok(parse_paused(&body))
    }
}

/// Minimal check for `"paused": true` in the JSON response body, tolerant of
/// optional whitespace. Avoids pulling a JSON parser into the audio crate.
fn parse_paused(response: &str) -> bool {
    let bytes = response.as_bytes();
    let needle = b"\"paused\"";
    let mut i = 0;
    while let Some(pos) = find_subslice(&bytes[i..], needle) {
        let mut j = i + pos + needle.len();
        // skip whitespace and the ':'
        while j < bytes.len() && (bytes[j] == b' ' || bytes[j] == b'\t') {
            j += 1;
        }
        if j < bytes.len() && bytes[j] == b':' {
            j += 1;
            while j < bytes.len() && (bytes[j] == b' ' || bytes[j] == b'\t') {
                j += 1;
            }
            return bytes[j..].starts_with(b"true");
        }
        i += pos + needle.len();
    }
    false
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_paused_true() {
        assert!(parse_paused("{\"ok\":true,\"paused\":true}"));
        assert!(parse_paused("{ \"paused\" : true }"));
    }

    #[test]
    fn parses_paused_false() {
        assert!(!parse_paused("{\"ok\":true,\"paused\":false}"));
        assert!(!parse_paused("{\"ok\":false}"));
        assert!(!parse_paused(""));
    }
}
