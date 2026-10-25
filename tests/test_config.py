import pytest

from grokbot_orb.cli import config_from_args
from grokbot_orb.config import Config


def test_env_file_and_process_env(tmp_path):
    env_file = tmp_path / "orb.env"
    env_file.write_text(
        "\n".join(
            [
                "# comment",
                "export WAKE_THRESHOLD=0.42",
                "ORB_CALLBACK_PORT=9001",
                'XAI_VOICE="ara"',
                "HALO_PIN=13",
            ]
        ),
        encoding="utf-8",
    )
    config = Config.from_env(
        environ={"WAKE_THRESHOLD": "0.7", "DRY_RUN": "yes"},
        env_file=env_file,
    )
    assert config.wake_threshold == 0.7  # process environment wins
    assert config.callback_port == 9001
    assert config.xai_voice == "ara"
    assert config.halo_pin == 13
    assert config.halo_channel == 1  # GPIO 13 is PWM channel 1
    assert config.dry_run is True


def test_invalid_threshold_is_rejected():
    with pytest.raises(ValueError):
        Config(wake_threshold=0)


def test_cli_flags_override_env(tmp_path, monkeypatch):
    monkeypatch.delenv("DRY_RUN", raising=False)
    monkeypatch.delenv("SIMULATE_WAKE", raising=False)
    env_file = tmp_path / ".env"
    env_file.write_text("SIMULATE_BOT=writer\n", encoding="utf-8")
    config = config_from_args(
        [
            "--env-file",
            str(env_file),
            "--dry-run",
            "--simulate-wake",
            "--text",
            "Check the milk",
            "--bot",
            "bob",
            "--log-level",
            "debug",
        ]
    )
    assert config.dry_run is True
    assert config.simulate_wake is True
    assert config.simulate_text == "Check the milk"
    assert config.simulate_bot == "bob"
    assert config.log_level == "DEBUG"


def test_help_exits_cleanly():
    with pytest.raises(SystemExit) as caught:
        config_from_args(["--help"])
    assert caught.value.code == 0
