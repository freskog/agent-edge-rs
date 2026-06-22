#!/usr/bin/env python3
"""
Guided, time-boxed "agent-edge" stand-in for XVF3800 wake-word / STT testing.

It speaks the audio process's *consumer* protocol (port 8080), replacing the real
agent-edge client. It runs a GUIDED test: it prints a command for you to read
("Hey Mycroft, <command>"), waits a bounded time for the wake-word to fire,
captures the following speech for a bounded window, POSTs it to the Whisper STT
service, and scores the transcript against the expected command. Every trial is
time-bounded, so a session always finishes.

Because the audio process serves a SINGLE consumer (consumer_server.rs), stop the
real agent-edge first so this can take its slot:

    # on the Pi
    systemctl --user stop agent-edge      # frees the consumer slot
    # leave audio.service running (it owns the mic + wake-word detector)

The wake-word detector runs INSIDE the audio process on the channel it captures
(ALSA *left* channel = AUDIO_MGR_OP_L). To test a different XVF output stage
feeding wake-word + STT, change the left mux and relabel — no rebuild:

    ./xvf_host -u i2c AUDIO_MGR_OP_L 8 0   # final processed beam (current)
    ./xvf_host -u i2c AUDIO_MGR_OP_L 7 3   # AEC/ASR auto-selected beam
    ./xvf_host -u i2c AUDIO_MGR_OP_L 3 0   # amplified mic 0 (pre-AEC)
    ./xvf_host -u i2c AUDIO_MGR_OP_L 1 0   # raw mic 0
    # do NOT save_to_flash during experimentation

Run one session per stage:

    ./fake_agent_edge.py --label L=8_0 --consumer mycroft.local:8080 \
        --stt-url http://10.10.100.102:8008

(Use the real consumer port; default 8080.) Play background music with spotifyd
to test barge-in. Stdlib only — no pip install needed on the Pi.
"""
from __future__ import annotations

import argparse
import json
import os
import re
import select
import socket
import statistics as st
import struct
import sys
import time
import urllib.request
import urllib.parse
import wave
from datetime import datetime

# ---- consumer protocol (see src/protocol.rs) -------------------------------
MSG_ERROR = 0x11
MSG_AUDIO = 0x12
MSG_WAKEWORD = 0x15

SAMPLE_RATE = 16000
BYTES_PER_SAMPLE = 2
CHUNK_SAMPLES = 1280            # 80 ms hops produced by the audio process

PREROLL_CHUNKS = 12            # ~960 ms kept before the wake event. Must exceed the
                              # wake-detection+delivery latency, or the command's first
                              # word (spoken right after "Mycroft", before the event
                              # arrives) gets clipped — worst on phrases with no natural
                              # pause ("Hey Mycroft, pause the music"). The wake phrase
                              # itself ends up in the clip, but Whisper's initial_prompt
                              # + strip_wake handle that.

# Default command corpus. Chosen for first-word variety (what's / play / pause /
# set / turn / skip / tell / stop) so first-word-deletion shows up clearly.
DEFAULT_COMMANDS = [
    "what's the weather tomorrow",
    "play some jazz",
    "pause the music",
    "set a timer for ten minutes",
    "what time is it",
    "turn up the volume",
    "skip this song",
    "what's on my calendar today",
    "tell me a joke",
    "stop",
]


# ---------------------------------------------------------------------------
# protocol framing
# ---------------------------------------------------------------------------
def recv_exact(sock: socket.socket, n: int) -> bytes:
    buf = bytearray()
    while len(buf) < n:
        chunk = sock.recv(n - len(buf))
        if not chunk:
            raise ConnectionError("consumer socket closed")
        buf.extend(chunk)
    return bytes(buf)


def read_message(sock: socket.socket):
    """Block until one full message is read. Returns (msg_type, payload)."""
    header = recv_exact(sock, 5)
    msg_type = header[0]
    (payload_len,) = struct.unpack_from("<I", header, 1)
    if payload_len > 10 * 1024 * 1024:
        raise ValueError(f"insane payload size {payload_len}")
    payload = recv_exact(sock, payload_len) if payload_len else b""
    return msg_type, payload


def read_message_until(sock: socket.socket, deadline: float):
    """Read one message, or return None if nothing arrives before `deadline`
    (monotonic seconds). Uses select() so we only start reading once data is
    present, avoiding partial-frame timeouts."""
    remaining = deadline - time.monotonic()
    if remaining <= 0:
        return None
    r, _, _ = select.select([sock], [], [], remaining)
    if not r:
        return None
    return read_message(sock)


