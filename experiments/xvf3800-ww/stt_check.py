#!/usr/bin/env python3
"""
Quick STT check on a captured XVF stereo WAV: extract one channel, downsample
48k->16k mono, POST to the Whisper STT service, print the transcript and WER
against an expected command. Used to validate intelligibility on a real
double-talk capture (speech over music) without re-recording.

  ./stt_check.py --wav speech_on.wav --channel 0 \
      --expected "play some jazz music please" --stt-url http://10.10.100.102:8008

channel 0 = left (6_3 in our diagnostic mux), 1 = right (3_0).
"""
from __future__ import annotations
import argparse, struct, wave
import fake_agent_edge as fae      # transcribe()
import rescore as rs               # wer() with number folding


def read_channel_16k(path: str, ch: int) -> bytes:
    with wave.open(path, "rb") as w:
        assert w.getnchannels() >= ch + 1
        sw, nch, sr = w.getsampwidth(), w.getnchannels(), w.getframerate()
        raw = w.readframes(w.getnframes())
    if sw == 2:
        flat = struct.unpack(f"<{len(raw)//2}h", raw); fs = 1.0
    elif sw == 4:
        flat = struct.unpack(f"<{len(raw)//4}i", raw); fs = 1 / 65536.0  # ->16-bit
    else:
        raise SystemExit(f"unsupported sample width {sw*8}-bit")
    mono = [flat[i] * fs for i in range(ch, len(flat), nch)]
    # 48k -> 16k: 3-sample box-filter decimation (cheap anti-alias, fine for STT)
    assert sr == 48000, f"expected 48k, got {sr}"
    out = []
    for i in range(0, len(mono) - 2, 3):
        v = (mono[i] + mono[i + 1] + mono[i + 2]) / 3.0
        out.append(max(-32768, min(32767, int(v))))
    return struct.pack(f"<{len(out)}h", *out)


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--wav", required=True)
    ap.add_argument("--channel", type=int, default=0, help="0=left(6_3) 1=right(3_0)")
    ap.add_argument("--expected", default="")
    ap.add_argument("--stt-url", default="http://10.10.100.102:8008")
    ap.add_argument("--prompt", default="Hey Mycroft")
    ap.add_argument("--language", default="detect", help="STT language ('detect' keeps auto)")
    args = ap.parse_args()

    pcm = read_channel_16k(args.wav, args.channel)
    dur = len(pcm) / 32000.0
    # transcribe with language honored (detect by default, as in production)
    resp, lat = rs._transcribe(args.stt_url, pcm, args.prompt, args.language, 30.0)
    text = (resp.get("text") or "").strip()
    stats = resp.get("stats") or {}
    print(f"  wav={args.wav} ch={args.channel}  {dur:.1f}s  {lat:.0f}ms")
    print(f"  language={resp.get('language')}  "
          f"max_no_speech_prob={stats.get('max_no_speech_prob')}")
    print(f"  → “{text}”")
    if args.expected:
        w, fwd = rs.wer(args.expected, text)
        flag = "✅" if w == 0 else ("⚠️ " if w <= 0.5 else "❌")
        print(f"  {flag} WER={w:.2f}  first_word_lost={fwd}   (expected: “{args.expected}”)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
