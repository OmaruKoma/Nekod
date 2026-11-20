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

#if IL2CPP
        public NekodBridgeBehaviour(IntPtr ptr) : base(ptr) { }
#endif

        private void Update()
        {
            Server?.PumpMainThread();
        }

        private void OnDestroy()
        {
            Server?.Stop();
        }

        private void OnApplicationQuit()
        {
            Server?.Stop();
        }
    }

#if IL2CPP
    [BepInPlugin("org.nekod.unity.bridge", "Nekod Standalone Unity Bridge", "0.1.0")]
    public sealed class NekodBridgePlugin : BasePlugin
    {
        public override void Load()
        {
            ConfigEntry<int> port = Config.Bind("Bridge", "Port", 19540, "Loopback TCP port for Nekod unity_bridge_v1.");
            ConfigEntry<string> token = Config.Bind("Bridge", "AuthToken", "", "Optional 16+ char auth token (random per launch when empty).");

            NekodBridgeBehaviour.Server = new BridgeServer(port.Value, token.Value, msg => Log.LogInfo(msg));
            NekodBridgeBehaviour.Server.Start();

            ClassInjector.RegisterTypeInIl2Cpp<NekodBridgeBehaviour>();
            var host = new GameObject("NekodUnityBridgeHost");
            UnityEngine.Object.DontDestroyOnLoad(host);
            host.hideFlags = HideFlags.HideAndDontSave;
            host.AddComponent<NekodBridgeBehaviour>();
        }
    }
#else
    [BepInPlugin("org.nekod.unity.bridge", "Nekod Standalone Unity Bridge", "0.1.0")]
    public sealed class NekodBridgePlugin : BaseUnityPlugin
    {
        private void Awake()
        {
            ConfigEntry<int> port = Config.Bind("Bridge", "Port", 19540, "Loopback TCP port for Nekod unity_bridge_v1.");
            ConfigEntry<string> token = Config.Bind("Bridge", "AuthToken", "", "Optional 16+ char auth token (random per launch when empty).");

            NekodBridgeBehaviour.Server = new BridgeServer(port.Value, token.Value, msg => Logger.LogInfo(msg));
            NekodBridgeBehaviour.Server.Start();

            var host = new GameObject("NekodUnityBridgeHost");
            DontDestroyOnLoad(host);
            host.hideFlags = HideFlags.HideAndDontSave;
            host.AddComponent<NekodBridgeBehaviour>();
        }
    }
#endif
}
