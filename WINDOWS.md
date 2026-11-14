# Floating Tray for Windows 11 x64

Extract the ZIP, exit YASB if it is running, then open `floating-tray.exe`. Keep `YASBTrayHook.dll` beside it. The dock opens on the middle of the primary screen's right edge.

- Click **+** to see available tray icons and **Alt+click** an icon to pin it.
- Once pinned, click either blank end to bring the picker back.
- Right-click **+** or a blank end to change the edge, refresh icons, or exit.
- Left and right edges stack icons. Top and bottom edges place them in a row.
- If an icon is missing, try **Refresh icons** or reopen its app. Slack's badge can appear here when Slack includes it in its notification-area icon.

The dock uses the native Explorer hook from [YASB v2.0.8](https://github.com/amnweb/yasb/tree/v2.0.8/src/core/widgets/services/systray/hook) to receive live updates. The licenses for the app and the included hook are in `LICENSE` and `HOOK-LICENSE.txt`.
