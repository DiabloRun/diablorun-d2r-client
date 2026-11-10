#!/usr/bin/env bash
set -euo pipefail

requested="${1:-}"

case "$requested" in
  start|stop|start-stop|start-or-split|reset|pause-resume|toggle-pause) ;;
  *)
    echo "usage: $(basename "$0") <start-stop|start|stop|reset|pause-resume>" >&2
    exit 2
    ;;
esac

case "$requested" in
  start-or-split) action="start-stop" ;;
  toggle-pause) action="pause-resume" ;;
  *) action="$requested" ;;
esac

base_url="${D2R_TIMER_URL:-http://127.0.0.1:8787}"

if ! curl -fsS --noproxy '*' --connect-timeout 1 --max-time 2 \
  -X POST "${base_url%/}/api/action/${action}" >/dev/null; then
  logger -t d2r-timer "global hotkey '${action}' failed: ${base_url}" 2>/dev/null || true
  exit 1
fi
