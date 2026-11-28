# HRTF sets

Head-related transfer function sets for the binaural (headphone) monitor
(`docs/plans/CFA4-BINAURAL-MONITOR-001.md`). Each set has a `.attribution` sidecar and a row in
[`../../ATTRIBUTION.md`](../../ATTRIBUTION.md).

| File | What | Licence | SHA-256 |
|---|---|---|---|
| `sadie2-ku100-48k.maudhrtf` | SADIE II, subject D1 (Neumann KU100), 48 kHz; the built-in default set | Apache-2.0 (`LICENSE-Apache-2.0-SADIE-II.txt`, `sadie2-ku100.notice.txt`) | `078fa52db8d39fd5bff5875ce888f9d3dcd24b06f9ecf3beaf6b7e7325784d41` |

## Format

`.maudhrtf` version 3, as specified by Maul Audio (MIT,
<https://github.com/Rawframe-Project/maul-audio>, `docs/internals/file-formats.md`): minimum-phase
int16 responses (128 taps at 48 kHz) with each ear's onset delay kept apart in 1/256 sample, on
elevation rings 5 degrees apart with evenly spaced azimuths, a CRC-32 over the payload, and the
dataset's licence text embedded. CFA4 reads it with its own Rust reader (`cfa4-hrtf`).

## Reproducing the file

The file is Maul Audio's, taken unchanged. To rebuild it: download subject D1's 48 kHz SOFA file
from the SADIE II record (Zenodo 12092466) and run Maul Audio's
`tools/sofa_to_maudhrtf.py <D1 48 kHz SOFA file> out.maudhrtf --license-file
sadie2-ku100.notice.txt` (numpy and h5py) at the commit above; the output is deterministic. This has
not been re-run in CFA4 yet (the build sandbox could not reach Zenodo); B-09 records the result.
