"""POST a task to a Grok Bot Webhook Routine."""

from __future__ import annotations

import asyncio
import logging
from dataclasses import dataclass
from datetime import datetime, timezone
from typing import Any
from urllib.parse import urlparse

import aiohttp

from grokbot_orb.bots import normalise_bot_name

logger = logging.getLogger("grokbot_orb.webhook")

SOURCE = "hey-grokbot-orb"


@dataclass(frozen=True)
class DispatchResult:
    ok: bool
    status: int | None
    body: str
    bot_name: str
    payload: dict[str, Any]


def build_headers(bearer: str, automation_key: str) -> dict[str, str]:
    headers = {
        "Content-Type": "application/json",
        "Accept": "application/json",
        "User-Agent": SOURCE,
    }
    if bearer:
        headers["Authorization"] = f"Bearer {bearer}"
    if automation_key:
        headers["X-Automation-Key"] = automation_key
    return headers


def build_payload(
    *,
    task: str,
    bot_name: str,
    callback_url: str,
    speak_token: str,
    now: str | None = None,
) -> dict[str, Any]:
    bot = normalise_bot_name(bot_name)
    callback_headers = {"Content-Type": "application/json"}
    if speak_token:
        callback_headers["Authorization"] = f"Bearer {speak_token}"
    return {
        "task": task.strip(),
        "bot_name": bot,
        "source": SOURCE,
        "callback": {
            "url": callback_url,
            "method": "POST",
            "headers": callback_headers,
            "json_shape": {
                "spoken_text": "what the orb should say, one or two sentences",
                "status": "done",
                "bot_name": bot,
            },
        },
        "requested_at": now or datetime.now(timezone.utc).isoformat(),
    }


def validate_webhook_url(url: str) -> str:
    parsed = urlparse(url.strip())
    if parsed.scheme not in {"http", "https"} or not parsed.netloc:
        raise ValueError(f"GROKBOT_WEBHOOK_URL is not an http(s) URL: {url!r}")
    return url.strip()


class WebhookClient:
    def __init__(
        self,
        *,
        url: str,
        bearer: str,
        automation_key: str,
        speak_token: str,
        timeout_s: float = 20.0,
    ) -> None:
        self.url = url.strip()
        self.bearer = bearer
        self.automation_key = automation_key
        self.speak_token = speak_token
        self.timeout_s = timeout_s

    def build(self, task: str, bot_name: str, callback_url: str) -> tuple[dict[str, Any], dict[str, str]]:
        payload = build_payload(
            task=task,
            bot_name=bot_name,
            callback_url=callback_url,
            speak_token=self.speak_token,
        )
        headers = build_headers(self.bearer, self.automation_key)
        return payload, headers

    async def dispatch(self, task: str, bot_name: str, callback_url: str) -> DispatchResult:
        try:
            bot = normalise_bot_name(bot_name)
        except ValueError as exc:
            return DispatchResult(False, None, str(exc), bot_name, {})
        cleaned = task.strip()
        if not cleaned:
            return DispatchResult(False, None, "task is empty", bot, {})
        payload, headers = self.build(cleaned, bot, callback_url)
        if not self.url:
            return DispatchResult(False, None, "GROKBOT_WEBHOOK_URL is not set", bot, payload)
        try:
            validate_webhook_url(self.url)
        except ValueError as exc:
            return DispatchResult(False, None, str(exc), bot, payload)

        timeout = aiohttp.ClientTimeout(total=self.timeout_s)
        try:
            async with aiohttp.ClientSession(timeout=timeout) as session:
                result = await self._post(session, payload, headers, bot)
                if result.status in {502, 503, 504}:
                    logger.warning("webhook HTTP %s; retrying once", result.status)
                    await asyncio.sleep(0.4)
                    result = await self._post(session, payload, headers, bot)
                return result
        except aiohttp.ClientError as exc:
            logger.warning("webhook request failed: %s", exc)
            return DispatchResult(False, None, str(exc), bot, payload)

    async def _post(
        self,
        session: aiohttp.ClientSession,
        payload: dict[str, Any],
        headers: dict[str, str],
        bot: str,
    ) -> DispatchResult:
        async with session.post(self.url, json=payload, headers=headers) as response:
            body = await response.text()
            ok = 200 <= response.status < 300
            if ok:
                logger.info("webhook accepted bot=%s status=%s", bot, response.status)
            else:
                logger.warning("webhook rejected bot=%s status=%s body=%s", bot, response.status, body[:300])
            return DispatchResult(ok, response.status, body[:2000], bot, payload)

