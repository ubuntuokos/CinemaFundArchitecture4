# CFA4 – R0 újraalapozás: Ubuntu 26.04 LTS

Nyomon követés: https://github.com/ubuntuokos/CinemaFundArchitecture4/issues/11
Referencia main SHA: `cdf43fd366e378da266cb6d3c94549a45244be1f`. A main változatlan.

## Mért kiindulási állapot (2026-10-21)
- Kötelező célplatform: **Ubuntu 26.04 LTS**. A régi Ubuntu 24.04 CI PASS eredmények nem igazolják a 26.04 célrendszert.
- A main öt történeti GitHub workflow-fájlt tartalmaz: `freebsd.yml`, `nix.yml`, `packaging-lint.yml`, `release.yml`, `windows-arm64.yml`; Ubuntu 26.04 CI nem található ezen a baseline commiton.
- A release és packaging workflow-k mozgó `storytold/craft-common-actions@main` hivatkozást használnak.
- A projekt `docs/gaps.md` G1–G20 ismert hiányosságot jelöl, ezeket újra kell mérni.

## Már mért Ubuntu 26.04 bizonyíték (részleges)
- PR #8 Actions: https://github.com/ubuntuokos/CinemaFundArchitecture4/actions/runs/39553206897
- PR #10 Actions: https://github.com/ubuntuokos/CinemaFundArchitecture4/actions/runs/39553292230
- A GitHub jobnaplók `Ubuntu 26.04.1 LTS`, `ubuntu-26.04` runner image-et igazolnak.
- A Python/Qt6, architecture/assets, vendored docpath jobból mért PASS; a PR #10 UI teszt 79/79 PASS. Ezek csak az adott nem egyesített PR headekhez köthetők.
- A Python job explicit `actions/setup-python` **3.13**; a 26.04 natív Python 3.14-kompatibilitása még nincs igazolva.
- A teljes Rust workspace, CLI, FFmpeg oracle és valamennyi további auditkapu **külön végleges ellenőrzést igényel**.

## R0-folyamat és kapuk

| Szakasz | Kötelező vizsgálat | R0 állapot |
| --- | --- | --- |
| F0-01 | OS/kernel/toolchain, Python 3.14, Qt6 és FFmpeg/ffprobe APV | RÉSZLEGES |
| F0-02 | `cargo metadata`, `cargo build --workspace --locked`, `cargo xtask ci` | NYITOTT |
| F0-03 | Kötelező release workspace FFmpeg oracle tesztek, skip nélkül | NYITOTT |
| F0-04 | Natív GUI, import, edit, save/open, export, audio/hardver | NEM FUTOTT |
| F0-05 | G1–G20 hiányosságok újramérése | NEM FUTOTT |
| F1-01 | Linux app ID, brand, desktop/AppStream/MIME és csomagolás | NYITOTT |
| F1-02 | Release pipeline Linux-only és mozgó upstream workflow refs | NYITOTT |
| SC-01 | Cargo/Python/Qt/FFmpeg SBOM, transzitív license/hash-pin | NYITOTT |
| SC-02 | Upstream Git-blob SHA, NOTICE és LICENSE forrásazonosság | ÚJRAVIZSGÁLANDÓ |
| Vendor-01 | PR #8 teljes Ubuntu 26.04 CI és audit | NYITOTT |
| Vendor-02 | PR #10 teljes Ubuntu 26.04 CI és audit | NYITOTT |

## Bizonyítás és korlát
Minden új kapuhoz OS, runner image, commit SHA, parancs, workflow/job ID és egyértelmű PASS/FAIL/BLOCKED/NOT_RUN státusz szükséges. Korábbi teszteredmény csak történeti hivatkozás. A két PR draft; nincs automatikus merge, main-módosítás vagy kiadási engedély. Az új audit az F0-01 környezeti leltárral kezdődik.
