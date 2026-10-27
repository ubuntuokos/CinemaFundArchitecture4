"""The engine's command registry as Qt menus, actions and shortcuts.

Everything in CFA4 is a command (`crates/engine`): id, label, menu path, shortcut, params and
enablement. The Qt GUI builds its menu bar and keyboard shortcuts from ``engine.commands`` instead
of re-declaring them, so every engine command is reachable from the Qt GUI the moment it exists.
"""

from __future__ import annotations

import json
import re
from collections.abc import Callable
from dataclasses import dataclass, field
from typing import Any

from PySide6.QtGui import QAction, QKeySequence
from PySide6.QtWidgets import QApplication, QMenu, QMenuBar, QWidget

from .i18n import tr

# egui key names (as the engine stores shortcuts) -> Qt key names
_KEYS = {
    "ArrowLeft": "Left",
    "ArrowRight": "Right",
    "ArrowUp": "Up",
    "ArrowDown": "Down",
    "Escape": "Esc",
    "Enter": "Return",
    "Delete": "Del",
    "Insert": "Ins",
    "PageUp": "PgUp",
    "PageDown": "PgDown",
    "Comma": ",",
    "Period": ".",
    "Minus": "-",
    "Plus": "+",
    "Equals": "=",
    "Semicolon": ";",
    "Colon": ":",
    "Slash": "/",
    "Backslash": "\\",
    "OpenBracket": "[",
    "CloseBracket": "]",
    "Backtick": "`",
    "Quote": "'",
    "Questionmark": "?",
    "Pipe": "|",
}
_MODS = {
    "Cmd": "Ctrl",
    "Command": "Ctrl",
    "Ctrl": "Ctrl",
    "Alt": "Alt",
    "Option": "Alt",
    "Shift": "Shift",
    "Mac": "Meta",
}


def qt_shortcut(keys: str | None) -> str:
    """``Cmd+Shift+ArrowLeft`` -> ``Ctrl+Shift+Left``; empty for nothing usable."""
    if not keys or not isinstance(keys, str):
        return ""
    parts = keys.split("+")
    # a literal plus key: "Cmd++"
    if keys.endswith("++"):
        parts = [*keys[:-2].split("+"), "+"]
    mods: list[str] = []
    key = ""
    for part in parts:
        if part in _MODS:
            mod = _MODS[part]
            if mod not in mods:
                mods.append(mod)
        elif part:
            key = _KEYS.get(part, part)
            if re.fullmatch(r"Num[0-9]", key):
                key = key[3:]
    if not key:
        return ""
    return "+".join([*mods, key])


@dataclass
class ParamSpec:
    name: str
    hint: str
    optional: bool


def parse_params(spec: str | None) -> list[ParamSpec]:
    """Top-level keys of a command's JSON-ish params description (``{"item":id?,"seconds":number}``)."""
    if not spec or not isinstance(spec, str):
        return []
    text = spec.strip()
    if not (text.startswith("{") and text.endswith("}")):
        return []
    body = text[1:-1]
    out: list[ParamSpec] = []
    depth = 0
    in_string = False
    start = 0
    pieces: list[str] = []
    for i, c in enumerate(body):
        if c == '"' and (i == 0 or body[i - 1] != "\\"):
            in_string = not in_string
        elif not in_string:
            if c in "{[(":
                depth += 1
            elif c in "}])":
                depth -= 1
            elif c == "," and depth == 0:
                pieces.append(body[start:i])
                start = i + 1
    pieces.append(body[start:])
    for piece in pieces:
        m = re.match(r'\s*"([^"]+)"\s*:\s*(.*)\s*$', piece, re.DOTALL)
        if not m:
            continue
        hint = m.group(2).strip()
        optional = hint.endswith("?")
        out.append(ParamSpec(m.group(1), hint.rstrip("?").strip(), optional))
    return out


def parse_value(text: str) -> Any:
    """A form field's text as JSON when it parses, else the text itself."""
    stripped = text.strip()
    if not stripped:
        return None
    try:
        return json.loads(stripped)
    except ValueError:
        return stripped


