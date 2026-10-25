"""Named Grok Bots the orb can hand work to, and the halo colour for each."""

from __future__ import annotations

import re
from dataclasses import dataclass


@dataclass(frozen=True)
class Bot:
    name: str
    title: str
    colour: tuple[int, int, int]
    summary: str
    aliases: tuple[str, ...] = ()


# Colours are the resting RGB of the 16-LED ring while that bot is working.
BOTS: dict[str, Bot] = {
    "bob": Bot(
        "bob",
        "Bob",
        (40, 130, 255),
        "general operator for research, calendar, messages, and anything else",
    ),
    "omabot": Bot(
        "omabot",
        "OmaBot",
        (16, 185, 110),
        "home and household",
        aliases=("oma", "omabot"),
    ),
    "writer": Bot(
        "writer",
        "Writer",
        (168, 85, 247),
        "drafts and edits",
        aliases=("write",),
    ),
}

GENERIC_WORKING_COLOUR: tuple[int, int, int] = (170, 186, 204)


def _alias_map() -> dict[str, str]:
    mapping: dict[str, str] = {}
    for bot in BOTS.values():
        mapping[_compact(bot.name)] = bot.name
        for alias in bot.aliases:
            mapping[_compact(alias)] = bot.name
    return mapping


def _compact(name: str) -> str:
    return re.sub(r"[^a-z0-9]", "", name.strip().lower())


_ALIASES = _alias_map()


def normalise_bot_name(name: str) -> str:
    """Return a canonical bot id. Unknown names pass through, compacted."""
    key = _compact(name or "")
    if not key:
        raise ValueError("bot_name is empty")
    if key in _ALIASES:
        return _ALIASES[key]
    return key[:32]


def bot_roster() -> str:
    return "; ".join(f"{bot.name} ({bot.summary})" for bot in BOTS.values())
