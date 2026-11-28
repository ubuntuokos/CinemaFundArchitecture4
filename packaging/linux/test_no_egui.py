#!/usr/bin/env python3
"""No CFA4 package carries or starts the egui editor (owner decisions 2026-11-12 and 2026-11-14,
docs/plans/CFA4-QT6-FINISH-001.md Q4.3).

Static checks of the packaging sources; the installer, AppRun and the package audits exercise the
same rule at run time (test_install.py, test_stage_pyside6.py, scripts/release/test_four_formats.py).
"""

import re
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
APP_ID = "io.github.ubuntuokos.cfa4"


class NoEguiInPackagesTests(unittest.TestCase):
    def test_package_build_compiles_only_the_engine(self):
        script = (HERE / "package.sh").read_text()
        builds = re.findall(r"cargo build [^\n]*", script)
        self.assertTrue(any("-p cfa4-cli" in b for b in builds), builds)
        for build in builds:
            # the engine, and for the .deb the root helper (ADR 0017); never the egui editor
            self.assertRegex(build, r"-p (cfa4-cli|cfa4-system-helper)(\s|\)|$)", build)
            self.assertNotRegex(build, r"-p cfa4(\s|$)", build)
            self.assertNotIn("cfa4/", build.replace("cfa4-cli/", ""), build)
        self.assertNotIn('"$BIN/cfa4"', script)
        # the familiar `cfa4` command is the Qt GUI's launcher
        self.assertIn('install -Dm755 "$HERE/cfa4-qt" "$STAGE/usr/bin/cfa4"', script)

    def test_menu_entries_start_the_qt_gui(self):
        for name in (f"{APP_ID}.desktop", f"{APP_ID}.credits.desktop"):
            entry = (HERE / name).read_text()
            self.assertRegex(entry, r"(?m)^Exec=cfa4-qt ", name)
            self.assertRegex(entry, r"(?m)^TryExec=cfa4-qt$", name)

    def test_apprun_has_no_egui_path(self):
        code = (HERE / "AppRun").read_text().split("\nhere=", 1)[1]
        self.assertNotIn("usr/bin/cfa4\"", code)
        self.assertNotIn("CFA4_GUI", code)
        self.assertIn('exec python3 "$here/usr/bin/cfa4-qt"', code)

    def test_installer_has_no_egui_branch(self):
        script = (HERE / "install.sh").read_text()
        self.assertNotIn("gui=cfa4\n", script)
        self.assertNotIn("keeps egui", script)
        self.assertIn('Exec=*) printf \'Exec="%s" %%F\\n\' "$exec_path"', script)
        self.assertIn('printf \'%s\' "$prefix/bin/cfa4-qt"', script)

    def test_flatpaks_build_and_offer_no_egui(self):
        for name in (f"{APP_ID}.yml", f"{APP_ID}.bundle.yml"):
            manifest = (HERE / "flatpak" / name).read_text()
            self.assertRegex(manifest, r"(?m)^command: cfa4-qt$", name)
            self.assertNotIn("--command=cfa4 ", manifest, name)
            self.assertNotIn("target/release/cfa4 ", manifest, name)
            self.assertNotRegex(manifest, r"build [^\n]*-p cfa4(\s|$)", name)
        source = (HERE / "flatpak" / f"{APP_ID}.yml").read_text()
        self.assertIn("install -Dm755 packaging/linux/cfa4-qt /app/bin/cfa4\n", source)
        self.assertIn("cargo --offline build --release --locked -p cfa4-cli\n", source)


if __name__ == "__main__":
    unittest.main()
