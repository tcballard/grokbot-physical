"""Environment configuration for the orb daemon."""

from __future__ import annotations

import os
from dataclasses import dataclass, replace
from pathlib import Path
from typing import Any, Mapping


def _env_bool(value: str | None, default: bool) -> bool:
    if value is None or value.strip() == "":
        return default
    return value.strip().lower() in {"1", "true", "yes", "on"}


def _load_env_file(path: Path, environ: dict[str, str]) -> None:
    """Fill keys that are not already set. Process environment wins."""
    text = path.read_text(encoding="utf-8")
    for raw in text.splitlines():
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        if line.startswith("export "):
            line = line[len("export ") :].strip()
        key, sep, value = line.partition("=")
        if not sep:
            continue
        key = key.strip()
        value = value.strip()
        if len(value) >= 2 and value[0] == value[-1] and value[0] in {'"', "'"}:
            value = value[1:-1]
        environ.setdefault(key, value)


@dataclass(frozen=True)
class Config:
    xai_api_key: str = ""
    xai_realtime_model: str = "grok-voice-latest"
    xai_realtime_url: str = ""
    xai_voice: str = "eve"
    xai_tts_url: str = "https://api.x.ai/v1/tts"
    webhook_url: str = ""
    webhook_bearer: str = ""
    automation_key: str = ""
    callback_host: str = "0.0.0.0"
    callback_port: int = 8765
    public_base: str = ""
    speak_token: str = ""
    wake_model_path: str = "models/hey_grok_bot.onnx"
    wake_fallback: str = "hey_jarvis"
    wake_threshold: float = 0.5
    wake_cooldown_s: float = 2.0
    audio_input_device: str = ""
    audio_output_device: str = ""
    audio_rate: int = 0
    barge_in: bool = True
    session_idle_s: float = 20.0
    halo_pin: int = 12
    halo_channel: int = 0
    halo_count: int = 16
    halo_brightness: float = 0.35
    halo_backend: str = "auto"
    tts_fallback: str = "auto"
    dry_run: bool = False
    simulate_wake: bool = False
    simulate_text: str = ""
    simulate_bot: str = "bob"
    log_level: str = "INFO"
    webhook_timeout_s: float = 20.0

    def __post_init__(self) -> None:
        if not 0.0 < self.wake_threshold <= 1.0:
            raise ValueError("WAKE_THRESHOLD must be greater than 0 and at most 1")
        if self.wake_cooldown_s < 0:
            raise ValueError("WAKE_COOLDOWN_S must be >= 0")
        if self.callback_port != 0 and not 1 <= self.callback_port <= 65535:
            raise ValueError("ORB_CALLBACK_PORT must be 0 or in 1..65535")
        if not 0.0 < self.halo_brightness <= 1.0:
            raise ValueError("HALO_BRIGHTNESS must be greater than 0 and at most 1")
        if self.halo_count < 1:
            raise ValueError("HALO_COUNT must be >= 1")
        if self.session_idle_s <= 0:
            raise ValueError("SESSION_IDLE_S must be > 0")
        if self.tts_fallback not in {"auto", "espeak", "none"}:
            raise ValueError("TTS_FALLBACK must be auto, espeak, or none")
        if self.halo_backend not in {"auto", "log", "ws281x"}:
            raise ValueError("HALO_BACKEND must be auto, log, or ws281x")
        if self.audio_rate < 0:
            raise ValueError("AUDIO_RATE must be >= 0")
        if self.webhook_timeout_s <= 0:
            raise ValueError("webhook timeout must be > 0")

    def realtime_url(self) -> str:
        if self.xai_realtime_url.strip():
            return self.xai_realtime_url.strip()
        return f"wss://api.x.ai/v1/realtime?model={self.xai_realtime_model}"

    @classmethod
    def from_env(
        cls,
        environ: Mapping[str, str] | None = None,
        env_file: str | Path | None = None,
    ) -> Config:
        env = dict(os.environ if environ is None else environ)
        if env_file:
            _load_env_file(Path(env_file), env)

        def raw(name: str, default: str = "") -> str:
            value = env.get(name)
            if value is None:
                return default
            return value.strip()

        pin = int(raw("HALO_PIN", "12") or "12")
        channel_raw = env.get("HALO_CHANNEL")
        if channel_raw is None or channel_raw.strip() == "":
            channel = 1 if pin in {13, 19} else 0
        else:
            channel = int(channel_raw.strip())

        return cls(
            xai_api_key=raw("XAI_API_KEY"),
            xai_realtime_model=raw("XAI_REALTIME_MODEL", "grok-voice-latest") or "grok-voice-latest",
            xai_realtime_url=raw("XAI_REALTIME_URL"),
            xai_voice=raw("XAI_VOICE", "eve") or "eve",
            xai_tts_url=raw("XAI_TTS_URL", "https://api.x.ai/v1/tts") or "https://api.x.ai/v1/tts",
            webhook_url=raw("GROKBOT_WEBHOOK_URL"),
            webhook_bearer=raw("GROKBOT_WEBHOOK_BEARER"),
            automation_key=raw("GROKBOT_AUTOMATION_KEY"),
            callback_host=raw("ORB_CALLBACK_HOST", "0.0.0.0") or "0.0.0.0",
            callback_port=int(raw("ORB_CALLBACK_PORT", "8765") or "8765"),
            public_base=raw("ORB_PUBLIC_BASE"),
            speak_token=raw("ORB_SPEAK_TOKEN"),
            wake_model_path=raw("WAKE_MODEL_PATH", "models/hey_grok_bot.onnx")
            or "models/hey_grok_bot.onnx",
            wake_fallback=raw("WAKE_FALLBACK", "hey_jarvis") or "hey_jarvis",
            wake_threshold=float(raw("WAKE_THRESHOLD", "0.5") or "0.5"),
            wake_cooldown_s=float(raw("WAKE_COOLDOWN_S", "2.0") or "2.0"),
            audio_input_device=raw("AUDIO_INPUT_DEVICE"),
            audio_output_device=raw("AUDIO_OUTPUT_DEVICE"),
            audio_rate=int(raw("AUDIO_RATE", "0") or "0"),
            barge_in=_env_bool(env.get("BARGE_IN"), True),
            session_idle_s=float(raw("SESSION_IDLE_S", "20") or "20"),
            halo_pin=pin,
            halo_channel=channel,
            halo_count=int(raw("HALO_COUNT", "16") or "16"),
            halo_brightness=float(raw("HALO_BRIGHTNESS", "0.35") or "0.35"),
            halo_backend=(raw("HALO_BACKEND", "auto") or "auto").lower(),
            tts_fallback=(raw("TTS_FALLBACK", "auto") or "auto").lower(),
            dry_run=_env_bool(env.get("DRY_RUN"), False),
            simulate_wake=_env_bool(env.get("SIMULATE_WAKE"), False),
            simulate_text=raw("SIMULATE_TEXT"),
            simulate_bot=raw("SIMULATE_BOT", "bob") or "bob",
            log_level=(raw("LOG_LEVEL", "INFO") or "INFO").upper(),
            webhook_timeout_s=float(raw("WEBHOOK_TIMEOUT_S", "20") or "20"),
        )

    @classmethod
    def for_test(cls, **overrides: Any) -> Config:
        base = cls(
            dry_run=True,
            callback_host="127.0.0.1",
            callback_port=0,
            tts_fallback="none",
            halo_backend="log",
            public_base="http://orb.local:8765",
            simulate_bot="bob",
        )
        return replace(base, **overrides)
