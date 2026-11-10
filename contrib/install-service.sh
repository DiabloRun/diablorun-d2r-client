#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
APP_DIR="$(cd -- "$SCRIPT_DIR/.." && pwd)"
UNIT_DIR="${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user"
UNIT_NAME="diablorun-web.service"
UNIT_PATH="$UNIT_DIR/$UNIT_NAME"

if ! command -v node >/dev/null 2>&1; then
  echo "node was not found in PATH" >&2
  exit 1
fi

if [[ -x /usr/bin/node ]]; then
  NODE_BIN="/usr/bin/node"
else
  NODE_BIN="$(command -v node)"
fi

if [[ ! -f "$APP_DIR/dist/index.js" ]]; then
  echo "Building the app..."
  (cd "$APP_DIR" && npm run build)
fi

mkdir -p "$UNIT_DIR"

sed \
  -e "s|@APP_DIR@|$APP_DIR|g" \
  -e "s|@NODE@|$NODE_BIN|g" \
  "$SCRIPT_DIR/diablorun-web.service.in" > "$UNIT_PATH"

# Make the current graphical session environment available to the user service.
if [[ -n "${DISPLAY:-}" ]]; then
  systemctl --user import-environment DISPLAY
fi

if [[ -n "${XAUTHORITY:-}" ]]; then
  systemctl --user import-environment XAUTHORITY
fi

systemctl --user daemon-reload
systemctl --user enable "$UNIT_NAME"
systemctl --user restart "$UNIT_NAME"

echo
echo "Installed $UNIT_PATH"
echo "Control page: http://127.0.0.1:8787/"
echo "Overlay:      http://127.0.0.1:8787/overlay"
echo
echo "Status: systemctl --user status $UNIT_NAME"
echo "Logs:   journalctl --user -u $UNIT_NAME -f"
