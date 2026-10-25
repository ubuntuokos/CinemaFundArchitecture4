# CFA4 F9 – Evidence-based release decision, 2026-10-23

**VERDICT: HOLD**. No CFA4 package publication, GitHub release, auto-merge,
APT repository or update rollout is authorized.

## Source and integration evidence

- [PR #23 full Linux AMD64 CI](https://github.com/ubuntuokos/CinemaFundArchitecture4/actions/runs/39963340491):
  **8/8 PASS**. Combined #17 native-media and #20 Cargo-detachment sources.
  This is an integration CI gate, **not** a merge into main.
- F2 source scope [#39967631893](https://github.com/ubuntuokos/CinemaFundArchitecture4/actions/runs/39967631893): PASS.
- F3 CFA4 release-chain policy [#39968833901](https://github.com/ubuntuokos/CinemaFundArchitecture4/actions/runs/39968833901): PASS, real build SKIPPED.
- F4 package validation source [#39970768868](https://github.com/ubuntuokos/CinemaFundArchitecture4/actions/runs/39970768868): PASS, four real packages still missing.
- F5 isolated staging source [#39972322877](https://github.com/ubuntuokos/CinemaFundArchitecture4/actions/runs/39972322877): PASS, no actual installed package.
- F6 XDG source [#39972851980](https://github.com/ubuntuokos/CinemaFundArchitecture4/actions/runs/39972851980): PASS, desktop runtime OPEN.
- F7 timestamps and capability truth [#39974409658](https://github.com/ubuntuokos/CinemaFundArchitecture4/actions/runs/39974409658): PASS, actual A/V measurement OPEN.
- F8 Qt6/Rust bridge source [#39975032470](https://github.com/ubuntuokos/CinemaFundArchitecture4/actions/runs/39975032470): PASS, full Qt6 editor OPEN.
- 2026-10-22 physical NVIDIA RTX A1000 hardware H264/HEVC acceptance retained
  as a separate PASS. No repeated user GUI/physical test.

## Blocking requirements

The exact machine-readable gate statuses and evidence are stored in
\`docs/audit/f9-release-readiness.json\`. The following remain blockers:
four actual CFA4 native outputs (.deb, AppImage, Flatpak, tar.gz) and same
commit SHA256SUMS; clean Ubuntu 26.04 AMD64 installation/removal and
KDE/GNOME/Cinnamon runtime; direct/dynamically loaded dependency audits;
AV1 native NVDEC; HEIF ICC/HDR/multi-picture; real content A/V precision;
full Qt6 NLE parity; all trademarks/third-party SBOM/codecs/Qt redistribution
approvals; and verified protected-main integration of the draft PR stack.

The asset/logo release gate remains OPEN until the user's accepted CFA4 icon
is verified across every inherited upstream binary/asset. A source SVG
with CFA4 identification alone is not adequate release evidence.

## Release decision mechanism

The offline validator \`python -m cfa4_devtools.release_gate\` confirms that
the registered gate set and explicit HOLD verdict are internally consistent.
For an intentional fails-closed release precondition:

\`\`\`sh
PYTHONPATH=python python3 -m cfa4_devtools.release_gate --require-ready
\`\`\`

**It must fail while any mandatory gate remains OPEN**. If every future
gate becomes PASS, it returns \`REQUIRES_EXPLICIT_FINAL_APPROVAL\`, not
automatic release approval. The checker cannot verify evidence URL
authenticity or replace human legal/security/acceptance sign-off; the release
authorization flag is hard-coded false.

## Preserved architecture

The upstream project is the read-only parent. CFA4 is developed separately,
Linux Ubuntu 26.04 AMD64 only, Qt6 future GUI, Rust media engine retained,
local AI only, no public packages or unnecessary repeat hardware tests.
