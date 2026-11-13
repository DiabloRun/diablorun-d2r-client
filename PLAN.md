# D2R Load Remover for LiveSplit (Windows, Rust) — Plan

Status: draft for review. Nothing below is implemented yet.

## 1. Goal

A small, portable Windows app that watches Diablo II: Resurrected and tells
LiveSplit when the game is on a loading screen. LiveSplit stays the timer; this
app only drives LiveSplit's game time.

Success looks like:

- Extract the zip, double-click the exe, and it works. No installer, no runtime,
  no ffmpeg, no Node, no admin rights.
- Loads are excluded from LiveSplit's game time within about one frame of the
  loading screen appearing/disappearing.
- If LiveSplit is not running, the app waits quietly and reconnects.
- Typical usage costs well under 1% CPU and does not affect game performance.

## 2. Scope

### In scope

- Find the D2R window on Windows.
- Capture the window and detect loading screens.
- Send `pausegametime` / `unpausegametime` to LiveSplit.
- Connect over named pipe (default), TCP, or WebSocket.
- Tray icon, native settings dialog, one global hotkey, log file.

### Out of scope (this branch)

- No overlay, no web server, no OBS browser source.
- No RTA/IGT display, no split list, no PB/delta UI.
- No control of LiveSplit beyond pausing/unpausing game time. The app does not
  start, split, reset, or pause the timer.
- No character-data sync, no diablo.run upload.
- No Linux/macOS support here. The existing TypeScript client remains the Linux
  app on other branches.

## 3. Product behavior

| Event | Action |
| --- | --- |
| Loading screen detected | send `pausegametime` |
| Gameplay detected after loading | send `unpausegametime` |
| D2R window lost / minimized / capture stalled (after capture had started) | send `pausegametime` |
| D2R window returns, not loading | send `unpausegametime` |
| LiveSplit (re)connects | re-send current desired state once |
| LiveSplit timer becomes `Running` while loading | re-send `pausegametime` |
| Load removal disabled, or app exits | send `unpausegametime` |

Commands are only sent when the desired state changes. A low-rate
`getcurrenttimerphase` poll (about 1 Hz) exists only for status text, resync
after a LiveSplit restart, and to avoid leaving game time paused across a
timer reset. If the connection is down, state changes are remembered and the
current state is sent on reconnect.

Manual pause/resume in LiveSplit is never overridden: `unpausegametime` does not
resume a paused timer.

## 4. Architecture

Single process, four responsibilities:

```
main thread (Win32 message loop)
  tray icon, menu, hotkeys, settings dialog, single-instance mutex
        │
        │ shared state (Mutex<AppState>)
        │
capture thread                         LiveSplit thread
  window polling                         owns transport connection
  WGC frame pool                         reconnect loop
  GPU downscale + readback               receives desired state changes
  detector                               sends pause/unpause, polls phase
        └──────── desired state change ─────────────┘
```

- The capture thread does not block on the network. It updates the desired
  paused/playing state and notifies the LiveSplit thread.
- The LiveSplit thread owns all socket/pipe I/O and never blocks the UI thread.
- The UI thread never touches capture or transport internals, only the shared
  snapshot for the tray tooltip and settings.

## 5. Capture (Windows)

### Window discovery

- `EnumWindows` + `GetWindowThreadProcessId` + `QueryFullProcessImageNameW` to
  match `D2R.exe`.
- Fallback: exact window title `Diablo II: Resurrected`.
- Reject invisible windows, minimized windows, and windows smaller than
  160x160.
- If several match, pick the largest.
- Poll every 500 ms while searching/lost; every 2 s while capturing (to notice
  size/mode changes cheaply).

### Capture pipeline

- `Windows.Graphics.Capture` (WGC) on the D2R `HWND`, using
  `CreateForWindow` via `IGraphicsCaptureItemInterop`.
- Free-threaded frame pool with 2 buffers, BGRA8.
- Cursor capture off; capture border off on Windows 11
  (`IsBorderRequired = false`).
- `MinUpdateInterval` about 1/60 s on Windows 11 22H2+.
- On frame: get the D3D11 texture and the frame's `SystemRelativeTime`.
- GPU downscale to about 160x90 (aspect preserved, even dimensions), using a
  mip-chain or two-pass box filter. Do not use a naive 4-tap bilinear: the
  detector was tuned on `ffmpeg fast_bilinear` output, and a thin gold border
  can alias away at 4K → 160.
- Copy to a staging texture and read back about 43 KB per frame into a reused
  buffer. No per-frame allocations in steady state.
- Newest-frame-wins mailbox between capture and detection. Stale frames are
  dropped, so latency cannot grow if detection or the GPU lags.
- Detector runs inline on the capture thread (target < 0.3 ms/frame).

### Timing

