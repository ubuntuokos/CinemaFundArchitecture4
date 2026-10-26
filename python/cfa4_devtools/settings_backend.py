"""Backend of the Qt Settings window: bounded local `cfa4-cli settings` / `cfa4-cli hardware` calls
and the pure data model the window is built from. No Qt here, so it is tested without a display.

Never call the `run_*` functions from the Qt GUI event thread (ADR F8): the window runs them on a
QThreadPool worker and receives the result through a signal.
No network, no shell, no package installation; the CLI only reads and writes the user's
`<data dir>/preferences.json` through the Rust engine's validated `prefs.*` commands.
"""
from __future__ import annotations

import json
import os
import subprocess
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

MAX_RESPONSE_BYTES = 4 * 1024 * 1024
MAX_TIMEOUT = 30
# The Qt-only page of the Hardware & Devices group: the accelerator inventory (`cfa4-cli hardware`).
ACCELERATORS_PAGE = "accelerators"
# The Media & Playback ▸ Media Backends page (ADR 0005): the engine's switches, plus the live
# status of the backends and the restricted codecs of the Ubuntu model (`cfa4-cli media-backends`).
MEDIA_BACKENDS_PAGE = "mediaBackends"
# How to add the patent-restricted codecs (docs/licensing.md §7). Shown, copied, never run.
RESTRICTED_INSTALL = "sudo apt install {package}"
# Bounded read of the system PCI id database (hwdata / pciutils) for device names.
PCI_IDS_PATHS = (Path("/usr/share/hwdata/pci.ids"), Path("/usr/share/misc/pci.ids"))
MAX_PCI_IDS_BYTES = 8 * 1024 * 1024


class SettingsBackendError(RuntimeError):
    """The CFA4 CLI is missing, failed, or answered with something that is not the expected JSON."""


def run_cli(cli: Path, args: list[str], *, data_dir: Path | None = None, timeout: int = 15) -> Any:
    """Run one `cfa4-cli` subcommand and return its JSON result (bounded size, time and shape)."""
    if not 1 <= timeout <= MAX_TIMEOUT:
        raise SettingsBackendError(f"timeout must be within 1..{MAX_TIMEOUT} seconds")
    cli = Path(cli)
    if not cli.is_file() or not os.access(cli, os.X_OK):
        raise SettingsBackendError(f"local CFA4 CLI is missing or not executable: {cli}")
    argv = [str(cli), "--compact"]
    if data_dir is not None:
        argv += ["--data-dir", str(data_dir)]
    argv += [str(a) for a in args]
    try:
        completed = subprocess.run(argv, capture_output=True, check=False, timeout=timeout)
    except (OSError, subprocess.TimeoutExpired) as exc:
        raise SettingsBackendError(f"CFA4 CLI unavailable or timed out: {exc}") from exc
    if completed.returncode:
        message = completed.stderr[:2000].decode("utf-8", "replace").strip()
        raise SettingsBackendError(message or f"CFA4 CLI failed: exit {completed.returncode}")
    if not completed.stdout or len(completed.stdout) > MAX_RESPONSE_BYTES:
        raise SettingsBackendError("CFA4 CLI response empty or too large")
    try:
        return json.loads(completed.stdout)
    except (ValueError, UnicodeDecodeError) as exc:
        raise SettingsBackendError("CFA4 CLI returned invalid JSON") from exc


def run_load(cli: Path, data_dir: Path | None = None) -> tuple[Schema, dict[str, Any]]:
    """The schema and the whole saved preferences document (duration units are not fields)."""
    schema = Schema.parse(run_cli(cli, ["settings", "schema"], data_dir=data_dir))
    prefs = run_cli(cli, ["settings", "get"], data_dir=data_dir)
    values = prefs.get("values") if isinstance(prefs, dict) else None
    if not isinstance(values, dict):
        raise SettingsBackendError("settings get must return the preferences object")
    return schema, values


def lookup(document: Any, key: str) -> Any:
    """The value at a dotted key (`timeline.stillImageUnit`) of a preferences document, or None."""
    for part in key.split("."):
        if not isinstance(document, dict):
            return None
        document = document.get(part)
    return document


def run_save(cli: Path, changes: dict[str, Any], data_dir: Path | None = None) -> dict[str, Any]:
    """Save changed keys; returns the whole preferences document as the engine stored it."""
    if not changes:
        raise SettingsBackendError("nothing to save")
    payload = json.dumps(changes, separators=(",", ":"), allow_nan=False)
    result = run_cli(cli, ["settings", "set", payload], data_dir=data_dir)
    if not isinstance(result, dict):
        raise SettingsBackendError("settings set must return the preferences object")
    return result


def run_hardware(cli: Path) -> Hardware:
    return Hardware.parse(run_cli(cli, ["hardware"]))


