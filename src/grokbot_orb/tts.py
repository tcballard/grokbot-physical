"""Speak a callback when no Realtime session is open."""

from __future__ import annotations

import asyncio
import io
import logging
import shutil
import wave

import aiohttp

logger = logging.getLogger("grokbot_orb.tts")


async def synthesize(
    text: str,
    *,
    api_key: str,
    url: str,
    voice: str,
    fallback: str,
) -> tuple[bytes, int]:
    """Return mono PCM16 and its sample rate. Empty bytes means nothing to play."""
    if api_key:
        try:
            return await _xai_tts(text, api_key=api_key, url=url, voice=voice)
        except Exception:
            logger.exception("xAI TTS failed")
            if fallback == "none":
                return b"", 24_000
    if fallback == "none":
        logger.info("spoken (no audio device path): %s", text)
        return b"", 24_000
    if fallback in {"auto", "espeak"}:
        spoken = await _espeak(text)
        if spoken[0]:
            return spoken
        if fallback == "espeak":
            logger.error("espeak-ng is not available")
        else:
            logger.info("spoken (no TTS backend): %s", text)
    return b"", 24_000


async def _xai_tts(text: str, *, api_key: str, url: str, voice: str) -> tuple[bytes, int]:
    payload = {
        "text": text,
        "voice_id": voice,
        "language": "en",
        "output_format": {"codec": "pcm", "sample_rate": 24_000},
    }
    headers = {
        "Authorization": f"Bearer {api_key}",
        "Content-Type": "application/json",
    }
    timeout = aiohttp.ClientTimeout(total=30)
    async with aiohttp.ClientSession(timeout=timeout) as session:
        async with session.post(url, json=payload, headers=headers) as response:
            body = await response.read()
            if response.status >= 400:
                raise RuntimeError(f"TTS HTTP {response.status}: {body[:200]!r}")
    if not body:
        raise RuntimeError("TTS returned an empty body")
    # PCM is raw. A WAV wrapper (some proxies) is unpacked.
    if body[:4] == b"RIFF":
        return _wav_pcm(body)
    return body, 24_000


async def _espeak(text: str) -> tuple[bytes, int]:
    exe = shutil.which("espeak-ng") or shutil.which("espeak")
    if not exe:
        return b"", 22_050
    proc = await asyncio.create_subprocess_exec(
        exe,
        "--stdout",
        "-s",
        "155",
        text,
        stdout=asyncio.subprocess.PIPE,
        stderr=asyncio.subprocess.DEVNULL,
    )
    wav, _ = await proc.communicate()
    if proc.returncode != 0 or not wav:
        return b"", 22_050
    try:
        return _wav_pcm(wav)
    except (wave.Error, EOFError):
        logger.exception("espeak did not return a wav")
        return b"", 22_050


def _wav_pcm(data: bytes) -> tuple[bytes, int]:
    with wave.open(io.BytesIO(data), "rb") as handle:
        rate = handle.getframerate()
        width = handle.getsampwidth()
        channels = handle.getnchannels()
        frames = handle.readframes(handle.getnframes())
    if width != 2:
        return b"", rate
    if channels == 1:
        return frames, rate
    import numpy as np

    samples = np.frombuffer(frames, dtype="<i2")
    mono = samples.reshape(-1, channels)[:, 0]
    return mono.astype("<i2").tobytes(), rate
