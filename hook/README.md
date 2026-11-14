# Explorer hook

The DLL, C++ source, CMake file, and resource file are from [YASB v2.0.8](https://github.com/amnweb/yasb/tree/v2.0.8/src/core/widgets/services/systray/hook), commit `538959f4723d6f34f6194f01ebde89b06492ef4b`. See [LICENSE](LICENSE) for the original notices.

The Rust host uses the upstream DLL as shipped. The hook runs on Explorer's tray thread and sends notification-area icon updates and their pixels to the host over a named pipe. The host verifies that the connected pipe client is the process that owns Explorer's tray window. The hook's watchdog removes it when the host exits.

To rebuild the DLL from source, open a Visual Studio Developer PowerShell with CMake and the C++ desktop workload installed, then run these commands from the repository root.

```powershell
cmake -S hook -B hook/build -A x64
cmake --build hook/build --config Release
```