@dataclass
class Command:
    id: str
    label: str
    menu: list[str]
    shortcut: str
    params: str
    enabled: bool
    shortcuts: list[str] = field(default_factory=list)
    # (keys, panel): bindings that only apply with that panel focused (History ▸ Left = Undo)
    panel_shortcuts: list[tuple[str, str]] = field(default_factory=list)

    @classmethod
    def from_json(cls, raw: dict[str, Any]) -> Command | None:
        cid = raw.get("id")
        if not isinstance(cid, str) or not cid:
            return None
        menu = raw.get("menu") if isinstance(raw.get("menu"), list) else []
        shortcuts = [str(s) for s in raw.get("shortcuts") or [] if isinstance(s, str)]
        # panel-only bindings must not take the key window-wide: build_menus dispatches them by focus
        panel_shortcuts: list[tuple[str, str]] = []
        for b in raw.get("panelShortcuts") or []:
            keys = b.get("keys") if isinstance(b, dict) else None
            panel = b.get("panel") if isinstance(b, dict) else None
            if not isinstance(keys, str) or not isinstance(panel, str):
                continue
            if keys in shortcuts:
                shortcuts.remove(keys)
            panel_shortcuts.append((keys, panel))
        return cls(
            id=cid,
            label=str(raw.get("label") or cid),
            menu=[str(m) for m in menu],
            shortcut=str(raw.get("shortcut") or ""),
            params=str(raw.get("params") or "{}"),
            enabled=bool(raw.get("enabled", True)),
            shortcuts=shortcuts,
            panel_shortcuts=panel_shortcuts,
        )

    @property
    def param_specs(self) -> list[ParamSpec]:
        return parse_params(self.params)

    @property
    def needs_params(self) -> bool:
        return any(not p.optional for p in self.param_specs)


