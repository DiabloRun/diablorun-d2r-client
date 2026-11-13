# D2R Load Remover

A small, portable Windows app that watches **Diablo II: Resurrected** and pauses
LiveSplit's game time while the game is loading. LiveSplit remains the timer;
this app only drives its game-time pause state.

- No installer, runtime, ffmpeg, Node, or admin rights.
- Named-pipe transport by default, with TCP and WebSocket options.
- Windowed and Fullscreen capture through Windows Graphics Capture, with a DXGI
  Desktop Duplication fallback.
- Tray menu, native settings dialog, global enable/disable hotkey, log file.
- Detector is ported from the Linux client and covered by fixtures at 16:9,
  16:10, and 4:3.

## Quick start

1. Start LiveSplit and load a D2R split layout that uses game time.
2. Run `d2r-load-remover.exe`.
3. Start D2R. The tray tooltip shows whether the game window is being captured
   and whether LiveSplit is connected.
4. Loads are paused automatically. Use the tray menu or `Ctrl+Alt+L` to toggle
   load removal.

If LiveSplit is not running yet, the app waits and reconnects every two seconds.
Commands are only sent when the desired state changes; on reconnect the current
state is re-sent.

## Transports

| Transport | Default endpoint | Notes |
| --- | --- | --- |
| Named pipe | `\\.\pipe\LiveSplit` | Always available while LiveSplit runs. Default. |
| TCP | `127.0.0.1:16834` | Enable LiveSplit's TCP server. |
| WebSocket | `ws://127.0.0.1:16834/livesplit` | Enable LiveSplit's WebSocket server. Cannot run together with TCP. |

Use **Settings → Test connection** to verify the currently entered values.

If the named pipe reports **Access is denied**, LiveSplit is probably running
elevated while the load remover is not. Run both at the same elevation, or
restart LiveSplit without elevation. TCP/WebSocket are not affected by this
named-pipe ACL difference.

## Detection and load timing

The detector is the same heuristic as the existing Linux client:

- centered gold loading frame with a black background (door and legacy frames),
- almost-black screen with the warm D2R logo in the bottom-right corner,
- frame geometry and border coverage are relative to the captured window, so
  window size and aspect ratio do not need to be whitelisted,
- titlebar rows are excluded in Windowed mode.

The door loading screen has an animated doorway, and the logo screen has a fire
animation. Detection does **not** wait for those animations to finish. The first
frame with the complete gold frame and the `LOADING` text is accepted even while
the doorway is still dark (see `test/3840x2160-door-closed.png`). The first
detectable frame is what pauses game time.

LiveSplit can only be paused after the first loading frame has been presented,
so each load still adds roughly one frame of load time (about 10–20 ms at
60 fps). This is inherent to screen-based removal.

`precisionCorrection` remains reserved and default-off in this build; the app
does not mutate LiveSplit's current game time. It should only be enabled after
the overshoot has been measured on recorded video.

### Capturing an animation sequence for tuning

Manual screenshots cannot reliably catch every animation step. Use the built-in
frame dump instead:

```powershell
# Start a load-removal run and trigger several loads while this is running.
.\d2r-load-remover.exe --save-frames C:\temp\d2r-frames
```

