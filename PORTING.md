# Python to Rust port

Source: `64018e7432ccb3be62b1b73a335e76b6dc50dfa9` on `main`.
The Rust branch replaces the Python application and package. It keeps the
`grokbot-orb` executable, CLI flags, environment names, webhook schema,
`/health`, `/speak`, bot routing, and halo colours. No Python interpreter,
virtualenv, Python wake service, or ONNX Runtime shared library is needed.

## API mapping

Python APIs were internal application code; they are not Python import shims.

| Python surface | Rust equivalent |
| --- | --- |
| `Config`, `from_env`, `_load_env_file`, `_env_bool`, validation | `config::Config::{from_map,merge_env,validate}` |
| `build_parser`, `config_from_args`, `main`, `_serve`, `__main__` | `config::Args`, `Config::from_args`, `src/main.rs` |
| `Config.realtime_url`, `for_test` | `Config::realtime_url`; test config constructors |
| `Bot`, `BOTS`, `_compact`, `_alias_map`, `normalise_bot_name`, `bot_roster` | `bots::{NAMES,ROSTER,normalise,colour}` |
| `resample_pcm16` | `audio::resample` |
| `PcmRing` constructor, extend, snapshot | `audio::PcmRing::{new,extend,snapshot}` |
| `PcmInput` start, callback, queue offer/read/close | `wakeword::capture`, owned `arecord` child and 32-block broadcast ring |
| `PcmOutput` open/write/close | `audio::Player::{new,write,close}`, owned `aplay` child |
| `Theme.level/levels`, `_scale`, `canonical_state`, `theme_for` | `halo::State::{from_str,fmt}`, `Halo::{colour,render}` |
| `Halo` state/history, set_state, render, flush, run, close | `halo::Halo`, daemon frame loop and `run_driver` |
| `HaloBackend`, `LogBackend`, `Ws281xBackend`, `open_backend` | log rendering; optional `rs_ws281x` controller on its owning thread |
| `WakeModel`, `load_openwakeword`, `describe_wake_model` | `wakeword::{WakeModel::load,describe_model}` |
| `WakeDetector.feed`, `_monotonic` | `WakeDetector::feed(score, monotonic_seconds)` plus `Instant` |
| `orb_instructions`, `delegate_tool`, `build_session_update` | `protocol::session_update` |
| `force_message_events`, `user_text_events`, `parse_tool_arguments` | `protocol::{force_message,user_text,parse_tool_arguments}` |
| `RealtimeSession` open/read/dispatch/send/close, `_connect_kwargs` | `realtime::run`, typed transport, `SessionGuard` cleanup |
| `send_audio`, inject text, force message, return tool | queued protocol events; `realtime::send` waits for socket-write acknowledgement |
| `SpeakRequest`, `SpeakError`, `parse_speak_request` | `speak::{SpeakRequest,Status,parse_request}` and explicit HTTP error responses |
| `create_app`, `_health`, `_speak`, `_authorized`, `_error` | `speak::router` and its route handlers |
| `SiteHandle`, `start_site` | `Daemon::start`, `Running::{addr,stop}` |
| `DispatchResult`, `build_headers`, `build_payload`, URL validation | `webhook::{DispatchResult,headers,payload,dispatch}` |
| `WebhookClient.build/dispatch/_post` | `webhook::dispatch`; same one retry after 400 ms on 502/503/504 |
| `synthesize`, `_xai_tts`, `_espeak`, `_wav_pcm` | `tts::{synthesize,wav_pcm}` and private native backends |
| `OrbDaemon` construction, start, run, stop, request_stop | `Daemon::{new,start}`, `Running::stop`, signal watch channel |
| `health`, `http_port`, `session_active`, `callback_url` | `Daemon::{health,callback_url}`, `Running::addr` |
| submit/handle speak, speak loop, idle speak, remember | bounded eight-request queue, `Daemon::handle_speak`, `remember` |
| simulated wake, capture loop, on_capture | daemon voice task, `wakeword::capture` |
| run/watch session, pump live, assistant audio, realtime states | `realtime::run` with independent bounded transport and playback queues |
| on_tool, dispatch_tool, restore_halo, touch, wake cooldown | `Daemon::{on_tool,dispatch,restore}`, `RuntimeState` |
| output_rate, session_busy, warn_config, reachability_warning | `Config::output_rate`, runtime state, startup diagnostics, `reachability_warning` |
| `_offer`, task error callback | bounded broadcast queues drop oldest capture/live blocks; task result handling |

