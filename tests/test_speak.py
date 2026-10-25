import asyncio
import json

import aiohttp

from grokbot_orb.speak import SpeakError, create_app, parse_speak_request, start_site


def test_parse_speak_request():
    parsed = parse_speak_request({"spoken_text": " Soup is ready. ", "status": "done", "bot_name": "Bob"})
    assert parsed.spoken_text == "Soup is ready."
    assert parsed.status == "done"
    assert parsed.bot_name == "Bob"

    working = parse_speak_request({"status": "WORKING"})
    assert working.status == "working"
    assert working.spoken_text == ""
    assert working.bot_name is None

    try:
        parse_speak_request({"status": "done"})
    except SpeakError as exc:
        assert exc.status == 400
        assert "spoken_text" in exc.message
    else:
        raise AssertionError("missing spoken_text should fail")

    try:
        parse_speak_request({"spoken_text": "no", "status": "later"})
    except SpeakError as exc:
        assert "status" in exc.message
    else:
        raise AssertionError("bad status should fail")

    try:
        parse_speak_request(["nope"])
    except SpeakError as exc:
        assert "JSON object" in exc.message
    else:
        raise AssertionError("non-object should fail")


def test_health_is_open_and_speak_checks_the_token():
    asyncio.run(_health_is_open_and_speak_checks_the_token())


async def _health_is_open_and_speak_checks_the_token():
    queued = []

    def submit(request):
        queued.append(request)
        return True

    app = create_app(
        submit=submit,
        health=lambda: {"status": "ok", "halo": "idle"},
        token="orb-token",
    )
    site = await start_site(app, "127.0.0.1", 0)
    base = f"http://127.0.0.1:{site.port}"
    try:
        async with aiohttp.ClientSession() as client:
            health = await client.get(base + "/health")
            assert health.status == 200
            assert (await health.json())["status"] == "ok"

            rejected = await client.post(
                base + "/speak",
                json={"spoken_text": "Hi", "status": "done"},
            )
            assert rejected.status == 401

            bad = await client.post(
                base + "/speak",
                data=json.dumps({"status": "done"}),
                headers={"Authorization": "Bearer orb-token", "Content-Type": "application/json"},
            )
            assert bad.status == 400

            accepted = await client.post(
                base + "/speak",
                json={"spoken_text": "The milk is in.", "status": "done", "bot_name": "omabot"},
                headers={"Authorization": "Bearer orb-token"},
            )
            assert accepted.status == 200
            assert (await accepted.json())["ok"] is True

            alt = await client.post(
                base + "/speak",
                json={"status": "working", "bot_name": "writer"},
                headers={"X-Orb-Token": "orb-token"},
            )
            assert alt.status == 200
    finally:
        await site.close()

    assert [item.status for item in queued] == ["done", "working"]
    assert queued[0].spoken_text == "The milk is in."
    assert queued[0].bot_name == "omabot"
    assert queued[1].bot_name == "writer"


def test_full_queue_returns_503():
    asyncio.run(_full_queue_returns_503())


async def _full_queue_returns_503():
    app = create_app(submit=lambda _request: False, health=lambda: {"status": "ok"}, token="")
    site = await start_site(app, "127.0.0.1", 0)
    try:
        async with aiohttp.ClientSession() as client:
            response = await client.post(
                f"http://127.0.0.1:{site.port}/speak",
                json={"spoken_text": "Hi", "status": "done"},
            )
            assert response.status == 503
    finally:
        await site.close()
