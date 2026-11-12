"""F5 isolated runtime helpers: lightweight tests with no actual media binary."""
import json
import os
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parent))
import f5_staged_runtime as f5
from verify_cfa4_packages import AuditError


class F5StagingTests(unittest.TestCase):
    def test_isolated_environment_does_not_use_real_home(self):
        with tempfile.TemporaryDirectory() as tmp:
            env = f5.isolated_env(Path(tmp))
            self.assertTrue(env["HOME"].startswith(tmp))
            self.assertTrue(Path(env["XDG_DATA_HOME"]).is_dir())

    def test_cli_version_must_match_cfa4_package(self):
        with tempfile.TemporaryDirectory() as tmp:
            app = Path(tmp) / "cfa4-cli"
            app.write_text("#!/bin/sh\necho 'cfa4-cli 0.10.0'\n")
            app.chmod(0o755)
            self.assertTrue(f5.cli_smoke(app, "0.10.0", os.environ)["version_match"])
            with self.assertRaisesRegex(AuditError, "lacks expected version"):
                f5.cli_smoke(app, "0.11.0", os.environ)

    def test_ldd_missing_dependency_fails_closed(self):
        with tempfile.TemporaryDirectory() as tmp:
            exe = Path(tmp) / "cfa4"
            exe.write_bytes(b"fake-ELF")
            with patch.object(f5, "invoke", return_value="libbroken.so => not found\n"):
                with self.assertRaisesRegex(AuditError, "missing ELF runtime dependencies"):
                    f5.library_inventory(exe, env=os.environ)

    def test_flatpak_ref_rejects_wrong_architecture(self):
        with tempfile.TemporaryDirectory() as tmp:
            with patch.object(f5, "invoke", side_effect=[
                "",
                "",
                "app/io.github.ubuntuokos.cfa4/aarch64/stable\n",
            ]):
                with self.assertRaisesRegex(AuditError, "unexpected app refs"):
                    f5.stage_flatpak(Path(tmp) / "fixture.flatpak", Path(tmp), os.environ)

    def test_flatpak_ref_accepts_exact_cfa4_amd64_id(self):
        with tempfile.TemporaryDirectory() as tmp:
            with patch.object(f5, "invoke", side_effect=[
                "",
                "",
                "app/io.github.ubuntuokos.cfa4/x86_64/stable\n",
            ]):
                result = f5.stage_flatpak(Path(tmp) / "fixture.flatpak", Path(tmp), os.environ)
                self.assertIn("/x86_64/", result["ref"])
                self.assertIn("NOT_FLATPAK_INSTALL", result["scope"])

    def fake_cli(self, tmp: Path, results: list[dict], *, write: int | None = None) -> Path:
        """A stand-in `cfa4-cli run` that prints `results` and writes the cut file."""
        cli = tmp / "cfa4-cli"
        out = tmp / "work" / "smart-cut.ts"
        accurate = tmp / "work" / "smart-cut-frame-accurate.ts"
        lines = "\n".join(json.dumps({"line": n + 1, "id": "x", **r}) for n, r in enumerate(results))
        size = 0 if write is None else write
        cli.write_text(
            "#!/bin/sh\n"
            f"head -c {size} /dev/zero > '{out}'\n"
            f"head -c {size} /dev/zero > '{accurate}'\n"
            f"cat <<'EOF'\nnoise before\n{lines}\nEOF\n"
        )
        cli.chmod(0o755)
        return cli

    def good_results(self, size: int) -> list[dict]:
        frames, accurate = f5.SMART_CUT_FRAMES, f5.FRAME_ACCURATE_FRAMES
        return [
            {"ok": True, "result": {"codec": "mpeg2-video", "frames": 100}},
            {"ok": True, "result": {"summary": {"lossless": True, "requiresConfirmation": False,
                                                "copiedFrames": frames, "reencodedFrames": 0}}},
            {"ok": True, "result": {"verified": True, "bytes": size, "report": {"samples": frames},
                                    "audio": {"tracks": [{"trackId": 1}]}}},
            {"ok": True, "result": {"codec": "mpeg2-video", "frames": frames}},
            {"ok": True, "result": {"boundaryReencode": {"ready": True, "problem": None},
                                    "summary": {"lossless": False, "reencodedFrames": 6}}},
            {"ok": True, "result": {"verified": True, "bytes": size,
                                    "report": {"samples": accurate, "copied": 34, "reencoded": 6,
                                               "minPsnr": 46.0}}},
            {"ok": True, "result": {"codec": "mpeg2-video", "frames": accurate}},
        ]

    def test_smart_cut_smoke_accepts_a_verified_gop_copy(self):
        with tempfile.TemporaryDirectory() as tmp:
            tmp = Path(tmp)
            cli = self.fake_cli(tmp, self.good_results(188), write=188)
            result = f5.smart_cut_smoke(cli, tmp / "src.ts", tmp / "work", os.environ)
            self.assertEqual(result["copied_frames"], f5.SMART_CUT_FRAMES)
            self.assertEqual(result["audio_tracks"], 1)
            script = (tmp / "work" / "smart-cut.jsonl").read_text().splitlines()
            ids = [json.loads(line)["id"] for line in script]
            self.assertEqual(ids, ["export.analyze", "export.plan", "export.start", "export.analyze",
                                   "export.plan", "export.start", "export.analyze"])
            self.assertEqual(json.loads(script[1])["params"]["mode"], "gop-boundary")
            fa = json.loads(script[5])["params"]
            self.assertEqual((fa["mode"], fa["confirm"]), ("frame-accurate", True))
            self.assertEqual(result["frame_accurate"]["reencoded"], 6)

    def test_smart_cut_smoke_fails_closed(self):
        with tempfile.TemporaryDirectory() as tmp:
            tmp = Path(tmp)
            cases = {
                "failed": lambda r: r[2].update(ok=False),
                "unexpected plan": lambda r: r[1]["result"]["summary"].update(reencodedFrames=2),
                "not verified": lambda r: r[2]["result"].update(verified=False),
                "reads back wrong": lambda r: r[3]["result"].update(frames=1),
                "not MPEG-2": lambda r: r[0]["result"].update(codec="h264"),
                "unexpected frame-accurate plan": lambda r: r[4]["result"].update(boundaryReencode={"ready": False}),
                "frame-accurate cut was not verified": lambda r: r[5]["result"]["report"].update(minPsnr=20.0),
                "frame-accurate output reads back wrong": lambda r: r[6]["result"].update(frames=39),
            }
            for message, spoil in cases.items():
                results = self.good_results(188)
                spoil(results)
                cli = self.fake_cli(tmp, results, write=188)
                with self.assertRaisesRegex(AuditError, message):
                    f5.smart_cut_smoke(cli, tmp / "src.ts", tmp / "work", os.environ)
            # the file on disk must be the one the CLI reported
            cli = self.fake_cli(tmp, self.good_results(188), write=10)
            with self.assertRaisesRegex(AuditError, "differs"):
                f5.smart_cut_smoke(cli, tmp / "src.ts", tmp / "work", os.environ)

    def test_a_package_with_gopchop_files_is_refused(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "usr/bin").mkdir(parents=True)
            (root / "usr/bin/cfa4-cli").write_text("")
            f5.assert_no_gopchop(root, "the tarball")
            (root / "usr/lib/GOPchop").mkdir(parents=True)
            with self.assertRaisesRegex(AuditError, "ships GOPchop"):
                f5.assert_no_gopchop(root, "the tarball")

    def fake_qc_cli(self, tmp: Path, cases: dict) -> Path:
        """A stand-in `cfa4-cli qc`: writes the report `cases` names for the run (sound,
        mismatch, truncated) to --out and exits with its code (default: the verdict's)."""
        spec = tmp / "qc-cases.json"
        spec.write_text(json.dumps(cases))
        cli = tmp / "cfa4-cli"
        cli.write_text(
            f"#!{sys.executable}\n"
            "import json, sys\n"
            f"cases = json.load(open({str(spec)!r}))\n"
            "args = sys.argv[1:]\n"
            "out = args[args.index('--out') + 1]\n"
            "kind = 'mismatch' if '--expect' in args else 'truncated' if 'truncated' in args[1] else 'sound'\n"
            "case = cases[kind]\n"
            "if case.get('report') is not None:\n"
            "    open(out, 'w').write(json.dumps(case['report']))\n"
            "exits = {'PASS': 0, 'WARN': 3, 'FAIL': 4, 'INCOMPLETE': 5}\n"
            "sys.exit(case.get('exit', exits.get(((case.get('report') or {}).get('summary') or {}).get('verdict'), 1)))\n"
        )
        cli.chmod(0o755)
        return cli

    @staticmethod
    def qc_report(verdict: str, **checks: str) -> dict:
        statuses = {"probe.open": "PASS", "integrity.decode": "PASS",
                    "integrity.frameCount": "NOT_RUN"}
        statuses.update({k.replace("_", "."): v for k, v in checks.items()})
        return {
            "schema": "cfa4-qc-report/1",
            "profile": {"id": "delivery-web", "version": 1, "sha256": "ab" * 32},
            "checks": [{"id": k, "status": v} for k, v in statuses.items()],
            "summary": {"verdict": verdict},
        }

    def good_qc_cases(self) -> dict:
        return {
            "sound": {"report": self.qc_report("INCOMPLETE")},
            "mismatch": {"report": self.qc_report("FAIL", integrity_frameCount="FAIL")},
            "truncated": {"report": self.qc_report("FAIL", integrity_decode="FAIL")},
        }

    def test_qc_smoke_accepts_the_packaged_qc(self):
        with tempfile.TemporaryDirectory() as tmp:
            tmp = Path(tmp)
            source = tmp / "qc-source.mp4"
            source.write_bytes(b"x" * 1000)
            cli = self.fake_qc_cli(tmp, self.good_qc_cases())
            result = f5.qc_smoke(cli, source, tmp / "work", os.environ)
            self.assertEqual(result["verdicts"], {"sound": "INCOMPLETE", "frame_count_mismatch": "FAIL",
                                                  "truncated": "FAIL"})
            plan = json.loads((tmp / "work" / "plan.json").read_text())
            self.assertEqual(plan, {"frames": f5.QC_FRAMES + 1})
            self.assertEqual((tmp / "work" / "truncated.mp4").stat().st_size, 600)

    def test_qc_smoke_fails_closed(self):
        with tempfile.TemporaryDirectory() as tmp:
            tmp = Path(tmp)
            source = tmp / "qc-source.mp4"
            source.write_bytes(b"x" * 1000)
            cases = {
                "exited 1": lambda c: c["sound"].update(exit=1),
                "no readable report": lambda c: c["sound"].update(report=None, exit=4),
                "is not cfa4-qc-report/1": lambda c: c["sound"]["report"].update(schema="other/1"),
                "names no built-in profile": lambda c: c["sound"]["report"]["profile"].update(sha256="x"),
                "but exit code": lambda c: c["sound"].update(exit=0),
                "did not read the fixture": lambda c: c["sound"]["report"]["checks"][1].update(status="FAIL"),
                "without a human review PASSed": lambda c: c["sound"]["report"]["summary"].update(verdict="PASS"),
                "misses the render plan": lambda c: c["mismatch"]["report"]["checks"][2].update(status="NOT_RUN"),
                "truncated file did not FAIL": lambda c: c["truncated"]["report"].update(
                    self.qc_report("INCOMPLETE", integrity_decode="NOT_RUN")),
            }
            for message, spoil in cases.items():
                qc_cases = self.good_qc_cases()
                spoil(qc_cases)
                cli = self.fake_qc_cli(tmp, qc_cases)
                with self.assertRaisesRegex(AuditError, message):
                    f5.qc_smoke(cli, source, tmp / f"work-{len(message)}", os.environ)

    def test_qc_smoke_refuses_a_half_written_report(self):
        with tempfile.TemporaryDirectory() as tmp:
            tmp = Path(tmp)
            source = tmp / "qc-source.mp4"
            source.write_bytes(b"x" * 1000)
            cli = self.fake_qc_cli(tmp, self.good_qc_cases())
            work = tmp / "work"
            work.mkdir()
            (work / ".sound.cfa4qc.json.123-0.tmp").write_text("{")
            with self.assertRaisesRegex(AuditError, "not written atomically"):
                f5.qc_smoke(cli, source, work, os.environ)

    def test_invalid_command_is_not_success(self):
        with self.assertRaisesRegex(AuditError, "unable to run"):
            f5.invoke(["/nonexistent-cfa4-f5-command"], timeout=1)


if __name__ == "__main__":
    unittest.main()
