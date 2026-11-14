# Contract: a saját oldal kimenetei (stdout, JSON)

## Parancsok – `cargo run -q -p cfa4-engine --example command_catalog` (új)

```json
[ { "id": "sequence.addEdit", "label": "Add Edit", "menu": ["Sequence"], "shortcut": "Cmd+K", "kind": "command" },
  { "id": "kb.selectClipAtPlayhead", "label": "Select Clip at Playhead", "menu": [], "shortcut": "D", "kind": "command" },
  { "id": "project.info", "label": "Project Info", "menu": [], "shortcut": null, "kind": "query" } ]
```

- `kind`: `command` (naplózott) vagy `query` (`journal == false`).
- Rendezés: `id` szerint. Hibánál nem 0 kilépési kód, az ok a stderr-en (a minta az `effect_catalog.rs`).
- A mérő ebből képzi az útvonalat:
  - ha a `menu` nem üres: `menu + [label]`;
  - különben `query`-nél `["Queries", label]`;
  - minden más esetben `["Keyboard", label]`.
- A példák azonosítói szemléltetők.

## Panelek – `PYTHONPATH=python .venv/bin/python scripts/dev/qt_panel_catalog.py` (új)

```json
[ { "name": "Project", "present": true, "dock_area": "left" },
  { "name": "Program", "present": false, "dock_area": null } ]
```

- A forrás a `python/cfa4_qt/main_window.py` `DOCK_PLAN`-je (`present: true`) és a
  `python/cfa4_qt/window_menu.py` `PANELS`-e (a teljes névlista). Nincs `QApplication`, nincs motor.
- A mérő útvonala: `["Window", name]`. A `present: false` tétel nem számít saját tételnek.
- Rendezés: a `PANELS` sorrendje, majd a csak `DOCK_PLAN`-ben szereplők név szerint.
- Kilépési kód 1 és az ok a stderr-en, ha a PySide6 vagy a modul nem importálható.

## Meglévő (változatlan)

- Menük: `cargo run -q -p cfa4-ui-egui --example menu_tree`, az egui menüfa. Q1 csak a paneleket érinti.
- Effektek és átmenetek: `cargo run -q -p cfa4-project --example effect_catalog`.
