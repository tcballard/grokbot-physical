"""Orb daemon.

The loop is:

1. A 16 kHz ring feeds openWakeWord. Nothing leaves the device until "Hey Grok Bot".
2. A wake opens wss://api.x.ai/v1/realtime?model=grok-voice-latest at 24 kHz PCM16
   with server VAD. Capture is resampled when the ALSA device is not already 24 kHz.
3. delegate_to_grok_bot(task, bot_name) POSTs the task to a Grok Bot Webhook Routine
   and the halo takes that bot's colour.
4. When the bot finishes it POSTs /speak. An open Realtime session speaks the text
   with force_message; otherwise the orb plays TTS and returns to the wake word.
"""

from __future__ import annotations

import asyncio
import ipaddress
import json
import logging
import time
from urllib.parse import urlparse

import numpy as np

from grokbot_orb import __version__
from grokbot_orb.audio import (
    REALTIME_RATE,
    WAKE_FRAME_SAMPLES,
    WAKE_RATE,
    PcmInput,
    PcmOutput,
    PcmRing,
    resample_pcm16,
)
from grokbot_orb.bots import normalise_bot_name
from grokbot_orb.config import Config
from grokbot_orb.halo import Halo, open_backend
from grokbot_orb.realtime import RealtimeSession, parse_tool_arguments
from grokbot_orb.speak import SiteHandle, SpeakRequest, create_app, start_site
from grokbot_orb.tts import synthesize
from grokbot_orb.wakeword import WakeDetector, describe_wake_model, load_openwakeword
from grokbot_orb.webhook import WebhookClient

logger = logging.getLogger("grokbot_orb.daemon")

SPOKEN_HISTORY = 50


