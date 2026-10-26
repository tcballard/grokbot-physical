"""Local openWakeWord detection. Audio stays on the device until a hit."""

from __future__ import annotations

import logging
from pathlib import Path
from typing import Callable, Protocol

import numpy as np

logger = logging.getLogger("grokbot_orb.wake")


class WakeModel(Protocol):
    def predict(self, frame: np.ndarray) -> dict[str, float]: ...


class WakeDetector:
    """Score 16 kHz frames and fire once per cooldown window."""

    def __init__(
        self,
        model: WakeModel,
        *,
        threshold: float,
        cooldown_s: float,
        clock: Callable[[], float] | None = None,
    ) -> None:
        if not 0.0 < threshold <= 1.0:
            raise ValueError("threshold must be in (0, 1]")
        self._model = model
        self.threshold = threshold
        self.cooldown_s = cooldown_s
        self._clock = clock or _monotonic
        self._last_fire = -1e9

    def feed(self, frame: np.ndarray) -> str | None:
        scores = self._model.predict(frame)
        if not scores:
            return None
        name = max(scores, key=scores.get)
        score = float(scores[name])
        now = self._clock()
        if score < self.threshold:
            return None
        if now - self._last_fire < self.cooldown_s:
            logger.debug("wake %s score %.3f suppressed by cooldown", name, score)
            return None
        self._last_fire = now
        logger.info("wake %s score %.3f", name, score)
        return name


def load_openwakeword(path: str, fallback: str) -> tuple[WakeModel, str]:
    """Load hey_grok_bot.onnx, or a pretrained model such as hey_jarvis."""
    try:
        from openwakeword.model import Model
    except ImportError as exc:
        raise RuntimeError(
            "openwakeword is not installed. On the orb run: pip install -e '.[pi]'"
        ) from exc

    model_path = Path(path) if path else None
    if model_path is not None and model_path.is_file():
        logger.info("wake word file %s", model_path)
        model = Model(wakeword_models=[str(model_path)], inference_framework="onnx")
        return model, model_path.stem

    logger.warning(
        "wake model %s is not on disk; using pretrained %s. "
        "Train hey_grok_bot.onnx with the openWakeWord notebook and place it at that path.",
        path or "(unset)",
        fallback,
    )
    try:
        model = Model(wakeword_models=[fallback], inference_framework="onnx")
    except Exception:
        logger.exception("onnx backend failed; retrying the default openWakeWord backend")
        model = Model(wakeword_models=[fallback])
    return model, fallback


def describe_wake_model(path: str, fallback: str) -> str:
    model_path = Path(path) if path else None
    if model_path is not None and model_path.is_file():
        return model_path.stem
    return f"{fallback} (fallback)"


def _monotonic() -> float:
    import time

    return time.monotonic()
