# CFA4 – beolvasztási napló

**Aktuális gazda:** Claude (claude.ai munkamenet, a tulajdonos indította 2026-10-31 14:21-kor). Az előző gazdát (2026-10-27 11:21) a tulajdonos 2026-10-31-én archiválta.

Szabályok: [`coordination.md`](coordination.md). Minden kör egy sor, a legújabb felül.

| Dátum | Gazda | Mi történt |
|---|---|---|
| 2026-10-30 | Claude | A #346 átadása szerint, a tulajdonos kérésére egyenként beolvasztva, mindegyik zöld teljes CI-vel a beolvasztott fejen és „Beolvasztható” megjegyzéssel: **#344** (`b715c5d`, run 41093466803) → **#338** (`0541a4a`, run 41097629670) → **#342** (`854277d`, run 41103823324). A `main` feje `f8d0f77`; a #344 előtti `main` teljes CI-je (run 41081470525) csak a #344 által javított Python-jobon bukott. Mindhárom ág automatikusan törlődött. |
| 2026-10-30 | Claude | **#334** (EGM tervezési-átnézési leírás, csak dokumentáció): teljes CI zöld a `82c71de` fejen, mind a 9 job (run 40824027165). 2026-10-29 23:52 UTC-kor `ubuntuokos` névvel beolvadt ezen a fejen, nem a gazda által; az ága törlődött. Utána beolvadt még a #335 és a #330; a `main` feje `973a73e`. |
| 2026-10-27 | Claude | A `main` @ `24ba4f7` teljes CI-je zöld, mind a 9 job (run 40576600633). Esti ágellenőrzés beállítva (ütemezett feladat, naponta 20:40, csak olvas). |
| 2026-10-27 | Claude | A tulajdonos jóváhagyásával beolvasztva: **#301** (`e6643d6`, teljes CI zöld: run 40538958767; „Beolvasztható” megjegyzés a PR-ban) → `main` @ `24ba4f7`; az ága automatikusan törlődött. Lezárva: **#303**, kiváltotta a `96ae630` (#271); az ága (`claude/fix-logo-attribution`) megmaradt, törölhető. Teljes CI indítva a `main`-en (run 40576600633). Tulajdonosi döntés: fölösleges ágat csak a gazda törölhet, és a gazda erről értesítést kap. |
| 2026-10-27 | Claude | Felmérés (`main` @ `16debf9`, 54 nyitott PR): a `main` teljes CI-je piros (rustfmt és 11 hiányzó magyar Qt-felirat a #272-ből, egy HEIF oracle-teszt). A #301 mindhármat javítja: naprakész a `main`-nel, teljes CI zöld a fején (run 40538958767) – beolvasztható, a tulajdonos jóváhagyására vár. A #303-at a `96ae630` (#271) kiváltotta. |
| 2026-10-27 | Claude | A tulajdonos szerint ő csak adatot (mintákat) adott a kodekes munkamenetnek, nem olvasztott be: a #134, #271, #272, #258, #256 és #260 beolvasztását a kodekes munkamenet végezte, a `coordination.md` 1. pontja ellenére. |
| 2026-10-27 | Claude | A #271 utolsó teljes futása (`3cd7b5f`) is az FFmpeg oracle-teszteken bukott, a `main` valószínűleg piros. 14:43–15:15 UTC között a gazdán kívül beolvadt: #272, #258, #256, #260 (`ubuntuokos` névvel, csak gyors CI-vel). A `main` feje `d91aaf1`. |
| 2026-10-27 | Claude | A #271 (kodekes gyűjtő-PR, a tulajdonos kivételével) 14:35 UTC-kor beolvadt `ubuntuokos` névvel, nem a gazda által, zöld teljes CI nélkül a beolvasztott fején (a megelőző 4 teljes futás az FFmpeg oracle-teszteken bukott; az utolsó, `3cd7b5f`-en még futott). 20 PR lett „merged” (köztük a mellékágba olvasztott #105, #112, #114, #118, #128, #160). Előtte a #287 is beolvadt. A `main` új feje `514421c`; az ágak törlődtek. |
| 2026-10-27 | Claude | Issue-felmérés: 110 nyitott issue csoportosítva. A tulajdonos jóváhagyásával lezárva 4, már a `main`-ben megoldott issue (#93 ← #168, #94 ← #181, #11 és #16 ← #17), mindegyik megjegyzéssel. |
| 2026-10-27 | Claude | A #134-et (DTS-dekóder) 10:28 UTC-kor `ubuntuokos` névvel olvasztották be (helyesbítés: nem a tulajdonos, hanem a kodekes munkamenet; a tulajdonos csak adatot adott); teljes CI nem futott rajta, a következő teljes CI-futás a `main`-en lefedi. Az ága automatikusan törlődött, a #256 alapága magától `main` lett. |
| 2026-10-27 | Claude | Ágtakarítás: a tulajdonos törölte a gazda által ellenőrzött 54, teljesen a `main`-ben lévő ágat (`torles.sh`); utóellenőrzés: egy sem maradt, új beolvasztott ág nincs, 75 ág maradt. |
| 2026-10-27 | Claude | 1. kör, felmérés (csak olvasás, `main` @ `48a4ab7`): 47 nyitott PR – 18 aktív, 14 beolvasztható, 14 ütközik vagy újraalapozás kell, 1 kiváltott (#46 → #206). A beolvasztás után 08:50 UTC óta törlődik az ág. Sorrendjavaslat a tulajdonosnál. |
| 2026-10-27 | Claude | Gazda kijelölve; állapotfelmérés indul (csak olvasás), a sorrendjavaslat a tulajdonos jóváhagyására vár. |
| 2026-10-27 | – | A szabályok elfogadva. Kiinduló állapot: 43 nyitott PR, 126 ág, ebből 54 ág már teljesen benne van a `main`-ben. |
