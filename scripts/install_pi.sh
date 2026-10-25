#!/usr/bin/env bash
# Install Hey Grok Bot Orb on Raspberry Pi OS.
#
#   sudo ./scripts/install_pi.sh
#   sudo ./scripts/install_pi.sh --with-voicecard
#
# --with-voicecard clones Seeed's seeed-voicecard installer (ReSpeaker 2-Mics
# V1, WM8960). V2 boards use the respeaker-2mic-v2 overlay instead; see the
# README. The Seeed script may ask you to reboot. This installer is idempotent,
# so run it again afterwards.
#
# The systemd unit is enabled and not started. Fill /etc/grokbot-orb.env first.

set -euo pipefail

WITH_VOICE_CARD=0
for arg in "$@"; do
    case "${arg}" in
        --with-voicecard) WITH_VOICE_CARD=1 ;;
        -h|--help)
            sed -n '2,16p' "$0"
            exit 0
            ;;
        *)
            echo "Unknown argument: ${arg}" >&2
            exit 2
            ;;
    esac
done

if [[ "${EUID}" -ne 0 ]]; then
    exec sudo -- "$0" "$@"
fi

TARGET_USER="${SUDO_USER:-}"
if [[ -z "${TARGET_USER}" || "${TARGET_USER}" == "root" ]]; then
    TARGET_USER="${ORB_USER:-pi}"
fi
if ! id "${TARGET_USER}" >/dev/null 2>&1; then
    echo "User ${TARGET_USER} does not exist. Set ORB_USER and run again." >&2
    exit 1
fi

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PREFIX="${ORB_PREFIX:-/opt/hey-grokbot-orb}"
ENV_FILE="${ORB_ENV_FILE:-/etc/grokbot-orb.env}"
BOOT_CONFIG=""
if [[ -f /boot/firmware/config.txt ]]; then
    BOOT_CONFIG=/boot/firmware/config.txt
elif [[ -f /boot/config.txt ]]; then
    BOOT_CONFIG=/boot/config.txt
fi

echo "Installing Hey Grok Bot Orb for user ${TARGET_USER} into ${PREFIX}"

export DEBIAN_FRONTEND=noninteractive
apt-get update
apt-get install -y \
    python3 \
    python3-venv \
    python3-dev \
    python3-pip \
    build-essential \
    swig \
    git \
    ca-certificates \
    alsa-utils \
    espeak-ng \
    libportaudio2 \
    portaudio19-dev

if [[ -n "${BOOT_CONFIG}" ]]; then
    if ! grep -q '^dtparam=i2s=on' "${BOOT_CONFIG}"; then
        echo 'dtparam=i2s=on' >> "${BOOT_CONFIG}"
        echo "Enabled I2S in ${BOOT_CONFIG}. A reboot is required before the HAT enumerates."
    fi
fi

if ! id -nG "${TARGET_USER}" | tr ' ' '\n' | grep -qx audio; then
    usermod -aG audio "${TARGET_USER}"
fi
if getent group gpio >/dev/null 2>&1; then
    usermod -aG gpio "${TARGET_USER}" || true
fi

install -d -o "${TARGET_USER}" -g "${TARGET_USER}" "${PREFIX}"
tar -C "${ROOT}" \
    --exclude .git \
    --exclude .venv \
    --exclude __pycache__ \
    --exclude .pytest_cache \
    --exclude '*.stl' \
    -cf - . | tar -C "${PREFIX}" -xf -
chown -R "${TARGET_USER}:${TARGET_USER}" "${PREFIX}"

if [[ ! -x "${PREFIX}/.venv/bin/python" ]]; then
    sudo -u "${TARGET_USER}" python3 -m venv "${PREFIX}/.venv"
fi
sudo -u "${TARGET_USER}" "${PREFIX}/.venv/bin/pip" install --upgrade pip wheel
sudo -u "${TARGET_USER}" "${PREFIX}/.venv/bin/pip" install -e "${PREFIX}[pi]"

sudo -u "${TARGET_USER}" "${PREFIX}/.venv/bin/python" - <<'PY' || \
    echo "Wake-model prefetch failed; the daemon retries on first start."
try:
    from openwakeword.utils import download_models
    download_models()
except Exception as exc:
    raise SystemExit(str(exc))
PY

if [[ ! -f "${ENV_FILE}" ]]; then
    install -m 600 -o root -g root "${ROOT}/.env.example" "${ENV_FILE}"
    echo "Wrote ${ENV_FILE}. Put your keys in it before starting the service."
else
    echo "Leaving existing ${ENV_FILE} in place."
fi

UNIT_PATH=/etc/systemd/system/grokbot-orb.service
sed "s/^User=.*/User=${TARGET_USER}/" "${ROOT}/systemd/grokbot-orb.service" > "${UNIT_PATH}"
chmod 644 "${UNIT_PATH}"
systemctl daemon-reload
systemctl enable grokbot-orb.service

cat <<EOF

Installed. The service is enabled and not started.

  1. Edit ${ENV_FILE}
     XAI_API_KEY, GROKBOT_WEBHOOK_URL, GROKBOT_WEBHOOK_BEARER,
     GROKBOT_AUTOMATION_KEY, ORB_PUBLIC_BASE, ORB_SPEAK_TOKEN.
  2. sudo systemctl start grokbot-orb
  3. journalctl -u grokbot-orb -f
  4. curl -s http://127.0.0.1:8765/health

If the halo stays dark, the journal will say the NeoPixel backend is
unavailable. The rpi_ws281x mailbox often needs root: set User=root in
${UNIT_PATH} and run systemctl daemon-reload.

EOF

if [[ "${WITH_VOICE_CARD}" -eq 1 ]]; then
    if [[ ! -d /usr/src/seeed-voicecard/.git ]]; then
        git clone --depth 1 https://github.com/respeaker/seeed-voicecard.git /usr/src/seeed-voicecard
    fi
    echo "Running seeed-voicecard install.sh (ReSpeaker 2-Mics V1). Re-run this script if it reboots."
    (cd /usr/src/seeed-voicecard && ./install.sh)
else
    echo "Skipped the Seeed voice-card installer. Re-run with --with-voicecard for a V1 WM8960 HAT."
fi