- Use the frame's presentation timestamp (QPC-based `SystemRelativeTime`) for
  state transitions, not the time the frame was processed.
- Load duration is unaffected by symmetric capture latency: start and end are
  both delayed by roughly one frame, so the difference is accurate.
- Expected absolute latency: about one frame (~16 ms at 60 fps) plus a few ms.

### Fallbacks and failure handling

- If WGC is unavailable or repeatedly fails, fall back to DXGI Desktop
  Duplication of the monitor containing the window, cropping to the window
  rectangle.
- If captured frames are all black (exclusive fullscreen), log a clear hint to
  use Borderless/Windowed and show it in the tray tooltip.
- Recreate the capture session on window size/mode change, display change, GPU
  reset, or WGC `Closed`.
- Never leave LiveSplit paused because capture died: treat capture loss as
  "pause" only while a run is plausibly active, as defined in section 3.

## 6. Detection

Port `loading-detection.ts` to Rust with identical logic and thresholds:

- `BLACK_LEVEL = 20`, `BRIGHT_LEVEL = 40`
- gold checks: red > 20, red-blue >= 6, green-blue >= 2, red >= green * 0.75
- border coverage: each edge >= 0.6, average >= 0.8, warm border color
- background black ratio >= 0.985 outside the frame
- logo screen: non-black ratio <= 0.02, position/size/aspect ranges
- door frame aspect ~1.32–1.68, legacy ~0.85–1.15

Input is RGBA (stride 4) from the GPU readback, which matches the existing
detector's stride-aware interface. Keep the detector free of Windows/GPU
dependencies so it can be tested with plain buffers and reused on Linux later.

Validation:

- Copy only the fixture images from the existing branch (`test/*.rgb`,
  `test/*.png`); no TypeScript source is carried over.
- Port the existing test cases: door, legacy, logo, dark/gameplay,
  screen-space variants, windowed titlebar, gold-border rejection, black
  background rejection, logo-shape rejection.
- Add a `--save-frames <dir>` debug flag that dumps raw 160x90 frames, so real
  Windows captures can become new fixtures.
- Optional `confirmFrames` (default 1): require N consecutive frames before a
  transition, taking the timestamp of the first of those frames so precision is
  preserved. Only enable if real D2R captures show flicker.

## 7. LiveSplit integration

### Transports

LiveSplit (1.8+) provides:

- Named pipe `\\.\pipe\LiveSplit`, **always open while LiveSplit runs**. Default.
- TCP server on port 16834, started manually or via LiveSplit's Startup Behavior.
- WebSocket server on `ws://<host>:16834/livesplit`, also manual or Startup
  Behavior. Cannot run at the same time as the TCP server.

Framing:

- Pipe and TCP: commands end with `\n`; one response per line.
- WebSocket: one command per message, no trailing newline; one response message
  per response.

Settings: `transport` = `pipe` | `tcp` | `websocket`; `host`, `port` (16834),
`websocketPath` (`/livesplit`), `reconnectMs` (2000).

### Protocol

Commands used:

- `pausegametime` (no response)
- `unpausegametime` (no response)
- `getcurrenttimerphase` → `NotRunning | Running | Paused | Ended`
- `ping` → `pong` (connection test)
- `getservertype` (diagnostics, optional)

Notes from LiveSplit's command server:

- Responses can be `-` for null. Times use .NET `c` format.
- `unpausegametime` also clears `alwayspausegametime`; do not use
  `alwayspausegametime`.
- `pausegametime` sets `IsGameTimePaused = true`; game time only advances while a
  run is running and game time is initialized (D2R splits use game time).

### Reconnect and resync

- Reconnect with 2 s backoff; reset backoff on success.
- On connect, send the current desired state and query the timer phase.
- On transport change from settings: tear down, connect with new settings,
  re-send state.
- Keep the last desired state in memory only; nothing is persisted about timers.

### Known accuracy limitation

LiveSplit can only pause after we have seen the first loading frame, so each
load adds about one frame of load time to game time (roughly 10–20 ms at 60 fps,
depending on where the transition falls relative to presented frames). This is
better than the old Electron client's 50 ms polling loop.

Optional `precisionCorrection` (default **off**):

- After sending `pausegametime`, read `getcurrentgametime` and compare with the
  expected value at the frame timestamp.
- If the overshoot is small and consistent, correct once with
  `setgametime <corrected>`.
- Enable only after validating against recorded video, because it mutates
  LiveSplit's current game time.

## 8. Settings menu

Tray → **Settings…** opens a native Win32 modal dialog (built from an in-memory
`DLGTEMPLATE`, standard Win32 controls — no GUI framework).

