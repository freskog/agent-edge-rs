#!/usr/bin/env python3
"""
Offline re-score of captured XVF3800 utterances at MATCHED LEVEL.

The live runs leave two confounds in the STT numbers:
  1. Level mismatch — e.g. the 8 0 processed beam is limiter-pinned near peak
     0.5 while the 7 3 AEC beam sits ~20 dB lower. Whisper accuracy tracks
     level, so an as-captured comparison partly measures gain, not signal.
  2. language=detect flips to the operator's other language (Swedish here) on
     short clips and hallucinates.

This tool reads the saved WAVs + their results-<label>.jsonl (for ground-truth
`expected` text), normalizes each clip to a target RMS, optionally pins the STT
language, re-transcribes, and prints a side-by-side stage comparison. Re-uses
fake_agent_edge for the STT call so the request matches the live path.

  ./rescore.py --labels L=8_0_music L=7_3_music \
      --stt-url http://10.10.100.102:8008 --language en --target-rms-dbfs -23

Compare the NORMALIZED column across stages: if 7_3 ties 8_0 once levels match,
the live gap was gain (just add capture gain); if it still trails, it's signal.
"""
from __future__ import annotations

import argparse
import json
import math
import os
import re
import statistics as st
import struct
import wave

import fake_agent_edge as fae   # transcribe(), strip_wake()

# ---- number-word folding so "ten minutes" == "10 minutes" ------------------
_NUMWORDS = {
    "zero": "0", "one": "1", "two": "2", "three": "3", "four": "4", "five": "5",
    "six": "6", "seven": "7", "eight": "8", "nine": "9", "ten": "10",
    "eleven": "11", "twelve": "12", "thirteen": "13", "fourteen": "14",
    "fifteen": "15", "sixteen": "16", "seventeen": "17", "eighteen": "18",
    "nineteen": "19", "twenty": "20", "thirty": "30", "forty": "40",
    "fifty": "50", "sixty": "60", "seventy": "70", "eighty": "80", "ninety": "90",
}


def normalize(text: str) -> str:
    text = text.lower()
    text = re.sub(r"[^a-z0-9' ]+", " ", text)
    toks = [_NUMWORDS.get(t, t) for t in text.split()]
    return " ".join(toks).strip()


def wer(ref: str, hyp: str):
    r = normalize(ref).split()
    h = normalize(fae.strip_wake(hyp)).split()
    if not r:
        return 0.0, False
    dp = list(range(len(h) + 1))
    for i in range(1, len(r) + 1):
        prev, dp[0] = dp[0], i
        for j in range(1, len(h) + 1):
            cur = dp[j]
            dp[j] = min(dp[j] + 1, dp[j - 1] + 1, prev + (r[i - 1] != h[j - 1]))
            prev = cur
    first_word_deleted = (not h) or (h[0] != r[0])
    return dp[len(h)] / len(r), first_word_deleted


def read_wav(path: str) -> bytes:
    with wave.open(path, "rb") as w:
        assert w.getsampwidth() == 2 and w.getnchannels() == 1
        return w.readframes(w.getnframes())


def normalize_rms(pcm: bytes, target_dbfs: float) -> tuple[bytes, float]:
    """Scale to target RMS (dBFS), then clamp peak to avoid clipping.
    Returns (pcm, applied_gain)."""
    n = len(pcm) // 2
    if n == 0:
        return pcm, 1.0
    s = list(struct.unpack(f"<{n}h", pcm))
    rms = math.sqrt(sum(x * x for x in s) / n) or 1.0
    target = (10 ** (target_dbfs / 20.0)) * 32768.0
    gain = target / rms
    peak = max(abs(x) for x in s) or 1
    if peak * gain > 32767:                 # don't clip
        gain = 32767 / peak
    out = [max(-32768, min(32767, int(round(x * gain)))) for x in s]
    return struct.pack(f"<{n}h", *out), gain


