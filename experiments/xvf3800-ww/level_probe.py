#!/usr/bin/env python3
"""
Level / SNR diagnostic for the XVF3800 "quiet conversational speech" theory.

Capture two stages SIMULTANEOUSLY into one stereo WAV so both see the identical
utterance, recorded at normal AND loud vocal effort:

    L (ch 0) = 6 3   final processed auto-selected beam   (has PP_AGCGAIN ~+25dB)
    R (ch 1) = 3 0   mic 0 after MIC_GAIN, before AEC/beam (raw-ish reference)

Then this tool reports, per channel per effort: RMS/peak in dBFS, estimated
noise floor and SNR, and — the decisive numbers — the normal→loud DELTA per
path. It applies your decision tree automatically:

  * proc delta ≈ raw delta            -> level tracks linearly: FINAL GAIN TOO LOW
                                         (raise AEC_ASROUTGAIN / use AGC), gain recovers it
  * proc delta >> raw delta, or proc  -> XVF processing SUPPRESSES/DISTORTS quiet
    SNR much worse than raw SNR           speech (NS / beam / gating); gain won't fix
  * raw (3 0) already very quiet       -> investigate MIC_GAIN / PCM scaling / physical SNR

Absolute levels differ by design (6 3 carries +25dB PP gain); read the DELTAs.

Usage (one stereo WAV per effort):
    ./level_probe.py --left 6_3 --right 3_0 normal=normal.wav loud=loud.wav

Handles 16- and 32-bit PCM WAV. Pure stdlib.
"""
from __future__ import annotations

import argparse
import math
import struct
import sys
import wave

FRAME_MS = 20


def load_channels(path: str):
    """Return (sample_rate, [ch0_floats, ch1_floats...]) normalized to ±1.0."""
    with wave.open(path, "rb") as w:
        sr, sw, ch, n = (w.getframerate(), w.getsampwidth(),
                         w.getnchannels(), w.getnframes())
        raw = w.readframes(n)
    if sw == 2:
        fs = 32768.0
        fmt = f"<{len(raw)//2}h"
        flat = struct.unpack(fmt, raw)
    elif sw == 4:
        fs = 2147483648.0
        fmt = f"<{len(raw)//4}i"
        flat = struct.unpack(fmt, raw)
    else:
        sys.exit(f"{path}: unsupported sample width {sw*8}-bit "
                 f"(convert with: sox in.wav -b 16 out.wav)")
    chans = [[flat[i] / fs for i in range(c, len(flat), ch)] for c in range(ch)]
    return sr, chans


def db(x: float) -> float:
    return 20.0 * math.log10(x) if x > 1e-12 else -120.0


