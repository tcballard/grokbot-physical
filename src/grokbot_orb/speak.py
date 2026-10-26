"""LAN callback server. Grok Bot POSTs here when a task is finished."""

from __future__ import annotations

import json
import logging
import socket
from dataclasses import dataclass
from typing import Any, Callable

import aiohttp
from aiohttp import web

logger = logging.getLogger("grokbot_orb.speak")

MAX_SPOKEN_CHARS = 2000
SPEAK_STATUSES = {"done", "error", "working"}


class SpeakError(Exception):
    def __init__(self, message: str, status: int = 400) -> None:
        super().__init__(message)
        self.message = message
        self.status = status


@dataclass(frozen=True)
class SpeakRequest:
    spoken_text: str
    status: str
    bot_name: str | None


def parse_speak_request(data: Any) -> SpeakRequest:
    if not isinstance(data, dict):
        raise SpeakError("body must be a JSON object")
    status = str(data.get("status", "done")).strip().lower()
    if status not in SPEAK_STATUSES:
        raise SpeakError("status must be done, error, or working")
    text = data.get("spoken_text", "")
    if text is None:
        text = ""
    if not isinstance(text, str):
        raise SpeakError("spoken_text must be a string")
    text = text.strip()
    if status != "working" and not text:
        raise SpeakError("spoken_text is required")
    if len(text) > MAX_SPOKEN_CHARS:
        raise SpeakError(f"spoken_text is longer than {MAX_SPOKEN_CHARS} characters")
    bot = data.get("bot_name")
    if bot is None or (isinstance(bot, str) and not bot.strip()):
        bot_name = None
    elif isinstance(bot, str):
        bot_name = bot.strip()
    else:
        raise SpeakError("bot_name must be a string")
    return SpeakRequest(spoken_text=text, status=status, bot_name=bot_name)


Submit = Callable[[SpeakRequest], bool]
HealthFn = Callable[[], dict[str, Any]]
SUBMIT_KEY: web.AppKey[Submit] = web.AppKey("submit")
HEALTH_KEY: web.AppKey[HealthFn] = web.AppKey("health")
TOKEN_KEY: web.AppKey[str] = web.AppKey("token")


def _authorized(request: web.Request, token: str) -> bool:
    if not token:
        return True
    header = request.headers.get("Authorization", "")
    if header == f"Bearer {token}":
        return True
    if request.headers.get("X-Orb-Token", "") == token:
        return True
    return False


def _error(status: int, message: str) -> web.Response:
    return web.json_response({"ok": False, "error": message}, status=status)


def create_app(
    *,
    submit: Submit,
    health: HealthFn,
    token: str,
) -> web.Application:
    app = web.Application()
    app[SUBMIT_KEY] = submit
    app[HEALTH_KEY] = health
    app[TOKEN_KEY] = token
    app.router.add_get("/health", _health)
    app.router.add_post("/speak", _speak)
    return app


async def _health(request: web.Request) -> web.Response:
    return web.json_response(request.app[HEALTH_KEY]())


async def _speak(request: web.Request) -> web.Response:
    token = request.app[TOKEN_KEY]
    if not _authorized(request, token):
        return _error(401, "unauthorized")
    if request.content_length is not None and request.content_length > 32_768:
        return _error(413, "body is too large")
    try:
        data = await request.json()
    except (json.JSONDecodeError, aiohttp.ContentTypeError):
        return _error(400, "body must be JSON")
    try:
        speak = parse_speak_request(data)
    except SpeakError as exc:
        return _error(exc.status, exc.message)
    accepted = request.app[SUBMIT_KEY](speak)
    if not accepted:
        return _error(503, "speak queue is full")
    logger.info(
        "callback status=%s bot=%s chars=%s",
        speak.status,
        speak.bot_name or "-",
        len(speak.spoken_text),
    )
    return web.json_response({"ok": True, "status": speak.status})


@dataclass
class SiteHandle:
    runner: web.AppRunner
    site: web.BaseSite
    port: int
    sock: socket.socket

    async def close(self) -> None:
        await self.site.stop()
        await self.runner.cleanup()
        try:
            self.sock.close()
        except OSError:
            pass


async def start_site(app: web.Application, host: str, port: int) -> SiteHandle:
    sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    sock.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    sock.bind((host, port))
    bound = sock.getsockname()[1]
    sock.listen(128)
    runner = web.AppRunner(app, access_log=None)
    await runner.setup()
    site = web.SockSite(runner, sock)
    await site.start()
    logger.info("callback server http://%s:%s", host, bound)
    return SiteHandle(runner=runner, site=site, port=bound, sock=sock)
