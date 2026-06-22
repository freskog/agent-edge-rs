# spotify-control

A small REST service that controls a local [`spotifyd`](https://spotifyd.rs)
instance over D-Bus. It exists so that other processes (the `audio` wakeword
service, the agent, scripts, future UIs) can control Spotify playback locally
without using the Spotify Web API — no OAuth, no token refresh, no rate limits,
and no dependency on the device being "visible" to the Web API.

## Why D-Bus instead of the Web API

`spotifyd --use-mpris` exposes two D-Bus interfaces:

- **`rs.spotifyd.Controls`** (bus name `rs.spotifyd.instance$PID`) — available as
  soon as spotifyd connects to Spotify, **even when it is not the active playback
  device**. Its `TransferPlayback` method activates spotifyd as the Connect
  device. This is the fix for the classic *"spotifyd is not in the list of
  available devices"* problem: the Web API can't transfer to a device it can't
  see, but this local call activates it directly.
- **`org.mpris.MediaPlayer2.Player`** (bus name
  `org.mpris.MediaPlayer2.spotifyd.instance$PID`) — present **only once spotifyd
  is the active device**. Provides `Play`, `Pause`, `Next`, `Previous`,
  `OpenUri`, etc.

This service hides that two-stage dance: `play` issues `TransferPlayback`, waits
for the MPRIS interface to appear, then `OpenUri`.

## Running

```bash
spotify-control --bind 0.0.0.0:3001
```

- `--bind <host:port>` — HTTP listen address. Default `0.0.0.0:3001`.
- Logging via `RUST_LOG` (e.g. `RUST_LOG=info`).

It must run on the **same session bus** as `spotifyd` (i.e. as a
`systemctl --user` service for the same user). See
[`deploy/systemd/spotify-control.service`](../deploy/systemd/spotify-control.service).

## HTTP interface

Base path: `/api/spotify`. All request and response bodies are JSON.

| Method & path                  | Body                      | Success response                       |
| ------------------------------ | ------------------------- | -------------------------------------- |
| `POST /api/spotify/transfer`   | _(none)_                  | `{ "ok": true }`                       |
| `POST /api/spotify/play`       | `{ "uri": "spotify:..." }`| `{ "ok": true }`                       |
| `POST /api/spotify/pause`      | _(none)_                  | `{ "ok": true, "paused": <bool> }`     |
| `POST /api/spotify/resume`     | _(none)_                  | `{ "ok": true }`                       |
| `POST /api/spotify/next`       | _(none)_                  | `{ "ok": true }`                       |
| `POST /api/spotify/previous`   | _(none)_                  | `{ "ok": true }`                       |
| `POST /api/spotify/volume`     | `{ "level": 0-100 }`      | `{ "ok": true }`                       |
| `GET  /api/spotify/status`     | _(none)_                  | `Status` object (see below)            |

### Endpoint notes

- **`transfer`** — activate spotifyd as the Connect device
  (`rs.spotifyd.Controls.TransferPlayback`). Callable even when inactive. Because
  activation is asynchronous, this **waits until spotifyd has actually become the
  active device** (the MPRIS interface appears) before returning, polling up to
  5s. A `200` therefore means the device is ready for follow-up MPRIS calls;
  a `504` means activation didn't complete in time. If already active it returns
  immediately.
- **`play`** — ensures spotifyd is active (transferring first if needed, polling
  up to 5s for the MPRIS interface), then starts the given `spotify:` URI
  (track/album/artist/playlist/episode/show). The URI must already be resolved;
  this service does not do search (that still needs the Web API).
- **`pause`** — only issues a pause if `PlaybackStatus == "Playing"`. The
  `paused` field reports whether a pause was actually sent. This is what the
  `audio` wakeword service calls; the "was playing" gate preserves its
  confirmation-beep behaviour.
- **`resume`** — MPRIS `Play`.
- **`volume`** — sets the MPRIS `Volume` property (0-100 mapped to 0.0-1.0).
  Requires spotifyd to be active.

### `Status` object (`GET /api/spotify/status`)

```json
{
  "available": true,        // spotifyd is on the bus (Controls present)
  "active": true,           // spotifyd is the active device (MPRIS present)
  "playback_status": "Playing", // or "Paused"/"Stopped"; null if inactive
  "track": "Artist - Title",    // best-effort from MPRIS metadata; null if inactive
  "volume": 50              // percent 0-100; null if inactive
}
```

If spotifyd is connected but not the active device, `available` is `true` and
`active` is `false` (call `transfer` or `play` to activate).

### Error responses

Errors use `{ "ok": false, "error": "<message>" }` with these status codes:

| Status | Condition                                                              |
| ------ | --------------------------------------------------------------------- |
| `503`  | spotifyd is not running / not on the session bus                      |
| `409`  | spotifyd is connected but not the active device (MPRIS unavailable)   |
| `504`  | timed out waiting for spotifyd to become active after a transfer      |
| `502`  | underlying D-Bus error                                                |

## Examples

```bash
# Start playing a playlist (activates spotifyd if needed)
curl -X POST localhost:3001/api/spotify/play \
  -H 'content-type: application/json' \
  -d '{"uri":"spotify:playlist:37i9dQZF1DXcBWIGoYBM5M"}'

# Pause (returns whether anything was actually paused)
curl -X POST localhost:3001/api/spotify/pause

# Set volume to 30%
curl -X POST localhost:3001/api/spotify/volume \
  -H 'content-type: application/json' -d '{"level":30}'

# Current state
curl localhost:3001/api/spotify/status
```

## Integration with the audio service

The `audio` wakeword service pauses Spotify on wake word by POSTing to
`/api/spotify/pause` (configured via its `--spotify-endpoint`, default
`127.0.0.1:3001`). That call uses short bounded timeouts and treats any
failure as "nothing was paused", so if this service or spotifyd is down,
wake-word detection is unaffected.