def run_media_backends(cli: Path, data_dir: Path | None = None) -> MediaBackends:
    return MediaBackends.parse(run_cli(cli, ["media-backends"], data_dir=data_dir))


def _str(value: Any, default: str = "") -> str:
    return value if isinstance(value, str) else default


@dataclass(frozen=True)
class Choice:
    value: str
    label: str


@dataclass(frozen=True)
class Field:
    key: str
    label: str
    kind: str
    wired: bool
    value: Any
    enabled_by: str | None = None
    minimum: float | None = None
    maximum: float | None = None
    decimals: int = 0
    unit: str = ""
    unit_key: str | None = None
    choices: tuple[Choice, ...] = ()

    @staticmethod
    def parse(raw: Any) -> Field | None:
        if not isinstance(raw, dict) or not isinstance(raw.get("key"), str):
            return None
        kind = raw.get("kind") if isinstance(raw.get("kind"), dict) else {}
        choices: list[Choice] = []
        for c in kind.get("choices") or []:
            if isinstance(c, dict) and isinstance(c.get("value"), str):
                choices.append(Choice(c["value"], _str(c.get("label"), c["value"])))
            elif isinstance(c, str):  # label choices are plain names
                choices.append(Choice(c, c))

        def number(name: str) -> float | None:
            v = kind.get(name)
            return float(v) if isinstance(v, (int, float)) and not isinstance(v, bool) else None

        decimals = kind.get("decimals")
        return Field(
            key=raw["key"],
            label=_str(raw.get("label"), raw["key"]),
            kind=_str(kind.get("type"), "text"),
            wired=raw.get("wired") is True,
            value=raw.get("value"),
            enabled_by=raw.get("enabledBy") if isinstance(raw.get("enabledBy"), str) else None,
            minimum=number("min"),
            maximum=number("max"),
            decimals=decimals if isinstance(decimals, int) and 0 <= decimals <= 6 else 0,
            unit=_str(kind.get("unit")),
            unit_key=kind.get("unitKey") if isinstance(kind.get("unitKey"), str) else None,
            choices=tuple(choices),
        )


@dataclass(frozen=True)
class Page:
    id: str
    title: str
    group: str
    fields: tuple[Field, ...] = ()


@dataclass(frozen=True)
class Group:
    id: str
    title: str
    pages: tuple[Page, ...] = ()


@dataclass(frozen=True)
class Schema:
    groups: tuple[Group, ...]

    @staticmethod
    def parse(raw: Any) -> Schema:
        if not isinstance(raw, dict) or not isinstance(raw.get("categories"), list):
            raise SettingsBackendError("settings schema must be an object with categories")
        raw_groups = [g for g in raw.get("groups") or [] if isinstance(g, dict) and isinstance(g.get("id"), str)]
        known = {g["id"] for g in raw_groups}
        pages: list[Page] = []
        for c in raw["categories"]:
            if not isinstance(c, dict) or not isinstance(c.get("id"), str):
                continue
            fields = tuple(f for f in (Field.parse(x) for x in c.get("fields") or []) if f is not None)
            group = _str(c.get("group"), "general")
            # a page of a group the schema does not list still shows, under General
            pages.append(Page(c["id"], _str(c.get("title"), c["id"]), group if group in known else "general", fields))
        groups: list[Group] = []
        for g in raw_groups:
            members = [p for p in pages if p.group == g["id"]]
            if g["id"] == "hardware":
                members.insert(0, Page(ACCELERATORS_PAGE, "Accelerators (GPU / NPU)", "hardware"))
            if members:
                groups.append(Group(g["id"], _str(g.get("title"), g["id"]), tuple(members)))
        if not raw_groups and pages:
            groups.append(Group("general", "General", tuple(pages)))
        return Schema(tuple(groups))

    def page(self, page_id: str) -> Page | None:
        return next((p for g in self.groups for p in g.pages if p.id == page_id), None)

    def page_keys(self, page_id: str) -> set[str]:
        """Every key a page edits: its fields and their duration unit keys."""
        page = self.page(page_id)
        fields = page.fields if page is not None else ()
        return {f.key for f in fields} | {f.unit_key for f in fields if f.unit_key}

    def values(self, prefs: dict[str, Any]) -> dict[str, Any]:
        """Every field's saved value by key, plus the unit keys of duration fields (from `prefs`)."""
        out: dict[str, Any] = {}
        for g in self.groups:
            for p in g.pages:
                for f in p.fields:
                    out[f.key] = f.value
                    if f.unit_key:
                        out[f.unit_key] = lookup(prefs, f.unit_key)
        return out


# `settings::sanitize` limits of a duration by its unit: (min, max, decimals); frames are whole.
DURATION_LIMITS = {"frames": (1.0, 100_000.0, 0), "seconds": (0.01, 3600.0, 2)}


