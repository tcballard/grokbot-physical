"""PCM16 helpers: resample, a short ring, and a speaker that can stay silent."""

from __future__ import annotations

import asyncio
import logging

import numpy as np

logger = logging.getLogger("grokbot_orb.audio")

WAKE_RATE = 16_000
REALTIME_RATE = 24_000
WAKE_FRAME_SAMPLES = 1280  # 80 ms at 16 kHz, the frame openWakeWord consumes


def resample_pcm16(pcm: bytes, src_rate: int, dst_rate: int) -> bytes:
    """Linear resample of little-endian mono PCM16.

    16 kHz, 24 kHz, and 48 kHz are the rates this orb actually meets: the
    wake word wants 16 kHz, the Realtime API wants 24 kHz, and the ReSpeaker
    card often enumerates at 48 kHz. Ratios between those are exact.
    """
    if src_rate <= 0 or dst_rate <= 0:
        raise ValueError("sample rates must be positive")
    if len(pcm) % 2:
        pcm = pcm[:-1]
    if not pcm or src_rate == dst_rate:
        return pcm
    samples = np.frombuffer(pcm, dtype="<i2").astype(np.float64)
    if samples.size == 1:
        return samples.astype("<i2").tobytes()
    n_out = max(1, int(round(samples.size * dst_rate / src_rate)))
    src_x = np.linspace(0.0, 1.0, num=samples.size, endpoint=False)
    dst_x = np.linspace(0.0, 1.0, num=n_out, endpoint=False)
    out = np.interp(dst_x, src_x, samples)
    clipped = np.clip(np.rint(out), -32768, 32767).astype("<i2")
    return clipped.tobytes()


class PcmRing:
    """Fixed-capacity little-endian PCM16 ring, oldest sample first on snapshot."""

    def __init__(self, capacity_samples: int) -> None:
        if capacity_samples < 1:
            raise ValueError("ring capacity must be >= 1")
        self._buf = np.zeros(capacity_samples, dtype=np.int16)
        self._i = 0
        self._n = 0

    @property
    def capacity(self) -> int:
        return int(self._buf.size)

    def extend(self, pcm: bytes) -> None:
        if not pcm:
            return
        if len(pcm) % 2:
            pcm = pcm[:-1]
        samples = np.frombuffer(pcm, dtype="<i2")
        if samples.size == 0:
            return
        cap = self.capacity
        if samples.size >= cap:
            self._buf[:] = samples[-cap:]
            self._i = 0
            self._n = cap
            return
        end = self._i + int(samples.size)
        if end <= cap:
            self._buf[self._i : end] = samples
        else:
            first = cap - self._i
            self._buf[self._i :] = samples[:first]
            self._buf[: end % cap] = samples[first:]
        self._i = end % cap
        self._n = min(cap, self._n + int(samples.size))

    def snapshot(self) -> bytes:
        if self._n == 0:
            return b""
        if self._n < self.capacity:
            return self._buf[: self._n].astype("<i2", copy=False).tobytes()
        ordered = np.concatenate((self._buf[self._i :], self._buf[: self._i]))
        return ordered.astype("<i2", copy=False).tobytes()


class PcmOutput:
    """Play mono PCM16. With no ALSA device, bytes are kept for tests."""

    def __init__(self, *, dry_run: bool, device: str, rate: int) -> None:
        self.dry_run = dry_run
        self.device = device or None
        self.rate = rate if rate > 0 else WAKE_RATE
        self.played = bytearray()
        self._stream: object | None = None
        self._lock = asyncio.Lock()
        self._warned = False

    async def open(self) -> None:
        if self.dry_run:
            return
        try:
            self._stream = await asyncio.to_thread(self._open_stream)
        except Exception:
            logger.exception("speaker did not open; assistant audio will be dropped")
            self._stream = None

    def _open_stream(self) -> object:
        import sounddevice as sd

        stream = sd.RawOutputStream(
            samplerate=self.rate,
            channels=1,
            dtype="int16",
            device=self.device,
        )
        stream.start()
        logger.info("speaker open at %s Hz device=%s", self.rate, self.device or "default")
        return stream

    async def write(self, pcm: bytes) -> None:
        if not pcm:
            return
        async with self._lock:
            stream = self._stream
            if stream is None:
                if self.dry_run:
                    self.played.extend(pcm)
                elif not self._warned:
                    self._warned = True
                    logger.warning("dropping assistant audio; no output device")
                return
            await asyncio.to_thread(stream.write, pcm)  # type: ignore[attr-defined]

    async def close(self) -> None:
        stream = self._stream
        self._stream = None
        if stream is not None:
            await asyncio.to_thread(stream.close)  # type: ignore[attr-defined]


class PcmInput:
    """ALSA capture into an asyncio queue. Overruns drop the oldest block."""

    def __init__(self, device: str, rate: int, block_ms: int = 20) -> None:
        self.device = device or None
        self.rate = rate
        self.block_ms = block_ms
        self._queue: asyncio.Queue[bytes] = asyncio.Queue(maxsize=32)
        self._stream: object | None = None

    async def start(self) -> int:
        import sounddevice as sd

        rate = self.rate
        if rate <= 0:
            info = sd.query_devices(self.device, "input")
            rate = int(round(float(info["default_samplerate"])))
        self.rate = rate
        loop = asyncio.get_running_loop()
        block = max(1, int(rate * self.block_ms / 1000))

        def callback(indata: bytes, frames: int, time_info: object, status: object) -> None:
            del frames, time_info
            if status:
                logger.warning("microphone: %s", status)
            data = bytes(indata)
            try:
                loop.call_soon_threadsafe(self._offer, data)
            except RuntimeError:
                return

        self._stream = sd.RawInputStream(
            samplerate=rate,
            channels=1,
            dtype="int16",
            device=self.device,
            blocksize=block,
            callback=callback,
        )
        self._stream.start()  # type: ignore[attr-defined]
        logger.info("microphone open at %s Hz device=%s", rate, self.device or "default")
        return rate

    def _offer(self, data: bytes) -> None:
        if self._queue.full():
            try:
                self._queue.get_nowait()
            except asyncio.QueueEmpty:
                pass
        self._queue.put_nowait(data)

    async def read(self) -> bytes:
        return await self._queue.get()

    def close(self) -> None:
        stream = self._stream
        self._stream = None
        if stream is not None:
            stream.stop()  # type: ignore[attr-defined]
            stream.close()  # type: ignore[attr-defined]
