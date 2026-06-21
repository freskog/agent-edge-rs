#!/usr/bin/env bash
#
# XVF3800 parameter restore script
# Generated from: params-20260621-215635.dump.txt (+ .caps.txt)
# Device: ReSpeaker XVF3800, build "inthost-lr48-sqr-i2c" (firmware VERSION 1 0 7),
#         serial 114993700253200099, control via I2C.
#
# Re-applies ONLY the READ/WRITE parameters captured in the snapshot, so a
# power-cycle (or CLEAR_CONFIGURATION) that reverts the device to flash/defaults
# can be undone. READ ONLY values (idle times, energies, version, build strings,
# converged/RT60, PLL, buffer-stable, GPI/GPO reads) are intentionally excluded.
#
# Notes on transforms applied vs the raw dump:
#   - AUDIO_MGR_OP_* : symbolic "MUX_xxx[<cat>] <src>" rewritten to numeric
#     "<cat> <src>". The full routing is restored via AUDIO_MGR_OP_ALL (which
#     supersedes the individual OP_L/OP_R/PK commands); equivalents shown in
#     comments.
#   - AEC_FIXEDBEAMS{AZIMUTH,ELEVATION}_VALUES : "(NN deg)" annotations stripped,
#     radian values kept.
#   - USB_BIT_DEPTH excluded: READ/WRITE but UA-only; a set is ignored on this
#     INT-device, and writing it could reboot a UA device.
#
# Usage (on the Pi, from the host_control dir that contains xvf_host):
#   cd ~/reSpeaker_XVF3800_USB_4MIC_ARRAY/host_control/rpi_64bit
#   XVF_HOST=./xvf_host bash /path/to/restore-20260621-215635.sh
# or over SSH:
#   ssh freskog@mycroft.local 'cd ~/reSpeaker_XVF3800_USB_4MIC_ARRAY/host_control/rpi_64bit && bash -s' < restore-20260621-215635.sh
#
# This script does NOT call SAVE_CONFIGURATION (see the very end). It only sets
# RAM state, exactly like the snapshot, which itself was never saved to flash.

set -euo pipefail

XVF="${XVF_HOST:-./xvf_host} -u i2c"

set_param() {
  echo "+ $XVF $*"
  $XVF "$@"
}

# ---- AEC ------------------------------------------------------------------
set_param SHF_BYPASS 0
set_param AEC_FIXEDBEAMSAZIMUTH_VALUES 0.00000 0.00000
set_param AEC_FIXEDBEAMSELEVATION_VALUES 0.00000 0.00000
set_param AEC_FIXEDBEAMSGATING 0
set_param AEC_HPFONOFF 2
set_param AEC_AECSILENCELEVEL 0.0000000 0.0000010
set_param AEC_AECEMPHASISONOFF 1
set_param AEC_FAR_EXTGAIN 0.0000000
set_param AEC_PCD_COUPLINGI -1.0000000     # outside [0..1] => PCD disabled (intentional)
set_param AEC_PCD_MINTHR 0.0050000
set_param AEC_PCD_MAXTHR 0.1000000
set_param AEC_ASROUTONOFF 1
set_param AEC_ASROUTGAIN 1.0000000
set_param AEC_FIXEDBEAMSONOFF 0
set_param AEC_FIXEDBEAMNOISETHR 0.4000000 0.4000000

# ---- Audio manager / output routing --------------------------------------
set_param AUDIO_MGR_MIC_GAIN 70.0000000
set_param AUDIO_MGR_REF_GAIN 8.0000000
set_param I2S_INPUT_PACKED 0
set_param AUDIO_MGR_SELECTED_CHANNELS 3 3
set_param AUDIO_MGR_OP_PACKED 0 0
set_param AUDIO_MGR_OP_UPSAMPLE 1 1
# OP_ALL = L_PK0 L_PK1 L_PK2 R_PK0 R_PK1 R_PK2 as (cat src) pairs:
#   L: (8 0)(1 0)(1 2)   R: (1 0)(1 1)(1 3)
# Equivalent individual commands (left=final processed beam, right=raw mic 0):
#   AUDIO_MGR_OP_L 8 0  ;  AUDIO_MGR_OP_R 1 0
set_param AUDIO_MGR_OP_ALL 8 0 1 0 1 2 1 0 1 1 1 3
set_param AUDIO_MGR_FAR_END_DSP_ENABLE 0
set_param AUDIO_MGR_SYS_DELAY 12
set_param I2S_DAC_DSP_ENABLE 0

# ---- LED ------------------------------------------------------------------
set_param LED_EFFECT 5
set_param LED_BRIGHTNESS 127
set_param LED_GAMMIFY 1
set_param LED_SPEED 8
set_param LED_COLOR 8256
set_param LED_DOA_COLOR 8256 49254

# ---- Post-processing (PP) -------------------------------------------------
set_param PP_AGCONOFF 0
set_param PP_AGCMAXGAIN 31.9999981
set_param PP_AGCDESIREDLEVEL 0.0045000
set_param PP_AGCGAIN 17.6831455
set_param PP_AGCTIME 0.8999988
set_param PP_AGCFASTTIME 0.1000000
set_param PP_AGCALPHAFASTGAIN 0.0000000
set_param PP_AGCALPHASLOW 0.9840000
set_param PP_AGCALPHAFAST 0.3600000
set_param PP_LIMITONOFF 1
set_param PP_LIMITPLIMIT 0.4700000
set_param PP_MIN_NS 0.1500000
set_param PP_MIN_NN 0.5100000
set_param PP_ECHOONOFF 1
set_param PP_GAMMA_E 1.0000000
set_param PP_GAMMA_ETAIL 1.0000000
set_param PP_GAMMA_ENL 1.1000000
set_param PP_NLATTENONOFF 0
set_param PP_NLAEC_MODE 0
set_param PP_MGSCALE 1000.0000000 1.0000000 1.0000000
set_param PP_FMIN_SPEINDEX 1300.0000000
set_param PP_DTSENSITIVE 15
set_param PP_ATTNS_MODE 0
set_param PP_ATTNS_NOMINAL 1.0000000
set_param PP_ATTNS_SLOPE 1.0000000

echo
echo "Restore complete (RAM only). The original snapshot was NOT saved to flash."
echo "To make this state survive a power-cycle, run manually AFTER validating:"
echo "  $XVF SAVE_CONFIGURATION 1"
