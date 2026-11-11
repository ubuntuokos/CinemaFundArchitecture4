# Smart Cut/Render R1 – WP-10 csomagolási validáció (2026-11-11)

Terv: [`docs/plans/CFA4-SMART-CUT-RENDER-R1.md`](../plans/CFA4-SMART-CUT-RENDER-R1.md), WP-10.

**Kapufeltétel:** a funkció a csomagolt alkalmazásban működik, és nem kell hozzá külön GOPchopot telepíteni.

## Mit ellenőriz a csomagok futásidejű auditja

A [`scripts/release/f5_staged_runtime.py`](../../scripts/release/f5_staged_runtime.py) audit a
[`cfa4-four-formats-hosted.yml`](../../.github/workflows/cfa4-four-formats-hosted.yml) munkafolyamat végén
fut. Mind a négy csomagban a csomagolt `cfa4-cli`-t futtatja:

| Csomag | Hogyan jut a CLI-hez | Smart Cut próba | GOPchop-mentesség |
|---|---|---|---|
| tar.gz | `install.sh --prefix` ideiglenes prefixbe | ✅ | fájlnevek |
| .deb | `dpkg-deb --extract` | ✅ | fájlnevek + Depends/Recommends/Suggests |
| AppImage | `--appimage-extract` | ✅ | fájlnevek |
| Flatpak | privát OSTree repóba importálás, majd `ostree checkout --user-mode` | ✅ (a gazdán, nem a sandboxban) | fájlnevek |

### A Smart Cut próba

1. A fixture-t FFmpeg készíti (CFA4 FFmpeg 9.0.2, `CFA4_FFMPEG_BUILD/bin/ffmpeg`; validációs eszköz, nem a végrehajtás része):
   - 4 s MPEG-2 transport stream, 25 fps;
   - zárt, 10 képes GOP-ok, két B-képpel;
   - MP2 hang.
2. A csomagolt `cfa4-cli run` egy JSON-lines szkriptet hajt végre:
   - `export.analyze`: a forrás `mpeg2-video`;
   - `export.plan` a [10, 50) képtartományra, `gop-boundary` módban: veszteségmentes, 40 másolt kép, 0 újrakódolt;
   - `export.start`: `verified: true`, 40 minta, 1 hangsáv; a fájl mérete egyezik a jelentettel;
   - `export.analyze` a kimeneten: `mpeg2-video`, 40 kép.
3. Bármelyik eltérés `AuditError`, és a munkafolyamat elbukik.

A végrehajtás végig CFA4 saját Rust kódja (`cfa4-engine`, `cfa4-export`, `cfa4-mpegts`, `cfa4-smart-edit`). Nem használ GOPchopot, és FFmpeg-et sem linkel (owner-döntés, 2026-10-27).

## Helyi bizonyíték

- A `smart_cut_smoke` a `main`-ből (f948beac) épített `cfa4-cli`-vel lefutott: 40 másolt kép, 1 hangsáv, 113 740 bájt, `verified: true`.
- A `test_f5_staging.py` szintetikus tesztjei lefedik:
  - az elfogadást;
  - az öt elutasítási ágat és az eltérő fájlméretet;
  - a GOPchop-fájlok elutasítását.

## Amit ez nem bizonyít

- Grafikus indítást (AppRun, `flatpak run`) és rendszerszintű `.deb`-telepítést.
- Képkocka-pontos MPEG-2 vágást a csomagban: a határ-újrakódolás motor-bekötése a WP-09e után kerül be, akkor a próba `frame-accurate` lépéssel bővül.
- Hardveres kódolókat (WP-09 jelentés, 24. eset, 🟡).
