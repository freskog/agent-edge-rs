#!/usr/bin/env python3
"""Per-channel peak / RMS / clip stats for a WAV (stdlib only).

Used to check XVF3800 mic clipping during loud playback: capture the output mux
(e.g. left = processed 8 0, right = amplified mic 3 0) and read the levels.

Usage:
  chan_stats.py <file.wav> [--label NAME] [--names L,R]
Outputs one JSON line per file with per-channel peak_dbfs, rms_dbfs, clipped,
peak (0..1), and the inter-channel peak delta in dB (ch1 - ch0), which is handy
for the XMOS "mic >= 6 dB below reference" check.
"""
import argparse
import json
import math
import struct
import sys
import wave


def stats(samples):
    n = len(samples)
    if n == 0:
        return {"peak": 0.0, "peak_dbfs": None, "rms_dbfs": None, "clipped": 0}
    peak = max(abs(s) for s in samples)
    clipped = sum(1 for s in samples if s >= 32767 or s <= -32768)
    rms = math.sqrt(sum(s * s for s in samples) / n)
    peak_f = peak / 32768.0
    rms_f = rms / 32768.0
    return {
        "peak": round(peak_f, 5),
        "peak_dbfs": round(20 * math.log10(peak_f), 2) if peak_f > 0 else None,
        "rms_dbfs": round(20 * math.log10(rms_f), 2) if rms_f > 0 else None,
        "clipped": clipped,
    }


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("wav")
    ap.add_argument("--label", default="")
    ap.add_argument("--names", default="", help="comma-separated channel names")
    args = ap.parse_args()

    with wave.open(args.wav, "rb") as w:
        ch = w.getnchannels()
        sw = w.getsampwidth()
        nframes = w.getnframes()
        raw = w.readframes(nframes)
    if sw != 2:
        print(f"error: expected 16-bit, got {sw*8}-bit", file=sys.stderr)
        return 1

    total = len(raw) // 2
    alls = struct.unpack(f"<{total}h", raw[: total * 2])
    chans = [alls[c::ch] for c in range(ch)]
    names = args.names.split(",") if args.names else [str(i) for i in range(ch)]
    names = (names + [str(i) for i in range(ch)])[:ch]

    out = {"file": args.wav, "label": args.label, "channels": {}}
    for i, c in enumerate(chans):
        out["channels"][names[i]] = stats(c)
    if ch >= 2:
        p0 = out["channels"][names[0]]["peak_dbfs"]
        p1 = out["channels"][names[1]]["peak_dbfs"]
        if p0 is not None and p1 is not None:
            out["peak_delta_db_ch1_minus_ch0"] = round(p1 - p0, 2)
    print(json.dumps(out))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
