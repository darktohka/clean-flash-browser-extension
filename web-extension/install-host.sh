#!/usr/bin/env bash
#
# Install the native messaging host manifest for Chrome/Chromium and Firefox.
#
# Usage:
#   ./install-host.sh [path-to-flash-player-host-binary]
#
# If no path is given, defaults to the binary in the workspace's
# target/release directory.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
DEFAULT_BIN="$(cd "$SCRIPT_DIR/.." && pwd)/target/release/flash-player-host"

HOST_BIN="${1:-$DEFAULT_BIN}"
HOST_BIN="$(realpath "$HOST_BIN")"

if [[ ! -x "$HOST_BIN" ]]; then
  echo "Error: host binary not found or not executable: $HOST_BIN"
  echo "Build with:  cargo build --release -p player-web"
  exit 1
fi

HOST_NAME="org.cleanflash.flash_player"

# ---- Firefox ----
FIREFOX_DIR="$HOME/.mozilla/native-messaging-hosts"
mkdir -p "$FIREFOX_DIR"

cat > "$FIREFOX_DIR/$HOST_NAME.json" <<EOF
{
  "name": "$HOST_NAME",
  "description": "Flash Player Native Messaging Host",
  "path": "$HOST_BIN",
  "type": "stdio",
  "allowed_extensions": ["flash-player@cleanflash.org"]
}
EOF
echo "Installed Firefox manifest: $FIREFOX_DIR/$HOST_NAME.json"

# ---- Chrome / Chromium ----
for CHROME_DIR in \
  "$HOME/.config/google-chrome/NativeMessagingHosts" \
  "$HOME/.config/chromium/NativeMessagingHosts" \
  "$HOME/.config/BraveSoftware/Brave-Browser/NativeMessagingHosts"; do

  mkdir -p "$CHROME_DIR"
  cat > "$CHROME_DIR/$HOST_NAME.json" <<EOF
{
  "name": "$HOST_NAME",
  "description": "Flash Player Native Messaging Host",
  "path": "$HOST_BIN",
  "type": "stdio",
  "allowed_origins": ["chrome-extension://dcikaadaeajidejkoekdflmfdgeoldcb/"]
}
EOF
  echo "Installed Chrome manifest: $CHROME_DIR/$HOST_NAME.json"
done

echo ""
echo "Done. Make sure FLASH_PLUGIN_PATH is set to the PepperFlash .so path."
