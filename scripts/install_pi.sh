#!/usr/bin/env bash
# Install the native Rust orb on Raspberry Pi OS 64-bit.
# Requires Rust 1.91+ in the invoking user's PATH or ~/.cargo/bin.
# sudo ./scripts/install_pi.sh [--with-voicecard]
# Optionally supply ORB_BINARY=/path/to/a/prebuilt/grokbot-orb.
# The unit is enabled but not started; existing credentials are preserved.
set -euo pipefail
WITH_VOICE_CARD=0
for arg in "$@"; do
    case "$arg" in
        --with-voicecard) WITH_VOICE_CARD=1 ;;
        -h|--help) sed -n '2,6p' "$0"; exit 0 ;;
        *) echo "Unknown argument: $arg" >&2; exit 2 ;;
    esac
done
if [[ "$EUID" -ne 0 ]]; then
    exec sudo -- "$0" "$@"
fi
TARGET_USER="${SUDO_USER:-${ORB_USER:-pi}}"
if [[ "$TARGET_USER" == root ]]; then TARGET_USER="${ORB_USER:-pi}"; fi
id "$TARGET_USER" >/dev/null
TARGET_GROUP="$(id -gn "$TARGET_USER")"
TARGET_USER_HOME="$(getent passwd "$TARGET_USER" | cut -d: -f6)"
PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PREFIX="${ORB_PREFIX:-/opt/hey-grokbot-orb}"
ENV_FILE="${ORB_ENV_FILE:-/etc/grokbot-orb.env}"
for path in "$PREFIX" "$ENV_FILE"; do
    if [[ ! "$path" =~ ^/[a-zA-Z0-9_./-]+$ ]]; then echo "Unsupported install path: $path" >&2; exit 2; fi
done
export DEBIAN_FRONTEND=noninteractive
apt-get update
apt-get install -y build-essential libclang-dev pkg-config git ca-certificates curl alsa-utils espeak-ng
if [[ -n "${ORB_BINARY:-}" ]]; then
    BINARY="$ORB_BINARY"
else
    BUILD_DIR="$(mktemp -d /tmp/grokbot-build.XXXXXX)"
    trap 'rm -rf "$BUILD_DIR"' EXIT
    chown "$TARGET_USER:$TARGET_GROUP" "$BUILD_DIR"
    sudo -u "$TARGET_USER" env PATH="$TARGET_USER_HOME/.cargo/bin:$PATH" \
        cargo build --release --locked --features pi --manifest-path "$PROJECT_ROOT/Cargo.toml" --target-dir "$BUILD_DIR"
    BINARY="$BUILD_DIR/release/grokbot-orb"
fi
install -d -m 755 "$PREFIX/bin" "$PREFIX/models"
install -m 755 "$BINARY" "$PREFIX/bin/grokbot-orb"
"$PROJECT_ROOT/scripts/fetch_wake_models.sh" "$PREFIX/models"
if [[ -f "$PROJECT_ROOT/models/hey_grok_bot.onnx" && ! -f "$PREFIX/models/hey_grok_bot.onnx" ]]; then
    install -m 644 "$PROJECT_ROOT/models/hey_grok_bot.onnx" "$PREFIX/models/hey_grok_bot.onnx"
fi
usermod -aG audio "$TARGET_USER"
getent group gpio >/dev/null || groupadd --system gpio
usermod -aG gpio "$TARGET_USER"
for boot_config in /boot/firmware/config.txt /boot/config.txt; do
    if [[ -f "$boot_config" ]]; then
        if ! grep -q '^dtparam=i2s=on' "$boot_config"; then echo 'dtparam=i2s=on' >> "$boot_config"; fi
        break
    fi
done
if [[ ! -f "$ENV_FILE" ]]; then
    install -m 600 -o root -g root "$PROJECT_ROOT/.env.example" "$ENV_FILE"
fi
sed -e "s|^User=.*|User=$TARGET_USER|" \
    -e "s|^WorkingDirectory=.*|WorkingDirectory=$PREFIX|" \
    -e "s|^EnvironmentFile=.*|EnvironmentFile=$ENV_FILE|" \
    -e "s|^ExecStart=.*|ExecStart=$PREFIX/bin/grokbot-orb|" \
    "$PROJECT_ROOT/systemd/grokbot-orb.service" > /etc/systemd/system/grokbot-orb.service
chmod 644 /etc/systemd/system/grokbot-orb.service
systemctl daemon-reload
systemctl enable grokbot-orb.service
if [[ "$WITH_VOICE_CARD" -eq 1 ]]; then
    if [[ ! -d /usr/src/seeed-voicecard/.git ]]; then
        git clone --depth 1 https://github.com/respeaker/seeed-voicecard.git /usr/src/seeed-voicecard
    fi
    (cd /usr/src/seeed-voicecard && ./install.sh)
fi
cat <<MSG
Installed the Rust daemon. The service is enabled and has not been started.
Edit $ENV_FILE, then run:
  sudo systemctl start grokbot-orb
  journalctl -u grokbot-orb -f
  curl -s http://127.0.0.1:8765/health
MSG