class CommandRegistry:
    """Commands by id, the menu tree, and one QAction per command."""

    def __init__(self, trigger: Callable[[Command], None]) -> None:
        self._trigger = trigger
        self.commands: dict[str, Command] = {}
        self.order: list[str] = []
        self.actions: dict[str, QAction] = {}
        # one action per panel-scoped key, picking the command by the focused panel
        self._dispatchers: list[QAction] = []
        # the frontend's own commands by id (the window fills it): scoped keys dispatch to them too
        self.local_actions: dict[str, QAction] = {}
        # every Qt key string build_menus gave a command, panel-scoped ones included
        self.claimed_keys: set[str] = set()

    def load(self, raw: Any) -> None:
        self.commands.clear()
        self.order.clear()
        for item in raw if isinstance(raw, list) else []:
            if isinstance(item, dict):
                cmd = Command.from_json(item)
                if cmd is not None and cmd.id not in self.commands:
                    self.commands[cmd.id] = cmd
                    self.order.append(cmd.id)

    def update_enabled(self, raw: Any) -> None:
        for item in raw if isinstance(raw, list) else []:
            if not isinstance(item, dict):
                continue
            cmd = self.commands.get(str(item.get("id")))
            if cmd is None:
                continue
            cmd.enabled = bool(item.get("enabled", True))
            action = self.actions.get(cmd.id)
            if action is not None:
                action.setEnabled(cmd.enabled)

    def action(self, cid: str, parent: QWidget) -> QAction | None:
        cmd = self.commands.get(cid)
        if cmd is None:
            return None
        action = self.actions.get(cid)
        if action is None:
            text = tr(cmd.label)
            action = QAction(text, parent)
            action.setObjectName(f"command.{cid}")
            action.setToolTip(f"{text}  ({cid})")
            action.triggered.connect(lambda _checked=False, c=cmd: self._trigger(c))
            self.actions[cid] = action
        action.setEnabled(cmd.enabled)
        return action

    def build_menus(
        self,
        bar: QMenuBar,
        parent: QWidget,
        extra: dict[str, list[QAction]] | None = None,
        local_scoped: dict[str, list[tuple[str, str]]] | None = None,
        local_keys: dict[str, list[str]] | None = None,
    ) -> set[str]:
        """Fill ``bar`` from the commands' menu paths. Returns the ids given a shortcut.
        ``local_scoped``: the panel-scoped keys (keys, panel) of the commands the frontend runs
        itself, dispatched with the engine's through :attr:`local_actions`; ``local_keys``: their
        application-wide keys, which such a key runs outside its panels."""
        bar.clear()
        menus: dict[tuple[str, ...], QMenu] = {}

        def menu_for(path: tuple[str, ...]) -> QMenu:
            if path in menus:
                return menus[path]
            title = tr(path[-1])
            if len(path) == 1:
                menu = bar.addMenu(title)
            else:
                menu = menu_for(path[:-1]).addMenu(title)
            menu.setObjectName("menu." + ".".join(path))
            menus[path] = menu
            return menu

        used: set[str] = set()
        bound: set[str] = set()
        for cid in self.order:
            cmd = self.commands[cid]
            if not cmd.menu:
                continue
            action = self.action(cid, parent)
            if action is None:
                continue
            menu_for(tuple(cmd.menu)).addAction(action)
        for title, actions in (extra or {}).items():
            menu = menu_for((title,))
            if actions:
                menu.addSeparator()
                for a in actions:
                    menu.addAction(a)
        # panel-scoped keys: Qt key -> panel -> command id (the first binding per panel wins)
        scoped: dict[str, dict[str, str]] = {}
        for cid in self.order:
            for k, panel in self.commands[cid].panel_shortcuts:
                q = qt_shortcut(k)
                if q:
                    scoped.setdefault(q, {}).setdefault(panel, cid)
        for cid, binds in (local_scoped or {}).items():
            for k, panel in binds:
                q = qt_shortcut(k)
                if q:
                    scoped.setdefault(q, {}).setdefault(panel, cid)
        # the command a scoped key runs when no binding matches the focused panel
        fallback: dict[str, str] = {}
        # shortcuts: every command, menu or not; the first command to claim a key keeps it
        for cid in self.order:
            cmd = self.commands[cid]
            keys = [cmd.shortcut, *cmd.shortcuts] if cmd.shortcut else list(cmd.shortcuts)
            seqs = []
            for k in keys:
                q = qt_shortcut(k)
                if not q or q in used:
                    continue
                used.add(q)
                if q in scoped:
                    fallback[q] = cid
                    bound.add(cid)
                else:
                    seqs.append(QKeySequence(q))
            if not seqs:
                continue
            action = self.action(cid, parent)
            if action is None:
                continue
            action.setShortcuts(seqs)
            if not cmd.menu:
                parent.addAction(action)  # menu-less commands still answer their keys
            bound.add(cid)
        for old in self._dispatchers:
            old.setShortcuts([])
            owner = old.parent()
            if isinstance(owner, QWidget):
                owner.removeAction(old)
            old.setParent(None)
        self._dispatchers = []
        # a frontend command's application-wide key that is also a panel's key: run outside it
        for cid, ks in (local_keys or {}).items():
            for k in ks:
                q = qt_shortcut(k)
                if q in scoped and q not in fallback:
                    fallback[q] = cid
        for q, panels in scoped.items():
            for cid in panels.values():
                self.action(cid, parent)
                bound.add(cid)
            if q in fallback:
                self.action(fallback[q], parent)
            dispatcher = QAction(parent)
            dispatcher.setObjectName(f"shortcut.{q}")
            dispatcher.setShortcut(QKeySequence(q))
            dispatcher.triggered.connect(self._dispatcher_for(dict(panels), fallback.get(q)))
            parent.addAction(dispatcher)
            self._dispatchers.append(dispatcher)
        self.claimed_keys = {QKeySequence(q).toString() for q in used | scoped.keys()}
        return bound

    def _dispatcher_for(self, panels: dict[str, str], fallback: str | None) -> Callable[..., None]:
        return lambda *_: self._dispatch(panels, fallback)

    def _dispatch(self, panels: dict[str, str], fallback: str | None) -> None:
        """Run the command bound to a key in the focused panel, else the key's window-wide command."""
        cid = panels.get(focused_panel() or "", fallback)
        action = (self.actions.get(cid) or self.local_actions.get(cid)) if cid else None
        if action is not None:
            action.trigger()  # does nothing while the command is disabled


def focused_panel() -> str | None:
    """The engine name of the panel holding keyboard focus (``panel.Timeline`` -> ``Timeline``)."""
    w = QApplication.focusWidget()
    while w is not None:
        name = w.objectName()
        if name.startswith("panel."):
            return name.removeprefix("panel.")
        w = w.parentWidget()
    return None
