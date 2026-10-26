"""Downward 16-LED NeoPixel halo.

The ring faces the counter. Listening is the warm orange from the concept
render. While a Grok Bot works, the ring takes that bot's colour and walks
a single brighter LED around the circle. On a laptop, or whenever the
ws281x driver cannot open, the same state machine logs transitions and
renders frames in memory.
"""

from __future__ import annotations

import asyncio
import logging
import math
from dataclasses import dataclass

from grokbot_orb.bots import BOTS, GENERIC_WORKING_COLOUR, normalise_bot_name

logger = logging.getLogger("grokbot_orb.halo")


@dataclass(frozen=True)
class Theme:
    colour: tuple[int, int, int]
    period_s: float
    min_level: float
    max_level: float
    comet: bool = False

    def level(self, t: float) -> float:
        phase = (math.sin(2.0 * math.pi * t / self.period_s) + 1.0) / 2.0
        return self.min_level + (self.max_level - self.min_level) * phase

    def levels(self, t: float, count: int) -> list[float]:
        base = self.level(t)
        if not self.comet or count <= 1:
            return [base] * count
        head = (t / self.period_s) % 1.0
        width = 1.2 / count
        out: list[float] = []
        for index in range(count):
            pos = index / count
            dist = abs(pos - head)
            dist = min(dist, 1.0 - dist)
            bump = 1.0 - dist / width if dist < width else 0.0
            out.append(max(base * 0.55, bump))
        return out


# idle is a dim coal so the sphere still pools a little light at rest.
THEMES: dict[str, Theme] = {
    "idle": Theme((92, 44, 14), period_s=4.2, min_level=0.08, max_level=0.28),
    "listening": Theme((255, 132, 28), period_s=1.25, min_level=0.35, max_level=1.0),
    "speaking": Theme((255, 214, 170), period_s=0.9, min_level=0.45, max_level=1.0),
    "working": Theme(
        GENERIC_WORKING_COLOUR,
        period_s=1.6,
        min_level=0.28,
        max_level=0.9,
        comet=True,
    ),
    "error": Theme((220, 40, 40), period_s=0.42, min_level=0.15, max_level=1.0),
}


def canonical_state(state: str) -> str:
    key = state.strip().lower()
    if key in THEMES:
        return key
    if key.startswith("working:"):
        bot = normalise_bot_name(key.split(":", 1)[1])
        return f"working:{bot}"
    raise ValueError(f"unknown halo state {state!r}")


def theme_for(state: str) -> Theme:
    key = canonical_state(state)
    if key in THEMES and not key.startswith("working:"):
        return THEMES[key]
    if key == "working":
        return THEMES["working"]
    bot_name = key.split(":", 1)[1]
    bot = BOTS.get(bot_name)
    colour = bot.colour if bot else GENERIC_WORKING_COLOUR
    return Theme(colour, period_s=1.6, min_level=0.28, max_level=0.95, comet=True)


class HaloBackend:
    def write(self, pixels: list[tuple[int, int, int]]) -> None:
        del pixels

    def show(self) -> None:
        return

    def close(self) -> None:
        return


class LogBackend(HaloBackend):
    """In-memory frame buffer used off the Pi and in tests."""

    def __init__(self) -> None:
        self.frames: list[list[tuple[int, int, int]]] = []

    def write(self, pixels: list[tuple[int, int, int]]) -> None:
        self.frames.append(list(pixels))

    def show(self) -> None:
        return


class Ws281xBackend(HaloBackend):
    def __init__(self, pin: int, count: int, channel: int) -> None:
        from rpi_ws281x import Color, PixelStrip, WS2811_STRIP_GRB

        self._color = Color
        self.count = count
        # Software scales brightness. The strip stays at full scale so a
        # theme's min/max levels mean the same thing on the Pi and in tests.
        self.strip = PixelStrip(
            count,
            pin,
            800_000,
            10,
            False,
            255,
            channel,
            WS2811_STRIP_GRB,
        )
        self.strip.begin()

    def write(self, pixels: list[tuple[int, int, int]]) -> None:
        for index, (red, green, blue) in enumerate(pixels[: self.count]):
            self.strip.setPixelColor(index, self._color(red, green, blue))

    def show(self) -> None:
        self.strip.show()


def open_backend(
    *,
    backend: str,
    pin: int,
    count: int,
    channel: int,
    dry_run: bool,
) -> HaloBackend:
    choice = backend
    if choice == "auto" and dry_run:
        choice = "log"
    if choice == "log":
        return LogBackend()
    try:
        logger.info("opening NeoPixel ring pin=%s count=%s channel=%s", pin, count, channel)
        return Ws281xBackend(pin, count, channel)
    except Exception as exc:
        logger.warning("NeoPixel backend unavailable (%s); halo will log state changes", exc)
        return LogBackend()


def _scale(
    colour: tuple[int, int, int],
    level: float,
    brightness: float,
) -> tuple[int, int, int]:
    gain = max(0.0, min(1.0, level * brightness))
    return tuple(max(0, min(255, int(round(channel * gain)))) for channel in colour)  # type: ignore[return-value]


class Halo:
    def __init__(
        self,
        *,
        count: int = 16,
        brightness: float = 0.35,
        backend: HaloBackend | None = None,
        frame_s: float = 1.0 / 30.0,
    ) -> None:
        if count < 1:
            raise ValueError("LED count must be >= 1")
        self.count = count
        self.brightness = brightness
        self.backend = backend if backend is not None else LogBackend()
        self.frame_s = frame_s
        self.state = "idle"
        self.history: list[str] = ["idle"]
        self._t0 = _now()
        self._failed = False

    @property
    def colour(self) -> tuple[int, int, int]:
        return theme_for(self.state).colour

    def set_state(self, state: str) -> str:
        resolved = canonical_state(state)
        if resolved != self.state:
            self.state = resolved
            self.history.append(resolved)
            red, green, blue = self.colour
            logger.info("halo %s rgb(%d, %d, %d)", resolved, red, green, blue)
            self.flush()
        return resolved

    def render(self, t: float | None = None) -> list[tuple[int, int, int]]:
        if t is None:
            t = _now() - self._t0
        theme = theme_for(self.state)
        return [_scale(theme.colour, level, self.brightness) for level in theme.levels(t, self.count)]

    def flush(self, t: float | None = None) -> None:
        if self._failed:
            return
        try:
            self.backend.write(self.render(t))
            self.backend.show()
        except Exception:
            logger.exception("halo write failed; switching to the log backend")
            self._failed = True
            self.backend = LogBackend()

    async def run(self) -> None:
        while True:
            self.flush()
            await asyncio.sleep(self.frame_s)

    def close(self) -> None:
        try:
            dark = [(0, 0, 0)] * self.count
            self.backend.write(dark)
            self.backend.show()
            self.backend.close()
        except Exception:
            logger.debug("halo close failed", exc_info=True)


def _now() -> float:
    import time

    return time.monotonic()
