# Diablo.run D2R Linux web timer

Local web timer for **Diablo II: Resurrected** on Linux. It watches the D2R
window and serves an OBS-friendly overlay with:

- **RTA** — wall-clock time. Keeps running during loads.
- **IGT** — in-game time. Pauses during loading and when capture is unavailable.

The control page at `/` shows capture status, a detector preview, and manual
timer controls. This branch replaces the old Electron/LiveSplit client; it does
not integrate with LiveSplit or sync character data to diablo.run.

## Requirements

- Linux with X11/XWayland. Gamescope and native Wayland are not supported.
- Node.js 22+, `ffmpeg`, `xdotool`.

```bash
# Arch
sudo pacman -S nodejs npm ffmpeg xdotool

# Debian/Ubuntu
sudo apt install nodejs npm ffmpeg xdotool
```

## Install and run

```bash
npm install
npm run build
npm start
```

- Control page: <http://127.0.0.1:8787/>
- OBS overlay: <http://127.0.0.1:8787/overlay>
- Development mode: `npm run dev`

The first run creates `~/.config/diablorun-web/config.json` (or under
`$XDG_CONFIG_HOME`).

## How it works

1. `xdotool` finds the D2R window; `ffmpeg` captures it at 25 fps and scales it
   to about 160 pixels wide as RGB24.
2. The detector reads each frame once and recognizes either the centered gold
   door/legacy loading frame or the nearly black bottom-right D2R logo screen.
   Gameplay, HUD, and UI panels are rejected.
3. Loading pauses IGT only. RTA keeps running. State is pushed to the browser
   at 4 Hz.
4. If the window disappears, IGT freezes until capture resumes.

Detection overhead is negligible; ffmpeg grabbing/scaling is the main cost.
Lower `capture.fps` for less CPU, or raise it for finer load timing. The
default is 25 fps.

## OBS setup

Add a **Browser** source pointing at `http://127.0.0.1:8787/overlay`, about
**360x160**, with a transparent background.

Optional URL parameters:

- `?scale=1.25` — overlay scale (0.25–5)
- `?decimals=3` — timer precision (0–3)
- `?status=1` — show PLAYING/LOADING/CAPTURE LOST
- `?labels=0` — hide RTA/IGT labels

Colors can be overridden with OBS custom CSS:

```css
:root {
  --rta-color: #fff;
  --igt-color: #9fd3ff;
  --label-color: rgba(255, 255, 255, 0.62);
}
```

## Hotkeys

`./scripts/install-hotkeys.sh` repoints the existing **Win+S** shortcut to
start/stop. `--all` also repoints **Win+R** (reset) and **Win+P** (pause/resume).
The timer must be running for hotkeys to work.

Manual commands:

```bash
curl -s -X POST http://127.0.0.1:8787/api/action/start-stop
curl -s -X POST http://127.0.0.1:8787/api/action/reset
curl -s -X POST http://127.0.0.1:8787/api/action/pause-resume
```

### API

| Method | Path | Description |
| --- | --- | --- |
| GET | `/api/state` | Timer and capture state |
| POST | `/api/action/{action}` | start, stop, start-stop, reset, pause-resume |
| GET | `/api/frame` | Latest detector frame (RGBA) |
| GET | `/healthz` | Health check |
| WS | `/ws` | State stream |

`start` always starts a fresh run. `stop` freezes the timers. Manual pause
freezes both timers; loading pauses only IGT.

## Configuration

Config path: `${XDG_CONFIG_HOME:-~/.config}/diablorun-web/config.json`
(override with `D2R_TIMER_CONFIG`). Restart after editing.

```json
{
  "host": "127.0.0.1",
  "port": 8787,
  "capture": {
    "fps": 25,
    "display": ":0",
    "windowClass": "d2r.exe",
    "windowTitle": "Diablo II: Resurrected",
    "minWidth": 160,
    "minHeight": 160
  },
  "timer": { "decimals": 2 },
  "overlay": { "scale": 1, "showStatus": false }
}
```

- `capture.fps` — capture/detection rate (25 default).
- `capture.display` — X display used by `xdotool`/`ffmpeg`.
- `capture.windowClass`/`windowTitle` — window matching.
- `capture.minWidth`/`minHeight` — ignore tiny helper windows.
- `timer.decimals` — default precision (0–3).
- `overlay.scale`/`showStatus` — overlay defaults.

## Autostart

```bash
./contrib/install-service.sh
```

Stop any manual instance first. Re-run `npm run build` after source changes. If
the repo is on a late-mounted drive, move it under your home directory or add a
mount dependency.

```bash
systemctl --user status diablorun-web.service
journalctl --user -u diablorun-web.service -f
systemctl --user restart diablorun-web.service
systemctl --user disable --now diablorun-web.service
```

If the service cannot reach X:

```bash
systemctl --user import-environment DISPLAY XAUTHORITY
systemctl --user restart diablorun-web.service
```

## Troubleshooting

- **Searching for D2R** — make sure the game is running and unminimized. Check
  `xdotool search --name 'Diablo' getwindowname %@`; adjust `windowClass` or
  `windowTitle` if needed.
- **Error** — inspect the error box or run from a terminal; make sure `ffmpeg`
  and `xdotool` are installed.
- **Wrong IGT pauses** — inspect the detector preview on the control page. Try a
  normal 16:9 resolution if heavily letterboxed. At 25 fps transitions can be
  off by up to about 40 ms; raise `capture.fps` for finer timing.
- **Blank or stale OBS overlay** — reload the browser source or refresh its
  cache; verify the port and that the server is running.
- **Port 8787 in use** — change `port` in the config and update the OBS URL.

## Limitations

No splits/PB tracking or LiveSplit integration; timer state is in memory; the
server binds to localhost without authentication; character-data sync is not
included.

## Tests

```bash
npm test
npm run typecheck
```

Covers door/legacy/logo loading fixtures, real ffmpeg output, in-game
false-positive cases, and the RTA/IGT state machine.

## License

MIT