class OrbDaemon:
    def __init__(self, config: Config) -> None:
        self.config = config
        self.halo = Halo(
            count=config.halo_count,
            brightness=config.halo_brightness,
            backend=open_backend(
                backend=config.halo_backend,
                pin=config.halo_pin,
                count=config.halo_count,
                channel=config.halo_channel,
                dry_run=config.dry_run,
            ),
        )
        self.webhook = WebhookClient(
            url=config.webhook_url,
            bearer=config.webhook_bearer,
            automation_key=config.automation_key,
            speak_token=config.speak_token,
            timeout_s=config.webhook_timeout_s,
        )
        self.player = PcmOutput(
            dry_run=config.dry_run,
            device=config.audio_output_device,
            rate=config.audio_rate or WAKE_RATE,
        )
        self.wake_name = describe_wake_model(config.wake_model_path, config.wake_fallback)
        self.spoken: list[str] = []
        self.device_rate = config.audio_rate
        self._site: SiteHandle | None = None
        self._session: RealtimeSession | None = None
        self._session_task: asyncio.Task[None] | None = None
        self._tasks: list[asyncio.Task[None]] = []
        self._speak_queue: asyncio.Queue[SpeakRequest] = asyncio.Queue(maxsize=8)
        self._speak_lock = asyncio.Lock()
        self._stopped = asyncio.Event()
        self._closed = False
        self._held_halo: str | None = None
        self._speaking = False
        self._responses_done = 0
        self._last_activity = time.monotonic()
        self._wake_cooldown_until = 0.0
        self._live_audio: asyncio.Queue[bytes] | None = None
        self._ring = PcmRing(WAKE_RATE * 2)
        self._wake_buf = bytearray()

    @property
    def http_port(self) -> int:
        if self._site is None:
            return self.config.callback_port
        return self._site.port

    @property
    def session_active(self) -> bool:
        return bool(self._session and self._session.active)

    def callback_url(self) -> str:
        base = self.config.public_base.strip().rstrip("/")
        if base.endswith("/speak"):
            base = base[: -len("/speak")]
        if base:
            return base + "/speak"
        host = self.config.callback_host
        if host in {"0.0.0.0", "::"}:
            host = "127.0.0.1"
        return f"http://{host}:{self.http_port}/speak"

    def request_stop(self) -> None:
        self._stopped.set()

    async def run(self) -> None:
        await self.start()
        try:
            await self._stopped.wait()
        finally:
            await self.stop()

    async def start(self) -> None:
        if not self.config.dry_run:
            await self.player.open()
            self.player.rate = self.device_rate or self.player.rate
        app = create_app(submit=self.submit_speak, health=self.health, token=self.config.speak_token)
        self._site = await start_site(app, self.config.callback_host, self.config.callback_port)
        self._tasks = [
            asyncio.create_task(self.halo.run(), name="halo"),
            asyncio.create_task(self._speak_loop(), name="speak"),
        ]
        if self.config.simulate_wake:
            self._tasks.append(asyncio.create_task(self._simulated_wake(), name="simulate"))
        elif not self.config.dry_run:
            self._tasks.append(asyncio.create_task(self._capture_loop(), name="capture"))
        logger.info(
            "Hey Grok Bot Orb %s on %s:%s mode=%s wake=%s",
            __version__,
            self.config.callback_host,
            self.http_port,
            "dry-run" if self.config.dry_run else "live",
            self.wake_name,
        )
        self._warn_config()

    async def stop(self) -> None:
        if self._closed:
            return
        self._closed = True
        self._stopped.set()
        for task in self._tasks:
            task.cancel()
        if self._session_task is not None:
            self._session_task.cancel()
        await asyncio.gather(*self._tasks, return_exceptions=True)
        if self._session_task is not None:
            await asyncio.gather(self._session_task, return_exceptions=True)
        if self._session is not None:
            await self._session.close()
            self._session = None
        await self.player.close()
        if self._site is not None:
            await self._site.close()
            self._site = None
        self.halo.close()

    def health(self) -> dict[str, object]:
        if self.config.dry_run:
            mode = "dry-run"
        elif self.session_active:
            mode = "session"
        else:
            mode = "listening"
        return {
            "status": "ok",
            "service": "hey-grokbot-orb",
            "version": __version__,
            "mode": mode,
            "halo": self.halo.state,
            "session_active": self.session_active,
            "wake_model": self.wake_name,
            "spoken": len(self.spoken),
        }

    def submit_speak(self, request: SpeakRequest) -> bool:
        try:
            self._speak_queue.put_nowait(request)
        except asyncio.QueueFull:
            return False
        return True

    async def handle_speak(self, request: SpeakRequest) -> None:
        """Process one callback. Tests call this directly; HTTP goes through the queue."""
        await self._handle_speak_now(request)

    async def _speak_loop(self) -> None:
        while True:
            request = await self._speak_queue.get()
            try:
                async with self._speak_lock:
                    await self._handle_speak_now(request)
            except asyncio.CancelledError:
                raise
            except Exception:
                logger.exception("speak callback failed")
                self.halo.set_state("error")
            finally:
                self._speak_queue.task_done()

    async def _handle_speak_now(self, request: SpeakRequest) -> None:
        bot = normalise_bot_name(request.bot_name) if request.bot_name else None
        if request.status == "working":
            self._held_halo = f"working:{bot}" if bot else "working"
            self.halo.set_state(self._held_halo)
            return
        if request.status == "error" and not self.session_active:
            self.halo.set_state("error")
        elif bot:
            self.halo.set_state(f"working:{bot}")
        elif not self.session_active:
            self.halo.set_state("speaking")

        session = self._session
        if session is not None and session.active and request.spoken_text:
            try:
                await session.force_message(request.spoken_text)
                self._remember(request.spoken_text)
                self._touch()
                self._held_halo = None
                return
            except Exception:
                logger.exception("force_message failed; falling back to TTS")

        if request.spoken_text:
            await self._speak_idle(request.spoken_text)
        self._held_halo = None
        if not self.session_active:
            self.halo.set_state("idle")
            self._arm_wake_cooldown()

    async def _speak_idle(self, text: str) -> None:
        self._remember(text)
        pcm, rate = await synthesize(
            text,
            api_key=self.config.xai_api_key,
            url=self.config.xai_tts_url,
            voice=self.config.xai_voice,
            fallback=self.config.tts_fallback,
        )
        if not pcm:
            return
        played = resample_pcm16(pcm, rate, self._output_rate())
        await self.player.write(played)

    async def _simulated_wake(self) -> None:
        await asyncio.sleep(0.05)
        try:
            text = self.config.simulate_text.strip()
            self.halo.set_state("listening")
            if self.config.xai_api_key:
                await self._run_realtime_session(preroll_16k=b"", text=text or "Say hello in one short sentence.")
                return
            logger.warning("XAI_API_KEY is not set; simulated wake will not open a Realtime session")
            if text:
                await self._dispatch_tool(text, self.config.simulate_bot)
            else:
                logger.info("simulated wake with no text; callback server is up")
                self.halo.set_state("idle")
        except asyncio.CancelledError:
            raise
        except Exception:
            logger.exception("simulated wake failed")
            self.halo.set_state("error")

    async def _capture_loop(self) -> None:
        try:
            model, name = load_openwakeword(self.config.wake_model_path, self.config.wake_fallback)
        except Exception:
            logger.exception("wake word model failed to load")
            self.halo.set_state("error")
            return
        self.wake_name = name
        detector = WakeDetector(
            model,
            threshold=self.config.wake_threshold,
            cooldown_s=self.config.wake_cooldown_s,
        )
        capture = PcmInput(self.config.audio_input_device, self.config.audio_rate)
        try:
            self.device_rate = await capture.start()
            self.player.rate = self.device_rate
            logger.info(
                "audio rates device=%s wake=%s realtime=%s",
                self.device_rate,
                WAKE_RATE,
                REALTIME_RATE,
            )
            while not self._stopped.is_set():
                block = await capture.read()
                await self._on_capture(block, detector)
        except asyncio.CancelledError:
            raise
        except Exception:
            logger.exception("microphone capture stopped")
            self.halo.set_state("error")
        finally:
            capture.close()

    async def _on_capture(self, block: bytes, detector: WakeDetector) -> None:
        if not block:
            return
        device_rate = self.device_rate or WAKE_RATE
        pcm16 = resample_pcm16(block, device_rate, WAKE_RATE)
        self._ring.extend(pcm16)
        live = self._live_audio
        if live is not None:
            if self.config.barge_in or not self._speaking:
                pcm24 = resample_pcm16(block, device_rate, REALTIME_RATE)
                _offer(live, pcm24)
            return
        self._wake_buf.extend(pcm16)
        frame_bytes = WAKE_FRAME_SAMPLES * 2
        while len(self._wake_buf) >= frame_bytes:
            frame = bytes(self._wake_buf[:frame_bytes])
            del self._wake_buf[:frame_bytes]
            samples = np.frombuffer(frame, dtype="<i2")
            hit = detector.feed(samples)
            if not hit:
                continue
            if time.monotonic() < self._wake_cooldown_until:
                continue
            if self._session_busy():
                continue
            preroll = self._ring.snapshot()
            self._session_task = asyncio.create_task(
                self._run_realtime_session(preroll_16k=preroll, text=None),
                name="realtime",
            )
            self._session_task.add_done_callback(_log_task_error)

    async def _run_realtime_session(self, *, preroll_16k: bytes, text: str | None) -> None:
        if not self.config.xai_api_key:
            logger.error("XAI_API_KEY is not set; staying on the wake word")
            self.halo.set_state("error")
            await asyncio.sleep(0.8)
            self._restore_halo()
            return
        self.halo.set_state("listening")
        self._responses_done = 0
        self._speaking = False
        self._touch()
        use_mic = not self.config.dry_run
        if use_mic:
            self._live_audio = asyncio.Queue(maxsize=64)
        session = RealtimeSession(
            self.config,
            on_audio=self._on_assistant_audio,
            on_tool=self._on_tool,
            on_state=self._on_realtime_state,
        )
        self._session = session
        pump: asyncio.Task[None] | None = None
        try:
            await session.open()
            if preroll_16k:
                await session.send_audio(resample_pcm16(preroll_16k, WAKE_RATE, REALTIME_RATE))
            if text:
                await session.inject_user_text(text)
            if self._live_audio is not None:
                pump = asyncio.create_task(self._pump_live(session), name="mic-to-realtime")
            await self._watch_session(session)
        except asyncio.CancelledError:
            raise
        except Exception:
            logger.exception("realtime session failed")
            self.halo.set_state("error")
            self._held_halo = None
        finally:
            self._live_audio = None
            if pump is not None:
                pump.cancel()
                await asyncio.gather(pump, return_exceptions=True)
            await session.close()
            if self._session is session:
                self._session = None
            self._speaking = False
            self._restore_halo()
            self._arm_wake_cooldown()

    async def _pump_live(self, session: RealtimeSession) -> None:
        queue = self._live_audio
        while queue is not None and session.active and not self._stopped.is_set():
            try:
                pcm = await asyncio.wait_for(queue.get(), timeout=0.2)
            except asyncio.TimeoutError:
                continue
            try:
                await session.send_audio(pcm)
            except ConnectionError:
                return

    async def _watch_session(self, session: RealtimeSession) -> None:
        while session.active and not self._stopped.is_set():
            await asyncio.sleep(0.2)
            if self.config.dry_run and self._responses_done:
                limit = min(self.config.session_idle_s, 3.0)
            else:
                limit = self.config.session_idle_s
            if time.monotonic() - self._last_activity > limit:
                logger.info("session idle for %.1fs; returning to the wake word", limit)
                return

    async def _on_assistant_audio(self, pcm24: bytes) -> None:
        self._touch()
        self._speaking = True
        if not self.halo.state.startswith("working") and self.halo.state != "error":
            self.halo.set_state("speaking")
        played = resample_pcm16(pcm24, REALTIME_RATE, self._output_rate())
        await self.player.write(played)

    async def _on_realtime_state(self, state: str) -> None:
        self._touch()
        if state in {"response.done", "response.output_audio.done"}:
            self._responses_done += 1
            self._speaking = False
        elif state == "input_audio_buffer.speech_started" and not self._held_halo:
            self.halo.set_state("listening")
        elif state == "error":
            self.halo.set_state("error")

    async def _on_tool(
        self,
        name: str,
        arguments: object,
        call_id: str,
        session: RealtimeSession,
    ) -> None:
        self._touch()
        if name != "delegate_to_grok_bot":
            await session.return_tool(call_id, {"ok": False, "detail": f"unknown tool {name}"})
            return
        try:
            args = parse_tool_arguments(arguments)
            bot = normalise_bot_name(str(args.get("bot_name") or "bob"))
            task = str(args.get("task") or "").strip()
        except (ValueError, json.JSONDecodeError) as exc:
            await session.return_tool(call_id, {"ok": False, "detail": str(exc)})
            return
        if not task:
            await session.return_tool(call_id, {"ok": False, "detail": "task is empty"})
            return
        result_ok, detail = await self._dispatch_tool(task, bot)
        await session.return_tool(
            call_id,
            {
                "ok": result_ok,
                "bot_name": bot,
                "detail": detail,
            },
        )

    async def _dispatch_tool(self, task: str, bot_name: str) -> tuple[bool, str]:
        try:
            bot = normalise_bot_name(bot_name)
        except ValueError as exc:
            self.halo.set_state("error")
            return False, str(exc)
        self._held_halo = f"working:{bot}"
        self.halo.set_state(self._held_halo)
        result = await self.webhook.dispatch(task, bot, self.callback_url())
        if result.ok:
            return True, "Task accepted. Say one short acknowledgement and nothing else."
        self._held_halo = None
        self.halo.set_state("error")
        return False, result.body or "webhook did not accept the task"

    def _restore_halo(self) -> None:
        if self._held_halo:
            self.halo.set_state(self._held_halo)
        elif self.halo.state != "idle":
            self.halo.set_state("idle")

    def _arm_wake_cooldown(self) -> None:
        self._wake_cooldown_until = time.monotonic() + self.config.wake_cooldown_s

    def _touch(self) -> None:
        self._last_activity = time.monotonic()

    def _remember(self, text: str) -> None:
        self.spoken.append(text)
        if len(self.spoken) > SPOKEN_HISTORY:
            del self.spoken[: len(self.spoken) - SPOKEN_HISTORY]

    def _output_rate(self) -> int:
        return self.device_rate or self.player.rate or WAKE_RATE

    def _session_busy(self) -> bool:
        return self._session_task is not None and not self._session_task.done()

    def _warn_config(self) -> None:
        if not self.config.xai_api_key:
            logger.warning("XAI_API_KEY is empty; voice sessions and xAI TTS are disabled")
        if not self.config.webhook_url:
            logger.warning("GROKBOT_WEBHOOK_URL is empty; delegate_to_grok_bot has nowhere to post")
        warning = reachability_warning(self.callback_url())
        if warning:
            logger.warning("%s", warning)


