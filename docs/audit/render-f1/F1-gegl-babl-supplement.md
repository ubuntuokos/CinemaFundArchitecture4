# F1 kiegészítő audit: GEGL és babl (CFA4-RENDER-INTEGRATION-PLAN-001)

**Dátum:** 2026-11-23 · **Jelleg:** csak olvasás (GitHub keresési API metaadat); sem build, sem telepítés, sem kódátvétel.
**Ok:** a tulajdonos 2026-11-23-án az F2 POC-jelöltek közé vette a GEGL-t és a babl-t (terv rev 117; S1 5. szabály: 4 jelölt). A terv F1 kiegészítést kér:
licenc, karbantartók és aktivitás, host-lábnyom (GLib/GObject), sandbox-profil, Python-igény.
**Bizonyítási szabály:** ami nincs itt mérve vagy forrásból igazolva, az PENDING; "általános ismeret" nem PASS.

## 1. Eredmény

| Szempont | Eredmény | Alap |
|---|---|---|
| Licenc | **PASS-feltétellel**: LGPL-3.0 mindkettőnél (a GitHub-tükör metaadata); a CFA4 GPL-3.0-or-later, a kettő összefér | metaadat; a fájlszintű SPDX és a tranzitív függőségek licencei **PENDING** (A9) |
| Karbantartók | **PASS**: ≥ 2 aktív szerző mindkettőnél (életképességi küszöb: ≥ 2) | 2. pont |
| Aktivitás | **PASS**: mindkét tárolóban van commit az elmúlt 12 hónapban, nincs archiválva | 2. pont |
| Kiadás 12 hónapon belül | **PENDING**: a kiadási címkék dátuma ebből a forrásból nem kérdezhető le | az F2 elején a címkék ellenőrzendők |
| Host-lábnyom (GLib/GObject) | **PENDING, kockázat**: a GEGL a GLib/GObject-re épül (általános ismeret); ez új futásidejű függőség | 3. pont |
| Sandbox-profil | **PENDING**: csak külön worker folyamatban, `renderWorker` profil (még nincs jóváhagyva) | 4. pont |
| Python-igény | **PASS (nem várt)**: a használat C könyvtárként vagy külön worker CLI-ként megy; Python-kötés nem kell | 5. pont |
| **Összesített döntés** | **PASS F2-POC-jelöltként, külön workerben, feltételekkel** (licenc-fájlaudit, lábnyom és sandbox PENDING) | |

BLOCKED nincs: egyik PENDING tétel sem zárja ki a jelöltet, de az F2 kezdete előtt a 6. pont feltételei zárandók.

## 2. Életképesség (GitHub-tükör, `GNOME/gegl`, `GNOME/babl`)

Az adatok a GitHub keresési API-jából valók (commit-szerzők 2025-11-23 óta). A "karbantartó" itt a commit-szerző, nem a jogosultság;
a bot-fiókokat és az egyazon személy több azonosítóját nem vontam össze, ezért a szerzőszám felső becslés.

| | GEGL | babl |
|---|---|---|
| Licenc (metaadat) | LGPL-3.0 | LGPL-3.0 |
| Utolsó push | 2026-11-21 | 2026-10-24 |
| Archivált | nem | nem |
| Commit 12 hónapon belül | 385 | 133 |
| Különböző szerző (az első 100 commitból) | 20 | 6 |
| Legaktívabb szerzők | Ondřej Míchal (38), Bruno Lopes (30), Øyvind Kolås (7), Jehan (3) | Bruno Lopes (71, két azonosító), Øyvind Kolås (19), Jehan (5) |

Megjegyzés: a GEGL 385 commitjából csak az első 100 szerzői listája érkezett meg (a keresési API oldalhatára); az eloszlás ezért a legutóbbi időszakot tükrözi.
A babl commitjainak zöme egy szerzőtől jön (Bruno Lopes), de legalább három további szerző is hozzájárult; az "egyszemélyes" kockázat (ReelForge, avio, rust-motion esete) itt nem áll fenn.

## 3. Host-lábnyom

A GEGL a babl-ra, a GLib/GObject-re (GIO, JSON-GLib) és számos választható képkodek-könyvtárra épül (általános ismeret, **nincs ebben a körben forrásból igazolva**).
CFA4 oldalról ez három kérdés, mind PENDING:

1. Mely GLib/GObject-elemek kellenek a render-workerhez (a minimum-konfiguráció `meson` opciói), és mi marad ki?
2. Az A6 (host-lábnyom) szerinti mérés: a worker memóriája, indulási ideje, szálkezelése (a GLib főhurok és a CFA4 szálmodellje nem keveredhet).
3. Az `apt` csomagok (`libgegl-dev`, `libbabl-dev`) verziója az Ubuntu 26.04-en és a forrásból építés közti választás (A9, A12 szerint a meglévő rendszercsomag előnyben).

A lábnyom miatt a **linkelés a CFA4 fő folyamatába nem javasolt**: a GEGL csak külön worker folyamatban fut (az F1 sandbox-döntés és a VapourSynth-eset mintájára).

## 4. Sandbox

Alap: `crates/sandbox` (Landlock + seccomp; profilfajták `mediaHost`, `pluginHost`; az F1 javasolja: `renderWorker`, `externalRender`).
A GEGL-worker a `renderWorker` profilt kapná: csak olvasható bemeneti fa, írható kimeneti és ideiglenes könyvtár, hálózat nélkül.
**PENDING:** a profil jóváhagyása (SECURITY-001 tulajdonosa), valamint annak mérése, hogy a GEGL kodek-bővítményei (dinamikusan betöltött modulok) a Landlock-szabályok alatt működnek-e.
A GEGL-nek a dinamikus operáció-modulok (`.so`) betöltése miatt olvasható modulútvonal kell; ezt a profilnak szűken kell megengednie.

## 5. Python-igény és A11

A GEGL használata nem igényel Python-értelmezőt (GObject Introspection kötés van, de a worker C API-n vagy parancssoron át megy). Új Python-verzió ezért nem kerül be; A11 (a lehető legkevesebb Python-verzió) érintetlen.

## 6. Feltételek az F2 kezdete előtt

| # | Feltétel | Állapot |
|---|---|---|
| 1 | Fájlszintű licenc- és SPDX-ellenőrzés a kiadási tarballon, tranzitív függőségek, SBOM (A9) | PENDING |
| 2 | Kiadási címkék és dátumok ellenőrzése (12 hónapon belüli kiadás) | PENDING |
| 3 | Minimum-konfiguráció és a GLib/GObject-lábnyom mérése | PENDING |
| 4 | `renderWorker` sandbox-profil jóváhagyása és a modulbetöltés mérése | PENDING |
| 5 | Az F2-hipotézisek és küszöbök (`F2-preregistration.md`) elfogadása | PENDING |

Ha az 1. feltétel nem teljesül (pl. GPL-inkompatibilis tranzitív komponens), a jelölt BLOCKED lesz, és kiesik (a terv szabálya).
