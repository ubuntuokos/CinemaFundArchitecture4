# CFA4 control surfaces — M1–M10 measurement record

**Status:** empty. Nothing in this table was measured. Do not treat a blank cell as a pass.
Fill it on the reference machine (Ubuntu Studio 26.04, see `docs/development/reference-machine.md`)
with a Contour ShuttlePRO or ShuttleXpress plugged in. The contract is
`docs/plans/CFA4-CONTROL-SURFACE-HID-EVALUATION.md` §4. If M4 does not show position 0, that is an
ADR change, not a guess.

## Header (copy the command output; do not invent it)

| Item | Command | Measured |
|---|---|---|
| Kernel | `uname -r` | |
| Image package | `dpkg -l linux-image-$(uname -r)` | |
| Device | `lsusb -v -d 0b33:` (excerpt) | |
| CPU | `lscpu` | |
| Desktop | KDE Plasma version | |
| Audio | PipeWire version | |

## Steps

| Step | Command / action | Expected | Measured | Difference |
|---|---|---|---|---|
| M1 Binding | `ls -l /dev/hidraw* /dev/input/by-id/*Contour*`; `cat /sys/class/hidraw/hidraw*/device/uevent` | A hidraw node and an event node; `HID_ID` 0B33:0030 (or 0020, 0010) | | |
| M2 Permission | `getfacl /dev/hidrawN` without the udev rule and with `TAG+="uaccess"` | Unreadable without the rule; the logged-in user can read it with the rule; other users cannot | | |
| M3 Report descriptor | `xxd /sys/class/hidraw/hidrawN/device/report_descriptor` | Field widths match HID evaluation §3 | | |
| M4 Shuttle | `xxd -c5 /dev/hidrawN` while turning, including the return to 0 | Every position, and 0, appears | | |
| M5 Jog | same, slow and fast, including the 255→0 wrap | The counter wraps; no skipped count that the decoder cannot explain | | |
| M6 Buttons | each button alone and together | Press and release both appear | | |
| M7 evdev | `evtest /dev/input/eventN` | Whether the centre position is missing | | |
| M8 Hot-plug | unplug and plug back while CFA4 is reading; `inotifywait -m /dev` | `IN_DELETE` / `IN_CREATE`; the read ends; open again works; no button stays down | | |
| M9 Desktop side effect | buttons under KDE with CFA4 not running | No unwanted action | | |
| M10 Load | `pidstat` for 60 s of turning (and a fader storm if a MIDI controller is attached) | CPU cost recorded, not assumed | | |

Queue capacity stays 256 until this record, or a later one, measures it (N-08). p95 from an L5
timestamp to `surfaceAction` is not measured here either.
