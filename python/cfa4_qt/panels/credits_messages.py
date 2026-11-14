"""The credits engine's spreadsheet and layout notes in the user's language.

The engine (``crates/credits``) writes its notes in English with the values in backticks
(``page style `Roll` does not exist; `Scroll` is used``). Each known note is matched here and
shown from a translatable template with the values filled in; a note nothing matches (a new one,
say) stays as the engine wrote it.
"""

from __future__ import annotations

import re

from ..i18n import N_, tr

V = r"`(?P<{}>[^`]*)`"  # a value the engine puts in backticks


def _p(pattern: str) -> re.Pattern[str]:
    return re.compile("^" + pattern + "$")


# (engine note, template with the same named values)
NOTES: tuple[tuple[re.Pattern[str], str], ...] = (
    (_p(re.escape("a picture or video tag stands alone in a body cell (or with one name, as a portrait); the other tags were removed")),
     N_("a picture or video tag stands alone in a body cell (or with one name, as a portrait); the other tags were removed")),
    (_p(re.escape("the page gap is not a duration; the page style's is used")), N_("the page gap is not a duration; the page style's is used")),
    (_p(re.escape("the page runtime is not a positive duration; the page style's is used")), N_("the page runtime is not a positive duration; the page style's is used")),
    (_p("background " + V.format("file") + " was not found; it is left out"), N_("background “{file}” was not found; it is left out")),
    (_p(r"card page (?P<page>\d+) is taller than the frame; use a Scroll page"), N_("card page {page} is taller than the frame; use a roll page")),
    (_p(r"page (?P<page>\d+): a spine's vertical position works on card pages only; it is ignored"),
     N_("page {page}: a spine's vertical position works on card pages only; it is ignored")),
    (_p("colour " + V.format("colour") + " of letter style " + V.format("style") + " is not #rrggbb; it is left out"),
     N_("colour “{colour}” of letter style “{style}” is not #rrggbb; it is left out")),
    (_p("colour " + V.format("colour") + " is not #rrggbb; black is used"), N_("colour “{colour}” is not #rrggbb; black is used")),
    (_p("colour " + V.format("colour") + " is not #rrggbb; white is used"), N_("colour “{colour}” is not #rrggbb; white is used")),
    (_p("content style " + V.format("style") + " does not exist; " + V.format("used") + " is used"), N_("content style “{style}” does not exist; “{used}” is used")),
    (_p("letter style " + V.format("style") + " does not exist; the text keeps the block's style"), N_("letter style “{style}” does not exist; the text keeps the block's style")),
    (_p("page gap " + V.format("value") + " is not a duration"), N_("page gap “{value}” is not a duration")),
    (_p("page runtime " + V.format("value") + " is not a duration"), N_("page runtime “{value}” is not a duration")),
    (_p("page style " + V.format("style") + " does not exist; `Scroll` is used"), N_("page style “{style}” does not exist; a roll page is used")),
    (_p(r"page (?P<page>\d+): the background names no file; it is left out"), N_("page {page}: the background names no file; it is left out")),
    (_p(r"page (?P<page>\d+): the runtime is reached by changing the scroll speed \(the page has no gaps between blocks to stretch\)"),
     N_("page {page}: the runtime is reached by changing the roll speed (the page has no gaps between blocks to stretch)")),
    (_p(r"page (?P<page>\d+): the runtime is reached by changing the scroll speed \(the credits are too long even without gaps\)"),
     N_("page {page}: the runtime is reached by changing the roll speed (the credits are too long even without gaps)")),
    (_p(r"page (?P<page>\d+): the runtime is reached by changing the scroll speed \(the gaps would have to grow more than 20-fold\)"),
     N_("page {page}: the runtime is reached by changing the roll speed (the gaps would have to grow more than 20-fold)")),
    (_p(r"page (?P<page>\d+): the transition needs the pages to touch \(page gap 0\); it is left out"),
     N_("page {page}: the transition needs the pages to touch (page gap 0); it is left out")),
    (_p(r"page (?P<page>\d+): there is no transition " + V.format("name") + "; the pages cut"), N_("page {page}: there is no transition “{name}”; the pages cut")),
    (_p("spine position " + V.format("value") + r" is not supported \(a pixel offset, optionally `parallel`, or `Hook N Bottom-Top dx dy` is\)"),
     N_("spine position “{value}” is not supported (a pixel offset, optionally “parallel”, or “Hook N Bottom-Top dx dy” is)")),
    (_p(r"page (?P<page>\d+): the hook reaches back further than there are spines; it hooks onto the first spine, or starts its own"),
     N_("page {page}: the hook reaches back further than there are spines; it hooks onto the first spine, or starts its own")),
    (_p("vertical gap " + V.format("value") + " is not a number"), N_("vertical gap “{value}” is not a number")),
    (_p("picture " + V.format("file") + " was not found or cannot be read; it is left out"), N_("picture “{file}” was not found or cannot be read; it is left out")),
    (_p("video " + V.format("file") + " was not found or cannot be read; it is left out"), N_("video “{file}” was not found or cannot be read; it is left out")),
    (_p(re.escape("there is no scroll page without its own runtime to stretch; the length cannot change")),
     N_("there is no roll page without its own runtime to stretch; the length cannot change")),
    (_p(r"the cards and gaps alone are (?P<seconds>[0-9.]+) s; the asked length is shorter, so the scroll pages keep their natural length"),
     N_("the cards and gaps alone are {seconds} s; the asked length is shorter, so the roll pages keep their natural length")),
    (_p("letter style " + V.format("style") + r": only the first coloured layer is used \(no outlines, shadows or stripes\)"),
     N_("letter style “{style}”: only the first coloured layer is used (no outlines, shadows or stripes)")),
    (_p(re.escape("the whole-credits runtime is not carried over; use Fit Credits to Music (or a length) instead")),
     N_("the whole-credits runtime is not carried over; use Fit Credits to Music (or a length) instead")),
    (_p("page style " + V.format("style") + ": fade curves become linear fades"), N_("page style “{style}”: fade curves become linear fades")),
    (_p("content style " + V.format("style") + " has no head in Cinecred; heads written in the sheet are still shown"),
     N_("content style “{style}” has no head in Cinecred; heads written in the sheet are still shown")),
    (_p("content style " + V.format("style") + ": sorting the names is not carried over"), N_("content style “{style}”: sorting the names is not carried over")),
    (_p(re.escape("transition curves are not carried over")), N_("transition curves are not carried over")),
    (_p(re.escape("picture styles (pictures keep their own size) are not carried over")), N_("picture styles (pictures keep their own size) are not carried over")),
    (_p(re.escape("video (tape) styles are not carried over")), N_("video (tape) styles are not carried over")),
    (_p(r"the Cinecred project has (?P<n>\d+) credits spreadsheets; " + V.format("name") + r" is used \(choose another with `credits`\)"),
     N_("the Cinecred project has {n} credits spreadsheets; “{name}” is used (choose another in the Spreadsheet field)")),
)


def note_text(message: str) -> str:
    """An engine note in the user's language (unchanged when it is not one CFA4 knows)."""
    text = str(message)
    for pattern, template in NOTES:
        m = pattern.match(text)
        if m:
            out = tr(template)
            for key, value in m.groupdict().items():
                out = out.replace("{" + key + "}", value)
            return out
    return text
