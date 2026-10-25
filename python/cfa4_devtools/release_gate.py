"""CFA4 release verdict guard. A green CI is NOT release authorization."""
from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
DEFAULT = ROOT / "docs/audit/f9-release-readiness.json"
REQUIRED = frozenset({
    "integration_ci",
    "ubuntu_2604_amd64_policy",
    "cfa4_release_ownership_source",
    "four_artifact_structure_source",
    "xdg_metadata_source",
    "native_media_prior_physical_h264_hevc",
    "qt6_architecture_source",
    "real_four_format_build",
    "clean_vm_install_uninstall",
    "desktop_kde_gnome_cinnamon_runtime",
    "runtime_abi_dependencies",
    "native_av1_nvdec_heif_icc_hdr",
    "av_content_sync_precision",
    "full_qt6_nle_parity",
    "licensing_brand_attribution_sbom",
    "protected_main_integration",
})
FORMATS = {"deb", "AppImage", "flatpak", "tar.gz"}


class ReleaseGateError(ValueError):
    pass


def evaluate(data: dict) -> dict:
    if not isinstance(data, dict):
        raise ReleaseGateError("release inventory must be a JSON object")
    for key, expected in (
        ("schema", "cfa4-release-readiness-v1"),
        ("repository", "ubuntuokos/CinemaFundArchitecture4"),
        ("target", "Ubuntu 26.04 AMD64"),
        ("parent_repository_policy", "READ_ONLY_UPSTREAM"),
    ):
        if data.get(key) != expected:
            raise ReleaseGateError(f"invalid CFA4 {key}")
    if set(data.get("artifact_formats", [])) != FORMATS:
        raise ReleaseGateError("CFA4 must require exactly four formats")
    items = data.get("gates")
    if not isinstance(items, list) or len(items) != len(REQUIRED):
        raise ReleaseGateError("missing or duplicated required release gates")
    by_id = {}
    for item in items:
        if not isinstance(item, dict):
            raise ReleaseGateError("malformed release gate")
        key = item.get("id")
        if key in by_id:
            raise ReleaseGateError(f"duplicate release gate: {key}")
        if item.get("status") not in {"PASS", "OPEN"}:
            raise ReleaseGateError(f"{key}: status must be PASS or OPEN")
        if not isinstance(item.get("reason"), str) or not item["reason"].strip():
            raise ReleaseGateError(f"{key}: missing reason")
        if not isinstance(item.get("evidence"), str) or not item["evidence"].strip():
            raise ReleaseGateError(f"{key}: missing evidence reference")
        by_id[key] = item
    if set(by_id) != REQUIRED:
        raise ReleaseGateError(f"release gate set mismatch: {sorted(set(REQUIRED)^set(by_id))}")
    open_ids = sorted(key for key, item in by_id.items() if item["status"] != "PASS")
    decision = "HOLD" if open_ids else "REQUIRES_EXPLICIT_FINAL_APPROVAL"
    if data.get("verdict") != decision:
        raise ReleaseGateError(
            f"document verdict {data.get('verdict')!r} inconsistent with computed {decision}"
        )
    if data.get("release_publication_authorized") is not False:
        raise ReleaseGateError("public release cannot be auto-authorized by source inventory")
    return {"decision": decision, "open_gates": open_ids, "pass_count": len(by_id)-len(open_ids)}


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--file", type=Path, default=DEFAULT)
    parser.add_argument("--require-ready", action="store_true",
                        help="Fail when release remains blocked; never publishes")
    args = parser.parse_args()
    try:
        data = json.loads(args.file.read_text(encoding="utf-8"))
        result = evaluate(data)
    except (OSError, ValueError, json.JSONDecodeError) as exc:
        print(f"FAIL CFA4 release inventory: {exc}", file=sys.stderr)
        return 2
    print(json.dumps(result, indent=2))
    if args.require_ready and result["decision"] != "REQUIRES_EXPLICIT_FINAL_APPROVAL":
        print("RELEASE HOLD: outstanding mandatory gates", file=sys.stderr)
        return 3
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