def parse_audio(payload: bytes):
    # [timestamp:u64][speech_detected:u8][data_len:u32][data]
    timestamp, speech = struct.unpack_from("<QB", payload, 0)
    (data_len,) = struct.unpack_from("<I", payload, 9)
    return timestamp, bool(speech), payload[13:13 + data_len]


def parse_wakeword(payload: bytes):
    # [timestamp:u64][spotify_was_paused:u8][mpv_was_paused:u8][model_len:u32][model]
    timestamp, spotify_p, mpv_p = struct.unpack_from("<QBB", payload, 0)
    (model_len,) = struct.unpack_from("<I", payload, 10)
    model = payload[14:14 + model_len].decode("utf-8", "replace")
    return timestamp, bool(spotify_p), bool(mpv_p), model


# ---------------------------------------------------------------------------
# audio + scoring helpers
# ---------------------------------------------------------------------------
def save_wav(path: str, pcm: bytes) -> None:
    with wave.open(path, "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(BYTES_PER_SAMPLE)
        w.setframerate(SAMPLE_RATE)
        w.writeframes(pcm)


def pcm_stats(pcm: bytes):
    n = len(pcm) // 2
    if n == 0:
        return 0.0, 0.0, 0
    samples = struct.unpack(f"<{n}h", pcm[: n * 2])
    peak = max(abs(s) for s in samples)
    clipped = sum(1 for s in samples if s >= 32767 or s <= -32768)
    rms = (sum(s * s for s in samples) / n) ** 0.5
    return peak / 32768.0, rms / 32768.0, clipped


_WAKE_RE = re.compile(r"^\s*(hey[ ,]+)?mycroft[\s,.!]*", re.IGNORECASE)


def normalize(text: str) -> str:
    text = text.lower()
    text = re.sub(r"[^a-z0-9' ]+", " ", text)
    return re.sub(r"\s+", " ", text).strip()


def strip_wake(text: str) -> str:
    return _WAKE_RE.sub("", text, count=1).strip()


def wer(ref: str, hyp: str):
    """Word error rate + first-word-deleted flag against normalized strings."""
    r = normalize(ref).split()
    h = normalize(strip_wake(hyp)).split()
    if not r:
        return 0.0, False
    # Levenshtein over words
    dp = list(range(len(h) + 1))
    for i in range(1, len(r) + 1):
        prev, dp[0] = dp[0], i
        for j in range(1, len(h) + 1):
            cur = dp[j]
            dp[j] = min(dp[j] + 1, dp[j - 1] + 1,
                        prev + (r[i - 1] != h[j - 1]))
            prev = cur
    first_word_deleted = (not h) or (h[0] != r[0])
    return dp[len(h)] / len(r), first_word_deleted


def spotify_post(base: str, path: str, timeout: float = 3.0) -> bool:
    """POST to the spotify-control service (port 3001). Returns True on 2xx.
    Best-effort: never raises, so a down service can't break the test."""
    try:
        req = urllib.request.Request(
            f"{base.rstrip('/')}{path}", data=b"", method="POST",
            headers={"Content-Type": "application/json"})
        with urllib.request.urlopen(req, timeout=timeout) as resp:
            return 200 <= resp.status < 300
    except Exception as e:  # noqa: BLE001
        print(f"      ⚠️  spotify {path} failed: {e}")
        return False


def transcribe(stt_url: str, pcm: bytes, prompt: str, timeout: float):
    qs = urllib.parse.urlencode({
        "beam_size": "1", "vad_filter": "true",
        "language": "detect", "initial_prompt": prompt,
    })
    req = urllib.request.Request(
        f"{stt_url.rstrip('/')}/transcribe?{qs}", data=pcm, method="POST",
        headers={"Accept": "application/json",
                 "Content-Type": "application/octet-stream"})
    t0 = time.monotonic()
    with urllib.request.urlopen(req, timeout=timeout) as resp:
        body = resp.read()
    return json.loads(body.decode("utf-8")), (time.monotonic() - t0) * 1000.0


# ---------------------------------------------------------------------------
# trial state machine
# ---------------------------------------------------------------------------
def drain(sock: socket.socket, preroll: list[bytes]) -> None:
    """Discard any backlogged frames so a trial starts clean. Keeps preroll
    fresh and throws away stale wake events from before this prompt."""
    while True:
        r, _, _ = select.select([sock], [], [], 0)
        if not r:
            return
        mtype, payload = read_message(sock)
        if mtype == MSG_AUDIO:
            _, _, data = parse_audio(payload)
            preroll.append(data)
            del preroll[:-PREROLL_CHUNKS]


def run_trial(sock, args, idx, expected, session, preroll):
    """Returns a result dict. Bounded by wake_timeout + capture_s."""
    drain(sock, preroll)
    print(f"\n  ▶ SAY:  “Hey Mycroft, {expected}”")
    print(f"      (waiting up to {args.wake_timeout:.0f}s for wake-word…)",
          end="", flush=True)

    # --- wait for wake ---
    wake_deadline = time.monotonic() + args.wake_timeout
    wake_evt = None
    while True:
        msg = read_message_until(sock, wake_deadline)
        if msg is None:
            print("  ❌ NO WAKE (miss)")
            return {"expected": expected, "wake": False}
        mtype, payload = msg
        if mtype == MSG_AUDIO:
            _, _, data = parse_audio(payload)
            preroll.append(data)
            del preroll[:-PREROLL_CHUNKS]
        elif mtype == MSG_WAKEWORD:
            ts, spotify_p, mpv_p, model = parse_wakeword(payload)
            wake_evt = {"model": model, "wake_ts": ts,
                        "spotify_was_paused": spotify_p, "mpv_was_paused": mpv_p}
            break

    print("  🎯 wake — recording…")
    wake_mono = time.monotonic()

    # --- bounded capture: hard cap, with early stop on trailing silence ---
    utt = bytearray(b"".join(preroll))
    cap_deadline = wake_mono + args.capture_s
    min_deadline = wake_mono + args.min_capture_s
    silence_needed = max(1, int(args.trailing_silence_s / 0.08))
    silence_run, speech_chunks = 0, 0
    while time.monotonic() < cap_deadline:
        msg = read_message_until(sock, cap_deadline)
        if msg is None:
            break
        mtype, payload = msg
        if mtype != MSG_AUDIO:
            continue  # ignore extra wakes during capture
        _, speech, data = parse_audio(payload)
        utt.extend(data)
        if speech:
            speech_chunks += 1
            silence_run = 0
        else:
            silence_run += 1
        if time.monotonic() >= min_deadline and silence_run >= silence_needed:
            break

    return finalize(args, idx, expected, session, wake_evt, bytes(utt), speech_chunks)


def finalize(args, idx, expected, session, wake_evt, pcm, speech_chunks):
    dur_s = len(pcm) / (SAMPLE_RATE * BYTES_PER_SAMPLE)
    peak, rms, clipped = pcm_stats(pcm)
    wav_path = os.path.join(args.outdir, f"utt-{args.label}-{session}-{idx:03d}.wav")
    save_wav(wav_path, pcm)
    res = {"expected": expected, "wake": True, **(wake_evt or {}),
           "utt_dur_s": round(dur_s, 3), "speech_chunks": speech_chunks,
           "peak": round(peak, 4), "rms": round(rms, 4), "clipped": clipped,
           "wav": os.path.basename(wav_path)}

    if args.no_stt:
        print(f"      captured {dur_s:.1f}s peak={peak:.2f} rms={rms:.3f} → {res['wav']}")
        return res

    try:
        resp, lat = transcribe(args.stt_url, pcm, args.prompt, args.stt_timeout)
        text = (resp.get("text") or "").strip()
        stats = resp.get("stats") or {}
        w, fwd = wer(expected, text)
        res.update({
            "text": text, "language": resp.get("language"),
            "stt_latency_ms": round(lat, 1),
            "mean_avg_logprob": stats.get("mean_avg_logprob"),
            "max_no_speech_prob": stats.get("max_no_speech_prob"),
            "wer": round(w, 3), "first_word_deleted": fwd,
        })
        flag = "✅" if w == 0 else ("⚠️ " if w <= 0.5 else "❌")
        fwd_tag = "  [FIRST-WORD LOST]" if fwd else ""
        print(f"      {flag} WER={w:.2f}{fwd_tag}  got: “{text}”  "
              f"({lat:.0f}ms, {dur_s:.1f}s)")
    except Exception as e:  # keep the session alive on STT errors
        res["text"] = f"<STT ERROR: {e}>"
        print(f"      ❌ STT error: {e}")
    return res


# ---------------------------------------------------------------------------
def main() -> int:
    ap = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--label", default="test",
                    help="stage label for this session, e.g. L=8_0 / L=7_3")
    ap.add_argument("--consumer", default=os.environ.get("CONSUMER", "127.0.0.1:8080"),
                    help="audio process consumer host:port")
    ap.add_argument("--stt-url", default=os.environ.get("STT_URL", "http://10.10.100.102:8008"))
    ap.add_argument("--keep-music", action="store_true",
                    help="resume spotifyd after each wake (the audio process pauses it "
                         "on barge-in) so every trial happens over live playback")
    ap.add_argument("--spotify-control",
                    default=os.environ.get("SPOTIFY_CONTROL_URL", "http://mycroft.local:3001"),
                    help="spotify-control base URL for --keep-music")
    ap.add_argument("--music-settle-s", type=float, default=2.0,
                    help="wait after resuming music before the next prompt")
    ap.add_argument("--prompt", default="Hey Mycroft",
                    help="Whisper initial_prompt (matches scala BasePrompt)")
    ap.add_argument("--commands-file",
                    help="file with one command per line (default: built-in corpus)")
    ap.add_argument("--repeats", type=int, default=1,
                    help="how many passes over the command list")
    ap.add_argument("--wake-timeout", type=float, default=12.0,
                    help="seconds to wait for the wake-word per trial (miss if exceeded)")
    ap.add_argument("--capture-s", type=float, default=8.0,
                    help="hard cap on capture length after wake")
    ap.add_argument("--min-capture-s", type=float, default=1.0,
                    help="don't stop on silence before this many seconds")
    ap.add_argument("--trailing-silence-s", type=float, default=0.8,
                    help="trailing silence that ends capture early")
    ap.add_argument("--stt-timeout", type=float, default=30.0)
    ap.add_argument("--no-stt", action="store_true", help="capture + save WAV only")
    ap.add_argument("--outdir", default=".")
    args = ap.parse_args()

    if args.commands_file:
        with open(args.commands_file) as f:
            commands = [ln.strip() for ln in f if ln.strip()]
    else:
        commands = DEFAULT_COMMANDS
    plan = commands * args.repeats

    host, _, port = args.consumer.partition(":")
    addr = (host, int(port or "8080"))
    os.makedirs(args.outdir, exist_ok=True)
    session = datetime.now().strftime("%Y%m%d-%H%M%S")
    jsonl_path = os.path.join(args.outdir, f"results-{args.label}.jsonl")
    jsonl = open(jsonl_path, "a")

    est = len(plan) * (args.wake_timeout * 0.4 + args.capture_s + 3)
    print(f"=== guided wake/STT test [{args.label}] → {addr[0]}:{addr[1]} ===")
    print(f"    STT: {'(disabled)' if args.no_stt else args.stt_url}")
    print(f"    {len(plan)} trials  (~{est/60:.0f} min upper bound)")
    print(f"    Read each line aloud as: “Hey Mycroft, <command>”. Ctrl-C to stop early.")
    if args.keep_music:
        print(f"    BARGE-IN MODE: start spotifyd playback NOW; music is resumed via "
              f"{args.spotify_control} after each wake.")

    sock = socket.create_connection(addr, timeout=10)
    preroll: list[bytes] = []
    results: list[dict] = []

    try:
        for i, expected in enumerate(plan, 1):
            print(f"\n[{i}/{len(plan)}]", end="")
            res = run_trial(sock, args, i, expected, session, preroll)
            res.update({"idx": i, "label": args.label, "session": session})
            results.append(res)
            jsonl.write(json.dumps(res) + "\n")
            jsonl.flush()

            # The audio process pauses spotifyd on wake; resume so the next trial
            # is again over live playback. Only needed when a wake actually paused it.
            if args.keep_music and res.get("spotify_was_paused"):
                if spotify_post(args.spotify_control, "/api/spotify/resume"):
                    print(f"      🎵 resumed music, settling {args.music_settle_s:.0f}s…")
                    time.sleep(args.music_settle_s)
    except (KeyboardInterrupt, ConnectionError) as e:
        print(f"\n  (stopped: {type(e).__name__})")
    finally:
        try:
            sock.close()
        except Exception:
            pass

    # ---- summary ----
    print(f"\n\n=== SUMMARY [{args.label}] ===")
    n = len(results)
    wakes = [r for r in results if r.get("wake")]
    print(f"  trials run:      {n}")
    if n:
        print(f"  wake recall:     {len(wakes)}/{n} = {len(wakes)/n:.0%}")
    transcribed = [r for r in wakes if "wer" in r]
    if transcribed:
        wers = [r["wer"] for r in transcribed]
        fwd = sum(1 for r in transcribed if r["first_word_deleted"])
        lats = [r["stt_latency_ms"] for r in transcribed]
        print(f"  median WER:      {st.median(wers):.2f}  "
              f"(perfect: {sum(1 for w in wers if w==0)}/{len(wers)})")
        print(f"  first-word lost: {fwd}/{len(transcribed)} = {fwd/len(transcribed):.0%}")
        print(f"  STT latency ms:  median={st.median(lats):.0f}")
    paused = sum(1 for r in wakes if r.get("spotify_was_paused"))
    if wakes:
        print(f"  fired during spotify playback: {paused}/{len(wakes)}")
    print(f"\n  rows -> {jsonl_path}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