def frame_rms(samples, sr):
    fl = max(1, sr * FRAME_MS // 1000)
    out = []
    for i in range(0, len(samples) - fl, fl):
        seg = samples[i:i + fl]
        out.append(math.sqrt(sum(s * s for s in seg) / len(seg)))
    return out


def pct(sorted_vals, p):
    if not sorted_vals:
        return 0.0
    k = min(len(sorted_vals) - 1, max(0, int(p / 100.0 * len(sorted_vals))))
    return sorted_vals[k]


def analyze(samples, sr):
    if not samples:
        return None
    overall_rms = math.sqrt(sum(s * s for s in samples) / len(samples))
    peak = max(abs(s) for s in samples)
    fr = sorted(frame_rms(samples, sr))
    noise = pct(fr, 10)                      # 10th pct frame ≈ noise floor
    gate = noise * 3.0                        # ~ +9.5 dB above floor = speech
    speech_frames = [r for r in fr if r > gate]
    speech = (math.sqrt(sum(r * r for r in speech_frames) / len(speech_frames))
              if speech_frames else overall_rms)
    snr = db(speech) - db(noise)
    return {"rms_dbfs": db(overall_rms), "peak_dbfs": db(peak),
            "speech_dbfs": db(speech), "noise_dbfs": db(noise), "snr_db": snr}


def main() -> int:
    ap = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--left", default="ch0", help="label for channel 0 (e.g. 6_3)")
    ap.add_argument("--right", default="ch1", help="label for channel 1 (e.g. 3_0)")
    ap.add_argument("recordings", nargs="+",
                    help="effort=path pairs, e.g. normal=normal.wav loud=loud.wav")
    args = ap.parse_args()

    import hashlib
    efforts = {}
    hashes = {}
    for spec in args.recordings:
        effort, _, path = spec.partition("=")
        if not path:
            sys.exit(f"bad arg '{spec}', expected effort=path")
        with open(path, "rb") as fh:
            h = hashlib.md5(fh.read()).hexdigest()
        for other_eff, (other_path, other_h) in hashes.items():
            if other_h == h:
                print(f"  ⚠️  '{path}' is BYTE-IDENTICAL to '{other_path}' "
                      f"({effort} == {other_eff}). The recording didn't change — "
                      f"re-record before trusting this comparison.\n")
        hashes[effort] = (path, h)
        sr, chans = load_channels(path)
        labels = [args.left, args.right][:len(chans)]
        efforts[effort] = {lab: analyze(c, sr) for lab, c in zip(labels, chans)}

    labels = [args.left] + ([args.right] if len(next(iter(efforts.values()))) > 1 else [])

    # ---- per-effort table ----
    for effort, by_label in efforts.items():
        print(f"\n=== {effort} ===")
        print(f"  {'path':<8} {'RMS dBFS':>9} {'peak':>7} {'speech':>8} "
              f"{'noise':>7} {'SNR dB':>7}")
        for lab in labels:
            a = by_label.get(lab)
            if a:
                print(f"  {lab:<8} {a['rms_dbfs']:>9.1f} {a['peak_dbfs']:>7.1f} "
                      f"{a['speech_dbfs']:>8.1f} {a['noise_dbfs']:>7.1f} "
                      f"{a['snr_db']:>7.1f}")

    # ---- verdict (needs normal + loud) ----
    if "normal" in efforts and "loud" in efforts:
        print("\n=== normal → loud delta ===")
        deltas = {}
        for lab in labels:
            nrm, lod = efforts["normal"].get(lab), efforts["loud"].get(lab)
            if nrm and lod:
                d = lod["speech_dbfs"] - nrm["speech_dbfs"]
                deltas[lab] = d
                print(f"  {lab:<8} speech +{d:5.1f} dB   "
                      f"(SNR {nrm['snr_db']:.0f}→{lod['snr_db']:.0f} dB)")

        print("\n=== verdict ===")
        proc, raw = args.left, args.right
        d_proc, d_raw = deltas.get(proc), deltas.get(raw)
        raw_normal = efforts["normal"].get(raw)
        proc_normal = efforts["normal"].get(proc)
        proc_loud = efforts["loud"].get(proc)
        HEALTHY_SPEECH_DBFS = -28.0   # above this, the path is well-leveled for STT
        CLIP_DBFS = -0.5

        if raw_normal and raw_normal["speech_dbfs"] < -45:
            print(f"  ⚠️  raw {raw} speech is already very quiet "
                  f"({raw_normal['speech_dbfs']:.0f} dBFS) at normal effort →")
            print(f"     investigate AUDIO_MGR_MIC_GAIN / PCM scaling / physical SNR.")
        if proc_loud and proc_loud["peak_dbfs"] >= CLIP_DBFS:
            print(f"  ⚠️  processed {proc} CLIPS at loud voice "
                  f"(peak {proc_loud['peak_dbfs']:.1f} dBFS) — it is not short on gain.")
        if d_proc is not None and d_raw is not None:
            spread = d_proc - d_raw
            proc_hot = proc_normal and proc_normal["speech_dbfs"] >= HEALTHY_SPEECH_DBFS
            if abs(spread) <= 3.0 and proc_hot:
                print(f"  ✅ LINEAR (processed tracks raw within {spread:+.1f} dB) AND "
                      f"processed is already well-leveled")
                print(f"     ({proc_normal['speech_dbfs']:.0f} dBFS speech, "
                      f"SNR {proc_normal['snr_db']:.0f} dB at normal voice).")
                print(f"     → Quiet-room level is NOT the bottleneck; more gain won't help")
                print(f"       (it already clips on loud). If you must speak up in real use,")
                print(f"       the cause is in conditions excluded here — AEC ducking your")
                print(f"       near-end speech under music/barge-in, or distance. Re-capture")
                print(f"       WITH music playing and compare {proc} speech/SNR quiet-vs-music.")
            elif abs(spread) <= 3.0:
                print(f"  ✅ processed tracks raw within {spread:+.1f} dB → LINEAR, and "
                      f"processed speech is low ({proc_normal['speech_dbfs']:.0f} dBFS).")
                print(f"     Quiet speech is just under-leveled — the fixed final gain is")
                print(f"     too low. Raise it (AEC_ASROUTGAIN on 7_3, or enable AGC) —")
                print(f"     gain WILL recover it (matches the 7_3 re-score).")
            elif spread > 3.0:
                print(f"  ❗ processed rises {spread:+.1f} dB MORE than raw with effort →")
                print(f"     NON-LINEAR: XVF processing (NS/beam/gating) is suppressing")
                print(f"     quiet speech and releasing on loud. Gain alone won't fix it;")
                print(f"     look at PP_MIN_NS / beam selection / VAD, or move upstream (3_0).")
            else:
                print(f"  processed rises {spread:+.1f} dB LESS than raw — unusual; "
                      f"check limiter on loud (PP_LIMITPLIMIT).")
        # SNR damage check
        if raw_normal and efforts["normal"].get(proc):
            ds = efforts["normal"][proc]["snr_db"] - raw_normal["snr_db"]
            if ds < -6:
                print(f"  ❗ at normal effort, processed SNR is {ds:.0f} dB WORSE than raw")
                print(f"     → processing is degrading quiet-speech SNR, not just level.")
    elif len(efforts) == 2:
        # generic baseline -> playback comparison (e.g. quiet -> music): is the
        # SNR loss from rising echo residual, and is the AEC under-cancelling?
        base_eff, play_eff = list(efforts.keys())
        print(f"\n=== {base_eff} → {play_eff} (echo / floor) ===")
        proc, raw = args.left, args.right
        floor_rise, snr_drop, speech_d = {}, {}, {}
        for lab in labels:
            b, p = efforts[base_eff].get(lab), efforts[play_eff].get(lab)
            if b and p:
                floor_rise[lab] = p["noise_dbfs"] - b["noise_dbfs"]
                snr_drop[lab] = b["snr_db"] - p["snr_db"]
                speech_d[lab] = p["speech_dbfs"] - b["speech_dbfs"]
                print(f"  {lab:<8} floor {floor_rise[lab]:+5.1f} dB   "
                      f"speech {speech_d[lab]:+5.1f} dB   "
                      f"SNR {b['snr_db']:.0f}→{p['snr_db']:.0f} dB")

        print("\n=== verdict ===")
        pr = floor_rise.get(proc)
        if pr is not None and raw in floor_rise:
            erle = floor_rise[raw] - pr      # how much less the proc floor rose
            print(f"  AEC attenuated the playback floor by ~{erle:.0f} dB "
                  f"(raw {floor_rise[raw]:+.0f} vs processed {pr:+.0f}).")
            preserved = speech_d.get(proc, 0) > -3
            p_play = efforts[play_eff].get(proc, {})
            if preserved and pr > 6:
                print(f"  ✅ near-end speech PRESERVED ({speech_d[proc]:+.1f} dB) — no ducking.")
                print(f"     SNR loss is from echo RESIDUAL: floor +{pr:.0f} dB under "
                      f"playback → SNR {p_play.get('snr_db', 0):.0f} dB.")
                if erle < 12:
                    print(f"  ❗ ERLE looks low (~{erle:.0f} dB). AEC is under-cancelling →")
                    print(f"     check REFERENCE ALIGNMENT: AUDIO_MGR_SYS_DELAY (12 looks")
                    print(f"     too small for the playback pipeline) and AUDIO_MGR_REF_GAIN.")
                    print(f"     Lowering residual here raises SNR WITHOUT touching near-end")
                    print(f"     speech — unlike PP_ECHOONOFF (gates speech) or adding gain.")
                print(f"  Next: capture MUSIC-ONLY (no speech) to get a clean ERLE number,")
                print(f"        then sweep SYS_DELAY and watch the processed floor drop.")
            elif not preserved:
                print(f"  ❗ near-end speech DROPPED {speech_d[proc]:+.1f} dB under playback →")
                print(f"     AEC/double-talk IS ducking speech; look at PP_DTSENSITIVE.")
    else:
        print("\n(provide normal=… loud=… for the gain verdict, or exactly two "
              "efforts e.g. quiet=… music=… for the echo verdict)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