## Test mapping

All 27 original tests retain their original names. The original suite passed
before replacement. Its source remains available at the source commit above.

| Original test | Rust test |
| --- | --- |
| `test_audio.py::test_resample_known_ratios_and_identity` | `tests/port.rs::test_resample_known_ratios_and_identity` |
| `test_audio.py::test_resample_holds_a_constant_level` | `tests/port.rs::test_resample_holds_a_constant_level` |
| `test_audio.py::test_ring_keeps_the_newest_samples` | `tests/port.rs::test_ring_keeps_the_newest_samples` |
| `test_config.py::test_env_file_and_process_env` | `tests/port.rs::test_env_file_and_process_env` |
| `test_config.py::test_invalid_threshold_is_rejected` | `tests/port.rs::test_invalid_threshold_is_rejected` |
| `test_config.py::test_cli_flags_override_env` | `tests/port.rs::test_cli_flags_override_env` |
| `test_config.py::test_help_exits_cleanly` | `tests/port.rs::test_help_exits_cleanly` |
| `test_daemon.py::test_speak_during_a_session_injects_force_message` | `tests/network.rs::test_speak_during_a_session_injects_force_message` |
| `test_daemon.py::test_idle_callback_is_spoken_and_health_stays_up` | `tests/network.rs::test_idle_callback_is_spoken_and_health_stays_up` |
| `test_daemon.py::test_simulated_wake_posts_the_webhook` | `tests/network.rs::test_simulated_wake_posts_the_webhook` |
| `test_daemon.py::test_reachability_warnings` | `tests/port.rs::test_reachability_warnings` |
| `test_halo.py::test_state_transitions_and_bot_colours` | `tests/port.rs::test_state_transitions_and_bot_colours` |
| `test_halo.py::test_unknown_state_is_rejected` | `tests/port.rs::test_unknown_state_is_rejected` |
| `test_halo.py::test_listening_breathes_and_working_comets` | `tests/port.rs::test_listening_breathes_and_working_comets` |
| `test_halo.py::test_brightness_caps_the_frame` | `tests/port.rs::test_brightness_caps_the_frame` |
| `test_realtime.py::test_session_update_is_24k_server_vad_with_the_tool` | `tests/port.rs::test_session_update_is_24k_server_vad_with_the_tool` |
| `test_realtime.py::test_force_message_asks_for_the_words_verbatim` | `tests/port.rs::test_force_message_asks_for_the_words_verbatim` |
| `test_realtime.py::test_parse_tool_arguments` | `tests/port.rs::test_parse_tool_arguments` |
| `test_speak.py::test_parse_speak_request` | `tests/port.rs::test_parse_speak_request` |
| `test_speak.py::test_health_is_open_and_speak_checks_the_token` | `tests/network.rs::test_health_is_open_and_speak_checks_the_token` |
| `test_speak.py::test_full_queue_returns_503` | `tests/network.rs::test_full_queue_returns_503` |
| `test_wakeword.py::test_threshold_and_cooldown` | `tests/port.rs::test_threshold_and_cooldown` |
| `test_wakeword.py::test_describe_missing_model_names_the_fallback` | `tests/port.rs::test_describe_missing_model_names_the_fallback` |
| `test_webhook.py::test_normalise_bot_names` | `tests/port.rs::test_normalise_bot_names` |
| `test_webhook.py::test_headers_and_payload_keep_secrets_apart` | `tests/port.rs::test_headers_and_payload_keep_secrets_apart` |
| `test_webhook.py::test_dispatch_posts_bearer_and_automation_key` | `tests/network.rs::test_dispatch_posts_bearer_and_automation_key` |
| `test_webhook.py::test_dispatch_without_url_does_not_raise` | `tests/port.rs::test_dispatch_without_url_does_not_raise` |

Additional tests cover gateway retry, callback body limits, xAI TTS requests,
Realtime tool-result delivery, and positive/negative native wake inference.
The fixture provenance and regeneration parameters are in
`tests/fixtures/README.md`. The example `wake_scores` scores a local WAV.

## Preserved behaviour and quirks

- Bot aliases compact ASCII letters/numbers; unknown identifiers pass through
  up to 32 characters. Roster order stays Bob, OmaBot, Writer.
