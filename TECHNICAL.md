# Technical notes

Developer notes for D2R LRT Tracker. For setup and usage, see
[README.md](README.md).

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
60 fps). That's unavoidable when detecting loads from the screen.

Game time is only paused while a loading screen is visible. If D2R is closed,
minimized, or capture is lost, game time keeps running. The app never changes
LiveSplit's current game time. Capture and detection always run at 60 FPS.

### Capturing an animation sequence for tuning

Screenshots rarely catch every animation step, so use the built-in frame dump:

```powershell
# Start a load-removal run and trigger several loads while this is running.
.\d2r-lrt-tracker.exe --save-frames C:\temp\d2r-frames
```

Every captured frame is written as a tight RGBA buffer at the detector's 160-px
width (`160x90`, `160x100`, `160x120`, or the window's actual aspect), together
with `frames.csv` containing frame number, timestamp, and detector result.
Stop the app after the load sequence.

Then replay the dump through the same detector without LiveSplit or D2R:

```powershell
.\d2r-lrt-tracker.exe --replay-frames C:\temp\d2r-frames
```

The replay prints every state transition and the first frame classified as
loading. Open the frames around that transition to see whether the detector
waited too long (for example, a fade-in or an animation-only cue), then add the
representative frame as a fixture. The first loading frame, a mid-animation
frame, and the last loading frame are the most useful fixtures. A full sequence
does not need to be committed.

`--save-frames` records every captured frame, so keep the run short and delete
old dumps when finished.

## LiveSplit client

Commands are only sent when the desired state changes. On reconnect the current
state is re-sent. If LiveSplit is not running yet, the app waits and reconnects
every two seconds.

## Capture and frame handling

- Windows Graphics Capture is the primary path, with DXGI Desktop Duplication as
  the fallback.
- Capture is in physical pixels, so Windows DPI scaling (100–200%) does not
  change the detector input.
- SDR, scRGB (`R16G16B16A16_FLOAT`), and 10-bit surfaces are converted in the
  readback path.
- Frames are downscaled to 160 px width before detection.
- Windowed and Fullscreen are both supported. Window size and aspect ratio are
  never checked against a resolution whitelist. Larger and unusual Windowed
  sizes are captured normally.
- Validated resolutions are 1920x1080 and above:
  - 16:9: 1920x1080, 2560x1440, 3072x1728, 3200x1800, 3840x2160
  - 16:10: 1920x1200
  - 4:3: 2048x1536
- Below 1920x1080 is best effort only.
- If a particular HDR/fullscreen configuration still returns unusable frames,
  switch to Windowed mode or disable HDR.

## LiveSplit connections

| Transport | Default endpoint | Notes |
| --- | --- | --- |
| Named pipe | `\\.\pipe\LiveSplit` | Default. Always available while LiveSplit runs. |
| TCP | `127.0.0.1:16834` | Enable LiveSplit's TCP server. |
| WebSocket | `ws://127.0.0.1:16834/livesplit` | Enable LiveSplit's WebSocket server. Cannot run together with TCP. |

Use **Settings → Test connection** to verify the entered values. A failed pipe
connection reports **Connection failed: is LiveSplit opened?**; TCP and
WebSocket failures report **Connection failed: start LiveSplit's TCP server.**
and **Connection failed: start LiveSplit's WebSocket server.** Start the
corresponding server in LiveSplit's settings. If LiveSplit is running but the
named pipe still fails, it is probably running elevated while the tracker is not.
Run both at the same elevation, or restart LiveSplit without elevation. TCP and
WebSocket are not affected by this named-pipe ACL difference.

A native settings dialog for the connection (transport, host, port, and
WebSocket path) opens on startup and from the tray menu. It follows the Windows
light/dark app theme, has its own taskbar entry, and is brought to the
foreground when opened.
Connection changes are saved automatically: the LiveSplit connection is
re-established without restarting the app. **Close to tray** dismisses the
settings window. Test connection uses the values currently in the dialog and a
1.5-second timeout, and shows a small green check or red cross next to the
result. The same dialog shows live status rows for the LiveSplit connection and
D2R capture, including the captured resolution. The tray icon tooltip is just
the app title, **D2R LRT Tracker**.

## Configuration

Configuration is portable: if `config.json` exists next to the executable it is
used, otherwise `%APPDATA%\d2r-lrt-tracker\config.json` is used. During normal
operation, `config.json` is the only file the app writes.

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

`--no-tray` is useful for automation. Stop it with Ctrl+C or terminate the
process. A second launch opens the running instance's settings dialog.

## Building

Requires Rust stable and a Windows linker toolchain. This project is released
with the GNU target and the WinLibs MinGW-w64 toolchain installed through
WinGet. The MSVC target with Visual Studio Build Tools also works. The release
profile uses a static CRT and LTO.

```powershell
cargo +stable-x86_64-pc-windows-gnu build --release
cargo +stable-x86_64-pc-windows-gnu test
```

The resulting `target\release\d2r-lrt-tracker.exe` is self-contained. Resource
embedding (icon and version info) uses `winresource` and requires `windres` from
WinLibs on `PATH`. Without it the app still builds, but it falls back to the
stock tray icon.

## Releasing

Releases are built and packaged locally into `target\`. No GitHub Actions or
remote publishing is used.

The release build uses the GNU target with the WinLibs MinGW-w64 toolchain. If
it is not installed:

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

   $zip = "target\d2r-lrt-tracker-v$version-windows-x64.zip"
   Remove-Item $zip -ErrorAction SilentlyContinue
   Compress-Archive -Path target\release\d2r-lrt-tracker.exe, README.md, LICENSE `
     -DestinationPath $zip

   (Get-Item target\release\d2r-lrt-tracker.exe).VersionInfo.FileVersion
   Get-FileHash $zip -Algorithm SHA256
   ```

   `FileVersion` should match the version in `Cargo.toml`. If it is empty, the
   icon/version resources were not embedded. Check that WinLibs is on `PATH`.

4. Commit the version bump and tag the release locally:

   ```powershell
   git add Cargo.toml Cargo.lock
   git commit -m "Release v$version"
   git tag "v$version"
   ```

The local release artifacts are:

- `target\release\d2r-lrt-tracker.exe`
- `target\d2r-lrt-tracker-v<version>-windows-x64.zip`

The zip contains the executable, `README.md`, and `LICENSE`. `TECHNICAL.md`
stays in the repository and isn't packaged.

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

Fixtures can't cover every GPU and display-mode combination. Before a release,
check at least:

- 1920x1080, 1920x1200, 2048x1536, 2560x1440, and 3840x2160 in both Windowed
  and Fullscreen.
- Door, closed/early door, legacy, logo, and gameplay on each aspect group.
- A 60-second gameplay sample with no false pause.
- Arbitrary large Windowed sizes, including very wide and very tall windows.
- Resolution/display-mode change while running. Capture should restart within
  about a second, and LiveSplit state should stay correct.
- Alt-tab, minimize, and close/reopen D2R mid-run.
- Restart LiveSplit while loading. Game time must not remain paused.
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
  theme.rs               Windows light/dark mode support
  tray.rs, state.rs
test/                    detector fixtures
assets/                  icon and manifest
```