def score_label(label, args):
    rows = [json.loads(l) for l in open(os.path.join(args.outdir, f"results-{label}.jsonl"))]
    out = []
    for r in rows:
        wav = r.get("wav")
        exp = r.get("expected")
        if not (wav and exp and r.get("wake")):
            continue
        pcm = read_wav(os.path.join(args.outdir, wav))
        if args.no_normalize:
            send, gain = pcm, 1.0
        else:
            send, gain = normalize_rms(pcm, args.target_rms_dbfs)
        prompt = args.prompt
        try:
            # pin language by appending it as a query override if requested
            stt_url = args.stt_url
            resp, _lat = _transcribe(stt_url, send, prompt, args.language, args.stt_timeout)
            text = (resp.get("text") or "").strip()
        except Exception as e:  # noqa: BLE001
            text = f"<STT ERROR: {e}>"
        w, fwd = wer(exp, text)
        out.append({"idx": r.get("idx"), "expected": exp, "text": text,
                    "wer": w, "fwd": fwd, "gain": gain})
        flag = "✅" if w == 0 else ("⚠️ " if w <= 0.5 else "❌")
        print(f"  [{label}] #{r.get('idx'):>2} {flag} WER={w:.2f} g×{gain:4.1f}  "
              f"“{text}”")
    return out


def _transcribe(stt_url, pcm, prompt, language, timeout):
    import time, urllib.request, urllib.parse, json as _json
    qs = urllib.parse.urlencode({
        "beam_size": "1", "vad_filter": "true",
        "language": language, "initial_prompt": prompt})
    req = urllib.request.Request(
        f"{stt_url.rstrip('/')}/transcribe?{qs}", data=pcm, method="POST",
        headers={"Accept": "application/json",
                 "Content-Type": "application/octet-stream"})
    t0 = time.monotonic()
    with urllib.request.urlopen(req, timeout=timeout) as resp:
        body = resp.read()
    return _json.loads(body.decode("utf-8")), (time.monotonic() - t0) * 1000.0


def summarize(label, rows):
    if not rows:
        return None
    wers = [r["wer"] for r in rows]
    perfect = sum(1 for w in wers if w == 0)
    fwd = sum(1 for r in rows if r["fwd"])
    return {"label": label, "n": len(rows), "median_wer": st.median(wers),
            "perfect": perfect, "fwd": fwd}


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--labels", nargs="+", required=True,
                    help="result labels to compare, e.g. L=8_0_music L=7_3_music")
    ap.add_argument("--stt-url", default=os.environ.get("STT_URL", "http://10.10.100.102:8008"))
    ap.add_argument("--language", default="en",
                    help="pin STT language (use 'detect' to keep auto-detect)")
    ap.add_argument("--prompt", default="Hey Mycroft")
    ap.add_argument("--target-rms-dbfs", type=float, default=-23.0)
    ap.add_argument("--no-normalize", action="store_true",
                    help="score as-captured (skip RMS normalization)")
    ap.add_argument("--stt-timeout", type=float, default=30.0)
    ap.add_argument("--outdir", default=".")
    args = ap.parse_args()

    mode = "AS-CAPTURED" if args.no_normalize else f"RMS→{args.target_rms_dbfs:.0f}dBFS"
    print(f"=== offline re-score [{mode}, language={args.language}] ===\n")
    summaries = []
    for label in args.labels:
        rows = score_label(label, args)
        summaries.append(summarize(label, rows))
        print()

    print("=== COMPARISON ===")
    print(f"  {'stage':<16} {'n':>3} {'med WER':>8} {'perfect':>8} {'1st-word lost':>13}")
    for s in summaries:
        if s:
            print(f"  {s['label']:<16} {s['n']:>3} {s['median_wer']:>8.2f} "
                  f"{s['perfect']:>5}/{s['n']:<2} {s['fwd']:>10}/{s['n']:<2}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
