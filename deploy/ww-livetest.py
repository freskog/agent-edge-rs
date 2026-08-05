#!/usr/bin/env python3
"""
Live "Hey Mycroft" wake-word detection-rate test.

Streams the Pi's audio.service journal, captures wake-word FIRES and sub-threshold
NEAR-MISSES (peak confidence per VAD speech segment), and prints a running tally
plus a summary on Ctrl-C. Requires the instrumented `audio` binary built with the
WW_NEARMISS_LOG path, and `WW_NEARMISS_LOG=1` in the Pi's ~/.env.

Run this on your Mac, then say "Hey Mycroft" N times in the condition you want to
measure (silence / over music / from across the room). Pause ~1s between attempts.

Usage:
  ./ww-livetest.py --label silence
  ./ww-livetest.py --label over-music --floor 0.05
  PI_HOST=freskog@mycroft.local ./ww-livetest.py --label distance-3m

Ctrl-C ends the session and prints the distribution. The raw matched lines are
also appended to ww-livetest-<label>.log for later analysis.
"""
from __future__ import annotations

import argparse
import os
import re
import signal
import statistics as st
import subprocess
import sys
from datetime import datetime

FIRE_RE = re.compile(r"WAKEWORD DETECTED:.*confidence ([0-9.]+)")
NEARMISS_RE = re.compile(r"WW-NEARMISS.*peak_confidence=([0-9.]+).*threshold=([0-9.]+)")


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--label", default="test", help="Condition label (silence / over-music / distance-3m)")
    ap.add_argument("--pi-host", default=os.environ.get("PI_HOST", "freskog@mycroft.local"))
    ap.add_argument("--floor", type=float, default=0.02,
                    help="Ignore near-misses below this peak (filters ambient-VAD 0.000 noise)")
    args = ap.parse_args()

    logpath = f"ww-livetest-{args.label}.log"
    logf = open(logpath, "a")
    fires: list[float] = []
    misses: list[float] = []
    threshold = None

    print(f"=== Live wake test [{args.label}] — say 'Hey Mycroft', pause ~1s between tries ===")
    print(f"    streaming {args.pi_host} audio.service journal (Ctrl-C to finish)\n")

    # NB: user-service stdout lands in the SYSTEM journal under _SYSTEMD_USER_UNIT
    # (the per-user journald has no files here), so query it that way — not
    # `journalctl --user -u audio.service`, which returns "No journal files".
    proc = subprocess.Popen(
        ["ssh", args.pi_host, "journalctl _SYSTEMD_USER_UNIT=audio.service -f -n 0 --no-pager"],
        stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True, bufsize=1,
    )

    def summarize(*_):
        proc.terminate()
        print("\n\n=== SUMMARY [%s] ===" % args.label)
        attempts = len(fires) + len(misses)
        print(f"attempts (fires + real near-misses): {attempts}")
        print(f"  FIRED  (>= {threshold or 0.5}): {len(fires)}")
        print(f"  MISSED (< threshold, peak>{args.floor}): {len(misses)}")
        if attempts:
            print(f"  detection rate: {len(fires)/attempts:.0%}")
        if fires:
            print(f"  fire confidence:  min={min(fires):.3f} median={st.median(fires):.3f}")
        if misses:
            print(f"  miss peak:        {sorted(round(m,3) for m in misses)}")
        print(f"\nraw matched lines appended to {logpath}")
        sys.exit(0)

    signal.signal(signal.SIGINT, summarize)

    for line in proc.stdout:
        mf = FIRE_RE.search(line)
        if mf:
            c = float(mf.group(1))
            fires.append(c)
            logf.write(line); logf.flush()
            print(f"  ✅ FIRE     confidence={c:.3f}   (total fires={len(fires)})")
            continue
        mm = NEARMISS_RE.search(line)
        if mm:
            peak = float(mm.group(1)); threshold = float(mm.group(2))
            if peak < args.floor:
                continue  # ambient VAD segment, not a real attempt
            misses.append(peak)
            logf.write(line); logf.flush()
            print(f"  ❌ MISS     peak={peak:.3f} (threshold={threshold:.2f})   (total misses={len(misses)})")
            continue
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
