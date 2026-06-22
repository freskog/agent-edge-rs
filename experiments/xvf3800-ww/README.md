# XVF3800 Wake-Word / STT Output-Path Experiment

Goal: decide **which XVF3800 output stage** to feed the wake-word detector and
which to feed Whisper, optimizing (in priority order) recall → false-accepts →
barge-in robustness → STT accuracy → latency. **Not** subjective audio quality.

This is a *measurement* exercise, not a tuning exercise. We do **not** sweep
firmware parameters first. We first find the best existing signal stage, with
levels normalized so we never confuse "louder" with "better".

---

## 0. Grounding facts from this repo (read before designing anything)

These constrain the whole design — they are why the method below looks the way
it does.

1. **Production detector input is 16 kHz / I16 / mono, single channel.**
   `src/audio_source.rs` rejects anything that isn't 16 kHz I16, and captures
   exactly one ALSA channel (`AudioCaptureConfig.channel`, CLI `--input-channel`,
   default `0` = left). ALSA's `plug` in front of `xvf_cap_rt` does the 48→16 kHz
   resample and channel pick. **So in the live system only ONE channel reaches
   the detector, and today that channel is the left output (`AUDIO_MGR_OP_L` =
   `8 0`, the firmware-selected processed beam).**

2. **The detector engine is openWakeWord** (`src/wakeword_model.rs`):
   mel → embedding → `hey_mycroft_v0.1`. Fixed 1280-sample (80 ms @ 16 kHz)
   hops, `detection_threshold = 0.5` (`src/main.rs:119`), debounce, 30-frame
   prediction buffer. Any offline evaluation **must reuse this exact code** or
   the numbers don't transfer to the device.

3. **Capture format depends on transport** (`deploy/alsa/*.conf`):
   - I2S (`dtoverlay=xvf3800`, card `XVF3800`): `xvf_cap_rt` = **S32_LE 48k 2ch**.
   - USB (card `Array`): `xvf_cap_rt` = **S16_LE 48k 2ch**.
   Record at the **native 48 kHz stereo** so the left (processed) and right
   (candidate) channels are sample-aligned to the *same* utterance.

4. **Whisper is downstream over TCP**, not in this repo. STT evaluation is a
   separate offline harness run on recorded command audio.

5. **Builds use the devcontainer.** The new offline eval binary
   (`ww_eval`, below) is built inside `.devcontainer` (cross-compiled / run for
   the Pi or run on the dev box), never ad-hoc on the host.

---

## 1. Exact test matrix

