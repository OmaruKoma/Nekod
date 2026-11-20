# Nekod Standalone Unity Bridge (`NekodUnityBridge`)

A lightweight, **zero-UnityExplorer** BepInEx companion plugin for **Nekod** (`unity_bridge_v1` / `nekod_json_line_v1`).

- **Zero dependency** on `UnityExplorer.dll`, `UniverseLib`, `mcs.dll`, or `Newtonsoft.Json`.
- Supports **BepInEx 6 IL2CPP (`net6.0`)** and **BepInEx 5 Mono (`netstandard2.0`)** from one compact source tree.
- Binds strictly to `127.0.0.1`, generates a fresh 32-char per-launch `auth_token` automatically, and publishes `%LOCALAPPDATA%\Nekod\bridges\unity-<pid>.json` for zero-config PID auto-discovery by Nekod.

## Build

```powershell
# BepInEx 6 IL2CPP game:
.\build.ps1 -Target IL2CPP -UnityGameDir "D:\SteamLibrary\steamapps\common\YourIl2CppGame"

# BepInEx 5 Mono game:
.\build.ps1 -Target Mono -UnityGameDir "D:\SteamLibrary\steamapps\common\YourMonoGame"
```

Drop the single compiled DLL (`NekodUnityBridge.IL2CPP.dll` or `NekodUnityBridge.Mono.dll`) into `<Game>\BepInEx\plugins\`. No other files required!
