import json

import pytest

from grokbot_orb.config import Config
from grokbot_orb.realtime import (
    FORCE_PREAMBLE,
    build_session_update,
    force_message_events,
    parse_tool_arguments,
)


def test_session_update_is_24k_server_vad_with_the_tool():
    event = build_session_update(Config.for_test(xai_voice="ara"))
    session = event["session"]
    assert event["type"] == "session.update"
    assert session["voice"] == "ara"
    assert session["turn_detection"] == {"type": "server_vad"}
    assert session["reasoning"] == {"effort": "none"}
    assert session["audio"]["input"]["format"] == {"type": "audio/pcm", "rate": 24000}
    assert session["audio"]["output"]["format"] == {"type": "audio/pcm", "rate": 24000}
    tool = session["tools"][0]
    assert tool["name"] == "delegate_to_grok_bot"
    assert tool["parameters"]["required"] == ["task", "bot_name"]
    assert "bob" in tool["parameters"]["properties"]["bot_name"]["enum"]
    assert "Hey Grok Bot" in session["instructions"]
    assert "delegate_to_grok_bot" in session["instructions"]


def test_force_message_asks_for_the_words_verbatim():
    events = force_message_events("Soup is ready.")
    assert events[0]["type"] == "conversation.item.create"
    text = events[0]["item"]["content"][0]["text"]
    assert text.startswith(FORCE_PREAMBLE)
    assert "Soup is ready." in text
    assert events[1] == {"type": "response.create"}


def test_parse_tool_arguments():
    assert parse_tool_arguments('{"task": "a", "bot_name": "bob"}') == {
        "task": "a",
        "bot_name": "bob",
    }
    assert parse_tool_arguments({"task": "a"}) == {"task": "a"}
    assert parse_tool_arguments("") == {}
    with pytest.raises(json.JSONDecodeError):
        parse_tool_arguments("{")
    with pytest.raises(ValueError):
        parse_tool_arguments("[1]")
