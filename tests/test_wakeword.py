import numpy as np

from grokbot_orb.wakeword import WakeDetector, describe_wake_model


class Clock:
    def __init__(self) -> None:
        self.t = 0.0

    def __call__(self) -> float:
        return self.t


class FakeModel:
    def __init__(self, score: float, name: str = "hey_jarvis") -> None:
        self.score = score
        self.name = name

    def predict(self, frame: np.ndarray) -> dict[str, float]:
        assert frame.dtype == np.int16
        return {self.name: self.score}


def test_threshold_and_cooldown():
    clock = Clock()
    frame = np.zeros(1280, dtype=np.int16)
    detector = WakeDetector(FakeModel(0.9), threshold=0.5, cooldown_s=2.0, clock=clock)
    assert detector.feed(frame) == "hey_jarvis"
    clock.t = 1.0
    assert detector.feed(frame) is None
    clock.t = 2.1
    assert detector.feed(frame) == "hey_jarvis"

    quiet = WakeDetector(FakeModel(0.2), threshold=0.5, cooldown_s=0.0, clock=clock)
    assert quiet.feed(frame) is None


def test_describe_missing_model_names_the_fallback(tmp_path):
    missing = tmp_path / "hey_grok_bot.onnx"
    assert "hey_jarvis" in describe_wake_model(str(missing), "hey_jarvis")
    present = tmp_path / "hey_grok_bot.onnx"
    present.write_bytes(b"not a real model")
    assert describe_wake_model(str(present), "hey_jarvis") == "hey_grok_bot"