Every captured frame is written as a tight RGBA buffer at the detector's 160-px
width (`160x90`, `160x100`, `160x120`, or the window's actual aspect), together
with `frames.csv` containing frame number, timestamp, and detector result.
Stop the app after the load sequence.

Then replay the dump through the same detector without LiveSplit or D2R:

```powershell
.\d2r-load-remover.exe --replay-frames C:\temp\d2r-frames
```

The replay prints every state transition and the first frame classified as
loading. Open the frames around that transition to see whether the detector
waited too long (for example, a fade-in or an animation-only cue), then add the
representative frame as a fixture. The first loading frame, a mid-animation
frame, and the last loading frame are the most useful fixtures; a full sequence
does not need to be committed.

`--save-frames` records every captured frame, so keep the run short and delete
old dumps when finished. The detector runs at the configured capture FPS.

## Settings

The tray menu opens a native Win32 dialog:

- transport, host, port, WebSocket path, reconnect delay,
- enable/disable load removal,
- pause game time when D2R capture is lost,
- capture FPS,
- toggle hotkey.

Save applies immediately: the LiveSplit connection is re-established and
detection settings are applied without restarting the app. Test connection uses
the values currently in the dialog and a 1.5-second timeout.

Configuration is portable: if `config.json` exists next to the executable it is
used, otherwise `%APPDATA%\diablorun-d2r-load-remover\config.json` is used.
Autostart is opt-in and writes only an `HKCU\...\Run` value.

## Command line

```text
--console              Attach a console for development output
--log-level <level>    error, warn, info, debug, or trace
--save-frames <dir>    Dump raw detector frames for fixture creation
--replay-frames <dir>  Replay a saved frame dump through the detector
--no-tray              Run headless without a tray icon
--help                 Show help
--version              Show the version
```

Logs are written to `logs\client.log` next to the config file and are size
capped. `--no-tray` is useful for automation; stop it with Ctrl+C or terminate
the process. A second launch opens the running instance's settings dialog.

## Supported display modes

Validated resolutions are 1920x1080 and above:

- 16:9: 1920x1080, 2560x1440, 3072x1728, 3200x1800, 3840x2160
- 16:10: 1920x1200
- 4:3: 2048x1536

Below 1920x1080 is best effort only. Window size is never checked against a
resolution whitelist: larger and unusual Windowed sizes are captured normally.
Capture is in physical pixels, so Windows DPI scaling (100–200%) does not change
the detector input. SDR, scRGB (`R16G16B16A16_FLOAT`), and 10-bit surfaces are
converted in the readback path; if a particular HDR/fullscreen configuration
still returns unusable frames, switch to Windowed mode or disable HDR.

## Building

Requires Rust stable and the MSVC C++ build tools (Visual Studio Build Tools with
the Desktop development with C++ workload). The release profile uses a static
CRT and LTO.

```powershell
cargo build --release
cargo test
```

The resulting `target\release\d2r-load-remover.exe` is self-contained. Resource
embedding (icon and manifest) uses `winresource`; if the resource compiler is not
available, the code still builds and the app falls back to the stock tray icon.

## Automated tests

- All detector fixtures, including the early closed-door frame and 16:9, 16:10,
  and 4:3 aspect groups.
- Config merge, clamp, validation, and save/load round-trip.
- Protocol framing (`\n` for pipe/TCP, no newline for WebSocket).
- State machine: send only on change, reconnect resync, reset/Running handling,
  disable, shutdown.
- In-process named-pipe, TCP, and WebSocket mock servers.

```powershell
cargo test
cargo test debug_metrics -- --ignored --nocapture   # detector score breakdown
cargo test settings_dialog -- --ignored --test-threads=1  # native dialog smoke tests
```

## Manual validation checklist

Automated fixtures cannot cover every GPU/display-mode combination. Before a
release, check at least:

- 1920x1080, 1920x1200, 2048x1536, 2560x1440, and 3840x2160 in both Windowed
  and Fullscreen.
- Door, closed/early door, legacy, logo, and gameplay on each aspect group.
- A 60-second gameplay sample with no false pause.
- Arbitrary large Windowed sizes, including very wide and very tall windows.
- Resolution/display-mode change while running; capture should restart within
  about a second and LiveSplit state should stay correct.
- Alt-tab, minimize, and close/reopen D2R mid-run.
- Restart LiveSplit while loading; game time must not remain paused.
- Switch transport from Settings while a load is showing.
- Mixed-DPI multi-monitor setups and HDR on/off.
- `--save-frames` + `--replay-frames` on a real load to confirm the first
  detected frame matches the visual start of the loading screen.

## Repository layout

```text
src/
  main.rs                CLI, single instance, message loop
  app.rs                 wiring, tray, lifecycle
  config.rs              load/merge/validate/save
  detector.rs            ported loading detector + fixture tests
  replay.rs              --replay-frames developer tool
  capture/               WGC, DXGI fallback, downscale, window tracking
  livesplit/             reconnecting client and state machine
  transport/             named pipe, TCP, WebSocket
  settings_dialog.rs     native Win32 dialog
  tray.rs, hotkeys.rs, logging.rs, state.rs
test/                    detector fixtures
assets/                  icon and manifest
```

## License

MIT. LiveSplit is a separate project; this app only uses its command server.