```
LiveSplit
  Transport:      [ Named pipe ▾ ]   (Named pipe / TCP / WebSocket)
  Host:           [ 127.0.0.1    ]   (disabled for named pipe)
  Port:           [ 16834        ]
  WS path:        [ /livesplit   ]   (WebSocket only)
  Reconnect:      [ 2000 ms      ]
  [ Test connection ]  Connected via named pipe / Failed: ...

Detection
  [x] Enable load removal
  [x] Pause game time when D2R capture is lost
  Capture FPS:    [ 60 ]

Hotkey
  Toggle enable:  [ Ctrl+Alt+L ]

[ Ready ]                 [ Save ]   [ Cancel ]
```

- Validation: port 1–65535; host required for TCP/WS; WS path must start with
  `/`; FPS 1–240; hotkey must be a valid combination or empty.
- **Save applies live**: reconnect the transport and apply detection settings
  without restarting, then write `config.json`.
- **Test connection** uses the dialog's current values (not yet saved), with a
  ~1.5 s timeout, and reports the transport that answered `ping`.
- Advanced/internal tuning stays in `config.json` to keep the dialog small.
- Tray menu also offers **Open config folder** and **Open log**.

## 9. Tray, hotkeys, lifecycle

- Tray icon with tooltip, e.g. `D2R: playing | LiveSplit: connected (pipe)`.
- Menu: Enable/disable load removal, Settings…, Reconnect LiveSplit, Open log,
  Open config folder, Autostart (opt-in), Quit.
- Handle Explorer restart (`TaskbarCreated` message) to restore the icon.
- Global hotkey to toggle enable, default `Ctrl+Alt+L`, configurable.
- Single instance via named mutex `Local\diablorun-d2r-load-remover`. A second
  launch focuses/opens the existing instance's settings or just exits.
- `SetProcessDpiAwarenessContext(PER_MONITOR_AWARE_V2)` for correct dialog/menu
  scaling.
- On exit/disable: send `unpausegametime` if connected, stop capture cleanly.

## 10. Configuration

Portable mode: if `config.json` exists next to the exe, use it. Otherwise use
`%APPDATA%\diablorun-d2r-load-remover\config.json`. Nothing is written to the
registry except an optional Autostart `Run` entry the user explicitly enables.

```json
{
  "enabled": true,
  "capture": {
    "fps": 60,
    "process": "D2R.exe",
    "windowTitle": "Diablo II: Resurrected",
    "minWidth": 160,
    "minHeight": 160,
    "pauseOnCaptureLost": true
  },
  "detection": {
    "confirmFrames": 1
  },
  "livesplit": {
    "transport": "pipe",
    "host": "127.0.0.1",
    "port": 16834,
    "websocketPath": "/livesplit",
    "reconnectMs": 2000,
    "precisionCorrection": false
  },
  "hotkeys": {
    "toggle": "Ctrl+Alt+L"
  },
  "logging": {
    "level": "info",
    "maxSizeMb": 2
  }
}
```

Config is merged with defaults, validated, and clamped the same way the current
TS client does it. Unknown keys are ignored.

## 11. Logging and diagnostics

- Log to `%APPDATA%\...\logs\client.log` (or next to the exe in portable mode),
  size-capped.
- `--console` attaches a console for development.
- `--log-level <level>` overrides config.
- `--save-frames <dir>` dumps raw detector frames (160x90 RGBA) for fixture
  creation.
- `--no-tray` runs headless (useful for testing/automation).
- Log window/detector transitions, transport connect/disconnect, commands sent,
  and capture errors. Avoid logging every frame.

## 12. Packaging and distribution

- Release profile: `opt-level = 3`, `lto = "fat"`, `codegen-units = 1`,
  `panic = "abort"`, `strip = true`, static CRT
  (`-C target-feature=+crt-static`) so no VC++ redistributable is needed.
- Target: one `d2r-load-remover.exe`, expected about 2–3 MB.
- Embedded icon and manifest (asInvoker, per-monitor DPI aware, Windows 10/11).
- Distribution: zip containing the exe and a short README. No installer.
- GitHub Actions builds and attaches the zip on tagged releases.
- Optional later: Authenticode signing to reduce SmartScreen warnings.

## 13. Repository layout

```
Cargo.toml
Cargo.lock
build.rs                 (icon/manifest embedding if needed)
assets/icon.ico
src/
  main.rs                CLI, single instance, message loop
  app.rs                 wiring, shared state, lifecycle
  config.rs              load/merge/validate/save
  detector.rs            ported loading detector + tests
  capture/
    mod.rs               CaptureSource trait, state machine
    win32/
      window.rs          window discovery/tracking
      wgc.rs             WGC session and frame pool
      dxgi.rs            DXGI duplication fallback
      scale.rs           D3D11 downscale/readback
  livesplit/
    mod.rs               reconnecting client, desired-state logic
    protocol.rs          command formatting, parsing, timer phase
  transport/
    pipe.rs
    tcp.rs
    ws.rs
  tray.rs
  settings_dialog.rs
  hotkeys.rs
  logging.rs
test/                    copied fixtures only
PLAN.md
README.md
.github/workflows/release.yml
```

