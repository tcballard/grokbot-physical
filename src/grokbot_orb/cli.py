"""Command line entry for the orb daemon."""

from __future__ import annotations

import argparse
import asyncio
import logging
import signal
from dataclasses import replace

from grokbot_orb import __version__
from grokbot_orb.config import Config
from grokbot_orb.daemon import OrbDaemon


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        prog="grokbot-orb",
        description="Hey Grok Bot Orb: local wake word, xAI Realtime voice, Grok Bot webhooks.",
    )
    parser.add_argument("--env-file", help="dotenv file to load (process environment wins)")
    parser.add_argument(
        "--dry-run",
        action="store_true",
        help="no microphone and no NeoPixel driver; serve /speak and /health anyway",
    )
    parser.add_argument(
        "--simulate-wake",
        action="store_true",
        help="fire one wake immediately, for a laptop or a headless server",
    )
    parser.add_argument(
        "--text",
        help="utterance to inject after a simulated wake, or to dispatch when no API key is set",
    )
    parser.add_argument("--bot", help="bot_name used by --simulate-wake when no Realtime session is opened")
    parser.add_argument("--log-level", help="DEBUG, INFO, WARNING, or ERROR")
    parser.add_argument("--version", action="version", version=f"grokbot-orb {__version__}")
    return parser


def config_from_args(argv: list[str] | None = None) -> Config:
    args = build_parser().parse_args(argv)
    config = Config.from_env(env_file=args.env_file)
    updates: dict[str, object] = {}
    if args.dry_run:
        updates["dry_run"] = True
    if args.simulate_wake:
        updates["simulate_wake"] = True
    if args.text is not None:
        updates["simulate_text"] = args.text
    if args.bot is not None:
        updates["simulate_bot"] = args.bot
    if args.log_level is not None:
        updates["log_level"] = args.log_level.upper()
    if updates:
        config = replace(config, **updates)
    return config


def main(argv: list[str] | None = None) -> int:
    config = config_from_args(argv)
    logging.basicConfig(
        level=getattr(logging, config.log_level, logging.INFO),
        format="%(asctime)s %(levelname)s %(name)s: %(message)s",
    )
    try:
        asyncio.run(_serve(config))
    except KeyboardInterrupt:
        logging.getLogger("grokbot_orb").info("stopped")
    return 0


async def _serve(config: Config) -> None:
    daemon = OrbDaemon(config)
    loop = asyncio.get_running_loop()
    for sig in (signal.SIGINT, signal.SIGTERM):
        try:
            loop.add_signal_handler(sig, daemon.request_stop)
        except NotImplementedError:
            break
    await daemon.run()
