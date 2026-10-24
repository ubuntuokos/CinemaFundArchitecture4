# Third-party source code reused in CFA4

CFA4 is licensed under GPL-3.0-or-later. Outside the codec and container code (which stays
clean-room), source code from other projects may be reused when its licence is GPL-3.0-compatible
([AGENTS.md](AGENTS.md) §2, [ADR 0003](docs/adr/ADR-0003-gpl3-licence.md)).

Every reuse gets one row here, and the files that contain it keep the original copyright and licence
notices plus a line naming the source project, its URL and the version or commit.

Allowed licences: GPL-3.0(-or-later), GPL-2.0-or-later, LGPL-2.1-or-later, LGPL-3.0, MPL-2.0, and
permissive licences (MIT, BSD, Apache-2.0, ISC, Zlib…). Not allowed: GPL-2.0-only, AGPL, licences
with extra restrictions, unclear or missing licences.

| Source project | Files or component taken | Licence | Version / commit | Where in CFA4 | Added |
|---|---|---|---|---|---|

No third-party source code has been reused yet. (Assets such as icons, images and fonts are listed in
[ATTRIBUTION.md](ATTRIBUTION.md); Cargo and Python dependencies are checked by `deny.toml` and the
dependency audit, not here.)
