//! Fire-and-forget LED event notifier.
//!
//! The audio service drives the LED ring directly via the `led_controller`
//! HTTP API so the ring can light up the instant a wake word is detected,
//! instead of waiting for the agent's TCP + STT round-trip. To avoid pulling
//! in an HTTP client dependency this uses a tiny hand-rolled HTTP/1.1 `POST`
//! over a raw `TcpStream`, and every call runs on a detached thread so it can
//! never block the wake-word detection hot path.

use std::io::Write;
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

const CONNECT_TIMEOUT: Duration = Duration::from_millis(300);
const WRITE_TIMEOUT: Duration = Duration::from_millis(300);

/// Fire a `ww_detected` LED event at the controller without blocking the
/// caller. `endpoint` is a `host:port` like `"127.0.0.1:3000"`.
pub fn notify_ww_detected(endpoint: &str) {
    notify_event(endpoint, "ww_detected");
}

/// Spawn a detached thread that POSTs `{"event":"<event>"}` to the LED
/// controller's `/api/event` endpoint. Failures are logged at debug level and
/// otherwise ignored — LED feedback is best-effort.
pub fn notify_event(endpoint: &str, event: &'static str) {
    let endpoint = endpoint.to_string();
    std::thread::spawn(move || {
        if let Err(e) = post_event(&endpoint, event) {
            log::debug!("[led] '{}' notify to {} failed: {}", event, endpoint, e);
        }
    });
}

fn post_event(endpoint: &str, event: &str) -> std::io::Result<()> {
    let addr = endpoint.to_socket_addrs()?.next().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::AddrNotAvailable,
            format!("could not resolve LED endpoint '{}'", endpoint),
        )
    })?;

    let body = format!("{{\"event\":\"{}\"}}", event);
    let request = format!(
        "POST /api/event HTTP/1.1\r\n\
         Host: {host}\r\n\
         Content-Type: application/json\r\n\
         Content-Length: {len}\r\n\
         Connection: close\r\n\
         \r\n\
         {body}",
        host = endpoint,
        len = body.len(),
        body = body,
    );

    let mut stream = TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT)?;
    stream.set_write_timeout(Some(WRITE_TIMEOUT))?;
    stream.write_all(request.as_bytes())?;
    stream.flush()?;
    // Fire-and-forget: we don't read or care about the response body.
    Ok(())
}
