"""The engine's hardware and codec reports, as the Settings pages read them (CFA4-SETTINGS-HW-002).

``hardware.inventory`` and ``codecs.capabilities`` (``cfa4-cli serve``) answer the same JSON as
``cfa4-cli hardware`` and ``cfa4-cli codecs``: the GPUs and NPUs the system reports, the backend
chain per job, and what this machine decodes and encodes. The parsers accept partial or odd
answers (an older engine, a field it leaves out) and never raise on them except for a missing top
level object; PCI names come from the system's ``pci.ids`` when it has one.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

from .i18n import N_

PCI_IDS_PATHS = (Path("/usr/share/hwdata/pci.ids"), Path("/usr/share/misc/pci.ids"))
MAX_PCI_IDS_BYTES = 8 * 1024 * 1024

# cfa4_route::Reason: why a backend is not used for a job
SKIP_REASONS = {
    "no_device": N_("no suitable device"),
    "not_implemented": N_("not implemented yet"),
    "not_registered": N_("not in this build"),
    "not_preferred": N_("left out by the preference"),
    "vendor_locked": N_("vendor-locked backends are switched off"),
    "disabled": N_("switched off in the settings"),
    "failed": N_("failed while running"),
}

# cfa4_route::Family: the jobs the routing table plans
WORKLOAD_TITLES = {
    "display": N_("Preview, compositing and effects"),
    "codec_decode": N_("Video decoding"),
    "codec_encode": N_("Video encoding"),
    "ai": N_("Local AI (speech, voices)"),
    "compute": N_("General compute"),
}


class ReportError(ValueError):
    """The answer is not a report at all."""


def _str(value: Any, default: str = "") -> str:
    return value if isinstance(value, str) else default


def _obj(value: Any) -> dict[str, Any]:
    return value if isinstance(value, dict) else {}


def _ident(value: Any) -> int:
    return value if isinstance(value, int) and not isinstance(value, bool) and 0 <= value <= 0xFFFFF else 0


@dataclass(frozen=True)
class Device:
    kind: str
    vendor: str
    pci_vendor: int
    pci_device: int
    driver: str
    node: str
    name: str = ""

    @property
    def pci_id(self) -> str:
        return f"{self.pci_vendor:04x}:{self.pci_device:04x}" if self.pci_vendor else "—"


@dataclass(frozen=True)
class Plan:
    workload: str
    chain: tuple[str, ...]
    skipped: tuple[tuple[str, str], ...] = ()


@dataclass(frozen=True)
class Hardware:
    devices: tuple[Device, ...]
    plans: tuple[Plan, ...]
    notes: tuple[str, ...] = field(default_factory=tuple)

    @staticmethod
    def parse(raw: Any) -> Hardware:
        if not isinstance(raw, dict) or not isinstance(raw.get("inventory"), dict):
            raise ReportError("hardware report must be an object with an inventory")
        inv = raw["inventory"]
        devices = tuple(
            Device(_str(d.get("kind"), "gpu"), _str(d.get("vendor"), "other"), _ident(d.get("pci_vendor")),
                   _ident(d.get("pci_device")), _str(d.get("driver")), _str(d.get("node")), _str(d.get("name")))
            for d in inv.get("devices") or [] if isinstance(d, dict)
        )
        plans = []
        for p in raw.get("plans") or []:
            if not isinstance(p, dict) or not isinstance(p.get("workload"), str):
                continue
            chain = tuple(b for b in p.get("chain") or [] if isinstance(b, str)) or ("cpu",)
            # `needs` says what is missing in words; `reason` is the code
            skipped = tuple(
                (s["backend"], _str(s.get("needs")) or SKIP_REASONS.get(_str(s.get("reason")), _str(s.get("reason"))))
                for s in p.get("skipped") or [] if isinstance(s, dict) and isinstance(s.get("backend"), str)
            )
            plans.append(Plan(p["workload"], chain, skipped))
        notes = tuple(n for n in inv.get("notes") or [] if isinstance(n, str))
        return Hardware(devices, tuple(plans), notes)


@dataclass(frozen=True)
class CodecRow:
    codec: str
    kind: str
    decode: str
    decode_note: str
    hardware_decode: str | None
    encode: str
    export_format: str | None


@dataclass(frozen=True)
class Backend:
    """A media backend found or not found on this machine (hardware decoder, GStreamer)."""

    name: str | None
    reason: str


@dataclass(frozen=True)
class Codecs:
    rows: tuple[CodecRow, ...]
    hardware_decoder: Backend
    gstreamer: Backend
    gstreamer_compiled_in: bool

    @staticmethod
    def parse(raw: Any) -> Codecs:
        if not isinstance(raw, dict) or not isinstance(raw.get("codecs"), list):
            raise ReportError("codec report must be an object with codecs")
        rows = tuple(
            CodecRow(r["codec"], _str(r.get("kind"), "video"), _str(r.get("decode"), "none"), _str(r.get("decodeNote")),
                     r.get("hardwareDecode") if isinstance(r.get("hardwareDecode"), str) else None,
                     _str(r.get("encode"), "none"),
                     r.get("exportFormat") if isinstance(r.get("exportFormat"), str) else None)
            for r in raw["codecs"] if isinstance(r, dict) and isinstance(r.get("codec"), str)
        )
        hw = _obj(raw.get("hardwareDecoder"))
        gst = _obj(raw.get("gstreamer"))
        return Codecs(
            rows,
            Backend(hw.get("backend") if isinstance(hw.get("backend"), str) else None, _str(hw.get("reason"))),
            Backend(gst.get("version") if gst.get("available") is True and isinstance(gst.get("version"), str) else None,
                    _str(gst.get("reason"))),
            gst.get("compiledIn") is True,
        )


def pci_name(vendor: int, device: int, paths: tuple[Path, ...] = PCI_IDS_PATHS) -> str | None:
    """``Vendor Device`` from the system's pci.ids, or None when it is missing or does not list it."""
    if not vendor:
        return None
    for path in paths:
        try:
            if not path.is_file() or path.stat().st_size > MAX_PCI_IDS_BYTES:
                continue
            text = path.read_text(encoding="utf-8", errors="replace")
        except OSError:
            continue
        vendor_name = None
        for line in text.splitlines():
            if not line or line.startswith("#"):
                continue
            if not line.startswith("\t"):
                if vendor_name is not None:
                    break  # past our vendor's block
                if line[:4].lower() == f"{vendor:04x}":
                    vendor_name = line[4:].strip()
            elif vendor_name is not None and not line.startswith("\t\t") and line[1:5].lower() == f"{device:04x}":
                return f"{vendor_name} {line[5:].strip()}"
        if vendor_name is not None:
            return vendor_name
    return None
