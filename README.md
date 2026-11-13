# D2R LRT Tracker

Pauses LiveSplit's game time while **Diablo II: Resurrected** is loading.
LiveSplit still handles the timing. This app just tells it when to pause.

## Quick start

1. Start LiveSplit with a D2R layout that uses game time.
2. Run `d2r-lrt-tracker.exe`. The settings window opens on launch; adjust the
   LiveSplit connection if needed, then close it to the tray.
3. Start D2R. Loading screens pause game time automatically.

The tray icon shows whether the game is being captured and whether LiveSplit is
connected. Connection settings are in the tray menu. If LiveSplit isn't running,
the app waits and reconnects by itself.

Game time keeps running if D2R closes, minimizes, or capture is lost.

See [TECHNICAL.md](TECHNICAL.md) in the repository for troubleshooting, display
modes, configuration, command-line options, and build/release notes. It isn't
included in the release zip.

## License

MIT. LiveSplit is a separate project. This app only talks to its command server.
