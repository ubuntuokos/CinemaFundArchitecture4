#!/usr/bin/env python3
"""Generate the BacklotHub country and currency lists (G2-02 / G2-24 section 3.1) from system data.

    python3 scripts/dev/gen-backlot-data.py            # rewrites python/cfa4_backlothub/data/*.json
    python3 scripts/dev/gen-backlot-data.py --check    # fails if the committed files differ (CI-friendly)

Sources (both on every Ubuntu with the ``tzdata`` and ``iso-codes`` packages):
* countries: IANA ``/usr/share/zoneinfo/iso3166.tab`` (public domain; ISO 3166-1 alpha-2 code and short name);
* currencies: Debian ``iso-codes`` ``/usr/share/iso-codes/json/iso_4217.json`` (LGPL-2.1-or-later; ISO 4217 alpha-3 code
  and name). Only circulating currencies are kept: funds, precious metals, testing and unit-of-account codes are left
  out (G2-24 3.1). The minor-unit exponent (ISO 4217 "minor unit": 0, 2 or 3 decimals) is added here, as it is not in
  the package.
"""

from __future__ import annotations

import json
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent.parent
OUT = ROOT / "python" / "cfa4_backlothub" / "data"

# Not circulating currencies: precious metals, testing, no currency, funds and unit-of-account codes.
EXCLUDED = {
    "XAU", "XAG", "XPT", "XPD", "XTS", "XXX", "XDR", "XSU", "XUA", "XBA", "XBB", "XBC", "XBD",
    "CLF", "UYI", "UYW", "BOV", "CHE", "CHW", "COU", "MXV", "USN",
}  # fmt: skip
ZERO = {"BIF", "CLP", "DJF", "GNF", "ISK", "JPY", "KMF", "KRW", "PYG", "RWF", "UGX", "VND", "VUV", "XAF", "XOF", "XPF"}
THREE = {"BHD", "IQD", "JOD", "KWD", "LYD", "OMR", "TND"}


def _version(package: str) -> str:
    out = subprocess.run(["dpkg-query", "-W", "-f=${Version}", package], capture_output=True, text=True, check=False)
    return out.stdout.strip() or "unknown"


def countries() -> dict[str, object]:
    rows = []
    for line in Path("/usr/share/zoneinfo/iso3166.tab").read_text(encoding="utf-8").splitlines():
        if line and not line.startswith("#"):
            code, name = line.split("\t", 1)
            rows.append({"code": code, "name": name})
    rows.sort(key=lambda r: str(r["code"]))
    return {
        "source": "IANA tzdata iso3166.tab (public domain), ISO 3166-1 alpha-2",
        "source_version": f"tzdata {_version('tzdata')}",
        "countries": rows,
    }


def currencies() -> dict[str, object]:
    data = json.loads(Path("/usr/share/iso-codes/json/iso_4217.json").read_text(encoding="utf-8"))["4217"]
    rows = []
    for item in data:
        code = item["alpha_3"]
        if code in EXCLUDED:
            continue
        exponent = 0 if code in ZERO else 3 if code in THREE else 2
        rows.append({"code": code, "exponent": exponent, "name": item["name"]})
    rows.sort(key=lambda r: str(r["code"]))
    return {
        "source": "Debian iso-codes iso_4217.json (LGPL-2.1-or-later), ISO 4217 alpha-3; exponents added by CFA4",
        "source_version": f"iso-codes {_version('iso-codes')}",
        "currencies": rows,
    }


def main() -> int:
    files = {"iso3166.json": countries(), "iso4217.json": currencies()}
    check = "--check" in sys.argv
    bad = False
    for name, doc in files.items():
        text = json.dumps(doc, ensure_ascii=False, indent=1) + "\n"
        path = OUT / name
        if check:
            # the version line depends on the machine; compare the lists only
            old = json.loads(path.read_text(encoding="utf-8")) if path.exists() else {}
            key = "countries" if "countries" in doc else "currencies"
            if old.get(key) != doc[key]:
                print(f"{path} differs from the system data", file=sys.stderr)
                bad = True
        else:
            OUT.mkdir(parents=True, exist_ok=True)
            path.write_text(text, encoding="utf-8")
            print(f"wrote {path} ({len(doc['countries' if 'countries' in doc else 'currencies'])} rows)")  # type: ignore[arg-type]
    return 1 if bad else 0


if __name__ == "__main__":
    raise SystemExit(main())
