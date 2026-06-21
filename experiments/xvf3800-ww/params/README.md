# XVF3800 parameter snapshots

Backups of the ReSpeaker XVF3800 control-parameter state, captured before any
NLAEC / FAR_EXTGAIN tuning so the live tuning can always be restored.

Device: build `inthost-lr48-sqr-i2c`, firmware `VERSION 1 0 7`, serial
`114993700253200099`, control over I2C (`xvf_host -u i2c`).

## Files

| File | What it is |
| --- | --- |
| `params-<TS>.dump.txt` | Full `xvf_host -u i2c -d` value dump (NUL padding from char fields stripped). |
| `params-<TS>.caps.txt` | Full `xvf_host -u i2c -l` capability list (READ/WRITE vs READ ONLY, types, ranges). |
| `restore-<TS>.sh` | Re-applies only the READ/WRITE params from the dump. |

Current snapshot: `20260621-215635`.

## Important: this state is RAM-only

`SAVE_CONFIGURATION` has **not** been run, so the live tuning exists only in the
device's RAM. A power-cycle reverts the device to whatever is in flash (defaults
or the last saved config). These files are therefore the only record of the
current tuning.

## Restore

Run on the Pi from the dir containing `xvf_host`:

```bash
cd ~/reSpeaker_XVF3800_USB_4MIC_ARRAY/host_control/rpi_64bit
bash /path/to/restore-20260621-215635.sh
```

or over SSH from elsewhere:

```bash
ssh freskog@mycroft.local \
  'cd ~/reSpeaker_XVF3800_USB_4MIC_ARRAY/host_control/rpi_64bit && bash -s' \
  < restore-20260621-215635.sh
```

The restore script sets RAM only and deliberately does **not** call
`SAVE_CONFIGURATION`. Only save to flash after a change has been validated.

## Notes on the restore transforms

- `AUDIO_MGR_OP_*`: the dump prints symbolic mux names (`MUX_USER_CHOSEN_CHANNELS[8] 0`);
  these are rewritten to numeric `<cat> <src>`. The whole routing is restored via
  `AUDIO_MGR_OP_ALL 8 0 1 0 1 2 1 0 1 1 1 3` (left = final processed beam `8 0`,
  right = raw mic 0 `1 0`), which supersedes the individual `OP_L/OP_R/PK` commands.
- `AEC_FIXEDBEAMS{AZIMUTH,ELEVATION}_VALUES`: `(NN deg)` annotations stripped.
- `USB_BIT_DEPTH` excluded: READ/WRITE but UA-only (ignored on this INT device).
- READ ONLY values (idle times, energies, `AEC_RT60`, `AEC_AECCONVERGED`, build
  strings, PLL, GPI/GPO reads, buffer-stable) are not restorable and are omitted.
