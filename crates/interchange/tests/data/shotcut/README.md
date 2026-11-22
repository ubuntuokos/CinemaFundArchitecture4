# Shotcut-style `.mlt` fixtures (SC-5.0)

Small MLT XML projects for the future Shotcut import (`docs/plans/CFA4-SHOTCUT-CAPABILITY-ADAPTATION-001-SC5-MLT-IMPORT.md`).
`shotcut-26.2.16-single-clip.mlt` is a **real Shotcut 26.2.16 (MLT 7.36.1) project** supplied by the project owner (one video clip on V1, `mix` and `qtblend` transitions, 1280x720 at 30 fps); only the media path (it contained a home directory name) and the caption were replaced, nothing else was edited.
The other two files are **not authored in the Shotcut GUI**: `mix-luma-melt.mlt` was serialised by MLT's own XML consumer
(`melt ... -mix 10 -mixer luma -consumer xml:`, MLT 7.22.0, `meta.*` properties stripped), and
`timeline-shotcut-style.mlt` was written by hand to follow the structure and the `shotcut:*` annotations read
from the Shotcut and MLT sources (see `docs/plans/CFA4-SHOTCUT-CAPABILITY-ADAPTATION-001-SC5-0-REPORT.md`).
Original test material made for CFA4, released as CC0. No Shotcut or MLT file content is copied.
The media they refer to (`a.mp4`, `b.mp4`) is generated, never committed: `scripts/dev/sc5-make-fixtures.sh`.
