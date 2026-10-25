import numpy as np

from grokbot_orb.audio import PcmRing, resample_pcm16


def test_resample_known_ratios_and_identity():
    silent = b"\x00\x00" * 480
    down = resample_pcm16(silent, 48_000, 16_000)
    assert len(down) == 160 * 2

    tone = np.array([1000, -1000] * 160, dtype="<i2")
    up = resample_pcm16(tone.tobytes(), 16_000, 24_000)
    assert len(up) // 2 == 480

    half = resample_pcm16(b"\x00\x10" * 480, 48_000, 24_000)
    assert len(half) // 2 == 240

    original = np.array([1, -2, 30_000], dtype="<i2").tobytes()
    assert resample_pcm16(original, 16_000, 16_000) == original
    assert resample_pcm16(b"", 16_000, 24_000) == b""
    assert len(resample_pcm16(b"\x01\x02\x03", 16_000, 16_000)) == 2


def test_resample_holds_a_constant_level():
    samples = np.full(120, 1000, dtype="<i2")
    out = np.frombuffer(resample_pcm16(samples.tobytes(), 16_000, 48_000), dtype="<i2")
    assert abs(int(out.mean()) - 1000) < 5


def test_ring_keeps_the_newest_samples():
    ring = PcmRing(4)
    first = np.array([1, 2, 3, 4, 5], dtype="<i2").tobytes()
    ring.extend(first)
    assert list(np.frombuffer(ring.snapshot(), dtype="<i2")) == [2, 3, 4, 5]
    ring.extend(np.array([6, 7], dtype="<i2").tobytes())
    assert list(np.frombuffer(ring.snapshot(), dtype="<i2")) == [4, 5, 6, 7]
    ring.extend(b"")
    assert list(np.frombuffer(ring.snapshot(), dtype="<i2")) == [4, 5, 6, 7]
