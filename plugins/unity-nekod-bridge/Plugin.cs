using System;
using BepInEx;
using BepInEx.Configuration;
using UnityEngine;
#if IL2CPP
using BepInEx.Unity.IL2CPP;
using Il2CppInterop.Runtime.Injection;
#endif

namespace Nekod.UnityBridge
{
    public sealed class NekodBridgeBehaviour : MonoBehaviour
    {
        internal static BridgeServer Server;
        internal static bool KeepRunningInBackground = true;
        private static int _processExitHooked;

#if IL2CPP
        public NekodBridgeBehaviour(IntPtr ptr) : base(ptr) { }
#endif

        internal static void StartBridge(ConfigFile config, Action<string> log)
        {
            ConfigEntry<int> port = config.Bind("Bridge", "Port", 19540, "Loopback TCP port for Nekod unity_bridge_v1.");
            ConfigEntry<string> token = config.Bind("Bridge", "AuthToken", "", "Optional 16+ char auth token (random per launch when empty).");
            ConfigEntry<bool> keepRunning = config.Bind(
                "Bridge",
                "KeepRunningInBackground",
                true,
                "Keep Unity running while unfocused so Nekod desktop toggles respond immediately."
            );

            KeepRunningInBackground = keepRunning.Value;
            EnsureBackgroundRun();

            Server = new BridgeServer(port.Value, token.Value, log);
            Server.Start();

            if (System.Threading.Interlocked.Exchange(ref _processExitHooked, 1) == 0)
            {
                AppDomain.CurrentDomain.ProcessExit += (_, _) => Server?.Stop();
            }
        }

        internal static void EnsureBackgroundRun()
        {
            if (KeepRunningInBackground && !Application.runInBackground)
            {
                Application.runInBackground = true;
            }
        }

        private void Update()
        {
            EnsureBackgroundRun();
            Server?.PumpMainThread();
        }

        private void LateUpdate()
        {
            Server?.PumpLateUpdate();
        }

        private void OnApplicationFocus(bool hasFocus)
        {
            EnsureBackgroundRun();
        }

        private void OnApplicationQuit()
        {
            Server?.Stop();
        }
    }

#if IL2CPP
    [BepInPlugin("org.nekod.unity.bridge", "Nekod Standalone Unity Bridge", BridgeServer.BridgeVersion)]
    public sealed class NekodBridgePlugin : BasePlugin
    {
        public override void Load()
        {
            NekodBridgeBehaviour.StartBridge(Config, msg => Log.LogInfo(msg));

            ClassInjector.RegisterTypeInIl2Cpp<NekodBridgeBehaviour>();
            var host = new GameObject("NekodUnityBridgeHost");
            UnityEngine.Object.DontDestroyOnLoad(host);
            host.hideFlags = HideFlags.HideAndDontSave;
            host.AddComponent<NekodBridgeBehaviour>();
        }
    }
#else
    [BepInPlugin("org.nekod.unity.bridge", "Nekod Standalone Unity Bridge", BridgeServer.BridgeVersion)]
    public sealed class NekodBridgePlugin : BaseUnityPlugin
    {
        private void Awake()
        {
            NekodBridgeBehaviour.StartBridge(Config, msg => Logger.LogInfo(msg));

            var host = new GameObject("NekodUnityBridgeHost");
            DontDestroyOnLoad(host);
            host.hideFlags = HideFlags.HideAndDontSave;
            host.AddComponent<NekodBridgeBehaviour>();
        }
    }
#endif
}
