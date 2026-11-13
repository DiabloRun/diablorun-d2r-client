# D2R Load Remover

A small, portable Windows app that watches **Diablo II: Resurrected** and pauses
LiveSplit's game time while the game is loading. LiveSplit remains the timer;
this app only drives its game-time pause state.

- No installer, runtime, ffmpeg, Node, or admin rights.
- Named-pipe transport by default, with TCP and WebSocket options.
- Windowed and Fullscreen capture through Windows Graphics Capture, with a DXGI
  Desktop Duplication fallback.
- Tray menu and native settings dialog.
- Detector is ported from the Linux client and covered by fixtures at 16:9,
  16:10, and 4:3.

## Quick start

1. Start LiveSplit and load a D2R split layout that uses game time.
2. Run `d2r-load-remover.exe`.
3. Start D2R. The tray tooltip shows whether the game window is being captured
   and whether LiveSplit is connected.
4. Loads are paused automatically. Use the tray menu to enable or disable load
   removal.

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

Game time is only paused while a loading screen is visible. If D2R is closed,
minimized, or capture is lost, game time keeps running.

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
old dumps when finished. Capture and detection always run at 60 FPS.

## Settings

The tray menu opens a native Win32 settings dialog for the LiveSplit connection
(transport, host, port, WebSocket path, and reconnect delay). Enable/disable load
removal is available from the tray menu. The settings window has its own taskbar
entry and is brought to the foreground when opened. Capture FPS is fixed at 60.
**Save & Close** applies immediately: the LiveSplit connection is re-established
and detection settings are applied without restarting the app. Test connection
uses the values currently in the dialog and a 1.5-second timeout.

Configuration is portable: if `config.json` exists next to the executable it is
used, otherwise `%APPDATA%\diablorun-d2r-load-remover\config.json` is used.
During normal operation, `config.json` is the only file the app writes.

## Command line

The release executable is a GUI-subsystem application, so it does not open a
console window when started normally. Use `--console` to attach one when running
from a terminal.

```text
--console              Attach a console for development output
--save-frames <dir>    Dump raw detector frames for fixture creation
--replay-frames <dir>  Replay a saved frame dump through the detector
--no-tray              Run headless without a tray icon
--help                 Show help
--version              Show the version
```

`--no-tray` is useful for automation; stop it with Ctrl+C or terminate the
process. A second launch opens the running instance's settings dialog.

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

Requires Rust stable and a Windows linker toolchain. This project is released
with the GNU target and the WinLibs MinGW-w64 toolchain installed through WinGet.
The MSVC target with Visual Studio Build Tools also works. The release profile
uses a static CRT and LTO.

```powershell
cargo +stable-x86_64-pc-windows-gnu build --release
cargo +stable-x86_64-pc-windows-gnu test
```

The resulting `target\release\d2r-load-remover.exe` is self-contained. Resource
embedding (icon and version info) uses `winresource` and requires `windres` from
WinLibs on `PATH`; without it the app still builds but falls back to the stock
tray icon.

## Releasing

Releases are built and packaged locally into `target\`. No GitHub Actions or
remote publishing is used.

The release build uses the GNU target with the WinLibs MinGW-w64 toolchain. If it
is not installed:

```powershell
winget install BrechtSanders.WinLibs.POSIX.UCRT
```

Open a new shell and put WinLibs first on `PATH` before building:

```powershell
$mingw = Get-ChildItem "$env:LOCALAPPDATA\Microsoft\WinGet\Packages" -Directory `
  -Filter 'BrechtSanders.WinLibs*' |
  ForEach-Object { Join-Path $_.FullName 'mingw64\bin' } |
  Select-Object -First 1
$env:PATH = "$mingw;$env:PATH"
```

1. Run the manual validation checklist below, then update the package version in
   `Cargo.toml` (for example, `0.2.0`) and refresh `Cargo.lock`:

   ```powershell
   cargo +stable-x86_64-pc-windows-gnu check
   ```

2. Run the tests:

   ```powershell
   cargo +stable-x86_64-pc-windows-gnu test
   ```

3. Build the release and create the distributable zip:

   ```powershell
   $version = (Select-String -Path Cargo.toml -Pattern '^version = "(.+)"').Matches[0].Groups[1].Value

   cargo +stable-x86_64-pc-windows-gnu build --release

   $zip = "target\d2r-load-remover-v$version-windows-x64.zip"
   Remove-Item $zip -ErrorAction SilentlyContinue
   Compress-Archive -Path target\release\d2r-load-remover.exe, README.md, LICENSE `
     -DestinationPath $zip

   (Get-Item target\release\d2r-load-remover.exe).VersionInfo.FileVersion
   Get-FileHash $zip -Algorithm SHA256
   ```

   `FileVersion` should match the version in `Cargo.toml`. If it is empty, the
   icon/version resources were not embedded; check that WinLibs is on `PATH`.

4. Commit the version bump and tag the release locally:

   ```powershell
   git add Cargo.toml Cargo.lock
   git commit -m "Release v$version"
   git tag "v$version"
   ```

The local release artifacts are:

- `target\release\d2r-load-remover.exe`
- `target\d2r-load-remover-v<version>-windows-x64.zip`

The zip contains the executable, `README.md`, and `LICENSE`.

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
  tray.rs, state.rs
test/                    detector fixtures
assets/                  icon and manifest
```

## License

MIT. LiveSplit is a separate project; this app only uses its command server.