def duration_limits(unit: Any) -> tuple[float, float, int]:
    return DURATION_LIMITS.get(unit if isinstance(unit, str) else "", DURATION_LIMITS["seconds"])


def changed_values(original: dict[str, Any], edited: dict[str, Any]) -> dict[str, Any]:
    """The keys whose edited value differs from the saved one (what OK sends to `settings set`)."""
    return {k: v for k, v in edited.items() if k in original and original[k] != v}


@dataclass(frozen=True)
class Device:
    kind: str
    vendor: str
    pci_vendor: int
    pci_device: int
    driver: str
    node: str

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
            raise SettingsBackendError("hardware report must be an object with an inventory")
        inv = raw["inventory"]

        def ident(v: Any) -> int:
            return v if isinstance(v, int) and not isinstance(v, bool) and 0 <= v <= 0xFFFFF else 0

        devices = tuple(
            Device(_str(d.get("kind"), "gpu"), _str(d.get("vendor"), "other"), ident(d.get("pci_vendor")),
                   ident(d.get("pci_device")), _str(d.get("driver")), _str(d.get("node")))
            for d in inv.get("devices") or [] if isinstance(d, dict)
        )
        plans = []
        for p in raw.get("plans") or []:
            if not isinstance(p, dict) or not isinstance(p.get("workload"), str):
                continue
            chain = tuple(b for b in p.get("chain") or [] if isinstance(b, str)) or ("cpu",)
            # `needs` says what is missing in words; `reason` is the code (noDevice, notImplemented, …)
            skipped = tuple(
                (s["backend"], _str(s.get("needs")) or SKIP_REASONS.get(_str(s.get("reason")), _str(s.get("reason"))))
                for s in p.get("skipped") or [] if isinstance(s, dict) and isinstance(s.get("backend"), str)
            )
            plans.append(Plan(p["workload"], chain, skipped))
        notes = tuple(n for n in inv.get("notes") or [] if isinstance(n, str))
        return Hardware(devices, tuple(plans), notes)


@dataclass(frozen=True)
class Restricted:
    name: str
    purpose: str
    available: bool


@dataclass(frozen=True)
class Backend:
    id: str
    name: str
    role: str
    compiled_in: bool
    available: bool
    enabled: bool
    can_disable: bool
    version: str = ""
    license: str = ""
    problem: str = ""
    restricted_package: str = ""
    restricted: tuple[Restricted, ...] = ()

    @property
    def status(self) -> str:
        if not self.compiled_in:
            return "Not part of this build"
        if self.available and self.enabled:
            return "In use"
        if self.available:
            return "Available, switched off"
        return "Not available"


@dataclass(frozen=True)
class MediaBackends:
    backends: tuple[Backend, ...]

    @staticmethod
    def parse(raw: Any) -> MediaBackends:
        if not isinstance(raw, dict) or not isinstance(raw.get("backends"), list):
            raise SettingsBackendError("media-backends must return an object with a backends list")
        out = []
        for b in raw["backends"]:
            if not isinstance(b, dict) or not isinstance(b.get("id"), str):
                continue
            restricted = tuple(
                Restricted(_str(r.get("name")), _str(r.get("purpose")), r.get("available") is True)
                for r in b.get("restricted") or [] if isinstance(r, dict) and isinstance(r.get("name"), str)
            )
            out.append(Backend(
                b["id"], _str(b.get("name"), b["id"]), _str(b.get("role")), b.get("compiledIn") is True,
                b.get("available") is True, b.get("enabled") is True, b.get("canDisable") is True,
                _str(b.get("version")), _str(b.get("license")), _str(b.get("problem")),
                _str(b.get("restrictedPackage")), restricted,
            ))
        return MediaBackends(tuple(out))

    @property
    def restricted(self) -> tuple[tuple[Backend, Restricted], ...]:
        return tuple((b, r) for b in self.backends for r in b.restricted)

    def install_command(self) -> str | None:
        """The command that adds the missing restricted codecs, or None when none is missing."""
        packages = sorted({b.restricted_package for b, r in self.restricted if not r.available and b.restricted_package})
        return RESTRICTED_INSTALL.format(package=" ".join(packages)) if packages else None


def pci_name(vendor: int, device: int, paths: tuple[Path, ...] = PCI_IDS_PATHS) -> str | None:
    """`Vendor Device` from the system's pci.ids, or None when it is missing or does not list it."""
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


SKIP_REASONS = {  # cfa4_accel::Skip
    "noDevice": "no suitable device",
    "notImplemented": "not implemented yet",
    "excludedByPreference": "left out by the workload's preference",
    "vendorLockedOff": "vendor-locked backends are switched off",
}

WORKLOAD_TITLES = {
    "composite": "Compositing and effects",
    "videoDecode": "Video decoding",
    "videoEncode": "Video encoding",
    "speechToText": "Speech to text",
    "textToSpeech": "Text to speech",
}