Branch is an orphan (`windows-rust`), so history starts clean and contains no
TypeScript/Node code from the existing client.

## 14. Dependencies

- `windows` (Win32, WGC/WinRT, D3D11, DXGI, shell, dialogs, process info).
- `serde`, `serde_json`.
- `tungstenite` for WebSocket only (no TLS; local `ws://`).
- A minimal logging crate or a small internal logger.
- Dev: `png` for decoding the PNG fixtures (the `.rgb` fixtures need no crate).

No async runtime is required: one thread per concern plus Win32 messages keeps
the binary small and the behavior easy to reason about.

## 15. Performance targets

- Idle (D2R not running): < 1% of one core, < 20 MB working set.
- Capturing D2R at 4K/60: about 1–2% of a modern CPU, 20–30 MB working set.
- Detector: < 0.3 ms per frame.
- Transition latency: about one frame plus a few ms.
- Startup to tray: under 150 ms.
- Binary: 2–3 MB.

## 16. Testing

### Automated

- Detector fixture tests (all cases listed in section 6).
- Config load/merge/clamp/save tests, including portable mode.
- Protocol tests: command framing for pipe/TCP (`\n`) and WS (no newline),
  response parsing, `-` handling, timer phase parsing.
- State machine tests:
  - loading start/end only sends on change;
  - reconnect re-sends state;
  - timer Running while loading re-sends pause;
  - disable/quit sends unpause;
  - capture loss behavior.
- Mock server for each transport (in-process TCP listener, WS server, named
  pipe server) to validate the client end to end.

### Manual

- Run against real D2R in Windowed, Borderless, and Fullscreen modes.
- Alt-tab, minimize, close/reopen the game mid-run.
- Restart LiveSplit while loading; verify resync and that game time is not left
  paused.
- Switch transport in settings while a loading screen is showing.
- Multiple monitors, mixed DPI, resolution change, HDR on/off.
- `--save-frames` captures of each loading variant for fixtures.

## 17. Phases and acceptance criteria

1. **Core without capture**
   - Crate builds; config, logging, tray shell, single instance.
   - Detector ported; all fixture tests green.
   - LiveSplit client + state machine with mock tests for pipe/TCP/WS.
   - Done when: a replay of saved frames drives a mock LiveSplit correctly.
2. **Real capture**
   - Window discovery, WGC session, GPU downscale, readback, detector wiring.
   - DXGI fallback, black-frame detection, session restart handling.
   - Done when: real D2R loading pauses/unpauses a live LiveSplit reliably.
3. **Settings and polish**
   - Native settings dialog with live apply and test connection.
   - Hotkey, tray states, log rotation, autostart toggle.
   - Done when: a fresh machine can configure and use it without editing JSON.
4. **Packaging**
   - Static CRT, icon/manifest, release workflow, README.
   - Done when: zip on a clean Windows VM runs without installing anything.
5. **Tuning**
   - Collect Windows fixtures; validate thresholds and downscale quality.
   - Decide on `precisionCorrection` with recorded-video measurements.

## 18. Risks / open questions

- **WGC and exclusive fullscreen:** WGC can return black frames for some
  fullscreen games. Mitigation: DXGI fallback plus a visible hint. D2R's
  recommended mode is Borderless, which is well supported.
- **Downscale differences:** GPU filtering may produce slightly different pixels
  than the original `ffmpeg fast_bilinear` fixtures. Mitigation: mip/box
  downscale, then validate and tune thresholds against real Windows captures.
- **Load screen fade-in:** "first detectable loading frame" may be later than the
  true load start, which systematically counts some load time. This is inherent
  to screen-based removal and not fixed by `precisionCorrection`.
- **`precisionCorrection`:** mutates LiveSplit's current game time; default off
  until measured.
- **WebSocket framing:** must not append newlines; protocol tests cover this.
- **Pipe availability:** assumed always present while LiveSplit runs; if not,
  the app reports a clear error and offers TCP/WS.
- **SmartScreen/AV:** unsigned capture + socket app may warn. Optional signing.
- **Multi-GPU laptops:** WGC handles cross-GPU capture; DXGI fallback may
  require the same adapter. Prefer WGC.
- **Naming/branding:** working name `d2r-load-remover`; final product name and
  icon still to be decided.

## 19. References

- LiveSplit server docs (transports, commands, framing):
  https://github.com/LiveSplit/LiveSplit/blob/master/README.md
- LiveSplit command server implementation:
  `src/LiveSplit.Core/Server/CommandServer.cs`
- Existing detector and fixtures: this repository's `linux-client` branch,
  `src/loading-detection.ts` and `test/`.
- Windows Graphics Capture, DXGI Desktop Duplication, D3D11.
