#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
APP_DIR="$(cd -- "$SCRIPT_DIR/.." && pwd)"
HOTKEY_SCRIPT="$APP_DIR/scripts/hotkey.sh"

MEDIA_KEYS_SCHEMA="org.gnome.settings-daemon.plugins.media-keys"
CUSTOM_SCHEMA="org.gnome.settings-daemon.plugins.media-keys.custom-keybinding"
CUSTOM_BASE="/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings"

# Reuse the existing D2 splitter shortcut IDs where possible so Win+S does not
# end up with two conflicting bindings.
SPLIT_PATH="$CUSTOM_BASE/d2-splitter-split/"
RESET_PATH="$CUSTOM_BASE/d2-splitter-reset/"
PAUSE_PATH="$CUSTOM_BASE/d2-splitter-pause/"
SPLIT_KEY="<Super>s"
ALL=0

usage() {
  cat <<'EOF'
Usage: install-hotkeys.sh [options]

Repoints the existing D2 splitter GNOME shortcuts at this timer.

Default:
  Win+S  Start / stop the RTA + IGT timers

With --all, also:
  Win+R  Reset the timers
  Win+P  Pause / resume the timers

Options:
  --url=URL        Timer service URL (default: http://127.0.0.1:8787)
  --all            Also repoint reset and pause/resume
  -h, --help       Show this help

The old D2 splitter's other shortcuts (previous/next split, OBS record,
new-run macro) are left untouched. Re-run the old splitter's
install-hotkeys.sh to restore its shortcuts.
EOF
}

if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
  usage
  exit 0
fi

while [ "$#" -gt 0 ]; do
  case "$1" in
    --all)
      ALL=1
      shift
      ;;
    --url=*)
      TIMER_URL="${1#--url=}"
      shift
      ;;
    --url)
      if [ "$#" -lt 2 ]; then
        echo "error: --url requires a value" >&2
        exit 2
      fi
      TIMER_URL="$2"
      shift 2
      ;;
    *)
      echo "error: unknown option: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

TIMER_URL="${TIMER_URL:-http://127.0.0.1:8787}"

if ! command -v gsettings >/dev/null 2>&1; then
  echo "error: gsettings was not found. This installer is for GNOME." >&2
  exit 1
fi

get_custom_paths() {
  gsettings get "$MEDIA_KEYS_SCHEMA" custom-keybindings
}

path_is_installed() {
  local needle="$1"
  get_custom_paths | grep -Fq "'$needle'"
}

ensure_path_installed() {
  local path="$1"

  if path_is_installed "$path"; then
    return
  fi

  local current
  current="$(get_custom_paths)"

  if [ "$current" = "@as []" ] || [ "$current" = "[]" ]; then
    gsettings set "$MEDIA_KEYS_SCHEMA" custom-keybindings "['$path']"
    return
  fi

  if [[ "$current" == "["*"]" ]]; then
    gsettings set "$MEDIA_KEYS_SCHEMA" custom-keybindings "${current%]*}, '$path']"
  else
    echo "error: unexpected custom-keybindings value: $current" >&2
    exit 1
  fi
}

safe_get() {
  local schema="$1"
  local key="$2"
  local fallback="$3"
  local value

  if ! value="$(gsettings get "$schema" "$key" 2>/dev/null)"; then
    echo "$fallback"
    return
  fi

  echo "$value"
}

set_shortcut() {
  local path="$1"
  local name="$2"
  local command="$3"
  local binding="$4"
  local schema="$CUSTOM_SCHEMA:$path"

  ensure_path_installed "$path"

  local old_command
  old_command="$(safe_get "$schema" command "''")"

  gsettings set "$schema" name "$name"
  gsettings set "$schema" command "$command"
  gsettings set "$schema" binding "$binding"

  local display="$binding"
  display="${display//<Super>/Win+}"
  display="${display//<Control>/Ctrl+}"
  display="${display//<Shift>/Shift+}"
  display="${display//<Alt>/Alt+}"

  printf '  %-12s %s\n' "$display" "$name"
  printf '               %s\n' "$HOTKEY_SCRIPT"
  if [ -n "$old_command" ] && [ "$old_command" != "''" ] && [ "$old_command" != "$command" ]; then
    printf '               previous command: %s\n' "$old_command"
  fi
}

COMMAND_PREFIX=""
if [ "$TIMER_URL" != "http://127.0.0.1:8787" ]; then
  COMMAND_PREFIX="D2R_TIMER_URL=${TIMER_URL} "
fi

echo "Installing GNOME shortcuts for the D2R timer..."
set_shortcut "$SPLIT_PATH" "D2R Timer: Start / Stop" "${COMMAND_PREFIX}${HOTKEY_SCRIPT} start-stop" "$SPLIT_KEY"

if [ "$ALL" -eq 1 ]; then
  set_shortcut "$RESET_PATH" "D2R Timer: Reset" "${COMMAND_PREFIX}${HOTKEY_SCRIPT} reset" "<Super>r"
  set_shortcut "$PAUSE_PATH" "D2R Timer: Pause / Resume" "${COMMAND_PREFIX}${HOTKEY_SCRIPT} pause-resume" "<Super>p"
fi

echo
echo "Done. Make sure the timer is running:"
echo "  cd \"$APP_DIR\" && npm start"
echo "Or install the user service:"
echo "  cd \"$APP_DIR\" && ./contrib/install-service.sh"
