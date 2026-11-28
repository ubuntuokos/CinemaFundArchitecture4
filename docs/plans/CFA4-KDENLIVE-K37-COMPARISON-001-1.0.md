# CFA4-KDENLIVE-K37-COMPARISON-001 — a Kdenlive-forkok eszközlistája a parancsregiszterrel szemben

- **Állapot:** Piszkozat (KDN-37, összevetés; kódot nem érint)
- **Dátum:** 2026-11-23
- **Alap:** `main@e493e956`
- **Forrás:** `docs/plans/CFA4-KDENLIVE-EXECUTION-PLAN-001-1.0.md` (KDN-37, 4. és 6. pont W1). Az eszköznevek a forkok nyilvános eszközlistájából valók (csak ötletként; kódot nem veszünk át).

A parancsazonosítókat a `crates/engine/src` forrásában ellenőriztük (`cmd!("…")`, `spec("…")`, `CommandSpec { id: "…" }`); a hivatkozott fájl a „Hol” oszlopban áll. Futtatás nem történt.

## Összevetés

| Fork-eszköz | CFA4-parancs (ellenőrizve) | Hol | Eredmény | Megjegyzés / tervnote |
|---|---|---|---|---|
| `ripple_delete` | `edit.rippleDelete`, `edit.rippleDeleteHead`; rés: `sequence.closeGap` | `commands.rs` (~1994, ~2028) | megvan | – |
| `ripple_trim` | `timeline.trim` (`"mode":"ripple"`, `edge`, `delta`/`deltaFrames`, `linked`); `trim.rippleNext`, `trim.ripplePrevious` (a lejátszófejig) | `commands.rs` (~3098, ~3220) | megvan | Külön `ripple_trim` azonosító nem kell: a `timeline.trim` mód-paramétere fedi |
| `roll_edit` | `timeline.roll` (`left`, `right`, `delta`/`deltaFrames`); lekérdezés: `timeline.rollDelta` | `commands.rs` (~3232, ~3260) | megvan | – |
| `slide_edit` | `timeline.slide` (+ `timeline.slideLeft/Right[5]`); `timeline.slip` | `commands.rs`; `ui-egui/src/panels/timeline.rs` hívja | megvan | – |
| `extract_zone` | `sequence.extract` (In–Out közti szakasz, hézagbezárással); `sequence.lift` (hézaggal) | `commands.rs` (~2442, ~2453) | megvan | A zóna a szekvencia In/Out jelölője; külön paraméteres zóna nincs, de `edit_sequence` + jelölő beállítás fedi |
| `render_contact_sheet` | – (a QC belső `contact_sheet` PNG-je létezik: `crates/qc/src/picture.rs`, a `qc.run` mellékterméke, nem önálló parancs) | `crates/qc`, `engine/src/qc.rs` | **hiányzik** | Lásd lent |
| `render_frame` | `file.exportFrame` (Shift+E; `target` source/program, `format`, `depth`); a Qt-oldali képkocka-`render` a `serve`-ön | `keyboard.rs` (~561) | megvan | – |
| `detect_scenes` | `clip.sceneEditDetection` (`sensitivity`, `minShotFrames`, `applyCuts`, `createSubclips`, `generateMarkers`, `wait`) | `scene_detect.rs` (32) | megvan | – |
| `import_srt_as_titles` | `captions.import` (SRT/VTT/SCC/MCC/STL/TTML/DFXP feliratsávba) | `captions.rs` (~1137; `.srt` felismerés ~102) | **részben** | Felirat igen, de **cím-/grafikai elemként** (`graphics.newText`) nem; lásd lent |
| `set_effect_expression` | – (`effects.setParam`, `effects.setKeyframe`, `effects.apply` megvan; kifejezés-paraméter nincs) | `effects.*` a `commands.rs`-ben | **hiányzik** | A kifejezésmag a VFX-4 terv gazdája (QuickJS a D-Ogi forkban); ez nem a KDN-terv feladata |
| `timeline_close_gaps` | `sequence.closeGap` (paraméter nélkül: a kijelölt rés, vagy az In–Out közti összes rés; `track`+`time` explicit) | `commands.rs` (~2508, `close_sequence_gaps`) | megvan | Üres paraméterű alak az „összes rést” zárja |
| `flow_delete_short_clips` | – (`clip.removeSilence`, `edit.rippleDelete` és `sequence.closeGap` van; rövid-klip szűrő nincs) | – | **hiányzik** | Lásd lent |
| `flow_export_and_wait` | `export.quick` (`wait:true`, `preset`, `path`); `export.queue.add`/`export.queue.start` (`wait`); `export.start` | `export_tools.rs` (~1589, ~1566); `smart_cut.rs` (481) | megvan | A `wait` paraméter adja a „várj” viselkedést |
| `render_start_with_guides` | – (`export.queue.add` `ranges:[range]` és `range: entire/inOut/workArea/custom` van; vezetővonalból/jelölőből tartomány nincs) | `export_tools.rs` (range, ~461) | **hiányzik** | Lásd lent |

## Valóban hiányzó parancsok

| Javasolt azonosító | Terv egy sorban (paraméterek, újrahasznosítás) |
|---|---|
| `qc.contactSheet` (vagy `file.exportContactSheet`) | `{"clip"\|"sequence":id, "frames":n=12, "columns":n?, "path":str?}`; a `cfa4-qc` `picture.rs` meglévő kontaktlap-rajzolóját hívja önállóan, a `file.exportFrame` képkocka-kinyerésével és a `wait` paraméterrel |
| `captions.importAsTitles` | `{"path":str,"style":str?,"track":id?}`; a `captions.rs` SRT-elemzőjét használja, és felirat helyett `graphics.newText` klipet vesz fel időzítve; a `captions.import` mellett marad |
| `effects.setExpression` | `{"clip":id,"effect":id,"param":str,"expr":str}`; **nem a KDN-terv**: a kifejezésmotort a VFX-4 hordozza, a KDN-37 csak a névegyeztetést adja |
| `edit.deleteShortClips` (vagy `sequence.removeShortClips`) | `{"minFrames":n,"tracks":[id]?,"ripple":bool=true,"dryRun":bool=true}`; a `clip.removeSilence` és az `edit.rippleDelete` kijelölés-logikáját használja, utána `sequence.closeGap` |
| `export.queue.addFromMarkers` (vagy `ranges` bővítés `markers` forrással) | `{"markers":"color|name"?, …export.queue.add paraméterei}`; a jelölők közti tartományokat `ranges`-szé alakítja, majd `export.queue.add` fut |

**Összegzés:** 14 fork-eszközből 9 megvan, 1 részben, 4 hiányzik (a hiányzókból az `effects.setExpression` a VFX-4 terv gazdájánál marad). A KDN-37 tényleges teendője legfeljebb 4 kis parancs; az E6 MCP-felülettel egyeztetendő.
