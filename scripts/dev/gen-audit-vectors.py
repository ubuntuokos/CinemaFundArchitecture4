#!/usr/bin/env python3
"""Regenerate tests/vectors/audit/v1/*.json (CFA4-AUDIT-001, AU-1).

The expected canonical strings below are written BY HAND from RFC 8785 (narrowed: no floats); only the
SHA-256 is computed here, with hashlib. Neither the Rust (`cfa4-audit`) nor the Python (`cfa4_audit`)
implementation is used, so the vectors are an independent oracle for both.
"""

from __future__ import annotations

import hashlib
import json
import pathlib

OUT = pathlib.Path(__file__).resolve().parents[2] / "tests" / "vectors" / "audit" / "v1"
DOMAIN = b"cfa4-audit/v1\x00"

# (file stem, raw JSON input text, expected canonical text or None, expected error code or None, is_event)
EV = True
CASES: list[tuple[str, str, str | None, str | None, bool]] = [
    (
        "genesis",
        '{"seq":1,"prev_hash":"' + "00" * 32 + '","action":"audit.chain.genesis","app":"cfa4-audit","scope":{}}',
        '{"action":"audit.chain.genesis","app":"cfa4-audit","prev_hash":"' + "00" * 32 + '","scope":{},"seq":1}',
        None,
        EV,
    ),
    (
        "nested-sorted",
        '{"b":[3,1,{"z":null,"a":true}],"a":{"y":false,"x":-7},"c":"s"}',
        '{"a":{"x":-7,"y":false},"b":[3,1,{"a":true,"z":null}],"c":"s"}',
        None,
        EV,
    ),
    (
        # RFC 8785 3.2.3: UTF-16 code unit order; the emoji (D83D DE00) sorts BEFORE U+FB33.
        "rfc8785-key-order",
        '{"\\u20ac":"Euro","\\r":"CR","\\ufb33":"Dalet","1":"One","\\ud83d\\ude00":"Emoji","\\u0080":"Control","\\u00f6":"o-diaeresis"}',
        '{"\\r":"CR","1":"One","\u0080":"Control","\u00f6":"o-diaeresis","\u20ac":"Euro","\U0001f600":"Emoji","\ufb33":"Dalet"}',
        None,
        EV,
    ),
    (
        "string-escapes",
        '{"s":"a\\"b\\\\c\\/d\\b\\t\\n\\f\\r\\u0001\\u001f\\u007f\\u00e9\\u20ac"}',
        '{"s":"a\\"b\\\\c/d\\b\\t\\n\\f\\r\\u0001\\u001f\u007f\u00e9\u20ac"}',
        None,
        EV,
    ),
    (
        "int64-extremes",
        '{"max":9223372036854775807,"min":-9223372036854775808,"zero":0,"neg":-1}',
        '{"max":9223372036854775807,"min":-9223372036854775808,"neg":-1,"zero":0}',
        None,
        EV,
    ),
    (
        # The hash field itself is not hashed; everything else (seq, prev_hash included) is.
        "hash-field-excluded",
        '{"seq":2,"prev_hash":"' + "ab" * 32 + '","action":"x.y","hash":"' + "ff" * 32 + '"}',
        '{"action":"x.y","prev_hash":"' + "ab" * 32 + '","seq":2}',
        None,
        EV,
    ),
    ("array-value", '[1,"a",null,true,{"k":[]}]', '[1,"a",null,true,{"k":[]}]', None, False),
    ("error-float", '{"a":1.5}', None, "AUDIT_FLOAT_FORBIDDEN", EV),
    ("error-integral-float", '{"a":1.0}', None, "AUDIT_FLOAT_FORBIDDEN", EV),
    ("error-exponent", '{"a":1e2}', None, "AUDIT_FLOAT_FORBIDDEN", EV),
    ("error-above-int64", '{"a":9223372036854775808}', None, "AUDIT_FLOAT_FORBIDDEN", EV),
    ("error-nested-float", '{"a":[{"b":0.1}]}', None, "AUDIT_FLOAT_FORBIDDEN", EV),
]


def main() -> None:
    OUT.mkdir(parents=True, exist_ok=True)
    for stem, raw, canonical, error, is_event in CASES:
        vec: dict[str, object] = {"name": stem, "input": raw, "event": is_event}
        if error is not None:
            vec["error"] = error
        else:
            assert canonical is not None
            vec["canonical"] = canonical
            if is_event:
                vec["hash"] = hashlib.sha256(DOMAIN + canonical.encode("utf-8")).hexdigest()
        (OUT / f"{stem}.json").write_text(json.dumps(vec, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    print(f"wrote {len(CASES)} vectors to {OUT}")


if __name__ == "__main__":
    main()
