# BacklotHub G2-13 – G2-26: összesítő jelentés (2026-11-21)

Állapot: mind a 14 javaslat megírva, **jóváhagyásra vár**. Új döntési kérdés a #58 óta nincs.

## Fő megállapítások

1. **G2-01 naplókivonat**: a naplóban tárolt kivonat sózatlan volt. Mostantól sózott, plusz törlőparancs (SEC-06).
2. **G2-20 §3.1 javítva**: az INT/EXT és a napszak a *jelenet* tulajdonsága, nem a díszleté (`scene_meta`).
3. **Pénznemlista és árfolyam-szolgáltatás (#15, #29)** hivatkozott volt, de sosem lett megtervezve. A G2-24 §2.1 és §3 definiálja. A pénznemlista a G2-02 migrációjába kerül, előfeltétel a 14., 19. és 21. lépcsőhöz.
4. **`cfa4-qc` megvan a `main`-ben** (01ff4e41), így a G1 feltétel a 25. lépcsőre fájlalapú ellenőrzésre teljesül. Korlát: csak MP4/MOV, a `qc.run` a fájl mellé írna, ezért a G2-25 a `cfa4-cli qc --out` utat használja.
5. **Új File Services kiterjesztések** (az ADR 0009 keretén belül): `files.export`, `extra_hashes`, `restore` (FS-3), később `files.relocate`.

## Eltérések a G1-től
- A helyszín és az eszköz nem `resource`.
- M1-ben nincs térkép.
- Munkajogi szabályt nem értékelünk.
- Nincs `.xlsx` M1-ben.
- Archívum: aktív fájl felszabadítása és újraimportálás = 26-2.
- Delivery: DCP/IMF = 25-2. Auto-ütemezés (OR-Tools) és scenario = 23-2.

## Kérések a koordinátornak (más projektek munkája)
- `qc.run` kimeneti útvonal paraméter.
- WAV hangosság-ellenőrző parancs.
- DCP-validate motorparancs (a `dcp-validate` ma csak könyvtár).

## Új függőségek a G3-hoz
`holidays`, `icalendar`, pyHanko, OR-Tools (később). `bagit-python` csak tesztorákulum-jelölt. Más új függőség nincs.

## Dokumentumok
G2-13…G2-26 a `/mnt/project-files/backlothub/` mappában, a G1 összefoglaló táblázata soronként hivatkozza őket. A közös eseménylista a G2-12 §13.

## Függőben
V1 proxy-mérés: a CFA4-MIN gépen, István által egyeztetett időben (a 8. lépcső G4 kapujához kell).