def reachability_warning(url: str) -> str | None:
    host = (urlparse(url).hostname or "").strip("[]").lower()
    if not host or host.endswith(".ts.net"):
        return None
    if host.endswith(".local") or host in {"localhost", "127.0.0.1", "::1", "0.0.0.0"}:
        return (
            f"callback {url} is only on this machine. "
            "Set ORB_PUBLIC_BASE to a Tailscale Funnel URL so Grok Bot can POST /speak."
        )
    try:
        ip = ipaddress.ip_address(host)
    except ValueError:
        return None
    if ip in ipaddress.ip_network("100.64.0.0/10"):
        return (
            f"callback {url} is a Tailscale address. The Grok Bot runner must be on this "
            "tailnet, or use `tailscale funnel` and put the .ts.net URL in ORB_PUBLIC_BASE."
        )
    if ip.is_private or ip.is_loopback or ip.is_link_local or not ip.is_global:
        return (
            f"callback {url} is not reachable from Cursor's cloud. "
            "Use Tailscale Funnel or run the automation on this tailnet."
        )
    return None


def _offer(queue: asyncio.Queue[bytes], pcm: bytes) -> None:
    if not pcm:
        return
    if queue.full():
        try:
            queue.get_nowait()
        except asyncio.QueueEmpty:
            pass
    try:
        queue.put_nowait(pcm)
    except asyncio.QueueFull:
        return


def _log_task_error(task: asyncio.Task[None]) -> None:
    if task.cancelled():
        return
    exc = task.exception()
    if exc is not None:
        logger.error("session task failed", exc_info=exc)
