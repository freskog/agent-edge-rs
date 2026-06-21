#!/usr/bin/env bash
#
# Run one fake_agent_edge.py wake/STT measurement session, wrapped with a
# before/after XVF3800 parameter snapshot so each result is tied to the exact
# device state that produced it.
#
# Run this ON the Pi (mycroft), in this directory. You read the prompts aloud.
#
# Usage:
#   ./run_session.sh <label> [options] [-- <extra fake_agent_edge.py args>]
#
# Options:
#   --softvol PCT       set "XVF3800 SoftMaster" to PCT% before running
#   --far-extgain DB    set AEC_FAR_EXTGAIN to DB before running
#   --no-stt            capture-only (no Whisper); passed through to the harness
#   --no-music          drop --keep-music (no barge-in resume)
#
# Anything after `--` is passed straight to fake_agent_edge.py.
#
# Examples:
#   ./run_session.sh sv-normal --softvol 39
#   ./run_session.sh fext6 --far-extgain 6
#   ./run_session.sh micclip-loud --no-stt -- --commands-file screen.txt
#
# Notes:
#   - Consumer port is 50051 on this build (audio --consumer-bind 0.0.0.0:50051).
#   - agent-edge is already stopped; if it ever runs, stop it first to free the
#     single consumer slot: systemctl --user stop agent-edge
#   - Device writes (--softvol/--far-extgain) are NOT saved to flash.

set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
XVF_DIR="${XVF_DIR:-$HOME/reSpeaker_XVF3800_USB_4MIC_ARRAY/host_control/rpi_64bit}"
XVF="$XVF_DIR/xvf_host -u i2c"
MIXER='XVF3800 SoftMaster'
CARD='XVF3800'
CONSUMER="${CONSUMER:-127.0.0.1:50051}"
SPOTIFY="${SPOTIFY:-http://127.0.0.1:3001}"

SNAP_PARAMS=(AEC_AECCONVERGED AEC_RT60 AEC_FAR_EXTGAIN PP_AGCONOFF PP_AGCGAIN \
  AUDIO_MGR_MIC_GAIN AUDIO_MGR_REF_GAIN AUDIO_MGR_SYS_DELAY PP_ECHOONOFF \
  PP_NLATTENONOFF PP_NLAEC_MODE PP_GAMMA_ENL)

if [[ $# -lt 1 ]]; then
  grep '^#' "$0" | sed 's/^# \{0,1\}//' ; exit 1
fi
LABEL="$1"; shift

softvol=""; far=""; keep_music=1; extra=()
while [[ $# -gt 0 ]]; do
  case "$1" in
    --softvol)     softvol="$2"; shift 2 ;;
    --far-extgain) far="$2"; shift 2 ;;
    --no-stt)      extra+=(--no-stt); shift ;;
    --no-music)    keep_music=0; shift ;;
    --)            shift; extra+=("$@"); break ;;
    *)             extra+=("$1"); shift ;;
  esac
done

SESSION="$(date +%Y%m%d-%H%M%S)"
OUTDIR="$HERE/data/$SESSION"
mkdir -p "$OUTDIR"

snapshot() {  # $1 = phase (before|after)
  local f="$OUTDIR/$LABEL.params.$1.txt"
  { echo "# $LABEL params $1  $(date -Is)"
    printf 'SOFTVOL %s\n' "$(amixer -c "$CARD" sget "$MIXER" 2>/dev/null | grep -oE '[0-9]+%' | head -1)"
    for p in "${SNAP_PARAMS[@]}"; do printf '%s ' "$p"; $XVF "$p" 2>/dev/null | tr -d '\000'; done
  } > "$f"
  echo "  snapshot -> ${f#$HERE/}"
}

[[ -n "$softvol" ]] && { echo "+ softvol -> ${softvol}%"; amixer -c "$CARD" sset "$MIXER" "${softvol}%" >/dev/null; }
[[ -n "$far" ]]     && { echo "+ AEC_FAR_EXTGAIN -> $far"; $XVF AEC_FAR_EXTGAIN "$far" >/dev/null; sleep 1; }

echo "=== session $SESSION  label=$LABEL ==="
snapshot before

cmd=("$HERE/fake_agent_edge.py" --label "$LABEL" --consumer "$CONSUMER" \
     --stt-url "${STT_URL:-http://10.10.100.102:8008}" --outdir "$OUTDIR")
[[ $keep_music -eq 1 ]] && cmd+=(--keep-music --spotify-control "$SPOTIFY")
cmd+=("${extra[@]}")

echo "+ ${cmd[*]}"
"${cmd[@]}" || true

snapshot after
echo "=== done. results: ${OUTDIR#$HERE/}/results-$LABEL.jsonl ==="
