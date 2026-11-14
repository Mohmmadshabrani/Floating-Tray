# Floating Tray

A small, always-on-top notification-area dock for Windows 11 x64, written in Rust with Win32. Keep app tray icons such as Slack in view while the Windows taskbar is set to auto-hide.

## Run

1. Extract the Windows x64 ZIP and keep `floating-tray.exe` and `YASBTrayHook.dll` in the same folder.
2. Exit YASB if it is running. Both apps use the same Explorer tray hook.
3. Open `floating-tray.exe`. The dock starts halfway down the right edge of the primary display.

| Action | Control |
| --- | --- |
| Show the available tray icons | Click **+** |
| Pin or unpin an icon | **Alt+click** the icon |
| Reopen the icon picker after pinning | Click either blank end of the dock |
| Move the dock | Right-click **+** or a blank end and select an edge |
| Interact with an app icon | Click or right-click the icon |
| Refresh icons or quit | Right-click **+** or a blank end |

Left and right edges use a vertical stack. Top and bottom edges use a horizontal row. The dock stays clear of the reserved taskbar area. Pins and edge placement are stored in `%LOCALAPPDATA%\FloatingTray\settings.txt`.

## Build

With Rust stable and the Visual Studio C++ build tools installed on Windows x64, run the following in PowerShell from the repository root.

```powershell
cargo test --locked
cargo build --release --locked
Copy-Item hook\YASBTrayHook.dll target\release\
.\target\release\floating-tray.exe
```

GitHub Actions runs tests on Windows and Linux and builds the Windows x64 ZIP. Push a tag such as `v0.1.0` to attach the ZIP to a GitHub release.

## What it displays

Floating Tray mirrors live **Windows notification-area icons**. An unread badge appears when an app publishes it as part of its tray icon. Windows toast history and taskbar-button overlays are separate surfaces. Apps must register a tray icon and some may need to be reopened after the dock starts.

The Rust executable uses Win32 and runs without Python, Qt, YASB, or a daemon. It ships with the [native Explorer hook from YASB](hook/README.md). The hook is loaded into `explorer.exe` to receive icon changes. Current builds target Windows 11 x64 and use the primary display.

## Windows smoke test

Start the dock with YASB closed. Pin Slack, receive a message, and confirm the icon changes. Move the dock through all four edges, restart to confirm the saved layout, and try **Exit** from a blank end. Compare Task Manager's idle memory for `floating-tray.exe` against the prior dock on the same machine.

## License

Floating Tray is [MIT licensed](LICENSE). The bundled Explorer hook retains [its upstream notices](hook/LICENSE).