- PCM ignores an odd trailing byte, uses linear interpolation and ties-to-even
  rounding, and leaves a single sample unchanged across sample rates.
- WAV input retains its first channel, rather than averaging stereo channels.
- The first five wake scores are zeroed; threshold and cooldown are unchanged.
- `working` callbacks update the halo without speaking, even if text is present.
- `done` with a bot first uses that bot's halo. Active sessions get verbatim
  force-message events; idle callbacks use xAI TTS, then configured espeak.
- `TTS_FALLBACK=espeak` still tries xAI first when an API key is configured.
- `--dry-run --simulate-wake --text` still dispatches a real webhook when no
  API key is configured. With a key it opens a real Realtime session.
- A failed microphone/wake backend leaves the callback server up with an error
  halo. `/health` retains the original response shape and status contract.
- GPIO 13/19 select PWM channel 1 when the channel is omitted. Driver failure
  falls back to log output. A successful driver is blanked on shutdown.

## Deliberate differences and migration

1. Audio uses native ALSA `arecord`/`aplay` instead of Python PortAudio.
   `AUDIO_RATE=0` selects 16000 Hz on both ends; it no longer queries the
   capture card's preferred rate. Set 48000 explicitly when necessary and use
   ALSA `plughw:...` device names. This also avoids the source's mismatch where
   the output stream opened at 16 kHz before its rate attribute was changed.
2. Native tract inference uses the same ONNX mel, embedding and classifier
   pipeline. Initial classifier history is deterministic zeros instead of
   openWakeWord's random startup embeddings. The 16-frame default window is
   fully real after 1.28 seconds. Positive/negative scores after this warm-up
   match Python within 0.000001 on the checked recordings; this is not a claim
   about every model or microphone. Custom fixed frame windows up to 120 work.
3. Run `scripts/fetch_wake_models.sh` once before capture. Downloads are explicit,
   versioned and SHA-256 checked instead of performed at first wake startup.
   The script supplies the default Jarvis classifier and feature models only.
   Other fallback classifiers must be installed as `<name>_v0.1.onnx` under
   `WAKE_ASSETS_DIR`. There is no TFLite fallback.
4. Halo transition history is capped at 128 entries; PCM is not retained during
   dry-run. Spoken history remains capped at 50. These remove unbounded
   diagnostic allocations from the original long-running daemon.
5. HTTP redirects are rejected for webhook/TTS requests to keep credentials
   and callback tokens at the configured endpoint. Use the final endpoint URL.
   Streamed/chunked callback bodies also enforce the 32 KiB body limit.
6. Configuration rejects non-finite durations and invalid PWM channels early.
   Rust uses bounded numeric types. Tool task/bot fields must be strings;
   arbitrary JSON values are no longer coerced through Python `str()`.
7. Audio playback has a 128-chunk bounded queue. An overflow drops incoming
   assistant audio and logs a warning. Capture and live-send queues retain
   the original drop-oldest behaviour (32 and 64 blocks respectively).
8. Explicit speaker write errors surface in the callback worker and set the
   error halo; the Python backend could silently drop the reply. The next
   playback attempt reopens the native player.

## Verification

- Original Python suite: 27 passed.
- Rust suite: 33 tests, including all 27 mapped tests and two real ONNX fixture
  tests. Run `scripts/fetch_wake_models.sh` then
  `cargo test --all-features -- --include-ignored`.
- `cargo build`, `cargo fmt --check`, strict all-feature Clippy and rustdoc.
- CI covers Linux x86-64 and ARM64, including optional Pi driver compilation,
  and a no-default-features build for HTTP/simulated bring-up.
- No real xAI or Grok Bot requests were sent during validation: protocol tests
  use loopback HTTP/WebSocket servers and dummy credentials.
- No physical Pi, HAT, NeoPixel ring, or real xAI account was available. Check
  capture/playback, wake latency/false triggers, echo/barge-in behaviour, driver
  privileges, service installation and shutdown on the target before merging.
  ARM64 CI is a build check, not evidence of GPIO operation.

## Rollback

This is a separate branch; `main` retains the Python implementation until merge.
An existing virtualenv is not deleted by the installer. If rolling back an
already installed preview, stop the service, restore the old unit's
`ExecStart=/opt/hey-grokbot-orb/.venv/bin/grokbot-orb`, run `daemon-reload`, and
start it again. Keep `/etc/grokbot-orb.env` and the models in place.
