# ADR: a render-worker kimeneti indexe és a változó képkockaszám (C0 v0.2)

**Dátum:** 2026-11-25 · **Állapot:** ELFOGADVA (István, 2026-11-25); megvalósítás: `cfa4-render-contracts` 0.2 ·
**Alap:** C0 jelölt-leképezés, M2 döntés (István, 2026-11-25) · **Érintett:** `cfa4-render-contracts` (worker protokoll), `cfa4-tasks` (`RenderProgress`)

## 1. Probléma
A worker-protokoll `Submit { job, frame_index }` és `FrameReady { job, frame_index }` üzenete ma 1:1 bemenet–kimenet viszonyt sugall, a `RenderProgress` összege pedig rögzített. A VapourSynth-szűrők ezt megtörik: az IVTC 5 bemeneti képkockából 4-et ad, a sebességváltó deinterlace (pl. QTGMC 2×) megkétszerezi a képkockaszámot és az időalapot. Az eredmény `Tick`-je hordozni tudja az időt, a protokoll indexelése és a progress összege viszont nem.

## 2. Döntés (javaslat, M2 szerint)
1. **Az index kimeneti index.** `frame_index` mindkét üzenetben a worker **kimeneti** képkockasorszáma. A kliens a kimeneti sorszámot kéri; a worker dönt arról, mely bemeneti képkockákból áll elő.
2. **Arány és időalap a kézfogás után.** A `HelloAck`-ot követő új `Describe { input_per_output: Ratio, output_rate: FrameRate }` üzenet (worker → kliens) közli a bemeneti/kimeneti arányt (racionális, pl. 5/4, 1/2) és a kimeneti képkockasebességet. Állandó arányú szűrőnél ez egyszer, változónál az arány nélküli módban (`input_per_output: None`) a kimenet ismeretlen hosszúságú, a lezárást a `Finished { job, frames }` jelzi.
3. **Progress összege változhat.** `RenderProgress.total` becslés lehet, és előre vagy hátra módosulhat; a kliens ezt nem hibaként kezeli. A `RenderEvent::is_consistent()` változatlan marad: mai szövege nem is vizsgálja az összeget, és `RenderProgress::fraction()` már korlátoz; a szerződésteszt (`the_progress_total_may_move_while_a_job_runs`) ezt rögzíti.
4. **Idő.** Minden kimeneti képkocka saját `Tick`-et kap (`FrameSideData`/`MatteFrame` szabálya); csendes CFR-kerekítés nincs. Az időalap változása a `Describe`-ban látszik, a konténer-írás (export) ezt kapja meg.
5. **Visszafelé kompatibilitás.** A nem módosított (1:1) workerek `Describe`-ot nem küldenek: ez `input_per_output = 1/1`, a kimeneti sebesség = a bemeneti. A `ContractVersion` 0.1 → 0.2. A meglévő 1.0 előtti szabály marad (a minornak egyeznie kell), tehát egy 0.1-es worker 0.2-es kliensen elutasított; ez nem jelent veszteséget, mert worker még nem létezik. A 0.2-es, `Describe`-ot nem küldő worker 1:1.
6. **Cancel és retry.** A `Cancel` változatlan. Retry csak a kimeneti index szerint idempotens lépésre; a worker állapotot hordozó szűrője (IVTC-minta) az újraindításnál a ciklus elejétől számol, ezért a kliens a kimeneti indexet a ciklushatárra igazítja (a `Describe` ezt a `restart_alignment` mezőben közli).

## 3. Mit nem érint
Az 1:1 workerek (OpenImageIO, EC-jelölt, GEGL+babl) viselkedését; a `VideoFrame` mezőit; a konverziós mátrixot; a bináris `MaskRef`-et. A többcsatornás (AOV) képek a v0-ban `Unsupported`; külön ADR az F4 előtt.

## 4. Szerződésteszt-bővítés (a jóváhagyás után)
(1) `Describe` JSON körút; (2) a kimeneti index 1:1, 5→4 és 1→2 aránynál a várt bemeneti tartományt adja; (3) a progress összege futás közben módosulhat, lezárva egyezik; (4) `Describe::one_to_one` a Describe nélküli worker leírása; (5) a verzió a meglévő handshake-szabály szerint egyeztetődik; (6) hostile input: nulla arány, negatív sebesség, túlcsordulás: hiba, nem pánik. A tesztanyag procedurálisan generált, számok szintetikusak.

## 5. Mit nem ellenőriztem
A VapourSynth tényleges képkocka-tulajdonságait (`_DurationNum/_DurationDen`, `_AbsoluteTime`) forrásból nem olvastam; a `restart_alignment` indokoltsága az IVTC-szűrő működéséről szóló általános ismeret, F2-ben igazolandó. 

## 6. Megvalósítás
`worker.rs`: `Describe`, `Ratio`, `DescribeError`, `WorkerMessage::{Describe, Finished}`, `first_input`, `output_count`, `restart_from`; szerződésteszt: kimeneti index 1:1, 5→4 és 1→2 aránynál, újraindítás a ciklushatáron, ismeretlen hossz, hostile input (nulla arány, rossz sebesség, túlcsordulás), JSON körút, mozgó progress-összeg.
