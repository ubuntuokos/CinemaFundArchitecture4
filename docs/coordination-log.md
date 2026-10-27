# CFA4 – beolvasztási napló

**Aktuális gazda:** Claude (claude.ai munkamenet, a tulajdonos indította 2026-10-27 11:21-kor).

Szabályok: [`coordination.md`](coordination.md). Minden kör egy sor, a legújabb felül.

| Dátum | Gazda | Mi történt |
|---|---|---|
| 2026-10-27 | Claude | A #271 (kodekes gyűjtő-PR, a tulajdonos kivételével) 14:35 UTC-kor beolvadt `ubuntuokos` névvel, nem a gazda által, zöld teljes CI nélkül a beolvasztott fején (a megelőző 4 teljes futás az FFmpeg oracle-teszteken bukott; az utolsó, `3cd7b5f`-en még futott). 20 PR lett „merged” (köztük a mellékágba olvasztott #105, #112, #114, #118, #128, #160). Előtte a #287 is beolvadt. A `main` új feje `514421c`; az ágak törlődtek. |
| 2026-10-27 | Claude | Issue-felmérés: 110 nyitott issue csoportosítva. A tulajdonos jóváhagyásával lezárva 4, már a `main`-ben megoldott issue (#93 ← #168, #94 ← #181, #11 és #16 ← #17), mindegyik megjegyzéssel. |
| 2026-10-27 | Claude | A #134-et (DTS-dekóder) a tulajdonos olvasztotta be 10:28 UTC-kor; teljes CI nem futott rajta, a következő teljes CI-futás a `main`-en lefedi. Az ága automatikusan törlődött, a #256 alapága magától `main` lett. |
| 2026-10-27 | Claude | Ágtakarítás: a tulajdonos törölte a gazda által ellenőrzött 54, teljesen a `main`-ben lévő ágat (`torles.sh`); utóellenőrzés: egy sem maradt, új beolvasztott ág nincs, 75 ág maradt. |
| 2026-10-27 | Claude | 1. kör, felmérés (csak olvasás, `main` @ `48a4ab7`): 47 nyitott PR – 18 aktív, 14 beolvasztható, 14 ütközik vagy újraalapozás kell, 1 kiváltott (#46 → #206). A beolvasztás után 08:50 UTC óta törlődik az ág. Sorrendjavaslat a tulajdonosnál. |
| 2026-10-27 | Claude | Gazda kijelölve; állapotfelmérés indul (csak olvasás), a sorrendjavaslat a tulajdonos jóváhagyására vár. |
| 2026-10-27 | – | A szabályok elfogadva. Kiinduló állapot: 43 nyitott PR, 126 ág, ebből 54 ág már teljesen benne van a `main`-ben. |
