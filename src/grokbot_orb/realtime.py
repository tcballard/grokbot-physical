"""Bidirectional session on wss://api.x.ai/v1/realtime."""

from __future__ import annotations

import asyncio
import base64
import inspect
import json
import logging
from typing import Any, Awaitable, Callable

from grokbot_orb.bots import BOTS, bot_roster
from grokbot_orb.config import Config

logger = logging.getLogger("grokbot_orb.realtime")

REALTIME_RATE = 24_000

OnAudio = Callable[[bytes], Awaitable[None]]
OnTool = Callable[[str, Any, str, "RealtimeSession"], Awaitable[None]]
OnState = Callable[[str], Awaitable[None]]

FORCE_PREAMBLE = (
    "The countertop orb must speak the following update aloud, verbatim, "
    "with no preamble and no extra words:\n"
)


def orb_instructions() -> str:
    return (
        "You are the voice of Hey Grok Bot, a small countertop orb in a kitchen. "
        "The person is speaking out loud, hands free, with no phone in front of them.\n\n"
        "Talk like a person in the room. One or two short sentences. Spoken English. "
        "No markdown, no lists, no symbols, no URLs.\n\n"
        "When they ask you to do something that needs tools, lookup, writing, scheduling, "
        "messages, or more than a sentence of knowledge, call delegate_to_grok_bot exactly once. "
        f"bot_name is one of: {bot_roster()}. "
        "task is their request in their words, including the details they gave.\n\n"
        "After the tool returns ok, say only a short acknowledgement such as \"Bob's on it.\" "
        "Do not invent the result. The orb speaks the result later, out loud, when the bot finishes.\n\n"
        "Answer a quick question yourself, in one breath, and do not call the tool.\n\n"
        "If they say stop, goodbye, or that's all, say goodbye in a few words and stop."
    )


def delegate_tool() -> dict[str, Any]:
    return {
        "type": "function",
        "name": "delegate_to_grok_bot",
        "description": (
            "Hand a real task to a Grok Bot that can use tools and take actions. "
            "Call this for lookup, writing, scheduling, messages, or any multi-step work. "
            "Do not pretend the work is already done. "
            f"bot_name is one of: {bot_roster()}."
        ),
        "parameters": {
            "type": "object",
            "properties": {
                "task": {
                    "type": "string",
                    "description": "The task in the person's words, with the details they gave.",
                },
                "bot_name": {
                    "type": "string",
                    "enum": list(BOTS),
                    "description": "Which Grok Bot should do the work.",
                },
            },
            "required": ["task", "bot_name"],
        },
    }


def build_session_update(config: Config) -> dict[str, Any]:
    return {
        "type": "session.update",
        "session": {
            "voice": config.xai_voice,
            "instructions": orb_instructions(),
            # High reasoning blows the conversational latency budget.
            "reasoning": {"effort": "none"},
            "turn_detection": {"type": "server_vad"},
            "audio": {
                "input": {
                    "format": {"type": "audio/pcm", "rate": REALTIME_RATE},
                    "transcription": {
                        "keyterms": ["Hey Grok Bot", "Grok Bot", "Bob", "OmaBot", "Writer"],
                    },
                },
                "output": {
                    "format": {"type": "audio/pcm", "rate": REALTIME_RATE},
                },
            },
            "tools": [delegate_tool()],
        },
    }


def force_message_events(spoken_text: str) -> list[dict[str, Any]]:
    return [
        {
            "type": "conversation.item.create",
            "item": {
                "type": "message",
                "role": "user",
                "content": [
                    {"type": "input_text", "text": FORCE_PREAMBLE + spoken_text.strip()},
                ],
            },
        },
        {"type": "response.create"},
    ]


def user_text_events(text: str) -> list[dict[str, Any]]:
    return [
        {
            "type": "conversation.item.create",
            "item": {
                "type": "message",
                "role": "user",
                "content": [{"type": "input_text", "text": text}],
            },
        },
        {"type": "response.create"},
    ]


def parse_tool_arguments(raw: Any) -> dict[str, Any]:
    if raw is None or raw == "":
        return {}
    if isinstance(raw, dict):
        return raw
    if not isinstance(raw, str):
        raise ValueError("tool arguments must be a JSON object")
    data = json.loads(raw)
    if not isinstance(data, dict):
        raise ValueError("tool arguments must be a JSON object")
    return data


