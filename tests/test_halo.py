import pytest

from grokbot_orb.bots import BOTS, GENERIC_WORKING_COLOUR
from grokbot_orb.halo import THEMES, Halo, LogBackend


def _halo() -> tuple[Halo, LogBackend]:
    backend = LogBackend()
    return Halo(count=16, brightness=1.0, backend=backend), backend


def test_state_transitions_and_bot_colours():
    halo, backend = _halo()
    assert halo.state == "idle"
    assert halo.history == ["idle"]

    halo.set_state("listening")
    halo.set_state("working:Bob")
    halo.set_state("working:Bob")
    halo.set_state("working:omabot")
    halo.set_state("working:writer")
    halo.set_state("working:research")
    halo.set_state("speaking")
    halo.set_state("error")
    halo.set_state("idle")

    assert halo.history == [
        "idle",
        "listening",
        "working:bob",
        "working:omabot",
        "working:writer",
        "working:research",
        "speaking",
        "error",
        "idle",
    ]
    assert halo.colour == THEMES["idle"].colour
    assert backend.frames, "state changes should push a frame"

    halo.set_state("working:bob")
    assert halo.colour == BOTS["bob"].colour
    halo.set_state("working:omabot")
    assert halo.colour == BOTS["omabot"].colour
    halo.set_state("working:writer")
    assert halo.colour == BOTS["writer"].colour
    halo.set_state("working:research")
    assert halo.colour == GENERIC_WORKING_COLOUR
    halo.set_state("listening")
    assert halo.colour == (255, 132, 28)


def test_unknown_state_is_rejected():
    halo, _backend = _halo()
    with pytest.raises(ValueError):
        halo.set_state("disco")


def test_listening_breathes_and_working_comets():
    halo, _backend = _halo()
    halo.set_state("listening")
    at_rest = halo.render(0.0)
    later = halo.render(0.45)
    assert at_rest != later
    assert len(set(at_rest)) == 1
    red, _green, blue = at_rest[0]
    assert red > blue

    halo.set_state("working:bob")
    pixels = halo.render(0.2)
    assert len(pixels) == 16
    assert len(set(pixels)) > 1


def test_brightness_caps_the_frame():
    halo, _backend = _halo()
    halo.brightness = 0.0
    # brightness must stay positive on the real config; the renderer still clamps.
    halo.brightness = 0.01
    pixel = halo.render(0.0)[0]
    assert max(pixel) < 20
