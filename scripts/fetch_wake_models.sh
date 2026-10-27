#!/usr/bin/env bash
set -euo pipefail
DEST="${1:-models}"
mkdir -p "$DEST"
RELEASE=https://github.com/dscripka/openWakeWord/releases/download/v0.5.1
fetch() {
    local name="$1" hash="$2" temporary
    if [[ -f "$DEST/$name" ]] && printf '%s  %s\n' "$hash" "$DEST/$name" | sha256sum --check --status; then
        return
    fi
    temporary="$(mktemp "$DEST/.download.XXXXXX")"
    if curl --fail --location --retry 2 --connect-timeout 20 --max-time 180 "$RELEASE/$name" -o "$temporary" &&
        printf '%s  %s\n' "$hash" "$temporary" | sha256sum --check --status; then
        chmod 644 "$temporary"
        mv "$temporary" "$DEST/$name"
    else
        rm -f "$temporary"
        echo "Download or checksum failed for $name" >&2
        return 1
    fi
}
fetch melspectrogram.onnx ba2b0e0f8b7b875369a2c89cb13360ff53bac436f2895cced9f479fa65eb176f
fetch embedding_model.onnx 70d164290c1d095d1d4ee149bc5e00543250a7316b59f31d056cff7bd3075c1f
fetch hey_jarvis_v0.1.onnx 94a13cfe60075b132f6a472e7e462e8123ee70861bc3fb58434a73712ee0d2cb