class RealtimeSession:
    """One voice session. The caller streams 24 kHz PCM16 in and plays PCM16 out."""

    def __init__(
        self,
        config: Config,
        *,
        on_audio: OnAudio,
        on_tool: OnTool,
        on_state: OnState | None = None,
    ) -> None:
        self.config = config
        self._on_audio = on_audio
        self._on_tool = on_tool
        self._on_state = on_state
        self.active = False
        self._ws: Any = None
        self._reader: asyncio.Task[None] | None = None
        self._send_lock = asyncio.Lock()

    async def open(self) -> None:
        import websockets

        headers = {"Authorization": f"Bearer {self.config.xai_api_key}"}
        kwargs = _connect_kwargs(websockets.connect, headers)
        logger.info("connecting %s", self.config.realtime_url())
        self._ws = await asyncio.wait_for(
            websockets.connect(
                self.config.realtime_url(),
                max_size=4_000_000,
                ping_interval=20,
                **kwargs,
            ),
            timeout=10,
        )
        self.active = True
        await self._send(build_session_update(self.config))
        self._reader = asyncio.create_task(self._read_loop(), name="realtime-reader")

    async def send_audio(self, pcm16_24k: bytes) -> None:
        if not pcm16_24k or not self.active:
            return
        await self._send(
            {
                "type": "input_audio_buffer.append",
                "audio": base64.b64encode(pcm16_24k).decode("ascii"),
            }
        )

    async def inject_user_text(self, text: str) -> None:
        for event in user_text_events(text):
            await self._send(event)

    async def force_message(self, spoken_text: str) -> None:
        for event in force_message_events(spoken_text):
            await self._send(event)

    async def return_tool(self, call_id: str, output: dict[str, Any]) -> None:
        await self._send(
            {
                "type": "conversation.item.create",
                "item": {
                    "type": "function_call_output",
                    "call_id": call_id,
                    "output": json.dumps(output),
                },
            }
        )
        await self._send({"type": "response.create"})

    async def close(self) -> None:
        self.active = False
        reader = self._reader
        self._reader = None
        if reader is not None and reader is not asyncio.current_task():
            reader.cancel()
            await asyncio.gather(reader, return_exceptions=True)
        ws = self._ws
        self._ws = None
        if ws is not None:
            try:
                await ws.close()
            except Exception:
                logger.debug("websocket close failed", exc_info=True)

    async def _send(self, event: dict[str, Any]) -> None:
        ws = self._ws
        if ws is None or not self.active:
            raise ConnectionError("realtime session is not open")
        async with self._send_lock:
            await ws.send(json.dumps(event))

    async def _read_loop(self) -> None:
        ws = self._ws
        try:
            async for raw in ws:
                await self._dispatch(raw)
        except asyncio.CancelledError:
            raise
        except Exception:
            logger.exception("realtime reader stopped")
        finally:
            self.active = False

    async def _dispatch(self, raw: Any) -> None:
        if isinstance(raw, (bytes, bytearray)):
            await self._on_audio(bytes(raw))
            return
        if isinstance(raw, str):
            try:
                event = json.loads(raw)
            except json.JSONDecodeError:
                logger.warning("ignoring non-json realtime frame")
                return
        elif isinstance(raw, dict):
            event = raw
        else:
            return
        if not isinstance(event, dict):
            return
        kind = str(event.get("type") or "")
        if kind in {"response.output_audio.delta", "response.audio.delta"}:
            payload = event.get("delta") or event.get("audio") or ""
            if isinstance(payload, str) and payload:
                try:
                    pcm = base64.b64decode(payload)
                except ValueError:
                    logger.warning("bad audio delta")
                    return
                await self._on_audio(pcm)
            return
        if kind == "response.function_call_arguments.done":
            await self._on_tool(
                str(event.get("name") or ""),
                event.get("arguments"),
                str(event.get("call_id") or ""),
                self,
            )
            return
        if kind in {"input_audio_buffer.speech_started", "input_audio_buffer.speech_stopped"}:
            await self._emit(kind)
            return
        if kind in {"response.done", "response.output_audio.done"}:
            await self._emit(kind)
            return
        if kind == "error":
            logger.error("realtime error: %s", event.get("error") or event)
            await self._emit("error")
            return
        if kind.endswith("transcript.done") or kind.endswith("transcription.completed"):
            text = event.get("transcript") or event.get("text") or ""
            if text:
                logger.info("transcript %s: %s", kind, text)

    async def _emit(self, state: str) -> None:
        if self._on_state is not None:
            await self._on_state(state)


def _connect_kwargs(connect: Callable[..., Any], headers: dict[str, str]) -> dict[str, Any]:
    parameters = inspect.signature(connect).parameters
    if "additional_headers" in parameters:
        return {"additional_headers": headers}
    return {"extra_headers": headers}
