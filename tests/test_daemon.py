import asyncio

import aiohttp
from aiohttp import web

from grokbot_orb.config import Config
from grokbot_orb.daemon import OrbDaemon, reachability_warning
from grokbot_orb.speak import SpeakRequest, start_site


class FakeSession:
    def __init__(self) -> None:
        self.active = True
        self.forced: list[str] = []

    async def force_message(self, text: str) -> None:
        self.forced.append(text)


def test_speak_during_a_session_injects_force_message():
    asyncio.run(_speak_during_a_session_injects_force_message())


async def _speak_during_a_session_injects_force_message():
    daemon = OrbDaemon(Config.for_test())
    fake = FakeSession()
    daemon._session = fake
    await daemon.handle_speak(SpeakRequest("Kettle's boiled.", "done", "Bob"))
    assert fake.forced == ["Kettle's boiled."]
    assert daemon.spoken == ["Kettle's boiled."]
    assert daemon.halo.state == "working:bob"

    await daemon.handle_speak(SpeakRequest("", "working", "writer"))
    assert daemon.halo.state == "working:writer"
    assert daemon.spoken == ["Kettle's boiled."]


def test_idle_callback_is_spoken_and_health_stays_up():
    asyncio.run(_idle_callback_is_spoken_and_health_stays_up())


async def _idle_callback_is_spoken_and_health_stays_up():
    daemon = OrbDaemon(Config.for_test(speak_token="orb-token"))
    await daemon.start()
    try:
        base = f"http://127.0.0.1:{daemon.http_port}"
        async with aiohttp.ClientSession() as client:
            health = await client.get(base + "/health")
            body = await health.json()
            assert health.status == 200
            assert body["status"] == "ok"
            assert body["service"] == "hey-grokbot-orb"
            assert body["mode"] == "dry-run"
            assert body["halo"] == "idle"
            assert "webhook" not in body

            denied = await client.post(
                base + "/speak",
                json={"spoken_text": "no", "status": "done"},
            )
            assert denied.status == 401

            accepted = await client.post(
                base + "/speak",
                json={"spoken_text": "Soup is ready.", "status": "done", "bot_name": "bob"},
                headers={"Authorization": "Bearer orb-token"},
            )
            assert accepted.status == 200
        for _ in range(50):
            if daemon.spoken:
                break
            await asyncio.sleep(0.02)
        assert daemon.spoken == ["Soup is ready."]
        assert daemon.halo.state == "idle"
        assert "working:bob" in daemon.halo.history
    finally:
        await daemon.stop()


def test_simulated_wake_posts_the_webhook():
    asyncio.run(_simulated_wake_posts_the_webhook())


async def _simulated_wake_posts_the_webhook():
    seen: dict[str, object] = {}

    async def hook(request: web.Request) -> web.Response:
        seen["json"] = await request.json()
        seen["authorization"] = request.headers.get("Authorization")
        seen["automation"] = request.headers.get("X-Automation-Key")
        return web.json_response({"ok": True})

    app = web.Application()
    app.router.add_post("/automations/webhook/test", hook)
    site = await start_site(app, "127.0.0.1", 0)
    daemon = OrbDaemon(
        Config.for_test(
            simulate_wake=True,
            simulate_text="What's on the kitchen calendar tomorrow?",
            simulate_bot="Bob",
            webhook_url=f"http://127.0.0.1:{site.port}/automations/webhook/test",
            webhook_bearer="hook-bearer",
            automation_key="auto-key",
            speak_token="speak-secret",
        )
    )
    try:
        await daemon.start()
        for _ in range(100):
            if "json" in seen and daemon.halo.state == "working:bob":
                break
            await asyncio.sleep(0.02)
        body = seen["json"]
        assert isinstance(body, dict)
        assert body["task"] == "What's on the kitchen calendar tomorrow?"
        assert body["bot_name"] == "bob"
        assert body["source"] == "hey-grokbot-orb"
        assert body["callback"]["url"] == "http://orb.local:8765/speak"
        assert body["callback"]["headers"]["Authorization"] == "Bearer speak-secret"
        assert seen["authorization"] == "Bearer hook-bearer"
        assert seen["automation"] == "auto-key"
        assert daemon.halo.state == "working:bob"
    finally:
        await daemon.stop()
        await site.close()


def test_reachability_warnings():
    assert reachability_warning("http://127.0.0.1:8765/speak")
    assert reachability_warning("http://192.168.1.20:8765/speak")
    assert "tailnet" in (reachability_warning("http://100.64.1.2:8765/speak") or "")
    assert reachability_warning("https://orb.example.ts.net/speak") is None