Two things vary: the **candidate signal stage** (what's on the right channel)
and the **acoustic condition**. The left channel is always the current
production signal (`8 0`) so every recording carries a built-in baseline.

### 1a. Candidate stages (right channel)

| ID | Mux        | Stage                                   |
|----|------------|-----------------------------------------|
| A  | `1 0`      | Raw mic 0 (no gain, no DSP)             |
| B  | `3 0`      | Amplified mic 0 (mic gain, pre-AEC)     |
| C  | `7 3`      | AEC/ASR auto-selected beam              |
| D  | `8 0`      | Final processed beam (= the baseline)   |

Left channel is **always** `8 0`. We run three capture passes, swapping only the
right output:

```
Pass 1 (vs RAW): AUDIO_MGR_OP_R 1 0
Pass 2 (vs AMP): AUDIO_MGR_OP_R 3 0
Pass 3 (vs AEC): AUDIO_MGR_OP_R 7 3
```

> Do **not** `save_to_flash` during the experiment. Reset with a power-cycle or
> by re-issuing the known-good `AUDIO_MGR_OP_R 8 0` after each session.

### 1b. Conditions (10) × positions (4)

| Cond | Description                                  |
|------|----------------------------------------------|
| C1   | quiet, close (~0.5 m)                         |
| C2   | quiet, 2–3 m                                  |
| C3   | stationary background noise                  |
| C4   | music, low volume                            |
| C5   | music, medium volume                         |
| C6   | music, loud volume                           |
| C7   | speech/news, medium                          |
| C8   | speech/news, loud                            |
| C9   | speaker talks *before* the wake phrase        |
| C10  | wake phrase *starts during* loud playback     |

Positions: **0° / 90° / 180° / 270°** relative to the array front.

### 1c. Sizing — phased, smallest-useful-first

The full matrix is 3 passes × 10 conditions × 4 positions × N utterances. Don't
run it all up front. Three phases:

- **Phase 0 — Pilot / sanity (≈30 min, deliverable #10).**
  Pass 3 only (`8 0` vs `7 3`), conditions **C1, C6, C10**, position **0°**,
  one speaker, **20 utterances per condition**. This alone answers "is the
  processed beam costing us recall during barge-in?" — the single most likely
  failure and the cheapest to test.

- **Phase 1 — Stage selection.** All three passes, conditions C1/C2/C5/C6/C8/C10,
  position 0° + 180°, one speaker, **30 utterances/cell**. Picks the winning
  wake-word stage and STT stage.

- **Phase 2 — Robustness confirm.** Full condition × position grid for the
  *winning* stage(s) only, ≥3 speakers, ≥30 utterances/cell. Confirms the
  Phase-1 choice generalizes; produces the false-accept-per-hour number from
  long no-wake recordings.

Rule of thumb for the recall CI: 30 utterances → ±~9 pp at 90 % recall; bump to
~100 for a cell only if a stage is borderline. Don't pay for precision you won't act on.

---

## 2. Recording & file-naming scheme

One recording = one utterance (or one continuous "no-wake" block for FA-rate),
captured at native 48 kHz stereo, plus a sidecar JSON with everything needed to
reproduce and to align the reference.

### 2a. Capture command (on the Pi)

I2S (S32_LE) — record from the dsnoop node so we don't fight the running service:
```bash
arecord -D xvf_cap_rt -f S32_LE -r 48000 -c 2 -t wav "$OUT.wav"
```
USB (S16_LE):
```bash
arecord -D xvf_cap_rt -f S16_LE -r 48000 -c 2 -t wav "$OUT.wav"
```
> If `xvf_cap_rt` is busy/exclusive, record `hw:XVF3800,1` (I2S) / `hw:Array,0`
> (USB) directly. Either way, capture **before** the `plug`/resample so we keep
> 48 kHz and both channels.

For controlled playback conditions, play a **known** file via the same sink the
product uses (`xvf_out` → `mpv`/`spotifyd`) and **save the exact playback file
path + start offset + ALSA softvol setting** in the sidecar — that file is our
far-end reference for AEC analysis (§9). Prefer playing a local file we own over
streaming, so the reference is bit-exact.

### 2b. Naming

```
xvf_<pass>_<cond>_<pos>_<spk>_<vol>_<NNN>.wav
        |       |     |     |     |      |
        |       |     |     |     |      └ utterance index, zero-padded
        |       |     |     |     └ playback level: q|lo|med|hi  (q = none)
        |       |     |     └ speaker id: s01..
        |       |     └ position: 000|090|180|270
        |       └ condition: c01..c10
        └ pass / right-stage: raw|amp|aec   (left is always proc=8 0)
```
Example: `xvf_aec_c10_000_s01_hi_004.wav` = Pass-3 (right=`7 3`), C10, front,
speaker 1, loud playback, utterance 4. Left channel = `8 0`.

### 2c. Sidecar (`<name>.json`) — required fields

```json
{
  "transport": "i2s|usb",
  "left_mux": "8 0", "right_mux": "7 3",
  "left_stage": "proc", "right_stage": "aec",
  "condition": "c10", "position_deg": 0, "speaker": "s01",
  "playback": { "file": "ref/news_med.wav", "start_offset_s": 3.21,
                "alsa_softvol_pct": 60, "level_label": "hi" },
  "ground_truth": { "wake": true, "wake_onset_s": 4.05,
                    "command_text": "what's the weather tomorrow",
                    "command_span_s": [4.95, 6.60] },
  "firmware": { "build": "inthost-lr48-sqr-i2c", "version": "1.0.7",
                "params_snapshot": "params/phase1.txt" }
}
```
`wake_onset_s` is the human-labelled true start of "Hey Mycroft" — this is the
anchor for detection-latency and "phrase lost before detection". Label it once
on the **left** channel and reuse for both channels (they're sample-aligned).

Dump the live param block once per session to `params/<phase>.txt` (the full
`xvf_host` get of every AEC_/PP_/AUDIO_MGR_ key) so a result is always tied to
the firmware state that produced it.

---

## 3. Splitting stereo channels

In the devcontainer (has sox/ffmpeg). Split, then resample 48→16 kHz with a
**high-quality** resampler (the production `plug` resampler is cheap; for the
offline judge we want clean 16 kHz so we measure the *signal*, not resampler
artefacts — and we cross-check against the cheap path in §8 note):

```bash
# left = processed baseline, right = candidate
sox "$f.wav" -b 16 "$f.L48.wav" remix 1
sox "$f.wav" -b 16 "$f.R48.wav" remix 2
# 48k -> 16k mono s16le, high-quality
sox "$f.L48.wav" -r 16000 -c 1 -b 16 "$f.L16.wav" rate -v -s
sox "$f.R48.wav" -r 16000 -c 1 -b 16 "$f.R16.wav" rate -v -s
```
Keep both the 48 k splits (for spectrograms / AEC xcorr) and the 16 k mono
(for the detector). Note S32_LE input downscales cleanly to S16; record peak so
we know if the 32-bit path was actually carrying >16-bit range.

---

## 4. Level-normalization strategy

The whole experiment is invalid if we compare a +25 dB limited beam (`8 0`,
`PP_AGCGAIN 17.68`, limiter at 0.47) against raw mic 0 at face value. So every
file is judged **twice**:

1. **As-captured (live realism).** What the device would actually feed the
   detector today. This is the operational truth.
2. **RMS-normalized (signal quality).** Each 16 k file scaled to a fixed target
   **RMS = −23 dBFS** (EBU R128-ish), measured over the wake-phrase span only
   (so silence/echo before the phrase doesn't skew the gain), hard-limited to
   avoid clip. This removes amplitude as a variable.

```bash
sox "$f.R16.wav" "$f.R16.norm.wav" gain -n -23   # peak-normalize variant
# RMS variant: compute RMS over [wake_onset, wake_onset+0.8s], apply scalar gain
```
Record **peak, RMS, clipped-sample count** for every file, every variant. If a
stage only wins as-captured but ties when normalized → the win was **gain**, not
signal (hypothesis 5; see §8).

---

## 5. Wake-word evaluation script — `ww_eval` (new Rust bin)

A new binary `src/bin/ww_eval.rs` that **reuses `wakeword_model::Model`** exactly
as production does. WAV in, JSON out. No reimplementation of the pipeline — that
reuse is the point; it's what makes offline numbers predict on-device behaviour.

### Design
```
ww_eval --wav <file.16k.wav> --sidecar <file.json>
        --threshold 0.5 --model hey_mycroft [--reset-per-file]
```
- Load `Model::new(vec!["hey_mycroft"], …)` once; `reset()` per file.
- Read s16le mono; feed in **1280-sample** hops (matches `CHUNK_SIZE`).
- For each hop call `predict(&chunk, Some({hey_mycroft:0.5}), debounce)`.
- Track per-hop confidence trace; record:
  - `fired` (bool), `peak_confidence`, `fire_frame` (first hop ≥ threshold),
  - `fire_time_s = fire_frame * 0.08`,
  - `detection_latency_s = fire_time_s − wake_onset_s` (from sidecar),
  - `phrase_lost_s = max(0, fire_time_s − wake_onset_s)` clamped to phrase end —
    "how much of the phrase elapsed before it fired",
  - full confidence trace (for spectrogram overlay).
- Emit one JSON line per file → aggregate with a small Python script.

> Why a binary and not Python+tflite: the Rust mel/embedding preprocessor,
> warmup-zeroing, multi-chunk max, and debounce in `wakeword_model.rs` are the
> behaviour we ship. Re-deriving them in Python would measure a *different*
> detector. Build in `.devcontainer`.

### Aggregation (`agg_ww.py`)
Group JSONL by `(stage, condition, position, variant)` →
TPR, FNR, median/p90 detection latency, median peak confidence, median
`phrase_lost_s`. FA/hour comes from running `ww_eval` over the long no-wake
recordings (count fires / hours of audio).

### Sanity tie-back to live
For ≥1 condition, also run the **live** `ww-livetest.py` (`deploy/ww-livetest.py`)
on the device with `--input-channel` pointed at the winning stage and confirm the
offline TPR is within a few pp of live. If they diverge, trust live and find why
(usually the offline resampler or normalization).

---

## 6. STT evaluation method

Separate from wake-word. Whisper is downstream, so this is offline on the
**command span** (`ground_truth.command_span_s`, padded ±200 ms).

- Cut the command span from each candidate's 16 k file (as-captured **and**
  normalized).
- Transcribe with the same Whisper model/size the product uses (pin model +
  params; record them).
- Metrics vs `ground_truth.command_text`:
  - **WER** (normalized: lowercase, strip punct, number words),
  - **first-word deletion rate** — fraction where the reference's first token is
    missing/substituted in hyp. *This is the headline STT metric*: DSP gating and
    wake→STT handover latency tend to eat the first phonemes.
  - **wake-tail leakage** — fraction where "mycroft"/"hey" bleeds into the hyp
    (window started too early / tail not trimmed),
  - **barge-in success** — for C9/C10, did the command transcribe at all.
- Report per stage × condition. Hypothesis 3 (best STT may be `8 0` even if it's
  not the best wake stage) falls straight out of this table.

The architecture question (hypothesis 7: split signals — one stage for wake, one
for STT) is answered by reading the §5 and §6 tables together: if stage X wins
wake and stage Y wins STT with a meaningful gap, route accordingly (the firmware
can drive different muxes to L/R, and the consumer can capture a second channel
for STT only).

---

## 7. Metrics & pass/fail thresholds

Thresholds are **relative to the current `8 0` baseline** in the *same recording*
(that's why `8 0` is always the left channel). Absolute targets in parentheses.

| Metric | Pass | Fail |
|---|---|---|
| Wake TPR, quiet (C1/C2) | ≥ baseline − 2 pp (and ≥ 95 %) | < baseline − 5 pp |
| Wake TPR, barge-in (C6/C10) | ≥ baseline + 5 pp (clear win) | ≤ baseline |
| FA / hour | ≤ baseline (and ≤ 1/hr) | > 2× baseline |
| Detection latency p90 | ≤ baseline + 50 ms | > baseline + 150 ms |
| `phrase_lost_s` median | ≤ 0.30 s | > 0.50 s |
| STT WER | ≤ baseline + 2 pp | > baseline + 5 pp |
| First-word deletion rate | ≤ baseline (and ≤ 5 %) | > 10 % |

**Decision rule:** a stage *replaces* `8 0` for wake only if it passes quiet
**and** wins barge-in **and** doesn't fail FA/latency. Otherwise keep `8 0`
for wake and only consider it as an STT-side or barge-in-only signal.

---

## 8. Separating DSP-quality problems from gain problems

This is the crux of hypothesis 5, and it's purely a read of the two variants
from §4:

| As-captured | Normalized | Conclusion |
|---|---|---|
| Stage wins | Stage still wins | **Real signal/DSP advantage** — act on it |
| Stage wins | Tie | Win was **gain**; just raise capture gain on the simpler stage |
| Stage loses | Stage wins | Stage was **under-levelled**; it's actually better — fix gain |
| Stage loses | Stage loses | Genuinely worse signal |

Concretely: compute Δrecall and Δpeak-confidence (stage − baseline) for both
variants per cell. Plot Δ_as-captured vs Δ_normalized. Points on the diagonal =
gain effects; points off-diagonal toward normalized = real DSP effects.

Supporting evidence per file: peak / RMS / clipped-sample counts (§4). A stage
that "wins" while also showing high clipped-sample counts is suspect — the
limiter is doing the work, and that distortion is what hurt STT under
`PP_ECHOONOFF 1`.

> Resampler caveat: the offline judge uses high-quality `rate -v -s` while the
> live `plug` uses a cheaper resampler. Run one cell through *both* resampling
> qualities; if the stage ranking flips, the resampler — not the XVF stage — is a
> confound, and we note it.

---

## 9. Identifying AEC / reference-alignment failures

Hypothesis 6: loud-playback degradation may be an AEC/reference problem, not a
detector problem. We can test this because we **played a known file** (sidecar
`playback.file`, `start_offset_s`) — that's the far-end reference.

For each playback condition (C4–C8, C10), on the 48 k candidate channel:

1. **Delay & drift via cross-correlation.** Cross-correlate the candidate
   channel against the known playback file (resampled to match). Expect a sharp
   peak at a stable lag ≈ `AUDIO_MGR_SYS_DELAY` (12 samples = 0.25 ms) + acoustic
   path. **Symptoms of misalignment:**
   - broad / low cross-correlation peak → poor coherence (EQ, compression,
     nonlinearity in the speaker path the linear AEC can't model),
   - lag that **drifts across the file** → clock drift / variable buffering
     (BT/USB) → AEC can never stay converged.
2. **ERLE on no-speech segments.** On a playback-only stretch (no wake phrase),
   `ERLE = 10·log10(RMS_rawmic² / RMS_processed²)` between the raw mic (`1 0`/`3 0`,
   right channel of the raw/amp passes) and the processed beam (`8 0`, left).
   Low ERLE during loud playback = AEC under-cancelling = reference mismatch,
   not a wake-word-model fault.
3. **Signal-to-reference ratio (SRR)** around the wake phrase: candidate RMS in
   the phrase span vs candidate RMS of residual echo just before it. Falling SRR
   as playback volume rises, *with* low ERLE, points squarely at AEC/reference —
   the fix is `AUDIO_MGR_REF_GAIN` / `AUDIO_MGR_SYS_DELAY` / speaker-path EQ, not
   the detector.

If ERLE is healthy and lag is stable but recall still drops on the processed
beam → it's genuine over-processing/gating of near-end speech (the
`PP_ECHOONOFF 1`-style failure), and the answer is to feed the detector a
*less* processed stage (`7 3` or `3 0`), which is exactly what this experiment
selects.

> `AUDIO_MGR_SELECTED_CHANNELS 3 3` means both selectors currently point at the
> auto-selected beam; when reading raw/amp passes remember the *right* channel is
> pre-AEC and will legitimately show full echo — that's the point of the ERLE
> comparison.

---

## 10. Recommended minimum experiment before touching any firmware param

**Run Phase 0 only, then stop and look:**

1. Set right channel to the AEC beam: `xvf_host -u i2c AUDIO_MGR_OP_R 7 3`
   (left stays `8 0`). Dump params to `params/phase0.txt`.
2. Record 20 utterances each for **C1 (quiet/close)**, **C6 (loud music)**,
   **C10 (wake during loud playback)**, position 0°, one speaker. Play a known
   local file for C6/C10 and log its path/offset/volume.
3. Split + resample (§3), run `ww_eval` both variants (§4–5), and run the AEC
   xcorr/ERLE check (§9) on the C6/C10 files.
4. Read three things:
   - Does `7 3` beat `8 0` on C10 recall (normalized)? → wake-word stage matters.
   - Is ERLE low / lag drifting on C6? → it's an **AEC/reference** problem; fix
     alignment *before* any PP tuning.
   - Does `8 0` still win STT WER / first-word? → keep `8 0` for STT regardless.

Only after this do we decide whether to (a) re-route the detector to a less
processed stage, (b) chase AEC alignment (`REF_GAIN`/`SYS_DELAY`/speaker EQ), or
(c) leave it. **No `PP_*`/`AEC_*` sweep until Phase 0 says which of those three
problems we actually have.** And never `save_to_flash` during experimentation.

---

## Build / run notes

- `ww_eval` and any helper bins build in `.devcontainer` (see `.devcontainer/`).
  Add `[[bin]] name = "ww_eval"` analogous to existing bins; it depends only on
  the already-present `wakeword_*` modules + a tiny WAV reader (`hound`).
- Keep all raw recordings, splits, sidecars, JSONL, and `params/*.txt` under
  `experiments/xvf3800-ww/data/<session>/` (git-ignore the WAVs).
