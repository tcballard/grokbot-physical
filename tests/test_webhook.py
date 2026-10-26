import asyncio
import json

from aiohttp import web

from grokbot_orb.bots import normalise_bot_name
from grokbot_orb.speak import start_site
from grokbot_orb.webhook import WebhookClient, build_headers, build_payload


def test_normalise_bot_names():
    assert normalise_bot_name("Bob") == "bob"
    assert normalise_bot_name("Oma Bot") == "omabot"
    assert normalise_bot_name("Oma-Bot") == "omabot"
    assert normalise_bot_name("writer") == "writer"
    assert normalise_bot_name("Research Bot") == "researchbot"
    try:
        normalise_bot_name("   ")
    except ValueError:
        return
    raise AssertionError("empty bot name should fail")


def test_headers_and_payload_keep_secrets_apart():
    headers = build_headers("hook-bearer", "auto-key")
    assert headers["Authorization"] == "Bearer hook-bearer"
    assert headers["X-Automation-Key"] == "auto-key"
    assert headers["Content-Type"] == "application/json"

    payload = build_payload(
        task="  check the milk ",
        bot_name="OmaBot",
        callback_url="http://orb.local:8765/speak",
        speak_token="speak-secret",
        now="2026-10-25T21:00:00+00:00",
    )
    assert payload["task"] == "check the milk"
    assert payload["bot_name"] == "omabot"
    assert payload["source"] == "hey-grokbot-orb"
    assert payload["callback"]["url"] == "http://orb.local:8765/speak"
    assert payload["callback"]["method"] == "POST"
    assert payload["callback"]["headers"]["Authorization"] == "Bearer speak-secret"
    assert payload["callback"]["json_shape"]["status"] == "done"
    assert payload["requested_at"] == "2026-10-25T21:00:00+00:00"
    blob = json.dumps(payload)
    assert "hook-bearer" not in blob
    assert "auto-key" not in blob
    assert "speak-secret" in blob


def test_dispatch_posts_bearer_and_automation_key():
    asyncio.run(_dispatch_posts_bearer_and_automation_key())


async def _dispatch_posts_bearer_and_automation_key():
    seen: dict[str, object] = {}

    async def hook(request: web.Request) -> web.Response:
        seen["headers"] = {k: v for k, v in request.headers.items()}
        seen["json"] = await request.json()
        return web.json_response({"accepted": True})

    app = web.Application()
    app.router.add_post("/hook", hook)
    site = await start_site(app, "127.0.0.1", 0)
    try:
        client = WebhookClient(
            url=f"http://127.0.0.1:{site.port}/hook",
            bearer="hook-bearer",
            automation_key="auto-key",
            speak_token="speak-secret",
        )
        result = await client.dispatch(
            "What's on tomorrow",
            "Bob",
            "http://orb.local:8765/speak",
        )
        assert result.ok
        assert result.status == 200
        assert result.bot_name == "bob"
    finally:
        await site.close()

    headers = seen["headers"]
    assert isinstance(headers, dict)
    lowered = {str(key).lower(): value for key, value in headers.items()}
    assert lowered["authorization"] == "Bearer hook-bearer"
    assert lowered["x-automation-key"] == "auto-key"
    body = seen["json"]
    assert isinstance(body, dict)
    assert body["task"] == "What's on tomorrow"
    assert body["bot_name"] == "bob"
    assert body["callback"]["url"].endswith("/speak")
    assert "auto-key" not in json.dumps(body)
    assert "hook-bearer" not in json.dumps(body)


def test_dispatch_without_url_does_not_raise():
    async def run() -> None:
        client = WebhookClient(url="", bearer="x", automation_key="y", speak_token="")
        result = await client.dispatch("task", "bob", "http://orb.local/speak")
        assert not result.ok
        assert "GROKBOT_WEBHOOK_URL" in result.body
        assert result.payload["bot_name"] == "bob"

    asyncio.run(run())

