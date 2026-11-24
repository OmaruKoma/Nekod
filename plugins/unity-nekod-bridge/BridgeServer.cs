using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Globalization;
using System.IO;
using System.Net;
using System.Net.Sockets;
using System.Reflection;
using System.Security.Cryptography;
using System.Text;
using System.Threading;
using HarmonyLib;
using UnityEngine;

namespace Nekod.UnityBridge
{
    /// <summary>
    /// Standalone authenticated loopback JSON-line server (`nekod_json_line_v1`) executing
    /// Unity reflection reads, field/property writes, and method invocations strictly on the
    /// Unity main thread without any dependency on UnityExplorer or UniverseLib.
    /// </summary>
    internal sealed class BridgeServer
    {
        public const string BridgeVersion = "0.5.5";

        private sealed class PendingWork
        {
            public const int StatePending = 0;
            public const int StateRunning = 1;
            public const int StateCancelled = 2;

            public string Op;
            public Dictionary<string, object> Params;
            public string ResponseLine;
            public int State;
            public readonly ManualResetEventSlim Done = new ManualResetEventSlim(false);
        }

        private readonly struct MemberKey : IEquatable<MemberKey>
        {
            public readonly Type Type;
            public readonly string Name;

            public MemberKey(Type type, string name)
            {
                Type = type;
                Name = name;
            }

            public bool Equals(MemberKey other)
            {
                return ReferenceEquals(Type, other.Type) &&
                    string.Equals(Name, other.Name, StringComparison.Ordinal);
            }

            public override bool Equals(object obj)
            {
                return obj is MemberKey other && Equals(other);
            }

            public override int GetHashCode()
            {
                unchecked
                {
                    int h = Type != null ? Type.GetHashCode() : 0;
                    return (h * 397) ^ (Name != null ? StringComparer.Ordinal.GetHashCode(Name) : 0);
                }
            }
        }

        private sealed class MemberBinding
        {
            public FieldInfo Field;
            public PropertyInfo Property;
            public Type MemberType;
            public bool CanRead;
            public bool CanWrite;
            public bool IsStaticRead;
            public bool IsStaticWrite;

            public object Read(object target)
            {
                return Field != null ? Field.GetValue(target) : Property.GetValue(target, null);
            }

            public void Write(object target, object value)
            {
                if (Field != null) Field.SetValue(target, value);
                else Property.SetValue(target, value, null);
            }
        }

        private sealed class InstanceCacheEntry
        {
            public readonly List<object> Instances = new List<object>();
            public long LastRefreshMs = -1;
            public bool HasEverFound;
        }

        private sealed class ActiveComponentEntry
        {
            public UnityEngine.Component Component;
            public GameObject GameObject;
            public Transform Transform;
            public Transform Root;
            public int InstanceId;
            public bool IsPlayerTag;
        }

        private sealed class ActiveComponentListCache
        {
            public readonly List<ActiveComponentEntry> Entries = new List<ActiveComponentEntry>();
            public readonly Dictionary<int, Transform> ColliderTransforms = new Dictionary<int, Transform>();
            public long LastRefreshMs = -1;
        }

        private sealed class PinnedField
        {
            public string Id;
            public Type RootType;
            public string FieldName;
            public object Value;
            public string[] Segments;
            public MemberBinding RootMember;
            public object SingleConvertedValue;
            public bool HasSingleConvertedValue;
            public Type CachedLeafType;
            public MemberBinding CachedLeafMember;
            public object CachedLeafConvertedValue;
            public bool HasCachedLeafConvertedValue;
        }

        private sealed class AliveCheckSpec
        {
            public Type ComponentType;
            public string DeadField;
            public string HealthField;
            public MemberBinding DeadMember;
            public MemberBinding HealthMember;
        }

        private sealed class TargetLockPin
        {
            public string Id;
            public Type PlayerType;
            public Type CameraType;
            public Type TargetType;
            public Type ColliderType;
            public string LockMethod;
            public string UnlockMethod;
            public MethodInfo LockMethod3Arg;
            public MethodInfo LockMethod1Arg;
            public MethodInfo UnlockMethodInfo;
            public object[] LockArgs3;
            public object[] LockArgs1;
            public List<AliveCheckSpec> AliveChecks;
            public float MaxRange;
            public float HeightOffset;
            public float LockSpeed;
            public bool RequireCollider;
            public long IntervalMs;
            public long LastTickMs;
            public long LastLockInvokeMs;
            public bool CurrentlyLocked;
            public UnityEngine.Component CachedPlayer;
            public long LastPlayerMissMs = -1;
            public UnityEngine.Component CachedCamera;
            public long LastCameraMissMs = -1;
            public UnityEngine.Component CurrentLockedEnemy;
            public Transform CurrentLockedTransform;
        }

        /// <summary>
        /// Vendor-neutral argument descriptor for a damage handler. The bridge never needs to
        /// know what a "vDamage" or "EmeraldAISystem" is; it only resolves these placeholders
        /// against the live hit context, so any game's damage API can be driven declaratively.
        /// </summary>
        private sealed class AuraArgSpec
        {
            public string Bind;   // damage | constant | attacker_transform | target_transform
                                  // | target_position | target_component | player_transform
                                  // | player_component | object
            public object ConstantValue;
            public Type ObjectType;
            public List<AuraArgSpec> CtorArgs;
            public List<KeyValuePair<string, AuraArgSpec>> ObjectFields;
        }

        private sealed class DamageHandlerSpec
        {
            public Type ComponentType;
            public string MethodName;
            public MethodInfo DamageMethod;
            public string DeadField;
            public string HealthField;
            public string DeadMethod;
            public MethodInfo DeadMethodInfo;
            public bool ZeroHealthBeforeCall;
            public MemberBinding DeadMember;
            public MemberBinding HealthMember;
            public List<AuraArgSpec> Args;
            public object[] InvokeArgs;
        }

        private sealed class DamageAuraPin
        {
            public string Id;
            public Type PlayerType;
            public Type TargetType;
            public Type RequireActiveType;
            public UnityEngine.Component CachedRequireActive;
            public long LastRequireActiveMissMs = -1;
            public string ExcludeBoolField;
            public MemberBinding ExcludeBoolMember;
            public int DamageAmount;
            public float Radius;
            public long IntervalMs;
            public long LastTickMs;
            public List<DamageHandlerSpec> Handlers;
            public UnityEngine.Component CachedPlayer;
            public long LastPlayerMissMs = -1;
        }

        private sealed class SpinBotPin
        {
            public string Id;
            public Type PlayerType;
            public Type TargetType;
            public Type RequireActiveType;
            public UnityEngine.Component CachedRequireActive;
            public long LastRequireActiveMissMs = -1;
            public List<AliveCheckSpec> AliveChecks;
            public float MaxRange;
            public float OrbitRadius;
            public float OrbitSpeedDeg;
            public float HeightOffset;
            public float RollSpeedDeg;
            public MemberBinding RollMember;
            public string SpinNodePath;
            public Transform SpinNode;
            public int SpinNodeOwnerId;
            public long SpinNodeMissMs = -1;
            public bool SpinLocal;
            public bool SpinInLateUpdate;
            public bool HasOriginalRotation;
            public Quaternion OriginalRotation;
            public bool AimAtTarget;
            public bool RequireAlive;
            public long IntervalMs;
            public long StartMs;
            public long LastTickMs;
            public long ElapsedMs;
            public float AngleDeg;
            public UnityEngine.Component CachedPlayer;
            public long LastPlayerMissMs = -1;
            public UnityEngine.Component CurrentTarget;
            public int AppliedTicks;
        }

        private sealed class MaterialBackupEntry
        {
            public UnityEngine.Component Renderer;
            public MemberBinding SharedMaterialsMember;
            public UnityEngine.Material[] OriginalMaterials;
        }

        private sealed class HookFieldWriteSpec
        {
            public Type TargetType;
            public string FieldName;
            public bool HasLiteralValue;
            public object Value;
            public bool HasRestoreValue;
            public object RestoreValue;
            public Type CopyFromType;
            public string CopyFromField;
            public Type EnsureComponentType;
            public bool InitNullLists;
            public bool OnlyIfNull;
            public bool IncludeInactive;
            public bool CloneUnderOwner;
        }

        private sealed class MethodHookPin
        {
            public string Id;
            public Type HookType;
            public MethodInfo TargetMethod;
            public string Timing;
            public List<HookFieldWriteSpec> Writes;
            public long FireCount;
        }

        private const int PortScanSpan = 20;
        private const int MaxPendingWork = 16;
        private const int MainThreadTimeoutMs = 3000;
        private const long PinIntervalMs = 500;
        private const long InstanceCacheTtlMs = 2500;
        private const long MissRetryMs = 1000;
        private const long EnemyScanIntervalMs = 500;
        private const long EnemyUrgentRefreshMs = 150;
        private const long LockReassertIntervalMs = 500;
        private const int MaxAuraHitsPerTick = 32;
        /// <summary>
        /// GameObject tags that identify the local player. Games rarely agree on this
        /// (many use no tag at all), so it stays overridable per request.
        /// </summary>
        private static string[] _playerTags = { "Player" };

        private static void ConfigurePlayerTags(Dictionary<string, object> p)
        {
            if (p == null || !p.TryGetValue("player_tags", out object raw) || raw is not List<object> list)
                return;
            var tags = new List<string>();
            foreach (object item in list)
            {
                if (item is string s && s.Length > 0) tags.Add(s);
            }
            if (tags.Count > 0) _playerTags = tags.ToArray();
        }

        private static bool IsConfiguredPlayerTag(string tag)
        {
            if (string.IsNullOrEmpty(tag)) return false;
            string[] tags = _playerTags;
            for (int i = 0; i < tags.Length; i++)
            {
                if (string.Equals(tag, tags[i], StringComparison.OrdinalIgnoreCase)) return true;
            }
            return false;
        }

        private const BindingFlags MemberFlags =
            BindingFlags.Public | BindingFlags.NonPublic | BindingFlags.Static | BindingFlags.Instance | BindingFlags.DeclaredOnly;
        private const BindingFlags InstanceMethodFlags =
            BindingFlags.Public | BindingFlags.NonPublic | BindingFlags.Instance | BindingFlags.FlattenHierarchy;

#if IL2CPP
        private static readonly MethodInfo _findObjectsIncludeInactive =
            typeof(UnityEngine.Object).GetMethod(
                "FindObjectsOfType",
                BindingFlags.Public | BindingFlags.Static,
                null,
                new[] { typeof(Il2CppSystem.Type), typeof(bool) },
                null);
#else
        private static readonly MethodInfo _findObjectsIncludeInactive =
            typeof(UnityEngine.Object).GetMethod(
                "FindObjectsOfType",
                BindingFlags.Public | BindingFlags.Static,
                null,
                new[] { typeof(Type), typeof(bool) },
                null);
#endif

        private readonly Queue<PendingWork> _queue = new Queue<PendingWork>();
        private readonly object _queueLock = new object();
        private readonly Dictionary<string, Type> _typeCache = new Dictionary<string, Type>(StringComparer.Ordinal);
        private readonly Dictionary<MemberKey, MemberBinding> _memberCache = new Dictionary<MemberKey, MemberBinding>();
        private readonly Dictionary<Type, InstanceCacheEntry> _instanceCache = new Dictionary<Type, InstanceCacheEntry>();
        private readonly Dictionary<Type, ActiveComponentListCache> _activeCompListCache = new Dictionary<Type, ActiveComponentListCache>();
        private readonly List<PinnedField> _scratchFieldPins = new List<PinnedField>();
        private readonly List<TargetLockPin> _scratchLockPins = new List<TargetLockPin>();
        private readonly List<DamageAuraPin> _scratchAuraPins = new List<DamageAuraPin>();

        private readonly List<object> _pinTargetsA = new List<object>();
        private readonly List<object> _pinTargetsB = new List<object>();
        private readonly Dictionary<string, PinnedField> _pins = new Dictionary<string, PinnedField>(StringComparer.Ordinal);
        private readonly Dictionary<string, TargetLockPin> _targetLocks = new Dictionary<string, TargetLockPin>(StringComparer.Ordinal);
        private readonly Dictionary<string, DamageAuraPin> _damageAuras = new Dictionary<string, DamageAuraPin>(StringComparer.Ordinal);
        private readonly Dictionary<string, SpinBotPin> _spinBots = new Dictionary<string, SpinBotPin>(StringComparer.Ordinal);
        private readonly List<SpinBotPin> _scratchSpinBots = new List<SpinBotPin>();
        private readonly Dictionary<string, List<MaterialBackupEntry>> _materialBackups = new Dictionary<string, List<MaterialBackupEntry>>(StringComparer.Ordinal);
        private readonly Dictionary<string, MethodHookPin> _methodHooks = new Dictionary<string, MethodHookPin>(StringComparer.Ordinal);
        private Harmony _harmony;

        private static readonly object _hookDispatchLock = new object();
        private static readonly Dictionary<MethodBase, List<MethodHookPin>> _prefixDispatch = new Dictionary<MethodBase, List<MethodHookPin>>();
        private static readonly Dictionary<MethodBase, List<MethodHookPin>> _postfixDispatch = new Dictionary<MethodBase, List<MethodHookPin>>();
        private static BridgeServer _activeHookServer;
        private static readonly MethodInfo _prefixCallbackMethod =
            typeof(BridgeServer).GetMethod(nameof(GenericHookPrefix), BindingFlags.NonPublic | BindingFlags.Static);
        private static readonly MethodInfo _postfixCallbackMethod =
            typeof(BridgeServer).GetMethod(nameof(GenericHookPostfix), BindingFlags.NonPublic | BindingFlags.Static);
        private readonly Stopwatch _loopClock = Stopwatch.StartNew();
        private readonly Action<string> _log;
        private readonly string _unityVersion;
        private readonly int _pid;

        private TcpListener _listener;
        private Thread _acceptThread;
        private volatile bool _running;
        private volatile bool _clearCachesRequested;
        private string _discoveryPath;
        private long _mainThreadTicks;
        private long _sceneScanCount;
        private int _activePinCount;
        private int _lastPinFrame = -1;
        private int _lastLateFrame = -1;
        private long _nextFieldPinMs;
        private long _nextDueTickMs;
        private bool _hasSceneCaches;

        public int Port { get; private set; }
        public string AuthToken { get; private set; }

        public static string Flavor =>
#if IL2CPP
            "il2cpp";
#else
            "mono";
#endif

        public static string LoaderKind =>
#if IL2CPP
            "bepinex_il2cpp";
#else
            "bepinex_mono";
#endif

        public BridgeServer(int preferredPort, string configuredToken, Action<string> log)
        {
            _log = log ?? (_ => { });
            _unityVersion = Application.unityVersion ?? "";
            _pid = Process.GetCurrentProcess().Id;
            Port = preferredPort > 0 ? preferredPort : 19540;
            AuthToken = !string.IsNullOrWhiteSpace(configuredToken) && configuredToken.Trim().Length >= 16
                ? configuredToken.Trim()
                : GenerateRandomToken();
        }

        public void Start()
        {
            if (_running) return;
            for (int offset = 0; offset <= PortScanSpan; offset++)
            {
                int candidate = Port + offset;
                try
                {
                    var listener = new TcpListener(IPAddress.Loopback, candidate);
                    listener.Start();
                    _listener = listener;
                    Port = candidate;
                    break;
                }
                catch (SocketException)
                {
                    if (offset == PortScanSpan) throw;
                }
            }

            _running = true;
            lock (_hookDispatchLock)
            {
                _activeHookServer = this;
            }
            PublishDiscoveryFile();

            _acceptThread = new Thread(AcceptLoop)
            {
                Name = "NekodUnityBridge.Listener",
                IsBackground = true
            };
            _acceptThread.Start();
            _log($"[NekodUnityBridge] Listening on 127.0.0.1:{Port} (flavor={Flavor}, zero-UnityExplorer)");
        }

        private void UpdateActivePinCountLocked()
        {
            Volatile.Write(ref _activePinCount, _pins.Count + _targetLocks.Count + _damageAuras.Count + _spinBots.Count);
        }

        private void ClearSceneCaches()
        {
            _instanceCache.Clear();
            _activeCompListCache.Clear();
            _hasSceneCaches = false;
        }

        public void Stop()
        {
            _running = false;
            Harmony harmonyToUnpatch;
            lock (_hookDispatchLock)
            {
                _methodHooks.Clear();
                if (ReferenceEquals(_activeHookServer, this))
                {
                    _prefixDispatch.Clear();
                    _postfixDispatch.Clear();
                    _activeHookServer = null;
                }
                harmonyToUnpatch = _harmony;
                _harmony = null;
            }
            if (harmonyToUnpatch != null)
            {
                try { harmonyToUnpatch.UnpatchSelf(); } catch { }
            }
            lock (_pins)
            {
                _pins.Clear();
                _targetLocks.Clear();
                _damageAuras.Clear();
                _spinBots.Clear();
                Volatile.Write(ref _activePinCount, 0);
            }
            _clearCachesRequested = true;
            try { _listener?.Stop(); } catch { }
            UnpublishDiscoveryFile();
        }

        /// <summary>Must be invoked from MonoBehaviour.Update() on the Unity main thread.</summary>
        public void PumpMainThread()
        {
            Interlocked.Increment(ref _mainThreadTicks);
            while (true)
            {
                PendingWork work;
                lock (_queueLock)
                {
                    if (_queue.Count == 0) break;
                    work = _queue.Dequeue();
                }

                if (Interlocked.CompareExchange(
                        ref work.State,
                        PendingWork.StateRunning,
                        PendingWork.StatePending) != PendingWork.StatePending)
                {
                    // Request already timed out on the socket thread; do not apply stale mutation later.
                    continue;
                }

                try
                {
                    work.ResponseLine = ProcessRequestOnMainThread(work.Op, work.Params);
                }
                catch (Exception ex)
                {
                    work.ResponseLine = ErrorJson("internal bridge error: " + ex.Message);
                }
                finally
                {
                    work.Done.Set();
                }
            }

            if (_clearCachesRequested)
            {
                _clearCachesRequested = false;
                ClearSceneCaches();
            }

            if (Volatile.Read(ref _activePinCount) == 0)
            {
                if (_hasSceneCaches) ClearSceneCaches();
                return;
            }

            int frame = Time.frameCount;
            if (frame == _lastPinFrame) return;
            _lastPinFrame = frame;

            long nowMs = _loopClock.ElapsedMilliseconds;

            // SpinBot must run every rendered frame, independent of the lazy pin scheduler.
            if (_spinBots.Count > 0)
            {
                _scratchSpinBots.Clear();
                foreach (SpinBotPin sb in _spinBots.Values) _scratchSpinBots.Add(sb);
                for (int i = 0; i < _scratchSpinBots.Count; i++)
                {
                    try { TickSpinBot(_scratchSpinBots[i], nowMs); }
                    catch { _scratchSpinBots[i].LastTickMs = nowMs; }
                }
                _scratchSpinBots.Clear();
            }

            if (nowMs < _nextDueTickMs) return;

            _scratchFieldPins.Clear();
            _scratchLockPins.Clear();
            _scratchAuraPins.Clear();

            lock (_pins)
            {
                long earliestNext = long.MaxValue;
                if (_pins.Count > 0)
                {
                    if (nowMs >= _nextFieldPinMs)
                    {
                        foreach (PinnedField fp in _pins.Values) _scratchFieldPins.Add(fp);
                        _nextFieldPinMs = nowMs + PinIntervalMs;
                    }
                    if (_nextFieldPinMs < earliestNext) earliestNext = _nextFieldPinMs;
                }

                foreach (TargetLockPin lockPin in _targetLocks.Values)
                {
                    if (nowMs - lockPin.LastTickMs >= lockPin.IntervalMs)
                    {
                        _scratchLockPins.Add(lockPin);
                        long nextLock = nowMs + lockPin.IntervalMs;
                        if (nextLock < earliestNext) earliestNext = nextLock;
                    }
                    else
                    {
                        long nextLock = lockPin.LastTickMs + lockPin.IntervalMs;
                        if (nextLock < earliestNext) earliestNext = nextLock;
                    }
                }

                foreach (DamageAuraPin auraPin in _damageAuras.Values)
                {
                    if (nowMs - auraPin.LastTickMs >= auraPin.IntervalMs)
                    {
                        _scratchAuraPins.Add(auraPin);
                        long nextAura = nowMs + auraPin.IntervalMs;
                        if (nextAura < earliestNext) earliestNext = nextAura;
                    }
                    else
                    {
                        long nextAura = auraPin.LastTickMs + auraPin.IntervalMs;
                        if (nextAura < earliestNext) earliestNext = nextAura;
                    }
                }

                _nextDueTickMs = earliestNext;
            }

            bool didSceneScanThisFrame = false;
            for (int i = 0; i < _scratchFieldPins.Count; i++)
            {
                try
                {
                    ApplyPinnedField(_scratchFieldPins[i], nowMs, ref didSceneScanThisFrame);
                }
                catch
                {
                    // Scene transition or unloaded instance; keep pin armed until gameplay resumes.
                }
            }
            _scratchFieldPins.Clear();

            for (int i = 0; i < _scratchLockPins.Count; i++)
            {
                TargetLockPin lockPin = _scratchLockPins[i];
                try
                {
                    TickTargetLock(lockPin, nowMs);
                }
                catch
                {
                    lockPin.LastTickMs = nowMs;
                }
            }
            _scratchLockPins.Clear();

            for (int i = 0; i < _scratchAuraPins.Count; i++)
            {
                DamageAuraPin auraPin = _scratchAuraPins[i];
                try
                {
                    TickDamageAura(auraPin, nowMs);
                }
                catch
                {
                    auraPin.LastTickMs = nowMs;
                }
            }
            _scratchAuraPins.Clear();
        }

        private bool IsRequiredActivePresent(
            Type reqType,
            ref UnityEngine.Component cached,
            ref long lastMissMs,
            long nowMs)
        {
            if (reqType == null) return true;
            return !IsNullOrDestroyed(GetCachedActiveComponent(reqType, ref cached, ref lastMissMs, nowMs));
        }

        /// <summary>
        /// Must be invoked from MonoBehaviour.LateUpdate() so visual-rig SpinBots run
        /// after Unity's Animator overwrites skeleton transforms, leaving camera/controls untouched.
        /// </summary>
        public void PumpLateUpdate()
        {
            if (Volatile.Read(ref _activePinCount) == 0) return;
            int frame = Time.frameCount;
            if (frame == _lastLateFrame) return;
            _lastLateFrame = frame;

            long nowMs;
            _scratchSpinBots.Clear();
            lock (_pins)
            {
                if (_spinBots.Count == 0) return;
                foreach (SpinBotPin sb in _spinBots.Values)
                {
                    if (sb.SpinInLateUpdate &&
                        !string.IsNullOrEmpty(sb.SpinNodePath) &&
                        sb.RollSpeedDeg != 0f &&
                        sb.RollMember == null)
                    {
                        _scratchSpinBots.Add(sb);
                    }
                }
            }
            if (_scratchSpinBots.Count == 0) return;

            nowMs = _loopClock.ElapsedMilliseconds;
            for (int i = 0; i < _scratchSpinBots.Count; i++)
            {
                SpinBotPin sb = _scratchSpinBots[i];
                try
                {
                    if (!IsRequiredActivePresent(
                        sb.RequireActiveType, ref sb.CachedRequireActive, ref sb.LastRequireActiveMissMs, nowMs))
                    {
                        RestoreSpinNode(sb);
                        sb.CachedPlayer = null;
                        continue;
                    }
                    UnityEngine.Component playerComp = GetCachedActiveComponent(
                        sb.PlayerType, ref sb.CachedPlayer, ref sb.LastPlayerMissMs, nowMs);
                    if (!IsNullOrDestroyed(playerComp))
                    {
                        ApplySpinNodeRotation(sb, playerComp.transform, nowMs);
                    }
                }
                catch { }
            }
            _scratchSpinBots.Clear();
        }

        private void AcceptLoop()
        {
            while (_running)
            {
                try
                {
                    TcpClient client = _listener.AcceptTcpClient();
                    ThreadPool.QueueUserWorkItem(_ => HandleClient(client));
                }
                catch
                {
                    if (!_running) break;
                }
            }
        }

        private void HandleClient(TcpClient client)
        {
            using (client)
            {
                try
                {
                    client.ReceiveTimeout = 5000;
                    client.SendTimeout = 5000;
                    using var stream = client.GetStream();
                    using var reader = new StreamReader(stream, Encoding.UTF8, false, 4096, true);
                    using var writer = new StreamWriter(stream, new UTF8Encoding(false), 4096, true);

                    string line = reader.ReadLine();
                    if (string.IsNullOrWhiteSpace(line)) return;

                    Dictionary<string, object> req;
                    try
                    {
                        req = MiniJson.Deserialize(line) as Dictionary<string, object>;
                    }
                    catch
                    {
                        req = null;
                    }

                    string reply;
                    if (req == null)
                    {
                        reply = ErrorJson("invalid JSON object");
                    }
                    else
                    {
                        string token = GetString(req, "token");
                        if (!ConstantTimeEquals(token, AuthToken))
                        {
                            reply = ErrorJson("unauthorized token");
                        }
                        else
                        {
                            string op = GetString(req, "op") ?? "";
                            var p = req.TryGetValue("params", out object pObj)
                                ? pObj as Dictionary<string, object>
                                : null;

                            if (op == "ping")
                            {
                                // Reply immediately on the socket thread so handshake succeeds even while
                                // the game window is unfocused or still loading scenes.
                                reply = OkJson(new Dictionary<string, object>
                                {
                                    { "flavor", Flavor },
                                    { "loader", LoaderKind },
                                    { "pid", _pid },
                                    { "port", Port },
                                    { "unity_version", _unityVersion },
                                    { "bridge_version", BridgeVersion },
                                    { "main_thread_ticks", Interlocked.Read(ref _mainThreadTicks) },
                                    { "active_pins", Volatile.Read(ref _activePinCount) },
                                    { "scene_scans", Interlocked.Read(ref _sceneScanCount) }
                                });
                            }
                            else
                            {
                                var work = new PendingWork { Op = op, Params = p };
                                bool enqueued = false;
                                lock (_queueLock)
                                {
                                    if (_queue.Count < MaxPendingWork)
                                    {
                                        _queue.Enqueue(work);
                                        enqueued = true;
                                    }
                                }

                                if (!enqueued)
                                {
                                    reply = ErrorJson("game main thread is busy (loading scene); please retry after loading finishes");
                                }
                                else if (work.Done.Wait(MainThreadTimeoutMs))
                                {
                                    reply = work.ResponseLine;
                                }
                                else if (Interlocked.CompareExchange(
                                             ref work.State,
                                             PendingWork.StateCancelled,
                                             PendingWork.StatePending) == PendingWork.StatePending)
                                {
                                    reply = ErrorJson("main thread dispatch timeout (game is loading or unfocused; wait until gameplay resumes and retry)");
                                }
                                else
                                {
                                    reply = work.Done.Wait(1000)
                                        ? work.ResponseLine
                                        : ErrorJson("main thread dispatch timeout (game is loading or unfocused; wait until gameplay resumes and retry)");
                                }
                            }
                        }
                    }

                    writer.WriteLine(reply);
                    writer.Flush();
                }
                catch
                {
                    // Connection closed or timed out cleanly.
                }
            }
        }

        private string ProcessRequestOnMainThread(string op, Dictionary<string, object> p)
        {
            switch (op)
            {
                case "list_types":
                    return HandleListTypes(p);

                case "list_members":
                    return HandleListMembers(p);

                case "find_instances":
                    return HandleFindInstances(p);

                case "dump_hierarchy":
                    return HandleDumpHierarchy(p);

                case "get_field":
                    return HandleGetField(p);

                case "set_field":
                    return HandleSetField(p);

                case "pin_field":
                    return HandlePinField(p);

                case "unpin_field":
                    return HandleUnpinField(p);

                case "pin_target_lock":
                    return HandlePinTargetLock(p);

                case "unpin_target_lock":
                    return HandleUnpinTargetLock(p);

                case "pin_damage_aura":
                    return HandlePinDamageAura(p);

                case "unpin_damage_aura":
                    return HandleUnpinDamageAura(p);

                case "pin_spinbot":
                    return HandlePinSpinBot(p);

                case "invoke_method_args":
                    return HandleInvokeMethodArgs(p);

                case "set_gameobject_active":
                    return HandleSetGameObjectActive(p);

                case "copy_field":
                    return HandleCopyField(p);

                case "replace_texture_material":
                    return HandleReplaceTextureMaterial(p);

                case "restore_texture_material":
                    return HandleRestoreTextureMaterial(p);

                case "unpin_spinbot":
                    return HandleUnpinSpinBot(p);

                case "pin_method_hook":
                    return HandlePinMethodHook(p);

                case "unpin_method_hook":
                    return HandleUnpinMethodHook(p);

                case "clear_pins":
                    return HandleClearPins();

                case "invoke_method":
                    return HandleInvokeMethod(p);

                default:
                    return ErrorJson("unsupported op: " + op);
            }
        }

        private string HandleListTypes(Dictionary<string, object> p)
        {
            string filter = GetString(p, "filter") ?? "";
            var names = new List<string>();
            foreach (Assembly asm in AppDomain.CurrentDomain.GetAssemblies())
            {
                Type[] types;
                try { types = asm.GetTypes(); } catch { continue; }
                foreach (Type t in types)
                {
                    if (t == null || string.IsNullOrEmpty(t.FullName)) continue;
                    if (filter.Length == 0 || t.FullName.IndexOf(filter, StringComparison.OrdinalIgnoreCase) >= 0)
                    {
                        names.Add(t.FullName);
                        if (names.Count >= 100) break;
                    }
                }
                if (names.Count >= 100) break;
            }
            return OkJson(new Dictionary<string, object> { { "types", names } });
        }

        private string HandleListMembers(Dictionary<string, object> p)
        {
            string className = GetString(p, "class_name");
            if (string.IsNullOrEmpty(className)) return ErrorJson("list_members requires class_name");

            Type type = ResolveType(className);
            if (type == null) return ErrorJson("type not found: " + className);

            string filter = GetString(p, "filter");
            int limit = ClampInt(GetInt(p, "limit", 400), 1, 2000);
            bool includeStatic = GetBool(p, "include_static", false);

            const BindingFlags flags = BindingFlags.Public | BindingFlags.NonPublic | BindingFlags.Instance
                | BindingFlags.Static | BindingFlags.DeclaredOnly;

            var fields = new List<object>();
            var props = new List<object>();
            var methods = new List<object>();
            var baseTypes = new List<string>();

            for (Type cur = type; cur != null && cur != typeof(object); cur = cur.BaseType)
            {
                if (cur != type) baseTypes.Add(cur.FullName);
                foreach (FieldInfo fi in cur.GetFields(flags))
                {
                    if (!includeStatic && fi.IsStatic) continue;
                    if (fields.Count >= limit) break;
                    if (filter != null && filter.Length > 0 && fi.Name.IndexOf(filter, StringComparison.OrdinalIgnoreCase) < 0) continue;
                    fields.Add(new Dictionary<string, object>
                    {
                        { "name", fi.Name },
                        { "type", fi.FieldType.FullName ?? fi.FieldType.Name },
                        { "declared_on", cur.FullName },
                        { "is_static", fi.IsStatic },
                        { "can_write", !(fi.IsInitOnly || fi.IsLiteral) }
                    });
                }
                foreach (PropertyInfo pi in cur.GetProperties(flags))
                {
                    MethodInfo getter = pi.GetGetMethod(true);
                    MethodInfo setter = pi.GetSetMethod(true);
                    if (!includeStatic && (getter?.IsStatic ?? setter?.IsStatic) == true) continue;
                    if (props.Count >= limit) break;
                    if (filter != null && filter.Length > 0 && pi.Name.IndexOf(filter, StringComparison.OrdinalIgnoreCase) < 0) continue;
                    props.Add(new Dictionary<string, object>
                    {
                        { "name", pi.Name },
                        { "type", pi.PropertyType.FullName ?? pi.PropertyType.Name },
                        { "declared_on", cur.FullName },
                        { "can_read", getter != null },
                        { "can_write", setter != null }
                    });
                }
            }

            if (GetBool(p, "include_methods", true))
            {
                const BindingFlags methodFlags = BindingFlags.Public | BindingFlags.NonPublic | BindingFlags.Instance
                    | BindingFlags.Static | BindingFlags.DeclaredOnly;
                for (Type cur = type; cur != null && cur != typeof(object); cur = cur.BaseType)
                {
                    foreach (MethodInfo mi in cur.GetMethods(methodFlags))
                    {
                        if (mi.IsSpecialName) continue;
                        if (!includeStatic && mi.IsStatic) continue;
                        if (methods.Count >= limit) break;
                        if (filter != null && filter.Length > 0 && mi.Name.IndexOf(filter, StringComparison.OrdinalIgnoreCase) < 0) continue;
                        var ps = new List<object>();
                        foreach (ParameterInfo param in mi.GetParameters())
                        {
                            ps.Add(new Dictionary<string, object>
                            {
                                { "name", param.Name },
                                { "type", param.ParameterType.FullName ?? param.ParameterType.Name }
                            });
                        }
                        methods.Add(new Dictionary<string, object>
                        {
                            { "name", mi.Name },
                            { "return_type", mi.ReturnType.FullName ?? mi.ReturnType.Name },
                            { "declared_on", cur.FullName },
                            { "is_static", mi.IsStatic },
                            { "params", ps }
                        });
                    }
                }
            }

            return OkJson(new Dictionary<string, object>
            {
                { "type", type.FullName },
                { "base_types", baseTypes },
                { "fields", fields },
                { "properties", props },
                { "methods", methods }
            });
        }

        private string HandleFindInstances(Dictionary<string, object> p)
        {
            string className = GetString(p, "class_name");
            if (string.IsNullOrEmpty(className)) return ErrorJson("find_instances requires class_name");

            Type type = ResolveType(className);
            if (type == null) return ErrorJson("type not found: " + className);

            bool includeInactive = GetBool(p, "include_inactive", false);
            int limit = ClampInt(GetInt(p, "limit", 20), 1, 200);

            var found = new List<object>();
            List<object> instances = FindAllInstances(type, includeInactive);
            int count = 0;
            foreach (object inst in instances)
            {
                count++;
                if (found.Count >= limit) continue;
                var entry = new Dictionary<string, object> { { "index", count - 1 } };
                var comp = inst as UnityEngine.Component;
                if (comp != null)
                {
                    try
                    {
                        entry["game_object"] = comp.gameObject != null ? comp.gameObject.name : null;
                        entry["path"] = BuildTransformPath(comp.transform);
                        entry["active_in_hierarchy"] = comp.gameObject != null && comp.gameObject.activeInHierarchy;
                        entry["position"] = comp.transform != null ? FormatVector(comp.transform.position) : null;
                    }
                    catch { }
                }
                else
                {
                    UnityEngine.Object obj = inst as UnityEngine.Object;
                    if (obj != null)
                    {
                        entry["name"] = obj.name;
                    }
                }
                entry["runtime_type"] = inst.GetType().FullName;
                found.Add(entry);
            }

            return OkJson(new Dictionary<string, object>
            {
                { "type", type.FullName },
                { "total", count },
                { "instances", found }
            });
        }

        private static string BuildTransformPath(Transform t)
        {
            if (t == null) return null;
            var parts = new List<string>();
            for (Transform cur = t; cur != null && parts.Count < 24; cur = cur.parent)
            {
                parts.Add(cur.name);
            }
            parts.Reverse();
            return string.Join("/", parts);
        }

        private static Dictionary<string, object> FormatVector(Vector3 v)
        {
            return new Dictionary<string, object>
            {
                { "x", v.x }, { "y", v.y }, { "z", v.z }
            };
        }

        /// <summary>
        /// Dumps the transform tree under a live component (or a named GameObject), with
        /// per-node path, tag, activity and optional renderer/animator presence. Used by MCP
        /// to locate the mesh/rig node a game actually rotates.
        /// </summary>
        private string HandleDumpHierarchy(Dictionary<string, object> p)
        {
            string className = GetString(p, "class_name");
            string gameObjectName = GetString(p, "game_object_name");
            int maxDepth = ClampInt(GetInt(p, "max_depth", 6), 1, 16);
            int limit = ClampInt(GetInt(p, "limit", 300), 1, 4000);

            Transform root = null;
            if (!string.IsNullOrEmpty(className))
            {
                Type type = ResolveType(className);
                if (type == null) return ErrorJson("type not found: " + className);
                UnityEngine.Component comp = FindActiveComponent(type);
                if (IsNullOrDestroyed(comp)) comp = FindAnyComponent(type);
                if (IsNullOrDestroyed(comp)) return ErrorJson("no live instance of '" + className + "'");
                root = comp.transform;
            }
            else if (!string.IsNullOrEmpty(gameObjectName))
            {
                GameObject go = FindGameObjectByName(gameObjectName);
                if (go == null) return ErrorJson("GameObject not found: " + gameObjectName);
                root = go.transform;
            }
            else
            {
                return ErrorJson("dump_hierarchy requires class_name or game_object_name");
            }

            var nodes = new List<object>();
            DumpTransformNode(root, 0, maxDepth, limit, nodes);

            return OkJson(new Dictionary<string, object>
            {
                { "root", BuildTransformPath(root) },
                { "count", nodes.Count },
                { "nodes", nodes }
            });
        }

        private void DumpTransformNode(Transform t, int depth, int maxDepth, int limit, List<object> nodes)
        {
            if (t == null || nodes.Count >= limit) return;
            var node = new Dictionary<string, object>
            {
                { "path", BuildTransformPath(t) },
                { "name", t.name },
                { "depth", depth },
                { "local_position", FormatVector(t.localPosition) },
                { "local_euler", FormatVector(t.localEulerAngles) }
            };
            try { node["tag"] = t.gameObject != null ? t.gameObject.tag : null; } catch { }
            try { node["active_self"] = t.gameObject != null && t.gameObject.activeSelf; } catch { }
            try { node["renderer"] = t.GetComponent<Renderer>() != null; } catch { }
            try
            {
                Type animType = ResolveTypeStatic("UnityEngine.Animator");
                if (animType != null)
                {
                    var host = t.GetComponent<UnityEngine.Component>();
                    GetComponentOn(host, animType, false, out object animObj);
                    node["animator"] = animObj != null;
                }
            }
            catch { }
            nodes.Add(node);

            if (depth >= maxDepth) return;
            int childCount = t.childCount;
            for (int i = 0; i < childCount; i++)
            {
                if (nodes.Count >= limit) return;
                DumpTransformNode(t.GetChild(i), depth + 1, maxDepth, limit, nodes);
            }
        }

        private UnityEngine.Component FindAnyComponent(Type type)
        {
            List<object> all = FindAllInstances(type, true);
            foreach (object obj in all)
            {
                UnityEngine.Component c = obj as UnityEngine.Component;
                if (!IsNullOrDestroyed(c)) return c;
            }
            return null;
        }

        /// <summary>
        /// Resolves a relative transform path (e.g. "Armature/Hips") under a root, and also
        /// accepts an absolute-ish path that contains the root's own name as its first segment.
        /// </summary>
        private static Transform ResolveTransformPath(Transform root, string path)
        {
            if (root == null || string.IsNullOrEmpty(path)) return null;
            string[] parts = path.Split('/');
            Transform cur = root;
            int start = 0;
            if (parts.Length > 0 && string.Equals(parts[0], root.name, StringComparison.Ordinal))
                start = 1;

            for (int i = start; i < parts.Length; i++)
            {
                string seg = parts[i];
                if (seg.Length == 0) continue;
                Transform next = null;
                for (int c = 0; c < cur.childCount; c++)
                {
                    Transform child = cur.GetChild(c);
                    if (child != null && string.Equals(child.name, seg, StringComparison.Ordinal))
                    {
                        next = child;
                        break;
                    }
                }
                if (next == null) return null;
                cur = next;
            }
            return cur;
        }

        private static GameObject FindGameObjectByName(string name)
        {
            foreach (GameObject go in UnityEngine.Object.FindObjectsOfType<GameObject>())
            {
                if (go != null && go.name == name) return go;
            }
            return null;
        }

        private string HandleGetField(Dictionary<string, object> p)
        {
            string className = GetString(p, "class_name");
            string fieldName = GetString(p, "field_name");
            if (string.IsNullOrEmpty(className) || string.IsNullOrEmpty(fieldName))
                return ErrorJson("get_field requires class_name and field_name");

            Type type = ResolveType(className);
            if (type == null) return ErrorJson("type not found: " + className);

            if (!TryGetFieldPath(type, fieldName, false, out object val, out bool missingTarget, out string error))
            {
                if (!missingTarget || !TryGetFieldPath(type, fieldName, true, out val, out _, out error))
                    return ErrorJson(error);
            }

            return OkJson(new Dictionary<string, object>
            {
                { "type", type.FullName },
                { "member", fieldName },
                { "value", val }
            });
        }

        private string HandleSetField(Dictionary<string, object> p)
        {
            string className = GetString(p, "class_name");
            string fieldName = GetString(p, "field_name");
            if (string.IsNullOrEmpty(className) || string.IsNullOrEmpty(fieldName) || p == null || !p.ContainsKey("value"))
                return ErrorJson("set_field requires class_name, field_name, and value");

            Type type = ResolveType(className);
            if (type == null) return ErrorJson("type not found: " + className);

            object rawValue = p["value"];
            if (!ApplySetFieldPath(type, fieldName, rawValue, false, out int updated, out string error))
                return ErrorJson(error);

            if (updated == 0)
            {
                if (!ApplySetFieldPath(type, fieldName, rawValue, true, out updated, out error))
                    return ErrorJson(error);
            }

            if (updated == 0)
                return ErrorJson($"no live instance of '{type.FullName}' found in active scene");

            return OkJson(new Dictionary<string, object>
            {
                { "type", type.FullName },
                { "member", fieldName },
                { "updated", updated }
            });
        }

        private string HandlePinField(Dictionary<string, object> p)
        {
            string id = GetString(p, "id");
            string className = GetString(p, "class_name");
            string fieldName = GetString(p, "field_name");
            if (string.IsNullOrEmpty(id) || string.IsNullOrEmpty(className) || string.IsNullOrEmpty(fieldName) || p == null || !p.ContainsKey("value"))
                return ErrorJson("pin_field requires id, class_name, field_name, and value");

            Type type = ResolveType(className);
            if (type == null) return ErrorJson("type not found: " + className);

            if (!SplitFieldPath(fieldName, out string[] segments, out string pathError))
                return ErrorJson(pathError);

            object rawValue = p["value"];
            if (!PrevalidateMemberPath(type, segments, rawValue, true, out string structuralError))
                return ErrorJson(structuralError);

            if (!ApplySetFieldPath(type, fieldName, rawValue, false, out int updated, out string applyError))
                return ErrorJson(applyError);

            if (updated == 0)
            {
                if (!ApplySetFieldPath(type, fieldName, rawValue, true, out updated, out applyError))
                    return ErrorJson(applyError);
            }

            MemberBinding rootMember = GetMemberCached(type, segments[0]);
            object singleConverted = null;
            bool hasSingleConverted = false;
            if (segments.Length == 1 && rootMember != null && rootMember.CanWrite)
            {
                try
                {
                    singleConverted = ConvertToType(rawValue, rootMember.MemberType);
                    hasSingleConverted = true;
                }
                catch
                {
                    hasSingleConverted = false;
                }
            }

            long nowMs = _loopClock.ElapsedMilliseconds;
            lock (_pins)
            {
                _pins[id] = new PinnedField
                {
                    Id = id,
                    RootType = type,
                    FieldName = fieldName,
                    Value = rawValue,
                    Segments = segments,
                    RootMember = rootMember,
                    SingleConvertedValue = singleConverted,
                    HasSingleConvertedValue = hasSingleConverted
                };
                UpdateActivePinCountLocked();
                _nextFieldPinMs = nowMs + PinIntervalMs;
                _nextDueTickMs = 0;
            }

            return OkJson(new Dictionary<string, object>
            {
                { "id", id },
                { "type", type.FullName },
                { "member", fieldName },
                { "pinned", true },
                { "updated", updated },
                { "pending", updated == 0 }
            });
        }

        private string HandleUnpinField(Dictionary<string, object> p)
        {
            string id = GetString(p, "id");
            if (string.IsNullOrEmpty(id))
                return ErrorJson("unpin_field requires id");

            lock (_pins)
            {
                _pins.Remove(id);
                UpdateActivePinCountLocked();
                _nextDueTickMs = 0;
            }

            int restored = 0;
            string className = GetString(p, "class_name");
            string fieldName = GetString(p, "field_name");
            if (p != null && p.ContainsKey("restore_value") && !string.IsNullOrEmpty(className) && !string.IsNullOrEmpty(fieldName))
            {
                Type type = ResolveType(className);
                if (type != null)
                {
                    if (ApplySetFieldPath(type, fieldName, p["restore_value"], false, out restored, out _) && restored == 0)
                    {
                        ApplySetFieldPath(type, fieldName, p["restore_value"], true, out restored, out _);
                    }
                }
            }

            return OkJson(new Dictionary<string, object>
            {
                { "id", id },
                { "unpinned", true },
                { "restored", restored }
            });
        }

        private string HandlePinTargetLock(Dictionary<string, object> p)
        {
            string id = GetString(p, "id");
            string playerClass = GetString(p, "player_class");
            string cameraClass = GetString(p, "camera_class");
            string targetClass = GetString(p, "target_class");
            if (string.IsNullOrEmpty(id) || string.IsNullOrEmpty(playerClass) || string.IsNullOrEmpty(cameraClass) || string.IsNullOrEmpty(targetClass))
                return ErrorJson("pin_target_lock requires id, player_class, camera_class, and target_class");
            ConfigurePlayerTags(p);

            Type playerType = ResolveType(playerClass);
            if (playerType == null) return ErrorJson("type not found: " + playerClass);
            Type cameraType = ResolveType(cameraClass);
            if (cameraType == null) return ErrorJson("type not found: " + cameraClass);
            Type targetType = ResolveType(targetClass);
            if (targetType == null) return ErrorJson("type not found: " + targetClass);

            var aliveChecks = new List<AliveCheckSpec>();
            foreach (Dictionary<string, object> rawCheck in GetDictList(p, "alive_checks"))
            {
                string compClass = GetString(rawCheck, "component_class");
                if (string.IsNullOrEmpty(compClass))
                    return ErrorJson("alive_checks entry requires component_class");
                Type compType = ResolveType(compClass);
                if (compType == null) return ErrorJson("type not found: " + compClass);
                string deadField = GetString(rawCheck, "dead_field");
                string healthField = GetString(rawCheck, "health_field");
                aliveChecks.Add(new AliveCheckSpec
                {
                    ComponentType = compType,
                    DeadField = deadField,
                    HealthField = healthField,
                    DeadMember = !string.IsNullOrEmpty(deadField) ? GetMemberCached(compType, deadField) : null,
                    HealthMember = !string.IsNullOrEmpty(healthField) ? GetMemberCached(compType, healthField) : null
                });
            }

            string lockMethod = string.IsNullOrEmpty(GetString(p, "lock_method")) ? "SetLockTarget" : GetString(p, "lock_method");
            string unlockMethod = string.IsNullOrEmpty(GetString(p, "unlock_method")) ? "RemoveLockTarget" : GetString(p, "unlock_method");
            float maxRange = ClampFloat(GetFloat(p, "max_range", 45f), 1f, 500f);
            float heightOffset = GetFloat(p, "height_offset", 0f);
            float lockSpeed = ClampFloat(GetFloat(p, "lock_speed", 10f), 0.5f, 100f);
            bool requireCollider = GetBool(p, "require_collider", true);
            long intervalMs = ClampLong(GetLong(p, "interval_ms", 80), 30, 2000);

            MethodInfo lock3 = null;
            MethodInfo lock1 = null;
            MethodInfo unlock = null;
            foreach (MethodInfo m in cameraType.GetMethods(InstanceMethodFlags))
            {
                ParameterInfo[] ps = m.GetParameters();
                if (m.Name == lockMethod)
                {
                    if (lock3 == null && ps.Length == 3 && ps[0].ParameterType.IsAssignableFrom(typeof(UnityEngine.Transform)))
                    {
                        lock3 = m;
                    }
                    else if (lock1 == null && ps.Length == 1 && ps[0].ParameterType.IsAssignableFrom(typeof(UnityEngine.Transform)))
                    {
                        lock1 = m;
                    }
                }
                if (unlock == null && m.Name == unlockMethod && ps.Length == 0)
                {
                    unlock = m;
                }
            }

            object[] lockArgs3 = null;
            if (lock3 != null)
            {
                ParameterInfo[] ps = lock3.GetParameters();
                lockArgs3 = new[]
                {
                    null,
                    ConvertToType(heightOffset, ps[1].ParameterType),
                    ConvertToType(lockSpeed, ps[2].ParameterType)
                };
            }

            var pin = new TargetLockPin
            {
                Id = id,
                PlayerType = playerType,
                CameraType = cameraType,
                TargetType = targetType,
                ColliderType = requireCollider ? ResolveType("UnityEngine.Collider") : null,
                LockMethod = lockMethod,
                UnlockMethod = unlockMethod,
                LockMethod3Arg = lock3,
                LockMethod1Arg = lock1,
                UnlockMethodInfo = unlock,
                LockArgs3 = lockArgs3,
                LockArgs1 = lock1 != null ? new object[1] : null,
                AliveChecks = aliveChecks,
                MaxRange = maxRange,
                HeightOffset = heightOffset,
                LockSpeed = lockSpeed,
                RequireCollider = requireCollider,
                IntervalMs = intervalMs,
                LastTickMs = _loopClock.ElapsedMilliseconds,
                CurrentlyLocked = false
            };

            lock (_pins)
            {
                _targetLocks[id] = pin;
                UpdateActivePinCountLocked();
                _nextDueTickMs = 0;
            }

            try
            {
                TickTargetLock(pin, _loopClock.ElapsedMilliseconds);
            }
            catch
            {
                // Keep lock armed if scene/combat objects are not yet spawned.
            }

            return OkJson(new Dictionary<string, object>
            {
                { "id", id },
                { "pinned", true },
                { "locked", pin.CurrentlyLocked }
            });
        }

        private string HandleUnpinTargetLock(Dictionary<string, object> p)
        {
            string id = GetString(p, "id");
            if (string.IsNullOrEmpty(id))
                return ErrorJson("unpin_target_lock requires id");

            TargetLockPin removed = null;
            lock (_pins)
            {
                if (_targetLocks.TryGetValue(id, out TargetLockPin existing))
                {
                    removed = existing;
                    _targetLocks.Remove(id);
                }
                UpdateActivePinCountLocked();
                _nextDueTickMs = 0;
            }

            if (removed != null)
            {
                try { InvokeCameraUnlock(removed); } catch { }
            }

            return OkJson(new Dictionary<string, object>
            {
                { "id", id },
                { "unpinned", true }
            });
        }

        private string HandlePinDamageAura(Dictionary<string, object> p)
        {
            string id = GetString(p, "id");
            string playerClass = GetString(p, "player_class");
            string targetClass = GetString(p, "target_class");
            if (string.IsNullOrEmpty(id) || string.IsNullOrEmpty(playerClass) || string.IsNullOrEmpty(targetClass))
                return ErrorJson("pin_damage_aura requires id, player_class, and target_class");
            ConfigurePlayerTags(p);

            Type playerType = ResolveType(playerClass);
            if (playerType == null) return ErrorJson("type not found: " + playerClass);
            Type targetType = ResolveType(targetClass);
            if (targetType == null) return ErrorJson("type not found: " + targetClass);

            Type requireActiveType = null;
            string requireActiveClass = GetString(p, "require_active_class");
            if (!string.IsNullOrEmpty(requireActiveClass))
            {
                requireActiveType = ResolveType(requireActiveClass);
                if (requireActiveType == null) return ErrorJson("type not found: " + requireActiveClass);
            }

            string excludeBoolField = GetString(p, "exclude_bool_field");
            MemberBinding excludeBoolMember = !string.IsNullOrEmpty(excludeBoolField)
                ? GetMemberCached(targetType, excludeBoolField)
                : null;

            List<Dictionary<string, object>> rawHandlers = GetDictList(p, "handlers");
            if (rawHandlers.Count == 0)
                return ErrorJson("pin_damage_aura requires at least one damage handler");

            int damageAmount = ClampInt(GetInt(p, "damage_amount", 999999), 1, 100000000);
            float radius = ClampFloat(GetFloat(p, "radius", 25f), 1f, 200f);
            long intervalMs = ClampLong(GetLong(p, "interval_ms", 250), 100, 5000);

            var handlers = new List<DamageHandlerSpec>(rawHandlers.Count);
            foreach (Dictionary<string, object> raw in rawHandlers)
            {
                string compClass = GetString(raw, "component_class");
                string methodName = GetString(raw, "method_name");
                if (string.IsNullOrEmpty(compClass) || string.IsNullOrEmpty(methodName))
                    return ErrorJson("damage handler requires component_class and method_name");

                Type compType = ResolveType(compClass);
                if (compType == null) return ErrorJson("type not found: " + compClass);

                List<AuraArgSpec> argSpecs = ParseAuraArgs(raw, damageAmount);
                if (argSpecs == null)
                    return ErrorJson("damage handler has an unparsable 'args' / legacy 'kind' entry");

                string deadMethodName = GetString(raw, "dead_method");
                MethodInfo deadMethodInfo = null;
                if (!string.IsNullOrEmpty(deadMethodName))
                {
                    foreach (MethodInfo m in compType.GetMethods(InstanceMethodFlags))
                    {
                        if (m.Name == deadMethodName && m.GetParameters().Length == 0)
                        {
                            deadMethodInfo = m;
                            break;
                        }
                    }
                }

                var handler = new DamageHandlerSpec
                {
                    ComponentType = compType,
                    MethodName = methodName,
                    DeadField = GetString(raw, "dead_field"),
                    HealthField = GetString(raw, "health_field"),
                    DeadMethod = deadMethodName,
                    DeadMethodInfo = deadMethodInfo,
                    ZeroHealthBeforeCall = GetBool(raw, "zero_health_before_call", false),
                    Args = argSpecs
                };
                if (!string.IsNullOrEmpty(handler.DeadField))
                    handler.DeadMember = GetMemberCached(compType, handler.DeadField);
                if (!string.IsNullOrEmpty(handler.HealthField))
                    handler.HealthMember = GetMemberCached(compType, handler.HealthField);

                BindAuraMethod(handler);
                if (handler.DamageMethod == null)
                    return ErrorJson($"method '{methodName}' on '{compClass}' has no {argSpecs.Count}-parameter overload");

                handlers.Add(handler);
            }

            var pin = new DamageAuraPin
            {
                Id = id,
                PlayerType = playerType,
                TargetType = targetType,
                RequireActiveType = requireActiveType,
                ExcludeBoolField = excludeBoolField,
                ExcludeBoolMember = excludeBoolMember,
                DamageAmount = damageAmount,
                Radius = radius,
                IntervalMs = intervalMs,
                LastTickMs = _loopClock.ElapsedMilliseconds,
                Handlers = handlers
            };

            lock (_pins)
            {
                _damageAuras[id] = pin;
                UpdateActivePinCountLocked();
                _nextDueTickMs = 0;
            }

            int hits = 0;
            try
            {
                hits = TickDamageAura(pin, _loopClock.ElapsedMilliseconds);
            }
            catch
            {
                // Keep aura armed if scene/combat objects are not yet spawned.
            }

            return OkJson(new Dictionary<string, object>
            {
                { "id", id },
                { "pinned", true },
                { "hits", hits }
            });
        }

        private string HandleUnpinDamageAura(Dictionary<string, object> p)
        {
            string id = GetString(p, "id");
            if (string.IsNullOrEmpty(id))
                return ErrorJson("unpin_damage_aura requires id");

            lock (_pins)
            {
                _damageAuras.Remove(id);
                UpdateActivePinCountLocked();
                _nextDueTickMs = 0;
            }

            return OkJson(new Dictionary<string, object>
            {
                { "id", id },
                { "unpinned", true }
            });
        }


        private string HandlePinSpinBot(Dictionary<string, object> p)
        {
            string id = GetString(p, "id");
            string playerClass = GetString(p, "player_class");
            string targetClass = GetString(p, "target_class");
            float orbitRadius = ClampFloat(GetFloat(p, "orbit_radius", 14f), 0f, 500f);
            bool aimAtTarget = GetBool(p, "aim_at_target", !string.IsNullOrEmpty(targetClass));
            if (string.IsNullOrEmpty(id) || string.IsNullOrEmpty(playerClass))
                return ErrorJson("pin_spinbot requires id and player_class");
            if (string.IsNullOrEmpty(targetClass) && (orbitRadius > 0f || aimAtTarget))
                return ErrorJson("pin_spinbot requires target_class when orbit_radius > 0 or aim_at_target is enabled");
            ConfigurePlayerTags(p);

            Type playerType = ResolveType(playerClass);
            if (playerType == null) return ErrorJson("type not found: " + playerClass);
            Type targetType = null;
            if (!string.IsNullOrEmpty(targetClass))
            {
                targetType = ResolveType(targetClass);
                if (targetType == null) return ErrorJson("type not found: " + targetClass);
            }

            Type requireActiveType = null;
            string requireActiveClass = GetString(p, "require_active_class");
            if (!string.IsNullOrEmpty(requireActiveClass))
            {
                requireActiveType = ResolveType(requireActiveClass);
                if (requireActiveType == null) return ErrorJson("type not found: " + requireActiveClass);
            }

            var aliveChecks = new List<AliveCheckSpec>();
            foreach (Dictionary<string, object> rawCheck in GetDictList(p, "alive_checks"))
            {
                string compClass = GetString(rawCheck, "component_class");
                if (string.IsNullOrEmpty(compClass))
                    return ErrorJson("alive_checks entry requires component_class");
                Type compType = ResolveType(compClass);
                if (compType == null) return ErrorJson("type not found: " + compClass);
                string deadField = GetString(rawCheck, "dead_field");
                string healthField = GetString(rawCheck, "health_field");
                aliveChecks.Add(new AliveCheckSpec
                {
                    ComponentType = compType,
                    DeadField = deadField,
                    HealthField = healthField,
                    DeadMember = !string.IsNullOrEmpty(deadField) ? GetMemberCached(compType, deadField) : null,
                    HealthMember = !string.IsNullOrEmpty(healthField) ? GetMemberCached(compType, healthField) : null
                });
            }

            string rollField = GetString(p, "roll_field");
            MemberBinding rollMember = null;
            if (!string.IsNullOrEmpty(rollField))
            {
                rollMember = GetMemberCached(playerType, rollField);
                if (rollMember == null || !rollMember.CanRead || !rollMember.CanWrite)
                    return ErrorJson("roll_field '" + rollField + "' must be a writable member of " + playerClass);
            }

            string spinNodePath = GetString(p, "spin_node_path");
            bool spinInLateUpdate = GetBool(p, "late_update", !string.IsNullOrEmpty(spinNodePath));
            long nowMs = _loopClock.ElapsedMilliseconds;

            var pin = new SpinBotPin
            {
                Id = id,
                PlayerType = playerType,
                TargetType = targetType,
                RequireActiveType = requireActiveType,
                AliveChecks = aliveChecks,
                MaxRange = ClampFloat(GetFloat(p, "max_range", 100000f), 1f, 1000000f),
                OrbitRadius = orbitRadius,
                OrbitSpeedDeg = ClampFloat(GetFloat(p, "orbit_speed_deg", 180f), 0f, 2160f),
                HeightOffset = ClampFloat(GetFloat(p, "height_offset", 4f), -200f, 200f),
                RollSpeedDeg = ClampFloat(GetFloat(p, "roll_speed_deg", 360f), -2160f, 2160f),
                SpinNodePath = spinNodePath,
                SpinLocal = GetBool(p, "spin_local", true),
                SpinInLateUpdate = spinInLateUpdate,
                RollMember = rollMember,
                AimAtTarget = aimAtTarget,
                RequireAlive = GetBool(p, "require_alive", true),
                IntervalMs = ClampLong(GetLong(p, "interval_ms", 16), 8, 1000),
                StartMs = nowMs,
                LastTickMs = nowMs
            };

            SpinBotPin previous = null;
            lock (_pins)
            {
                _spinBots.TryGetValue(id, out previous);
                _spinBots[id] = pin;
                UpdateActivePinCountLocked();
                _nextDueTickMs = 0;
            }
            if (previous != null) RestoreSpinNode(previous);

            return OkJson(new Dictionary<string, object>
            {
                { "id", id },
                { "pinned", true },
                { "has_roll_channel", rollMember != null },
                { "spin_node_path", spinNodePath },
                { "late_update", spinInLateUpdate }
            });
        }

        private string HandleUnpinSpinBot(Dictionary<string, object> p)
        {
            string id = GetString(p, "id");
            if (string.IsNullOrEmpty(id)) return ErrorJson("unpin_spinbot requires id");

            SpinBotPin removed = null;
            lock (_pins)
            {
                if (_spinBots.TryGetValue(id, out removed))
                    _spinBots.Remove(id);
                UpdateActivePinCountLocked();
                _nextDueTickMs = 0;
            }
            if (removed != null) RestoreSpinNode(removed);
            return OkJson(new Dictionary<string, object> { { "id", id }, { "unpinned", true } });
        }

        private static void RestoreSpinNode(SpinBotPin pin)
        {
            if (pin == null) return;
            if (pin.HasOriginalRotation && !IsNullOrDestroyed(pin.SpinNode))
            {
                try
                {
                    if (pin.SpinLocal) pin.SpinNode.localRotation = pin.OriginalRotation;
                    else pin.SpinNode.rotation = pin.OriginalRotation;
                }
                catch { }
            }
            pin.HasOriginalRotation = false;
            pin.SpinNode = null;
            pin.SpinNodeOwnerId = 0;
            pin.SpinNodeMissMs = -1;
        }

        private void ApplySpinNodeRotation(SpinBotPin pin, Transform playerTf, long nowMs)
        {
            if (playerTf == null || string.IsNullOrEmpty(pin.SpinNodePath) || pin.RollSpeedDeg == 0f || pin.RollMember != null)
                return;

            int ownerId = playerTf.GetInstanceID();
            if (pin.SpinNodeOwnerId != ownerId)
            {
                pin.SpinNode = null;
                pin.SpinNodeOwnerId = 0;
                pin.HasOriginalRotation = false;
                pin.SpinNodeMissMs = -1;
            }

            Transform node = pin.SpinNode;
            if (IsNullOrDestroyed(node) || (node.gameObject != null && !node.gameObject.activeInHierarchy))
            {
                pin.SpinNode = null;
                pin.SpinNodeOwnerId = 0;
                pin.HasOriginalRotation = false;
                if (pin.SpinNodeMissMs < 0 || nowMs - pin.SpinNodeMissMs > MissRetryMs)
                {
                    node = ResolveTransformPath(playerTf, pin.SpinNodePath);
                    pin.SpinNode = node;
                    pin.SpinNodeOwnerId = node != null ? ownerId : 0;
                    pin.SpinNodeMissMs = nowMs;
                }
                else
                {
                    return;
                }
            }
            if (IsNullOrDestroyed(node)) return;

            float elapsedSec = Math.Max(0L, nowMs - pin.StartMs) / 1000f;
            float spin = (pin.RollSpeedDeg * elapsedSec) % 360f;
            try
            {
                if (!pin.HasOriginalRotation)
                {
                    pin.OriginalRotation = pin.SpinLocal ? node.localRotation : node.rotation;
                    pin.HasOriginalRotation = true;
                }
                Quaternion spun = Quaternion.AngleAxis(spin, Vector3.up) * pin.OriginalRotation;
                if (pin.SpinLocal) node.localRotation = spun;
                else node.rotation = spun;
            }
            catch { }
        }

        private void TickSpinBot(SpinBotPin pin, long nowMs)
        {
            long deltaMs = pin.LastTickMs > 0 ? nowMs - pin.LastTickMs : 16;
            if (deltaMs <= 0 || deltaMs > 500) deltaMs = 16;
            pin.LastTickMs = nowMs;
            pin.ElapsedMs += deltaMs;

            if (!IsRequiredActivePresent(
                pin.RequireActiveType, ref pin.CachedRequireActive, ref pin.LastRequireActiveMissMs, nowMs))
            {
                if (!pin.SpinInLateUpdate) RestoreSpinNode(pin);
                pin.CachedPlayer = null;
                return;
            }

            UnityEngine.Component playerComp = GetCachedActiveComponent(
                pin.PlayerType, ref pin.CachedPlayer, ref pin.LastPlayerMissMs, nowMs);
            if (IsNullOrDestroyed(playerComp)) return;

            Transform playerTf = playerComp.transform;

            if (!pin.SpinInLateUpdate)
            {
                ApplySpinNodeRotation(pin, playerTf, nowMs);
            }

            if (pin.RollMember != null && pin.RollSpeedDeg != 0f)
            {
                try
                {
                    float next = pin.RollSpeedDeg * (pin.ElapsedMs / 1000f);
                    pin.RollMember.Write(playerComp, ConvertToType(next % 360f, pin.RollMember.MemberType));
                }
                catch
                {
                    // Game rejected the roll channel; keep the pin armed.
                }
            }

            UnityEngine.Component target = null;
            Transform targetTf = null;
            if (pin.TargetType != null && (pin.AimAtTarget || pin.OrbitRadius > 0f))
            {
                ActiveComponentListCache cache = GetCachedActiveComponents(pin.TargetType, EnemyScanIntervalMs, nowMs);
                float bestSqr = pin.MaxRange * pin.MaxRange;
                Vector3 playerPos = playerTf.position;
                Transform playerRoot = playerTf.root;
                List<ActiveComponentEntry> entries = cache.Entries;
                for (int i = 0; i < entries.Count; i++)
                {
                    ActiveComponentEntry e = entries[i];
                    UnityEngine.Component tc = e.Component;
                    if (IsNullOrDestroyed(tc)) continue;
                    GameObject go = e.GameObject;
                    if (go == null || !go.activeInHierarchy) continue;
                    if (e.Root == playerRoot) continue;
                    if (e.IsPlayerTag) continue;
                    if (pin.RequireAlive && !IsTargetAlive(tc, pin.AliveChecks)) continue;

                    float sqr = (e.Transform.position - playerPos).sqrMagnitude;
                    if (sqr > bestSqr) continue;
                    bestSqr = sqr;
                    target = tc;
                    targetTf = e.Transform;
                }
            }

            pin.CurrentTarget = target;
            if (targetTf == null || pin.OrbitRadius <= 0f) return;

            pin.AngleDeg = (pin.AngleDeg + pin.OrbitSpeedDeg * (deltaMs / 1000f)) % 360f;
            float rad = pin.AngleDeg * Mathf.Deg2Rad;
            Vector3 targetPos = targetTf.position;
            Vector3 orbitPos = new Vector3(
                targetPos.x + Mathf.Cos(rad) * pin.OrbitRadius,
                targetPos.y + pin.HeightOffset,
                targetPos.z + Mathf.Sin(rad) * pin.OrbitRadius);

            try
            {
                playerTf.position = orbitPos;
                KillRigidbodyMotion(playerComp, playerTf);
            }
            catch
            {
                return;
            }

            if (pin.AimAtTarget)
            {
                Vector3 lookDir = targetPos - orbitPos;
                if (lookDir.sqrMagnitude > 0.0001f)
                {
                    try { playerTf.rotation = Quaternion.LookRotation(lookDir.normalized, Vector3.up); }
                    catch { }
                }
            }

            pin.AppliedTicks++;
        }

        private void KillRigidbodyMotion(UnityEngine.Component comp, Transform tf)
        {
            try
            {
                Type rbType = ResolveType("UnityEngine.Rigidbody");
                if (rbType == null) return;
                UnityEngine.Component source = IsNullOrDestroyed(comp) ? (UnityEngine.Component)tf : comp;
                if (IsNullOrDestroyed(source)) return;
                GetComponentOn(source, rbType, false, out object typed);
                if (typed == null) return;
                MemberBinding vel = GetMemberCached(rbType, "velocity");
                if (vel != null && vel.CanWrite) vel.Write(typed, Vector3.zero);
                MemberBinding ang = GetMemberCached(rbType, "angularVelocity");
                if (ang != null && ang.CanWrite) ang.Write(typed, Vector3.zero);
            }
            catch
            {
                // No rigidbody on this actor; nothing to damp.
            }
        }


        private string HandleInvokeMethodArgs(Dictionary<string, object> p)
        {
            string className = GetString(p, "class_name");
            string methodName = GetString(p, "method_name");
            if (string.IsNullOrEmpty(className) || string.IsNullOrEmpty(methodName))
                return ErrorJson("invoke_method_args requires class_name and method_name");

            Type type = ResolveType(className);
            if (type == null) return ErrorJson("type not found: " + className);

            var rawArgs = new List<object>();
            if (p != null && p.TryGetValue("args", out object rawArgObj) && rawArgObj is List<object> argList)
                rawArgs.AddRange(argList);

            const BindingFlags flags = BindingFlags.Public | BindingFlags.NonPublic | BindingFlags.Static
                | BindingFlags.Instance | BindingFlags.FlattenHierarchy;

            MethodInfo mi = null;
            object[] converted = null;
            foreach (MethodInfo candidate in type.GetMethods(flags))
            {
                if (candidate.Name != methodName) continue;
                ParameterInfo[] ps = candidate.GetParameters();
                if (ps.Length != rawArgs.Count) continue;
                var attempt = new object[ps.Length];
                bool ok = true;
                for (int i = 0; i < ps.Length; i++)
                {
                    try
                    {
                        if (rawArgs[i] is Dictionary<string, object> typeRef)
                        {
                            string targetClass = GetString(typeRef, "class_name");
                            if (!string.IsNullOrEmpty(targetClass))
                            {
                                Type targetType = ResolveType(targetClass);
                                if (targetType == null) { ok = false; break; }
                                List<object> instances = FindAllInstances(targetType, GetBool(typeRef, "include_inactive", false));
                                if (instances.Count == 0) { ok = false; break; }
                                int idx = ClampInt(GetInt(typeRef, "index", 0), 0, instances.Count - 1);
                                attempt[i] = instances[idx];
                                continue;
                            }
                        }
                        attempt[i] = ConvertToType(rawArgs[i], ps[i].ParameterType);
                    }
                    catch
                    {
                        ok = false;
                        break;
                    }
                }
                if (!ok) continue;
                mi = candidate;
                converted = attempt;
                break;
            }

            if (mi == null)
                return ErrorJson($"no overload of '{methodName}' on '{type.FullName}' matched {rawArgs.Count} argument(s)");

            try
            {
                if (mi.IsStatic)
                {
                    object ret = mi.Invoke(null, converted);
                    return OkJson(new Dictionary<string, object>
                    {
                        { "type", type.FullName },
                        { "method", methodName },
                        { "invoked", 1 },
                        { "result", ret?.ToString() }
                    });
                }

                List<object> instances = FindAllInstances(type, false);
                if (instances.Count == 0) instances = FindAllInstances(type, true);
                if (instances.Count == 0)
                    return ErrorJson($"no live instance of '{type.FullName}' found in active scene");

                object last = null;
                foreach (object inst in instances) last = mi.Invoke(inst, converted);
                return OkJson(new Dictionary<string, object>
                {
                    { "type", type.FullName },
                    { "method", methodName },
                    { "invoked", instances.Count },
                    { "result", last?.ToString() }
                });
            }
            catch (TargetInvocationException tie)
            {
                return ErrorJson("method invocation failed: " + (tie.InnerException?.Message ?? tie.Message));
            }
        }

        private static List<string> GetStringList(Dictionary<string, object> dict, string key, string singleFallbackKey)
        {
            var values = new List<string>();
            if (dict != null && dict.TryGetValue(key, out object rawObj) && rawObj is List<object> rawList)
            {
                foreach (object item in rawList)
                {
                    if (item is string s && s.Length > 0) values.Add(s);
                }
            }
            if (!string.IsNullOrEmpty(singleFallbackKey))
            {
                string single = GetString(dict, singleFallbackKey);
                if (!string.IsNullOrEmpty(single) && !values.Contains(single))
                    values.Add(single);
            }
            return values;
        }

        private List<UnityEngine.GameObject> EnumerateSceneGameObjects(bool includeInactive)
        {
            var result = new List<UnityEngine.GameObject>();
            if (!includeInactive)
            {
                foreach (UnityEngine.GameObject go in UnityEngine.Object.FindObjectsOfType<UnityEngine.GameObject>())
                {
                    if (go != null) result.Add(go);
                }
                return result;
            }

            if (_findObjectsIncludeInactive != null)
            {
                try
                {
#if IL2CPP
                    var il2Type = Il2CppInterop.Runtime.Il2CppType.From(typeof(UnityEngine.GameObject), false);
                    object raw = il2Type != null ? _findObjectsIncludeInactive.Invoke(null, new object[] { il2Type, true }) : null;
#else
                    object raw = _findObjectsIncludeInactive.Invoke(null, new object[] { typeof(UnityEngine.GameObject), true });
#endif
                    if (raw is System.Collections.IEnumerable found)
                    {
                        foreach (object item in found)
                        {
                            if (item is UnityEngine.GameObject go && go != null) result.Add(go);
                        }
                        return result;
                    }
                }
                catch { }
            }

            foreach (UnityEngine.GameObject go in UnityEngine.Resources.FindObjectsOfTypeAll<UnityEngine.GameObject>())
            {
                if (go == null) continue;
                try
                {
                    if (go.hideFlags != HideFlags.None) continue;
                    if (!go.scene.IsValid()) continue;
                }
                catch
                {
                    continue;
                }
                result.Add(go);
            }
            return result;
        }

        private string HandleSetGameObjectActive(Dictionary<string, object> p)
        {
            List<string> prefixes = GetStringList(p, "game_object_prefixes", "game_object_prefix");
            List<string> exactNames = GetStringList(p, "game_object_names", "game_object_name");
            if (prefixes.Count == 0 && exactNames.Count == 0)
                return ErrorJson("set_gameobject_active requires game_object_prefix(es) or game_object_name(s)");
            if (p == null || !p.ContainsKey("active"))
                return ErrorJson("set_gameobject_active requires active");

            bool active = GetBool(p, "active", false);
            bool includeInactive = GetBool(p, "include_inactive", true);
            int changed = 0;
            int scanned = 0;

            foreach (UnityEngine.GameObject go in EnumerateSceneGameObjects(includeInactive))
            {
                scanned++;
                string goName = go.name;
                bool match = false;
                for (int i = 0; i < exactNames.Count; i++)
                {
                    if (string.Equals(goName, exactNames[i], StringComparison.Ordinal))
                    {
                        match = true;
                        break;
                    }
                }
                if (!match)
                {
                    for (int i = 0; i < prefixes.Count; i++)
                    {
                        if (goName.StartsWith(prefixes[i], StringComparison.Ordinal))
                        {
                            match = true;
                            break;
                        }
                    }
                }
                if (!match) continue;
                if (go.activeSelf == active) continue;
                go.SetActive(active);
                changed++;
            }

            return OkJson(new Dictionary<string, object>
            {
                { "scanned", scanned },
                { "changed", changed },
                { "active", active }
            });
        }

        private string HandleCopyField(Dictionary<string, object> p)
        {
            string fromClass = GetString(p, "from_class");
            string fromField = GetString(p, "from_field");
            string toClass = GetString(p, "to_class");
            string toField = GetString(p, "to_field");
            if (string.IsNullOrEmpty(fromClass) || string.IsNullOrEmpty(fromField)
                || string.IsNullOrEmpty(toClass) || string.IsNullOrEmpty(toField))
                return ErrorJson("copy_field requires from_class, from_field, to_class, and to_field");

            Type srcType = ResolveType(fromClass);
            if (srcType == null) return ErrorJson("type not found: " + fromClass);
            Type dstType = ResolveType(toClass);
            if (dstType == null) return ErrorJson("type not found: " + toClass);

            object value = null;
            if (!TryGetFieldPath(srcType, fromField, false, out value, out _, out string readErr))
            {
                if (!TryGetFieldPath(srcType, fromField, true, out value, out _, out readErr))
                    return ErrorJson(readErr);
            }
            if (value == null) return ErrorJson($"source '{fromClass}.{fromField}' resolved to null");

            if (!ApplySetFieldPath(dstType, toField, value, false, out int updated, out string setErr))
                return ErrorJson(setErr);
            if (updated == 0)
            {
                if (!ApplySetFieldPath(dstType, toField, value, true, out updated, out setErr))
                    return ErrorJson(setErr);
            }
            if (updated == 0)
                return ErrorJson($"no live instance of '{dstType.FullName}' found in active scene");

            return OkJson(new Dictionary<string, object>
            {
                { "from", fromClass + "." + fromField },
                { "to", toClass + "." + toField },
                { "updated", updated },
                { "value", value }
            });
        }

        private static bool TryValidateSafeTexturePath(string rawPath, out string fullPath, out string error)
        {
            fullPath = null;
            if (string.IsNullOrWhiteSpace(rawPath) || rawPath.IndexOf('\0') >= 0)
            {
                error = "texture_path must be a non-empty valid file path";
                return false;
            }
            string[] segments = rawPath.Split(new[] { '/', '\\' }, StringSplitOptions.RemoveEmptyEntries);
            for (int i = 0; i < segments.Length; i++)
            {
                if (segments[i] == "..")
                {
                    error = "texture_path must not contain parent-directory traversal ('..')";
                    return false;
                }
            }
            if (!Path.IsPathRooted(rawPath))
            {
                error = "texture_path must be a resolved canonical absolute asset path";
                return false;
            }
            try
            {
                fullPath = Path.GetFullPath(rawPath);
            }
            catch (Exception ex)
            {
                error = "invalid texture_path: " + ex.Message;
                return false;
            }
            string ext = Path.GetExtension(fullPath);
            if (!string.Equals(ext, ".png", StringComparison.OrdinalIgnoreCase) &&
                !string.Equals(ext, ".jpg", StringComparison.OrdinalIgnoreCase) &&
                !string.Equals(ext, ".jpeg", StringComparison.OrdinalIgnoreCase))
            {
                error = "texture_path must point to a .png, .jpg, or .jpeg image";
                return false;
            }
            if (!File.Exists(fullPath))
            {
                error = "texture file does not exist: " + fullPath;
                return false;
            }
            var info = new FileInfo(fullPath);
            if (info.Length <= 0 || info.Length > 16L * 1024L * 1024L)
            {
                error = "texture file size must be between 1 byte and 16 MiB";
                return false;
            }
            error = null;
            return true;
        }

        private string HandleReplaceTextureMaterial(Dictionary<string, object> p)
        {
            string id = GetString(p, "id") ?? "default";
            List<string> rendererPrefixes = GetStringList(p, "renderer_name_prefixes", "renderer_name_prefix");
            List<string> materialPrefixes = GetStringList(p, "material_name_prefixes", "material_name_prefix");
            string texturePath = GetString(p, "texture_path");
            string shaderName = GetString(p, "shader_name");
            if (materialPrefixes.Count == 0)
                return ErrorJson("replace_texture_material requires material_name_prefix or material_name_prefixes");
            if (!TryValidateSafeTexturePath(texturePath, out string safeTexturePath, out string pathErr))
                return ErrorJson(pathErr);

            Texture2D tex = LoadTextureFromFile(safeTexturePath);
            if (tex == null) return ErrorJson("failed to load texture: " + safeTexturePath);

            Type skinnedType = ResolveType("UnityEngine.SkinnedMeshRenderer");
            Type meshType = ResolveType("UnityEngine.Renderer");
            if (skinnedType == null) return ErrorJson("UnityEngine.SkinnedMeshRenderer unavailable");

            MemberBinding sharedMats = GetMemberCached(meshType != null ? meshType : skinnedType, "sharedMaterials");
            if (sharedMats == null || !sharedMats.CanRead || !sharedMats.CanWrite)
                return ErrorJson("renderer lacks readable/writable sharedMaterials member");

            Type materialType = ResolveType("UnityEngine.Material");
            if (materialType == null) return ErrorJson("UnityEngine.Material unavailable");
            ConstructorInfo matCtor = materialType.GetConstructor(new[] { materialType });
            Type shaderType = ResolveTypeStatic("UnityEngine.Shader");
            ConstructorInfo shaderCtor = shaderType != null ? materialType.GetConstructor(new[] { shaderType }) : null;
            MemberBinding matMainTex = GetMemberCached(materialType, "mainTexture");
            MemberBinding matColor = GetMemberCached(materialType, "color");
            MemberBinding matName = GetMemberCached(materialType, "name");
            if ((matCtor == null && shaderCtor == null) || matMainTex == null)
                return ErrorJson("UnityEngine.Material lacks required constructors or mainTexture");

            Shader overrideShader = null;
            if (!string.IsNullOrEmpty(shaderName) && shaderCtor != null)
            {
                try { overrideShader = Shader.Find(shaderName); } catch { overrideShader = null; }
            }

            bool hasColorOverride = false;
            Color colorOverride = Color.white;
            if (p != null && p.TryGetValue("color", out object rawColor) && rawColor is List<object> colList && colList.Count >= 3)
            {
                try
                {
                    float r = Convert.ToSingle(colList[0], CultureInfo.InvariantCulture);
                    float g = Convert.ToSingle(colList[1], CultureInfo.InvariantCulture);
                    float b = Convert.ToSingle(colList[2], CultureInfo.InvariantCulture);
                    float a = colList.Count >= 4 ? Convert.ToSingle(colList[3], CultureInfo.InvariantCulture) : 1f;
                    colorOverride = new Color(r, g, b, a);
                    hasColorOverride = true;
                }
                catch { hasColorOverride = false; }
            }

            var backups = new List<MaterialBackupEntry>();
            int replaced = 0;
            int scanned = 0;
            List<object> renderers = FindAllInstances(skinnedType, true);
            foreach (object rObj in renderers)
            {
                UnityEngine.Component renderer = rObj as UnityEngine.Component;
                if (IsNullOrDestroyed(renderer) || renderer.gameObject == null) continue;
                if (rendererPrefixes.Count > 0)
                {
                    string goName = renderer.gameObject.name;
                    bool rMatch = false;
                    for (int i = 0; i < rendererPrefixes.Count; i++)
                    {
                        if (goName.StartsWith(rendererPrefixes[i], StringComparison.Ordinal))
                        {
                            rMatch = true;
                            break;
                        }
                    }
                    if (!rMatch) continue;
                }
                scanned++;

                object arr = sharedMats.Read(renderer);
                if (arr is System.Collections.IList list && list.Count > 0)
                {
                    UnityEngine.Material[] originalSnapshot = null;
                    bool rendererModified = false;

                    for (int i = 0; i < list.Count; i++)
                    {
                        if (!(list[i] is UnityEngine.Material srcMat) || srcMat == null) continue;
                        string mName = srcMat.name ?? "";
                        bool mMatch = false;
                        for (int mIdx = 0; mIdx < materialPrefixes.Count; mIdx++)
                        {
                            if (mName.StartsWith(materialPrefixes[mIdx], StringComparison.Ordinal))
                            {
                                mMatch = true;
                                break;
                            }
                        }
                        if (!mMatch) continue;

                        if (originalSnapshot == null)
                        {
                            originalSnapshot = new UnityEngine.Material[list.Count];
                            for (int k = 0; k < list.Count; k++)
                                originalSnapshot[k] = list[k] as UnityEngine.Material;
                        }

                        UnityEngine.Material clone = overrideShader != null && shaderCtor != null
                            ? (UnityEngine.Material)shaderCtor.Invoke(new object[] { overrideShader })
                            : (UnityEngine.Material)matCtor.Invoke(new object[] { srcMat });

                        if (matMainTex.CanWrite) matMainTex.Write(clone, tex);
                        if (matColor != null && matColor.CanWrite)
                        {
                            if (hasColorOverride) matColor.Write(clone, colorOverride);
                            else if (overrideShader != null) matColor.Write(clone, srcMat.color);
                        }
                        if (matName != null && matName.CanWrite) matName.Write(clone, mName);

                        list[i] = clone;
                        rendererModified = true;
                        replaced++;
                    }

                    if (rendererModified)
                    {
                        sharedMats.Write(renderer, list);
                        if (originalSnapshot != null)
                        {
                            backups.Add(new MaterialBackupEntry
                            {
                                Renderer = renderer,
                                SharedMaterialsMember = sharedMats,
                                OriginalMaterials = originalSnapshot
                            });
                        }
                    }
                }
            }

            if (backups.Count > 0)
            {
                if (_materialBackups.TryGetValue(id, out List<MaterialBackupEntry> existing))
                {
                    existing.AddRange(backups);
                }
                else
                {
                    _materialBackups[id] = backups;
                }
            }

            return OkJson(new Dictionary<string, object>
            {
                { "id", id },
                { "scanned", scanned },
                { "replaced", replaced },
                { "texture", safeTexturePath }
            });
        }

        private int RestoreMaterialBackupList(List<MaterialBackupEntry> entries)
        {
            if (entries == null || entries.Count == 0) return 0;
            int restoredRenderers = 0;
            for (int i = entries.Count - 1; i >= 0; i--)
            {
                MaterialBackupEntry entry = entries[i];
                if (IsNullOrDestroyed(entry.Renderer) || entry.SharedMaterialsMember == null || entry.OriginalMaterials == null)
                    continue;
                try
                {
                    object curArr = entry.SharedMaterialsMember.Read(entry.Renderer);
                    if (curArr is System.Collections.IList curList && curList.Count == entry.OriginalMaterials.Length)
                    {
                        for (int m = 0; m < entry.OriginalMaterials.Length; m++)
                            curList[m] = entry.OriginalMaterials[m];
                        entry.SharedMaterialsMember.Write(entry.Renderer, curList);
                        restoredRenderers++;
                    }
                }
                catch { }
            }
            return restoredRenderers;
        }

        private string HandleRestoreTextureMaterial(Dictionary<string, object> p)
        {
            string id = GetString(p, "id") ?? "default";
            if (!_materialBackups.TryGetValue(id, out List<MaterialBackupEntry> entries))
            {
                return OkJson(new Dictionary<string, object>
                {
                    { "id", id },
                    { "restored", 0 }
                });
            }
            _materialBackups.Remove(id);
            int restored = RestoreMaterialBackupList(entries);
            return OkJson(new Dictionary<string, object>
            {
                { "id", id },
                { "restored", restored }
            });
        }

        private static Texture2D LoadTextureFromFile(string path)
        {
            try
            {
                if (!File.Exists(path)) return null;
                byte[] bytes = File.ReadAllBytes(path);
                Type texType = ResolveTypeStatic("UnityEngine.Texture2D");
                if (texType == null) return null;
                var tex = (Texture2D)Activator.CreateInstance(texType, new object[] { 2, 2 });
                Type imgConv = ResolveTypeStatic("UnityEngine.ImageConversion");
                if (imgConv == null) return null;
                MethodInfo load = null;
                foreach (MethodInfo m in imgConv.GetMethods(BindingFlags.Public | BindingFlags.Static))
                {
                    if (m.Name != "LoadImage") continue;
                    ParameterInfo[] ps = m.GetParameters();
                    if (ps.Length == 2) { load = m; break; }
                    if (ps.Length == 3 && load == null) load = m;
                }
                if (load == null) return null;

                ParameterInfo[] target = load.GetParameters();
                object[] args = new object[target.Length];
                args[0] = tex;
                Type arrType = target[1].ParameterType;

                object arr = null;
                if (arrType == typeof(byte[]))
                {
                    arr = bytes;
                }
                else
                {
                    Type structArrOpen = ResolveTypeStatic("Il2CppInterop.Runtime.InteropTypes.Arrays.Il2CppStructArray`1");
                    if (structArrOpen != null)
                    {
                        try
                        {
                            Type closed = structArrOpen.MakeGenericType(typeof(byte));
                            arr = Activator.CreateInstance(closed, new object[] { (long)bytes.Length });
                            if (arr is System.Collections.IList list)
                            {
                                for (int i = 0; i < bytes.Length; i++) list[i] = bytes[i];
                            }
                        }
                        catch { arr = null; }
                    }
                    if (arr == null)
                    {
                        try { arr = Convert.ChangeType(bytes, arrType, CultureInfo.InvariantCulture); }
                        catch { }
                    }
                }
                if (arr == null) return null;
                args[1] = arr;
                if (target.Length == 3) args[2] = false;

                load.Invoke(null, args);
                return tex;
            }
            catch
            {
                return null;
            }
        }

        private static Type ResolveTypeStatic(string typeName)
        {
            if (string.IsNullOrEmpty(typeName)) return null;
            Type resolved = Type.GetType(typeName, false);
            if (resolved != null) return resolved;
            foreach (Assembly asm in AppDomain.CurrentDomain.GetAssemblies())
            {
                try
                {
                    Type exact = asm.GetType(typeName, false);
                    if (exact != null) return exact;
                    foreach (Type t in asm.GetTypes())
                    {
                        if (t != null && (t.FullName == typeName || t.Name == typeName)) return t;
                    }
                }
                catch { }
            }
            return null;
        }

        private string HandlePinMethodHook(Dictionary<string, object> p)
        {
            string id = GetString(p, "id");
            string hookClass = GetString(p, "hook_class");
            string hookMethod = GetString(p, "hook_method");
            if (string.IsNullOrEmpty(id) || string.IsNullOrEmpty(hookClass) || string.IsNullOrEmpty(hookMethod))
                return ErrorJson("pin_method_hook requires id, hook_class, and hook_method");

            string timing = (GetString(p, "timing") ?? "prefix").Trim().ToLowerInvariant();
            if (timing != "prefix" && timing != "postfix")
                return ErrorJson($"invalid timing '{timing}': expected 'prefix' or 'postfix'");

            int paramCount = GetInt(p, "param_count", -1);
            bool applyImmediately = GetBool(p, "apply_immediately", false);

            Type hookType = ResolveType(hookClass);
            if (hookType == null)
                return ErrorJson("type not found: " + hookClass);

            MethodInfo targetMethod = FindHookTargetMethod(hookType, hookMethod, paramCount);
            if (targetMethod == null)
            {
                return paramCount >= 0
                    ? ErrorJson($"hook method '{hookMethod}' with {paramCount} parameter(s) not found on '{hookType.FullName}'")
                    : ErrorJson($"hook method '{hookMethod}' not found on '{hookType.FullName}'");
            }

            if (p == null || !p.TryGetValue("writes", out object rawWritesObj) || !(rawWritesObj is List<object> rawWrites) || rawWrites.Count == 0)
                return ErrorJson("pin_method_hook requires non-empty writes array");

            var specs = new List<HookFieldWriteSpec>(rawWrites.Count);
            for (int i = 0; i < rawWrites.Count; i++)
            {
                if (!(rawWrites[i] is Dictionary<string, object> wDict))
                    return ErrorJson($"writes[{i}] must be an object");

                string wClass = GetString(wDict, "class_name");
                string wField = GetString(wDict, "field_name");
                if (string.IsNullOrEmpty(wClass) || string.IsNullOrEmpty(wField))
                    return ErrorJson($"writes[{i}] requires class_name and field_name");

                bool hasValue = wDict.ContainsKey("value");
                string copyFromClass = GetString(wDict, "copy_from_class");
                string copyFromField = GetString(wDict, "copy_from_field");
                bool hasCopy = !string.IsNullOrEmpty(copyFromClass) || !string.IsNullOrEmpty(copyFromField);
                string ensureCompName = GetString(wDict, "ensure_component");
                bool hasEnsure = !string.IsNullOrEmpty(ensureCompName);

                int modeCount = (hasValue ? 1 : 0) + (hasCopy ? 1 : 0) + (hasEnsure ? 1 : 0);
                if (modeCount != 1)
                    return ErrorJson($"writes[{i}] must specify exactly one of value, copy_from_class/copy_from_field, or ensure_component");

                Type wType = ResolveType(wClass);
                if (wType == null)
                    return ErrorJson($"writes[{i}] type not found: {wClass}");

                if (!SplitFieldPath(wField, out string[] segs, out string pathErr))
                    return ErrorJson($"writes[{i}] invalid field_name: {pathErr}");

                object wVal = null;
                bool hasRestore = false;
                object restoreVal = null;
                Type copyType = null;
                Type ensureType = null;

                if (hasValue)
                {
                    wVal = wDict["value"];
                    if (!PrevalidateMemberPath(wType, segs, wVal, true, out string structErr))
                        return ErrorJson($"writes[{i}] {structErr}");

                    hasRestore = wDict.ContainsKey("restore_value");
                    restoreVal = hasRestore ? wDict["restore_value"] : null;
                    if (hasRestore && !PrevalidateMemberPath(wType, segs, restoreVal, true, out string restoreErr))
                        return ErrorJson($"writes[{i}] invalid restore_value: {restoreErr}");
                }
                else if (hasCopy)
                {
                    if (string.IsNullOrEmpty(copyFromClass) || string.IsNullOrEmpty(copyFromField))
                        return ErrorJson($"writes[{i}] copy requires both copy_from_class and copy_from_field");

                    copyType = ResolveType(copyFromClass);
                    if (copyType == null)
                        return ErrorJson($"writes[{i}] copy_from_class type not found: {copyFromClass}");

                    if (!SplitFieldPath(copyFromField, out string[] srcSegs, out string srcPathErr))
                        return ErrorJson($"writes[{i}] invalid copy_from_field: {srcPathErr}");

                    if (!PrevalidateMemberPath(copyType, srcSegs, null, false, out string srcStructErr))
                        return ErrorJson($"writes[{i}] {srcStructErr}");

                    if (!PrevalidateMemberPath(wType, segs, null, true, out string dstStructErr, false))
                        return ErrorJson($"writes[{i}] {dstStructErr}");
                }
                else
                {
                    ensureType = ResolveType(ensureCompName);
                    if (ensureType == null)
                        return ErrorJson($"writes[{i}] ensure_component type not found: {ensureCompName}");
                    if (!typeof(UnityEngine.Component).IsAssignableFrom(ensureType))
                        return ErrorJson($"writes[{i}] ensure_component '{ensureType.FullName}' is not a UnityEngine.Component");

                    if (!PrevalidateMemberPath(wType, segs, null, true, out string dstStructErr, false))
                        return ErrorJson($"writes[{i}] {dstStructErr}");
                }

                specs.Add(new HookFieldWriteSpec
                {
                    TargetType = wType,
                    FieldName = wField,
                    HasLiteralValue = hasValue,
                    Value = wVal,
                    HasRestoreValue = hasRestore,
                    RestoreValue = restoreVal,
                    CopyFromType = copyType,
                    CopyFromField = copyFromField,
                    EnsureComponentType = ensureType,
                    InitNullLists = GetBool(wDict, "init_null_lists", false),
                    OnlyIfNull = GetBool(wDict, "only_if_null", hasEnsure),
                    IncludeInactive = GetBool(wDict, "include_inactive", false),
                    CloneUnderOwner = GetBool(wDict, "clone_under_owner", false)
                });
            }

            MethodInfo callbackMethod = timing == "prefix" ? _prefixCallbackMethod : _postfixCallbackMethod;
            if (callbackMethod == null)
                return ErrorJson("internal error: Harmony callback method unavailable");

            MethodHookPin oldPin = null;
            MethodInfo oldUnpatchMethod = null;
            MethodInfo oldUnpatchCallback = null;

            lock (_hookDispatchLock)
            {
                if (_methodHooks.TryGetValue(id, out oldPin))
                {
                    _methodHooks.Remove(id);
                    RemoveHookFromDispatchLocked(oldPin, out oldUnpatchMethod, out oldUnpatchCallback);
                }
            }

            if (oldUnpatchMethod != null && oldUnpatchCallback != null)
            {
                try { _harmony?.Unpatch(oldUnpatchMethod, oldUnpatchCallback); } catch { }
            }

            var newPin = new MethodHookPin
            {
                Id = id,
                HookType = hookType,
                TargetMethod = targetMethod,
                Timing = timing,
                Writes = specs
            };

            Harmony harmonyInstance;
            bool needHarmonyPatch = false;
            lock (_hookDispatchLock)
            {
                if (_harmony == null)
                    _harmony = new Harmony("org.nekod.unitybridge." + _pid);
                harmonyInstance = _harmony;
                _activeHookServer = this;

                Dictionary<MethodBase, List<MethodHookPin>> dispatchMap =
                    timing == "prefix" ? _prefixDispatch : _postfixDispatch;
                if (!dispatchMap.TryGetValue(targetMethod, out List<MethodHookPin> bucket))
                {
                    bucket = new List<MethodHookPin>();
                    dispatchMap[targetMethod] = bucket;
                }
                bucket.Add(newPin);
                needHarmonyPatch = bucket.Count == 1;
                _methodHooks[id] = newPin;
            }

            if (needHarmonyPatch)
            {
                try
                {
                    var hm = new HarmonyMethod(callbackMethod);
                    if (timing == "prefix")
                        harmonyInstance.Patch(targetMethod, prefix: hm);
                    else
                        harmonyInstance.Patch(targetMethod, postfix: hm);
                }
                catch (Exception ex)
                {
                    lock (_hookDispatchLock)
                    {
                        _methodHooks.Remove(id);
                        RemoveHookFromDispatchLocked(newPin, out _, out _);
                    }
                    return ErrorJson($"Harmony patch failed on '{hookType.FullName}.{targetMethod.Name}': {ex.GetBaseException().Message}");
                }
            }

            int appliedCount = applyImmediately ? ApplyHookWriteSpecs(specs, id) : 0;

            return OkJson(new Dictionary<string, object>
            {
                { "id", id },
                { "hook_class", hookType.FullName },
                { "hook_method", targetMethod.Name },
                { "timing", timing },
                { "writes", specs.Count },
                { "applied_immediately", appliedCount }
            });
        }

        private string HandleUnpinMethodHook(Dictionary<string, object> p)
        {
            string id = GetString(p, "id");
            if (string.IsNullOrEmpty(id))
                return ErrorJson("unpin_method_hook requires id");

            MethodHookPin removedPin;
            MethodInfo unpatchMethod = null;
            MethodInfo unpatchCallback = null;
            Harmony harmonyInstance;

            lock (_hookDispatchLock)
            {
                harmonyInstance = _harmony;
                if (!_methodHooks.TryGetValue(id, out removedPin))
                {
                    return OkJson(new Dictionary<string, object>
                    {
                        { "id", id },
                        { "removed", false },
                        { "restored", 0 },
                        { "fire_count", 0L }
                    });
                }
                _methodHooks.Remove(id);
                RemoveHookFromDispatchLocked(removedPin, out unpatchMethod, out unpatchCallback);
            }

            if (harmonyInstance != null && unpatchMethod != null && unpatchCallback != null)
            {
                try { harmonyInstance.Unpatch(unpatchMethod, unpatchCallback); } catch { }
            }

            int restored = RestoreMethodHookWrites(removedPin);
            long fireCount = Interlocked.Read(ref removedPin.FireCount);

            return OkJson(new Dictionary<string, object>
            {
                { "id", id },
                { "removed", true },
                { "restored", restored },
                { "fire_count", fireCount }
            });
        }

        private static void RemoveHookFromDispatchLocked(
            MethodHookPin pin,
            out MethodInfo unpatchMethod,
            out MethodInfo unpatchCallback)
        {
            unpatchMethod = null;
            unpatchCallback = null;
            if (pin?.TargetMethod == null) return;

            bool isPrefix = pin.Timing == "prefix";
            Dictionary<MethodBase, List<MethodHookPin>> dispatchMap = isPrefix ? _prefixDispatch : _postfixDispatch;
            if (dispatchMap.TryGetValue(pin.TargetMethod, out List<MethodHookPin> bucket))
            {
                bucket.Remove(pin);
                if (bucket.Count == 0)
                {
                    dispatchMap.Remove(pin.TargetMethod);
                    unpatchMethod = pin.TargetMethod;
                    unpatchCallback = isPrefix ? _prefixCallbackMethod : _postfixCallbackMethod;
                }
            }
        }

        private static MethodInfo FindHookTargetMethod(Type type, string methodName, int paramCount)
        {
            if (type == null || string.IsNullOrEmpty(methodName)) return null;
            MethodInfo firstMatch = null;
            const BindingFlags declaredFlags =
                BindingFlags.Public | BindingFlags.NonPublic | BindingFlags.Static | BindingFlags.Instance | BindingFlags.DeclaredOnly;
            const BindingFlags flatFlags =
                BindingFlags.Public | BindingFlags.NonPublic | BindingFlags.Static | BindingFlags.Instance | BindingFlags.FlattenHierarchy;

            for (int pass = 0; pass < 2; pass++)
            {
                BindingFlags flags = pass == 0 ? declaredFlags : flatFlags;
                foreach (MethodInfo candidate in type.GetMethods(flags))
                {
                    if (candidate == null || candidate.Name != methodName) continue;
                    if (candidate.IsAbstract || candidate.IsGenericMethodDefinition) continue;
                    int pLen = candidate.GetParameters().Length;
                    if (paramCount >= 0)
                    {
                        if (pLen == paramCount) return candidate;
                        continue;
                    }
                    if (pLen == 0) return candidate;
                    if (firstMatch == null) firstMatch = candidate;
                }
                if (firstMatch != null) return firstMatch;
            }
            return null;
        }

        private static void GenericHookPrefix(MethodBase __originalMethod)
        {
            DispatchMethodHooks(__originalMethod, _prefixDispatch);
        }

        private static void GenericHookPostfix(MethodBase __originalMethod)
        {
            DispatchMethodHooks(__originalMethod, _postfixDispatch);
        }

        private static void DispatchMethodHooks(
            MethodBase originalMethod,
            Dictionary<MethodBase, List<MethodHookPin>> dispatchMap)
        {
            if (originalMethod == null) return;
            BridgeServer server;
            List<MethodHookPin> snapshot = null;
            lock (_hookDispatchLock)
            {
                server = _activeHookServer;
                if (server != null && dispatchMap.TryGetValue(originalMethod, out List<MethodHookPin> list) && list.Count > 0)
                    snapshot = new List<MethodHookPin>(list);
            }
            if (server == null || snapshot == null) return;

            for (int i = 0; i < snapshot.Count; i++)
            {
                MethodHookPin pin = snapshot[i];
                if (pin.Writes == null) continue;
                Interlocked.Increment(ref pin.FireCount);
                server.ApplyHookWriteSpecs(pin.Writes, pin.Id);
            }
        }

        private int ApplyHookWriteSpecs(List<HookFieldWriteSpec> writes, string hookId)
        {
            if (writes == null) return 0;
            int totalUpdated = 0;
            Dictionary<string, GameObject> cloneCache = null;
            List<PendingPlayableRebind> pendingRebinds = null;
            for (int w = 0; w < writes.Count; w++)
            {
                HookFieldWriteSpec spec = writes[w];
                try
                {
                    if (spec.HasLiteralValue)
                    {
                        if (spec.OnlyIfNull && !IsHookTargetNull(spec.TargetType, spec.FieldName, spec.IncludeInactive))
                            continue;
                        ApplySetFieldPath(spec.TargetType, spec.FieldName, spec.Value, false, out int updated, out _);
                        if (updated == 0 && spec.IncludeInactive)
                        {
                            ApplySetFieldPath(spec.TargetType, spec.FieldName, spec.Value, true, out updated, out _);
                        }
                        totalUpdated += updated;
                    }
                    else if (spec.CopyFromType != null)
                    {
                        if (spec.CloneUnderOwner)
                        {
                            if (cloneCache == null) cloneCache = new Dictionary<string, GameObject>();
                            if (pendingRebinds == null) pendingRebinds = new List<PendingPlayableRebind>();
                        }
                        totalUpdated += ApplyHookCopyWrite(spec, cloneCache, pendingRebinds);
                    }
                    else if (spec.EnsureComponentType != null)
                    {
                        totalUpdated += ApplyHookEnsureComponentWrite(spec);
                    }
                }
                catch (Exception ex)
                {
                    _log($"[NekodUnityBridge] hook '{hookId}' write error on {spec.TargetType?.FullName}.{spec.FieldName}: {ex.Message}");
                }
            }
            if (pendingRebinds != null && pendingRebinds.Count > 0)
            {
                ProcessPendingRebinds(cloneCache ?? new Dictionary<string, GameObject>(), pendingRebinds, hookId);
            }
            return totalUpdated;
        }

        private bool IsHookTargetNull(Type targetType, string fieldName, bool includeInactive)
        {
            bool ok = TryGetFieldPath(targetType, fieldName, false, out object cur, out bool missing, out _);
            if ((!ok || missing) && includeInactive)
            {
                ok = TryGetFieldPath(targetType, fieldName, true, out cur, out _, out _);
            }
            return ok && IsNullOrDestroyed(cur);
        }

        private sealed class PendingPlayableRebind
        {
            public GameObject ClonedGo;
            public GameObject SrcOwnerGo;
            public GameObject DstOwnerGo;
        }

        private bool ResolveFieldPathOwners(
            Type rootType,
            string fieldPath,
            bool includeInactive,
            out List<object> owners,
            out string leafName)
        {
            owners = new List<object>();
            leafName = null;
            if (!SplitFieldPath(fieldPath, out string[] segments, out _))
                return false;

            leafName = segments[segments.Length - 1];
            MemberBinding root = GetMemberCached(rootType, segments[0]);
            if (root == null) return false;

            if (segments.Length == 1)
            {
                if (root.IsStaticRead || root.IsStaticWrite)
                {
                    owners.Add(null);
                    return true;
                }
                List<object> roots = FindAllInstances(rootType, false);
                if (roots.Count == 0 && includeInactive)
                    roots = FindAllInstances(rootType, true);
                for (int r = 0; r < roots.Count; r++)
                {
                    if (!IsNullOrDestroyed(roots[r])) owners.Add(roots[r]);
                }
                return owners.Count > 0;
            }

            if (!root.CanRead) return false;
            var curList = new List<object>();
            if (root.IsStaticRead)
            {
                object v = root.Read(null);
                if (!IsNullOrDestroyed(v)) curList.Add(v);
            }
            else
            {
                List<object> roots = FindAllInstances(rootType, false);
                if (roots.Count == 0 && includeInactive)
                    roots = FindAllInstances(rootType, true);
                for (int r = 0; r < roots.Count; r++)
                {
                    if (IsNullOrDestroyed(roots[r])) continue;
                    object v = root.Read(roots[r]);
                    if (!IsNullOrDestroyed(v)) curList.Add(v);
                }
            }

            for (int s = 1; s < segments.Length - 1 && curList.Count > 0; s++)
            {
                var nextList = new List<object>();
                for (int c = 0; c < curList.Count; c++)
                {
                    object curObj = curList[c];
                    MemberBinding m = GetMemberCached(curObj.GetType(), segments[s]);
                    if (m == null || !m.CanRead) continue;
                    object nextObj = m.Read(curObj);
                    if (!IsNullOrDestroyed(nextObj)) nextList.Add(nextObj);
                }
                curList = nextList;
            }

            owners = curList;
            return owners.Count > 0;
        }

        private bool ResolveSourceValueForCopy(
            Type srcType,
            string srcPath,
            bool includeInactive,
            out object srcOwner,
            out object srcValue)
        {
            srcOwner = null;
            srcValue = null;
            for (int pass = 0; pass < (includeInactive ? 2 : 1); pass++)
            {
                bool allowInactive = pass == 1;
                if (!ResolveFieldPathOwners(srcType, srcPath, allowInactive, out List<object> owners, out string leafName))
                    continue;
                for (int i = 0; i < owners.Count; i++)
                {
                    object owner = owners[i];
                    Type ownerType = owner != null ? owner.GetType() : srcType;
                    MemberBinding leaf = GetMemberCached(ownerType, leafName);
                    if (leaf == null || !leaf.CanRead) continue;
                    object candidate = owner == null ? (leaf.IsStaticRead ? leaf.Read(null) : null) : leaf.Read(owner);
                    if (!IsNullOrDestroyed(candidate))
                    {
                        srcOwner = owner;
                        srcValue = candidate;
                        return true;
                    }
                }
            }
            return false;
        }

        private int ApplyHookCopyWrite(
            HookFieldWriteSpec spec,
            Dictionary<string, GameObject> cloneCache = null,
            List<PendingPlayableRebind> pendingRebinds = null)
        {
            if (!spec.CloneUnderOwner)
            {
                if (spec.OnlyIfNull && !IsHookTargetNull(spec.TargetType, spec.FieldName, spec.IncludeInactive))
                    return 0;

                bool ok = TryGetFieldPath(spec.CopyFromType, spec.CopyFromField, false, out object srcVal, out _, out _);
                if ((!ok || IsNullOrDestroyed(srcVal)) && spec.IncludeInactive)
                {
                    ok = TryGetFieldPath(spec.CopyFromType, spec.CopyFromField, true, out srcVal, out _, out _);
                }
                if (!ok || IsNullOrDestroyed(srcVal))
                    return 0;

                ApplySetFieldPath(spec.TargetType, spec.FieldName, srcVal, false, out int directUpdated, out _);
                if (directUpdated == 0 && spec.IncludeInactive)
                {
                    ApplySetFieldPath(spec.TargetType, spec.FieldName, srcVal, true, out directUpdated, out _);
                }
                return directUpdated;
            }

            if (!ResolveSourceValueForCopy(spec.CopyFromType, spec.CopyFromField, spec.IncludeInactive, out object srcOwner, out object srcValue))
                return 0;

            if (!ResolveFieldPathOwners(spec.TargetType, spec.FieldName, false, out List<object> dstOwners, out string dstLeafName)
                && spec.IncludeInactive)
            {
                ResolveFieldPathOwners(spec.TargetType, spec.FieldName, true, out dstOwners, out dstLeafName);
            }
            if (dstOwners == null || dstOwners.Count == 0 || string.IsNullOrEmpty(dstLeafName))
                return 0;

            var cache = cloneCache ?? new Dictionary<string, GameObject>();
            bool runRebindsNow = pendingRebinds == null;
            var rebinds = pendingRebinds ?? new List<PendingPlayableRebind>();
            int updated = 0;

            for (int i = 0; i < dstOwners.Count; i++)
            {
                object dstOwner = dstOwners[i];
                Type ownerType = dstOwner != null ? dstOwner.GetType() : spec.TargetType;
                MemberBinding dstLeaf = GetMemberCached(ownerType, dstLeafName);
                if (dstLeaf == null || !dstLeaf.CanWrite) continue;

                if (spec.OnlyIfNull && dstLeaf.CanRead)
                {
                    object curVal = dstOwner == null ? (dstLeaf.IsStaticRead ? dstLeaf.Read(null) : null) : dstLeaf.Read(dstOwner);
                    if (!IsNullOrDestroyed(curVal)) continue;
                }

                object toWrite = CloneReferenceUnderOwner(srcValue, srcOwner, dstOwner, dstLeaf.MemberType, cache, rebinds);
                if (IsNullOrDestroyed(toWrite)) continue;

                try
                {
                    object converted = ConvertToType(toWrite, dstLeaf.MemberType);
                    dstLeaf.Write(dstOwner, converted);
                    updated++;
                }
                catch { }
            }

            if (runRebindsNow && rebinds.Count > 0)
            {
                ProcessPendingRebinds(cache, rebinds, spec.FieldName);
            }

            return updated;
        }

        private void ProcessPendingRebinds(
            Dictionary<string, GameObject> cloneCache,
            List<PendingPlayableRebind> pendingRebinds,
            string contextLabel)
        {
            if (pendingRebinds == null || pendingRebinds.Count == 0) return;
            var cache = cloneCache ?? new Dictionary<string, GameObject>();
            const int maxRebindPasses = 256;
            for (int r = 0; r < pendingRebinds.Count && r < maxRebindPasses; r++)
            {
                PendingPlayableRebind rebind = pendingRebinds[r];
                if (rebind == null || IsNullOrDestroyed(rebind.ClonedGo)) continue;
                try
                {
                    RebindCinemachineCameras(rebind.ClonedGo, rebind.SrcOwnerGo, rebind.DstOwnerGo, cache, pendingRebinds);
                }
                catch (Exception ex)
                {
                    _log($"[NekodUnityBridge] hook '{contextLabel}' cinemachine rebind error: {ex.Message}");
                }
                try
                {
                    RebindPlayableDirectors(rebind.ClonedGo, rebind.SrcOwnerGo, rebind.DstOwnerGo, cache, pendingRebinds);
                }
                catch (Exception ex)
                {
                    _log($"[NekodUnityBridge] hook '{contextLabel}' timeline rebind error: {ex.Message}");
                }
            }
        }

        private static GameObject ExtractGameObject(object obj)
        {
            if (obj is GameObject go) return go;
            if (obj is Component comp) return comp.gameObject;
            return null;
        }

        private static string ComputeRelativeTransformPath(Transform root, Transform target)
        {
            if (root == null || target == null) return null;
            if (target.GetInstanceID() == root.GetInstanceID()) return string.Empty;

            var parts = new List<string>();
            Transform cur = target;
            while (cur != null && cur.GetInstanceID() != root.GetInstanceID())
            {
                parts.Add(cur.name);
                cur = cur.parent;
            }
            if (cur == null) return null;
            parts.Reverse();
            return string.Join("/", parts.ToArray());
        }

        private static object WrapUnityObjectToType(UnityEngine.Object uObj, Type fallbackType)
        {
            if (uObj == null) return null;
#if IL2CPP
            Type targetType = fallbackType;
            try
            {
                string il2cppFullName = uObj.GetIl2CppType()?.FullName;
                if (!string.IsNullOrEmpty(il2cppFullName))
                {
                    Type runtimeType = ResolveTypeStatic(il2cppFullName);
                    if (runtimeType != null && (fallbackType == null || fallbackType.IsAssignableFrom(runtimeType)))
                    {
                        targetType = runtimeType;
                    }
                }
            }
            catch { }
            if (targetType == null || targetType.IsInstanceOfType(uObj)) return uObj;
            try
            {
                return Activator.CreateInstance(targetType, new object[] { uObj.Pointer }) ?? (object)uObj;
            }
            catch
            {
                return uObj;
            }
#else
            return uObj;
#endif
        }

        private static List<object> CollectSequenceItems(object seq, Type declaredSeqType = null)
        {
            var result = new List<object>();
            if (seq == null) return result;
            if (seq is System.Collections.IEnumerable managedEnum)
            {
                foreach (object item in managedEnum)
                {
                    if (item != null) result.Add(item);
                }
                return result;
            }
#if IL2CPP
            Type seqType = declaredSeqType ?? seq.GetType();
            MethodInfo getEnum = seqType.GetMethod("GetEnumerator", BindingFlags.Public | BindingFlags.Instance);
            if (getEnum == null && declaredSeqType == null)
            {
                Type[] ifaces = seqType.GetInterfaces();
                for (int i = 0; i < ifaces.Length && getEnum == null; i++)
                {
                    if (ifaces[i].FullName != null && ifaces[i].FullName.StartsWith("Il2CppSystem.Collections"))
                    {
                        getEnum = ifaces[i].GetMethod("GetEnumerator", BindingFlags.Public | BindingFlags.Instance);
                    }
                }
            }
            if (getEnum == null || getEnum.GetParameters().Length != 0) return result;
            object rawEnum = null;
            try { rawEnum = getEnum.Invoke(seq, null); } catch { return result; }
            if (rawEnum == null) return result;
            PropertyInfo curProp = getEnum.ReturnType.GetProperty("Current", BindingFlags.Public | BindingFlags.Instance);
            var baseEnum = (rawEnum as Il2CppInterop.Runtime.InteropTypes.Il2CppObjectBase)?.TryCast<Il2CppSystem.Collections.IEnumerator>();
            if (baseEnum == null || curProp == null) return result;
            try
            {
                while (baseEnum.MoveNext())
                {
                    object item = null;
                    try { item = curProp.GetValue(rawEnum, null); } catch { item = null; }
                    if (item != null) result.Add(item);
                }
            }
            finally
            {
                try { (rawEnum as Il2CppInterop.Runtime.InteropTypes.Il2CppObjectBase)?.TryCast<Il2CppSystem.IDisposable>()?.Dispose(); } catch { }
            }
#endif
            return result;
        }

        private static bool TryReadMemberValue(object target, string memberName, out object value)
        {
            value = null;
            if (target == null || string.IsNullOrEmpty(memberName)) return false;
            const BindingFlags flags = BindingFlags.Public | BindingFlags.NonPublic | BindingFlags.Instance;
            Type t = target.GetType();
            PropertyInfo prop = t.GetProperty(memberName, flags);
            if (prop != null && prop.CanRead && prop.GetIndexParameters().Length == 0)
            {
                try { value = prop.GetValue(target, null); return true; } catch { return false; }
            }
            FieldInfo field = t.GetField(memberName, flags);
            if (field != null)
            {
                try { value = field.GetValue(target); return true; } catch { return false; }
            }
            return false;
        }

        private static bool TryWriteMemberValue(object target, string memberName, object value)
        {
            if (target == null || string.IsNullOrEmpty(memberName)) return false;
            const BindingFlags flags = BindingFlags.Public | BindingFlags.NonPublic | BindingFlags.Instance;
            Type t = target.GetType();
            PropertyInfo prop = t.GetProperty(memberName, flags);
            if (prop != null && prop.CanWrite && prop.GetIndexParameters().Length == 0)
            {
                try { prop.SetValue(target, value, null); return true; } catch { return false; }
            }
            FieldInfo field = t.GetField(memberName, flags);
            if (field != null && !field.IsInitOnly)
            {
                try { field.SetValue(target, value); return true; } catch { return false; }
            }
            return false;
        }

        private static object GetExistingComponentByType(GameObject go, Type compType, Component templateComp)
        {
            if (go == null) return null;
            if (compType == typeof(Transform)) return go.transform;
#if IL2CPP
            Il2CppSystem.Type il2cppType = null;
            if (compType != null && compType != typeof(Component) && compType != typeof(UnityEngine.Object))
            {
                il2cppType = Il2CppInterop.Runtime.Il2CppType.From(compType, false);
            }
            else if (templateComp != null)
            {
                try { il2cppType = templateComp.GetIl2CppType(); } catch { il2cppType = null; }
            }
            if (il2cppType == null) return null;
            Component comp = go.GetComponent(il2cppType);
            if (comp == null) return null;
            if (compType == null || compType == typeof(Component) || compType == typeof(UnityEngine.Object))
                return comp;
            try
            {
                return Activator.CreateInstance(compType, new object[] { comp.Pointer }) ?? (object)comp;
            }
            catch
            {
                return comp;
            }
#else
            Type queryType = (compType != null && typeof(Component).IsAssignableFrom(compType) && compType != typeof(Component))
                ? compType
                : (templateComp != null ? templateComp.GetType() : compType);
            if (queryType == null) return null;
            return go.GetComponent(queryType);
#endif
        }

        private GameObject EnsureClonedChildGameObject(
            GameObject srcObjGo,
            GameObject srcOwnerGo,
            GameObject dstOwnerGo,
            Dictionary<string, GameObject> cloneCache,
            List<PendingPlayableRebind> pendingRebinds)
        {
            if (IsNullOrDestroyed(srcObjGo) || IsNullOrDestroyed(dstOwnerGo))
                return null;

            if (!IsNullOrDestroyed(srcOwnerGo) && srcObjGo.GetInstanceID() == srcOwnerGo.GetInstanceID())
                return null;

            if (srcObjGo.transform.IsChildOf(dstOwnerGo.transform))
                return srcObjGo;

            string cacheKey = dstOwnerGo.GetInstanceID().ToString() + ":" + srcObjGo.GetInstanceID().ToString();
            if (cloneCache != null && cloneCache.TryGetValue(cacheKey, out GameObject cachedGo) && !IsNullOrDestroyed(cachedGo))
                return cachedGo;

            GameObject clonedGo;
#if IL2CPP
            UnityEngine.Object rawInst = UnityEngine.Object.Instantiate((UnityEngine.Object)srcObjGo);
            if (rawInst == null) return null;
            try { clonedGo = rawInst.TryCast<GameObject>(); } catch { clonedGo = null; }
            if (clonedGo == null)
            {
                try { clonedGo = new GameObject(rawInst.Pointer); } catch { clonedGo = null; }
            }
#else
            clonedGo = UnityEngine.Object.Instantiate(srcObjGo) as GameObject;
#endif
            if (IsNullOrDestroyed(clonedGo))
                return null;

            Transform targetParent = dstOwnerGo.transform;
            if (!IsNullOrDestroyed(srcOwnerGo) && srcObjGo.transform.parent != null)
            {
                string parentRel = ComputeRelativeTransformPath(srcOwnerGo.transform, srcObjGo.transform.parent);
                if (!string.IsNullOrEmpty(parentRel))
                {
                    Transform existingParent = dstOwnerGo.transform.Find(parentRel);
                    if (existingParent != null) targetParent = existingParent;
                }
            }

            clonedGo.name = srcObjGo.name;
            clonedGo.transform.SetParent(targetParent, false);
            clonedGo.transform.localPosition = srcObjGo.transform.localPosition;
            clonedGo.transform.localRotation = srcObjGo.transform.localRotation;
            clonedGo.transform.localScale = srcObjGo.transform.localScale;
            clonedGo.SetActive(srcObjGo.activeSelf);

            if (cloneCache != null) cloneCache[cacheKey] = clonedGo;
            if (pendingRebinds != null)
            {
                pendingRebinds.Add(new PendingPlayableRebind
                {
                    ClonedGo = clonedGo,
                    SrcOwnerGo = srcOwnerGo,
                    DstOwnerGo = dstOwnerGo
                });
            }
            return clonedGo;
        }

        private GameObject MapTargetUnderOwner(
            GameObject boundGo,
            GameObject srcOwnerGo,
            GameObject dstOwnerGo,
            bool cloneIfMissing,
            Dictionary<string, GameObject> cloneCache,
            List<PendingPlayableRebind> pendingRebinds)
        {
            if (IsNullOrDestroyed(boundGo) || IsNullOrDestroyed(srcOwnerGo) || IsNullOrDestroyed(dstOwnerGo))
                return null;

            string dstIdPrefix = dstOwnerGo.GetInstanceID().ToString() + ":";
            if (cloneCache != null && cloneCache.TryGetValue(dstIdPrefix + boundGo.GetInstanceID().ToString(), out GameObject cached)
                && !IsNullOrDestroyed(cached))
            {
                return cached;
            }

            string relPath = ComputeRelativeTransformPath(srcOwnerGo.transform, boundGo.transform);
            if (relPath == null) return null;
            if (relPath.Length == 0) return dstOwnerGo;

            Transform foundT = dstOwnerGo.transform.Find(relPath);
            if (foundT != null) return foundT.gameObject;

            if (cloneCache != null)
            {
                Transform cur = boundGo.transform.parent;
                int srcRootId = srcOwnerGo.transform.GetInstanceID();
                while (cur != null && cur.GetInstanceID() != srcRootId)
                {
                    if (cloneCache.TryGetValue(dstIdPrefix + cur.gameObject.GetInstanceID().ToString(), out GameObject ancClone)
                        && !IsNullOrDestroyed(ancClone))
                    {
                        string subRel = ComputeRelativeTransformPath(cur, boundGo.transform);
                        if (!string.IsNullOrEmpty(subRel))
                        {
                            Transform subFound = ancClone.transform.Find(subRel);
                            if (subFound != null) return subFound.gameObject;
                        }
                    }
                    cur = cur.parent;
                }
            }

            if (!cloneIfMissing) return null;
            return EnsureClonedChildGameObject(boundGo, srcOwnerGo, dstOwnerGo, cloneCache, pendingRebinds);
        }

        private UnityEngine.Object RemapSceneObjectReference(
            UnityEngine.Object boundObj,
            GameObject srcOwnerGo,
            GameObject dstOwnerGo,
            bool cloneIfMissing,
            Dictionary<string, GameObject> cloneCache,
            List<PendingPlayableRebind> pendingRebinds)
        {
            if (IsNullOrDestroyed(boundObj)) return null;

            GameObject boundGo;
            Component boundComp;
#if IL2CPP
            boundGo = boundObj.TryCast<GameObject>();
            boundComp = null;
            if (boundGo == null)
            {
                boundComp = boundObj.TryCast<Component>();
                if (boundComp != null) boundGo = boundComp.gameObject;
            }
#else
            boundGo = boundObj as GameObject;
            boundComp = boundObj as Component;
            if (boundGo == null && boundComp != null) boundGo = boundComp.gameObject;
#endif
            if (IsNullOrDestroyed(boundGo)) return null;

            GameObject mappedGo = MapTargetUnderOwner(boundGo, srcOwnerGo, dstOwnerGo, cloneIfMissing, cloneCache, pendingRebinds);
            if (IsNullOrDestroyed(mappedGo)) return null;

            if (boundComp == null) return mappedGo;

#if IL2CPP
            Il2CppSystem.Type compIl2 = null;
            try { compIl2 = boundComp.GetIl2CppType(); } catch { compIl2 = null; }
            Component mappedComp = compIl2 != null ? mappedGo.GetComponent(compIl2) : null;
#else
            Component mappedComp = mappedGo.GetComponent(boundComp.GetType());
#endif
            return !IsNullOrDestroyed(mappedComp) ? (UnityEngine.Object)mappedComp : null;
        }

        private object CloneReferenceUnderOwner(
            object srcVal,
            object srcOwner,
            object dstOwner,
            Type targetMemberType,
            Dictionary<string, GameObject> cloneCache,
            List<PendingPlayableRebind> pendingRebinds)
        {
            if (IsNullOrDestroyed(srcVal)) return srcVal;

            GameObject srcObjGo;
            Component srcComp = null;
            bool targetIsGameObject = false;
            if (srcVal is GameObject goVal)
            {
                srcObjGo = goVal;
                targetIsGameObject = true;
            }
            else if (srcVal is Component compVal)
            {
                srcComp = compVal;
                srcObjGo = compVal.gameObject;
            }
            else
            {
                // ScriptableObjects, primitives, and non-scene assets can be shared directly.
                return srcVal;
            }

            GameObject dstOwnerGo = ExtractGameObject(dstOwner);
            GameObject srcOwnerGo = ExtractGameObject(srcOwner);
            GameObject clonedGo = EnsureClonedChildGameObject(srcObjGo, srcOwnerGo, dstOwnerGo, cloneCache, pendingRebinds);
            if (IsNullOrDestroyed(clonedGo))
                return srcVal;

            if (targetIsGameObject)
                return clonedGo;

            Type desiredType = (targetMemberType != null && typeof(Component).IsAssignableFrom(targetMemberType))
                ? targetMemberType
                : srcVal.GetType();
            object clonedComp = GetExistingComponentByType(clonedGo, desiredType, srcComp);
            return !IsNullOrDestroyed(clonedComp) ? clonedComp : srcVal;
        }

        private void RebindCinemachineCameras(
            GameObject clonedGo,
            GameObject srcOwnerGo,
            GameObject dstOwnerGo,
            Dictionary<string, GameObject> cloneCache,
            List<PendingPlayableRebind> pendingRebinds)
        {
            if (IsNullOrDestroyed(clonedGo) || IsNullOrDestroyed(srcOwnerGo) || IsNullOrDestroyed(dstOwnerGo))
                return;

            const BindingFlags pubInst = BindingFlags.Public | BindingFlags.Instance;
            Type vcamBaseType = ResolveTypeStatic("Cinemachine.CinemachineVirtualCameraBase");
            if (vcamBaseType != null)
            {
                PropertyInfo followProp = vcamBaseType.GetProperty("Follow", pubInst);
                PropertyInfo lookAtProp = vcamBaseType.GetProperty("LookAt", pubInst);

#if IL2CPP
                var il2cppVcamType = Il2CppInterop.Runtime.Il2CppType.From(vcamBaseType, false);
                var rawVcams = il2cppVcamType != null ? clonedGo.GetComponentsInChildren(il2cppVcamType, true) : null;
#else
                Component[] rawVcams = clonedGo.GetComponentsInChildren(vcamBaseType, true);
#endif
                if (rawVcams != null)
                {
                    for (int v = 0; v < rawVcams.Length; v++)
                    {
                        Component vcamComp = rawVcams[v];
                        if (IsNullOrDestroyed(vcamComp)) continue;
                        object vcamObj = WrapUnityObjectToType(vcamComp, vcamBaseType);
                        for (int pIdx = 0; pIdx < 2; pIdx++)
                        {
                            PropertyInfo prop = pIdx == 0 ? followProp : lookAtProp;
                            if (prop == null || !prop.CanRead || !prop.CanWrite) continue;
                            Transform curT = null;
                            try { curT = prop.GetValue(vcamObj, null) as Transform; } catch { curT = null; }
                            if (IsNullOrDestroyed(curT)) continue;

                            GameObject mappedGo = MapTargetUnderOwner(curT.gameObject, srcOwnerGo, dstOwnerGo, true, cloneCache, pendingRebinds);
                            if (!IsNullOrDestroyed(mappedGo) && mappedGo.transform.GetInstanceID() != curT.GetInstanceID())
                            {
                                try { prop.SetValue(vcamObj, mappedGo.transform, null); } catch { }
                            }
                        }
                    }
                }
            }

            Type tgType = ResolveTypeStatic("Cinemachine.CinemachineTargetGroup");
            if (tgType != null)
            {
                const BindingFlags allInst = BindingFlags.Public | BindingFlags.NonPublic | BindingFlags.Instance;
                PropertyInfo targetsProp = tgType.GetProperty("m_Targets", allInst);
                FieldInfo targetsField = tgType.GetField("m_Targets", allInst);
#if IL2CPP
                var il2cppTgType = Il2CppInterop.Runtime.Il2CppType.From(tgType, false);
                var rawTgs = il2cppTgType != null ? clonedGo.GetComponentsInChildren(il2cppTgType, true) : null;
#else
                Component[] rawTgs = clonedGo.GetComponentsInChildren(tgType, true);
#endif
                if (rawTgs != null)
                {
                    for (int t = 0; t < rawTgs.Length; t++)
                    {
                        Component tgComp = rawTgs[t];
                        if (IsNullOrDestroyed(tgComp)) continue;
                        object tgObj = WrapUnityObjectToType(tgComp, tgType);
                        object rawTargets = null;
                        try
                        {
                            rawTargets = targetsProp != null ? targetsProp.GetValue(tgObj, null) : targetsField?.GetValue(tgObj);
                        }
                        catch { rawTargets = null; }
                        if (rawTargets == null) continue;

                        if (rawTargets is Array managedArr)
                        {
                            for (int idx = 0; idx < managedArr.Length; idx++)
                            {
                                object elem = managedArr.GetValue(idx);
                                if (elem == null) continue;
                                if (!TryReadMemberValue(elem, "target", out object rawT) || !(rawT is Transform elemT) || IsNullOrDestroyed(elemT))
                                    continue;
                                GameObject mappedGo = MapTargetUnderOwner(elemT.gameObject, srcOwnerGo, dstOwnerGo, false, cloneCache, pendingRebinds);
                                if (!IsNullOrDestroyed(mappedGo) && mappedGo.transform.GetInstanceID() != elemT.GetInstanceID())
                                {
                                    if (TryWriteMemberValue(elem, "target", mappedGo.transform) && elem.GetType().IsValueType)
                                    {
                                        try { managedArr.SetValue(elem, idx); } catch { }
                                    }
                                }
                            }
                        }
                        else
                        {
                            PropertyInfo lenProp = rawTargets.GetType().GetProperty("Length", pubInst);
                            PropertyInfo itemProp = rawTargets.GetType().GetProperty("Item", pubInst);
                            if (lenProp != null && itemProp != null && itemProp.GetIndexParameters().Length == 1)
                            {
                                int len = 0;
                                try { len = Convert.ToInt32(lenProp.GetValue(rawTargets, null)); } catch { len = 0; }
                                for (int idx = 0; idx < len; idx++)
                                {
                                    object elem = null;
                                    try { elem = itemProp.GetValue(rawTargets, new object[] { idx }); } catch { elem = null; }
                                    if (elem == null) continue;
                                    if (!TryReadMemberValue(elem, "target", out object rawT) || !(rawT is Transform elemT) || IsNullOrDestroyed(elemT))
                                        continue;
                                    GameObject mappedGo = MapTargetUnderOwner(elemT.gameObject, srcOwnerGo, dstOwnerGo, false, cloneCache, pendingRebinds);
                                    if (!IsNullOrDestroyed(mappedGo) && mappedGo.transform.GetInstanceID() != elemT.GetInstanceID())
                                    {
                                        TryWriteMemberValue(elem, "target", mappedGo.transform);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        private void RebindPlayableDirectors(
            GameObject clonedGo,
            GameObject srcOwnerGo,
            GameObject dstOwnerGo,
            Dictionary<string, GameObject> cloneCache,
            List<PendingPlayableRebind> pendingRebinds)
        {
            if (IsNullOrDestroyed(clonedGo) || IsNullOrDestroyed(srcOwnerGo) || IsNullOrDestroyed(dstOwnerGo))
                return;

            Type directorType = ResolveTypeStatic("UnityEngine.Playables.PlayableDirector");
            if (directorType == null) return;

            const BindingFlags flags = BindingFlags.Public | BindingFlags.Instance;
            PropertyInfo playableAssetProp = directorType.GetProperty("playableAsset", flags);
            MethodInfo getBinding = directorType.GetMethod("GetGenericBinding", flags);
            MethodInfo setBinding = directorType.GetMethod("SetGenericBinding", flags);
            if (playableAssetProp == null || getBinding == null || setBinding == null)
                return;

            MethodInfo getReference = FindDirectorTwoArgMethod(directorType, "GetReferenceValue");
            MethodInfo setReference = FindDirectorTwoArgMethod(directorType, "SetReferenceValue");

#if IL2CPP
            var il2cppDirType = Il2CppInterop.Runtime.Il2CppType.From(directorType, false);
            if (il2cppDirType == null) return;
            var rawDirectors = clonedGo.GetComponentsInChildren(il2cppDirType, true);
#else
            Component[] rawDirectors = clonedGo.GetComponentsInChildren(directorType, true);
#endif
            if (rawDirectors == null || rawDirectors.Length == 0)
                return;

            for (int d = 0; d < rawDirectors.Length; d++)
            {
                Component dirComp = rawDirectors[d];
                if (IsNullOrDestroyed(dirComp)) continue;

                object dirObj = WrapUnityObjectToType(dirComp, directorType);
                object rawAsset = null;
                try { rawAsset = playableAssetProp.GetValue(dirObj, null); } catch { rawAsset = null; }

#if IL2CPP
                UnityEngine.Playables.PlayableAsset playableAsset = (rawAsset as UnityEngine.Object)?.TryCast<UnityEngine.Playables.PlayableAsset>();
#else
                UnityEngine.Playables.PlayableAsset playableAsset = rawAsset as UnityEngine.Playables.PlayableAsset;
#endif
                if (IsNullOrDestroyed(playableAsset)) continue;

                var bindingKeys = new List<UnityEngine.Object>();
                var outputs = playableAsset.outputs;
                if (outputs != null)
                {
#if IL2CPP
                    var genEnum = outputs.GetEnumerator();
                    try
                    {
                        var baseEnum = (genEnum as Il2CppInterop.Runtime.InteropTypes.Il2CppObjectBase)?.TryCast<Il2CppSystem.Collections.IEnumerator>();
                        while (baseEnum != null && baseEnum.MoveNext())
                        {
                            UnityEngine.Playables.PlayableBinding b = genEnum.Current;
                            if (b.sourceObject != null) bindingKeys.Add(b.sourceObject);
                        }
                    }
                    finally
                    {
                        try { (genEnum as Il2CppInterop.Runtime.InteropTypes.Il2CppObjectBase)?.TryCast<Il2CppSystem.IDisposable>()?.Dispose(); } catch { }
                    }
#else
                    foreach (UnityEngine.Playables.PlayableBinding b in outputs)
                    {
                        if (b.sourceObject != null) bindingKeys.Add(b.sourceObject);
                    }
#endif
                }

                for (int k = 0; k < bindingKeys.Count; k++)
                {
                    UnityEngine.Object sourceKey = bindingKeys[k];
                    UnityEngine.Object boundObj = null;
                    try { boundObj = getBinding.Invoke(dirObj, new object[] { sourceKey }) as UnityEngine.Object; }
                    catch { boundObj = null; }
                    UnityEngine.Object mappedObj = RemapSceneObjectReference(
                        boundObj, srcOwnerGo, dstOwnerGo, false, cloneCache, pendingRebinds);
                    if (!IsNullOrDestroyed(mappedObj))
                    {
                        try { setBinding.Invoke(dirObj, new object[] { sourceKey, mappedObj }); } catch { }
                    }
                }

                if (getReference != null && setReference != null)
                {
                    RebindPlayableDirectorExposedReferences(
                        dirObj,
                        playableAsset,
                        getReference,
                        setReference,
                        srcOwnerGo,
                        dstOwnerGo,
                        cloneCache,
                        pendingRebinds);
                }
            }
        }

        private static MethodInfo FindDirectorTwoArgMethod(Type directorType, string methodName)
        {
            const BindingFlags flags = BindingFlags.Public | BindingFlags.Instance;
            MethodInfo[] methods = directorType.GetMethods(flags);
            for (int i = 0; i < methods.Length; i++)
            {
                if (methods[i].Name != methodName) continue;
                ParameterInfo[] pars = methods[i].GetParameters();
                if (pars.Length == 2 && !pars[0].ParameterType.IsByRef)
                    return methods[i];
            }
            return null;
        }

        private void RebindPlayableDirectorExposedReferences(
            object dirObj,
            UnityEngine.Playables.PlayableAsset playableAsset,
            MethodInfo getReference,
            MethodInfo setReference,
            GameObject srcOwnerGo,
            GameObject dstOwnerGo,
            Dictionary<string, GameObject> cloneCache,
            List<PendingPlayableRebind> pendingRebinds)
        {
            Type timelineAssetType = ResolveTypeStatic("UnityEngine.Timeline.TimelineAsset");
            Type trackAssetType = ResolveTypeStatic("UnityEngine.Timeline.TrackAsset");
            Type clipType = ResolveTypeStatic("UnityEngine.Timeline.TimelineClip");
            if (timelineAssetType == null || trackAssetType == null || clipType == null)
                return;

            object tlObj = WrapUnityObjectToType(playableAsset, timelineAssetType);
            if (tlObj == null || !timelineAssetType.IsInstanceOfType(tlObj))
                return;

            const BindingFlags pubInst = BindingFlags.Public | BindingFlags.Instance;
            const BindingFlags allInst = BindingFlags.Public | BindingFlags.NonPublic | BindingFlags.Instance;

            PropertyInfo flattenedProp = timelineAssetType.GetProperty("flattenedTracks", pubInst);
            MethodInfo getOutputTracksMethod = timelineAssetType.GetMethod("GetOutputTracks", pubInst);
            MethodInfo getClipsMethod = trackAssetType.GetMethod("GetClips", pubInst);
            PropertyInfo clipAssetProp = clipType.GetProperty("asset", pubInst);
            if (getClipsMethod == null || clipAssetProp == null)
                return;

            object tracksSeq = null;
            Type tracksDeclaredType = null;
            if (flattenedProp != null && flattenedProp.CanRead)
            {
                try { tracksSeq = flattenedProp.GetValue(tlObj, null); } catch { tracksSeq = null; }
            }
            if (tracksSeq == null && getOutputTracksMethod != null && getOutputTracksMethod.GetParameters().Length == 0)
            {
                try
                {
                    tracksSeq = getOutputTracksMethod.Invoke(tlObj, null);
                    tracksDeclaredType = getOutputTracksMethod.ReturnType;
                }
                catch { tracksSeq = null; }
            }
            if (tracksSeq == null) return;

            List<object> tracks = CollectSequenceItems(tracksSeq, tracksDeclaredType);
            for (int t = 0; t < tracks.Count; t++)
            {
                object track = tracks[t];
                if (track == null) continue;
                object clipsSeq = null;
                try { clipsSeq = getClipsMethod.Invoke(track, null); } catch { clipsSeq = null; }
                if (clipsSeq == null) continue;

                List<object> clips = CollectSequenceItems(clipsSeq, getClipsMethod.ReturnType);
                for (int c = 0; c < clips.Count; c++)
                {
                    object clip = clips[c];
                    if (clip == null) continue;
                    object rawClipAsset = null;
                    try { rawClipAsset = clipAssetProp.GetValue(clip, null); } catch { rawClipAsset = null; }
                    if (!(rawClipAsset is UnityEngine.Object clipUnityAsset) || IsNullOrDestroyed(clipUnityAsset))
                        continue;

                    object concreteAsset = WrapUnityObjectToType(clipUnityAsset, null);
                    if (concreteAsset == null) continue;
                    Type assetType = concreteAsset.GetType();

                    var expRefs = new List<object>();
                    foreach (PropertyInfo prop in assetType.GetProperties(allInst))
                    {
                        if (!prop.CanRead || prop.GetIndexParameters().Length != 0) continue;
                        if (!IsExposedReferenceType(prop.PropertyType)) continue;
                        try
                        {
                            object erVal = prop.GetValue(concreteAsset, null);
                            if (erVal != null) expRefs.Add(erVal);
                        }
                        catch { }
                    }
                    foreach (FieldInfo field in assetType.GetFields(allInst))
                    {
                        if (!IsExposedReferenceType(field.FieldType)) continue;
                        try
                        {
                            object erVal = field.GetValue(concreteAsset);
                            if (erVal != null) expRefs.Add(erVal);
                        }
                        catch { }
                    }

                    for (int e = 0; e < expRefs.Count; e++)
                    {
                        if (!TryReadMemberValue(expRefs[e], "exposedName", out object rawPropName))
                            continue;
                        if (!(rawPropName is PropertyName propName) || PropertyName.IsNullOrEmpty(propName))
                            continue;

                        object[] getArgs = new object[] { propName, false };
                        UnityEngine.Object resolvedObj = null;
                        try
                        {
                            resolvedObj = getReference.Invoke(dirObj, getArgs) as UnityEngine.Object;
                        }
                        catch { resolvedObj = null; }
                        bool isValid = getArgs[1] is bool validFlag && validFlag;
                        if (!isValid || IsNullOrDestroyed(resolvedObj))
                            continue;

                        UnityEngine.Object mappedObj = RemapSceneObjectReference(
                            resolvedObj, srcOwnerGo, dstOwnerGo, true, cloneCache, pendingRebinds);
                        if (!IsNullOrDestroyed(mappedObj) && mappedObj.GetInstanceID() != resolvedObj.GetInstanceID())
                        {
                            try { setReference.Invoke(dirObj, new object[] { propName, mappedObj }); } catch { }
                        }
                    }
                }
            }
        }

        private static bool IsExposedReferenceType(Type t)
        {
            return t != null
                && t.IsGenericType
                && t.GetGenericTypeDefinition().FullName == "UnityEngine.ExposedReference`1";
        }

        private int ApplyHookEnsureComponentWrite(HookFieldWriteSpec spec)
        {
            if (!ResolveFieldPathOwners(spec.TargetType, spec.FieldName, false, out List<object> owners, out string leafName)
                && spec.IncludeInactive)
            {
                ResolveFieldPathOwners(spec.TargetType, spec.FieldName, true, out owners, out leafName);
            }
            if (owners == null || owners.Count == 0 || string.IsNullOrEmpty(leafName))
                return 0;

            int updated = 0;
            for (int i = 0; i < owners.Count; i++)
            {
                object owner = owners[i];
                if (owner == null) continue;
                MemberBinding leaf = GetMemberCached(owner.GetType(), leafName);
                if (leaf == null || !leaf.CanWrite) continue;
                if (spec.OnlyIfNull && leaf.CanRead)
                {
                    object existing = leaf.Read(owner);
                    if (!IsNullOrDestroyed(existing)) continue;
                }

                GameObject go = ExtractGameObject(owner);
                if (go == null) continue;

                object wrapped = EnsureComponentOnGameObject(go, spec.EnsureComponentType);
                if (IsNullOrDestroyed(wrapped)) continue;

                if (spec.InitNullLists)
                {
                    InitializeNullListMembers(wrapped, spec.EnsureComponentType);
                }

                leaf.Write(owner, wrapped);
                updated++;
            }
            return updated;
        }

        private static object EnsureComponentOnGameObject(UnityEngine.GameObject go, Type compType)
        {
            if (go == null || compType == null) return null;
#if IL2CPP
            var il2cppType = Il2CppInterop.Runtime.Il2CppType.From(compType, false);
            if (il2cppType == null) return null;
            UnityEngine.Component comp = go.GetComponent(il2cppType);
            if (comp == null) comp = go.AddComponent(il2cppType);
            if (comp == null) return null;
            if (compType == typeof(UnityEngine.Component)) return comp;
            try
            {
                return Activator.CreateInstance(compType, new object[] { comp.Pointer }) ?? (object)comp;
            }
            catch
            {
                return comp;
            }
#else
            UnityEngine.Component comp = go.GetComponent(compType) ?? go.AddComponent(compType);
            return comp;
#endif
        }

        private static void InitializeNullListMembers(object target, Type targetType)
        {
            if (target == null || targetType == null) return;
            const BindingFlags flags = BindingFlags.DeclaredOnly | BindingFlags.Public | BindingFlags.NonPublic | BindingFlags.Instance;

            foreach (PropertyInfo prop in targetType.GetProperties(flags))
            {
                if (!prop.CanRead || !prop.CanWrite || prop.GetIndexParameters().Length != 0) continue;
                Type pt = prop.PropertyType;
                if (!IsGenericListType(pt)) continue;
                try
                {
                    if (prop.GetValue(target, null) == null)
                    {
                        object instance = Activator.CreateInstance(pt);
                        if (instance != null) prop.SetValue(target, instance, null);
                    }
                }
                catch { }
            }

            foreach (FieldInfo field in targetType.GetFields(flags))
            {
                if (field.IsInitOnly) continue;
                Type ft = field.FieldType;
                if (!IsGenericListType(ft)) continue;
                try
                {
                    if (field.GetValue(target) == null)
                    {
                        object instance = Activator.CreateInstance(ft);
                        if (instance != null) field.SetValue(target, instance);
                    }
                }
                catch { }
            }
        }

        private static bool IsGenericListType(Type t)
        {
            if (t == null || !t.IsGenericType) return false;
            string defName = t.GetGenericTypeDefinition()?.FullName;
            return defName == "System.Collections.Generic.List`1"
                || defName == "Il2CppSystem.Collections.Generic.List`1";
        }

        private int RestoreMethodHookWrites(MethodHookPin pin)
        {
            if (pin?.Writes == null) return 0;
            int totalRestored = 0;
            for (int i = pin.Writes.Count - 1; i >= 0; i--)
            {
                HookFieldWriteSpec spec = pin.Writes[i];
                if (!spec.HasRestoreValue) continue;
                try
                {
                    ApplySetFieldPath(spec.TargetType, spec.FieldName, spec.RestoreValue, false, out int restored, out _);
                    if (restored == 0 && spec.IncludeInactive)
                    {
                        ApplySetFieldPath(spec.TargetType, spec.FieldName, spec.RestoreValue, true, out restored, out _);
                    }
                    totalRestored += restored;
                }
                catch (Exception ex)
                {
                    _log($"[NekodUnityBridge] hook '{pin.Id}' restore error on {spec.TargetType?.FullName}.{spec.FieldName}: {ex.Message}");
                }
            }
            return totalRestored;
        }

        private string HandleClearPins()
        {
            int count;
            List<TargetLockPin> locksToRelease;
            List<SpinBotPin> spinsToRestore;
            List<MethodHookPin> hooksToRestore;
            Harmony harmonyToUnpatch;

            lock (_hookDispatchLock)
            {
                hooksToRestore = _methodHooks.Count > 0 ? new List<MethodHookPin>(_methodHooks.Values) : null;
                _methodHooks.Clear();
                if (ReferenceEquals(_activeHookServer, this))
                {
                    _prefixDispatch.Clear();
                    _postfixDispatch.Clear();
                }
                harmonyToUnpatch = _harmony;
            }

            if (harmonyToUnpatch != null)
            {
                try { harmonyToUnpatch.UnpatchSelf(); } catch { }
            }

            lock (_pins)
            {
                count = _pins.Count + _targetLocks.Count + _damageAuras.Count + _spinBots.Count
                    + (hooksToRestore != null ? hooksToRestore.Count : 0);
                locksToRelease = _targetLocks.Count > 0 ? new List<TargetLockPin>(_targetLocks.Values) : null;
                spinsToRestore = _spinBots.Count > 0 ? new List<SpinBotPin>(_spinBots.Values) : null;
                _pins.Clear();
                _targetLocks.Clear();
                _damageAuras.Clear();
                _spinBots.Clear();
                Volatile.Write(ref _activePinCount, 0);
                _nextDueTickMs = 0;
            }

            ClearSceneCaches();

            if (hooksToRestore != null)
            {
                foreach (MethodHookPin hookPin in hooksToRestore)
                {
                    try { RestoreMethodHookWrites(hookPin); } catch { }
                }
            }

            if (spinsToRestore != null)
            {
                foreach (SpinBotPin spinPin in spinsToRestore)
                {
                    try { RestoreSpinNode(spinPin); } catch { }
                }
            }

            if (locksToRelease != null)
            {
                foreach (TargetLockPin lockPin in locksToRelease)
                {
                    try { InvokeCameraUnlock(lockPin); } catch { }
                }
            }

            if (_materialBackups.Count > 0)
            {
                foreach (List<MaterialBackupEntry> entries in _materialBackups.Values)
                {
                    try { RestoreMaterialBackupList(entries); } catch { }
                }
                _materialBackups.Clear();
            }

            return OkJson(new Dictionary<string, object> { { "cleared", count } });
        }

        private string HandleInvokeMethod(Dictionary<string, object> p)
        {
            string className = GetString(p, "class_name");
            string methodName = GetString(p, "method_name");
            if (string.IsNullOrEmpty(className) || string.IsNullOrEmpty(methodName))
                return ErrorJson("invoke_method requires class_name and method_name");

            Type type = ResolveType(className);
            if (type == null) return ErrorJson("type not found: " + className);

            const BindingFlags flags = BindingFlags.Public | BindingFlags.NonPublic | BindingFlags.Static | BindingFlags.Instance | BindingFlags.FlattenHierarchy;
            MethodInfo mi = null;
            foreach (MethodInfo candidate in type.GetMethods(flags))
            {
                if (candidate.Name == methodName && candidate.GetParameters().Length == 0)
                {
                    mi = candidate;
                    break;
                }
            }
            if (mi == null)
                return ErrorJson($"parameterless method '{methodName}' not found on '{type.FullName}'");

            try
            {
                if (mi.IsStatic)
                {
                    object ret = mi.Invoke(null, null);
                    return OkJson(new Dictionary<string, object>
                    {
                        { "type", type.FullName },
                        { "method", methodName },
                        { "invoked", 1 },
                        { "result", ret?.ToString() }
                    });
                }

                List<object> instances = FindAllInstances(type, false);
                if (instances.Count == 0)
                    instances = FindAllInstances(type, true);
                if (instances.Count == 0)
                    return ErrorJson($"no live instance of '{type.FullName}' found in active scene");

                foreach (object inst in instances)
                {
                    mi.Invoke(inst, null);
                }
                return OkJson(new Dictionary<string, object>
                {
                    { "type", type.FullName },
                    { "method", methodName },
                    { "invoked", instances.Count }
                });
            }
            catch (TargetInvocationException tie)
            {
                return ErrorJson("method invocation failed: " + (tie.InnerException?.Message ?? tie.Message));
            }
        }

        private Type ResolveType(string typeName)
        {
            if (string.IsNullOrEmpty(typeName)) return null;

            lock (_typeCache)
            {
                if (_typeCache.TryGetValue(typeName, out Type cached)) return cached;
            }

            Type resolved = Type.GetType(typeName, false);
            Assembly[] assemblies = AppDomain.CurrentDomain.GetAssemblies();

            if (resolved == null)
            {
                foreach (Assembly asm in assemblies)
                {
                    try
                    {
                        Type exact = asm.GetType(typeName, false);
                        if (exact != null)
                        {
                            resolved = exact;
                            break;
                        }
                    }
                    catch { }
                }
            }

            if (resolved == null)
            {
                foreach (Assembly asm in assemblies)
                {
                    Type[] types;
                    try { types = asm.GetTypes(); } catch { continue; }
                    foreach (Type t in types)
                    {
                        if (t != null && (t.FullName == typeName || t.Name == typeName))
                        {
                            resolved = t;
                            break;
                        }
                    }
                    if (resolved != null) break;
                }
            }

            if (resolved != null)
            {
                lock (_typeCache)
                {
                    _typeCache[typeName] = resolved;
                }
            }
            return resolved;
        }

        private static bool IsNullOrDestroyed(object obj)
        {
            if (obj == null) return true;
            if (obj is UnityEngine.Object uo) return uo == null;
            return false;
        }

        private static bool SplitFieldPath(string fieldPath, out string[] segments, out string error)
        {
            if (string.IsNullOrEmpty(fieldPath))
            {
                segments = null;
                error = "field_name cannot be empty";
                return false;
            }
            segments = fieldPath.Split('.');
            foreach (string seg in segments)
            {
                if (string.IsNullOrEmpty(seg))
                {
                    error = "invalid dotted field_name path: " + fieldPath;
                    return false;
                }
            }
            error = null;
            return true;
        }

        private MemberBinding GetMemberCached(Type t, string name)
        {
            if (t == null || string.IsNullOrEmpty(name)) return null;
            var key = new MemberKey(t, name);
            if (_memberCache.TryGetValue(key, out MemberBinding cached)) return cached;

            FieldInfo fi = null;
            PropertyInfo pi = null;
            for (Type cur = t; cur != null && cur != typeof(object); cur = cur.BaseType)
            {
                fi = cur.GetField(name, MemberFlags);
                if (fi != null) break;
                pi = cur.GetProperty(name, MemberFlags);
                if (pi != null && pi.GetIndexParameters().Length == 0) break;
                pi = null;
            }

            MemberBinding binding = null;
            if (fi != null)
            {
                binding = new MemberBinding
                {
                    Field = fi,
                    MemberType = fi.FieldType,
                    CanRead = true,
                    CanWrite = true,
                    IsStaticRead = fi.IsStatic,
                    IsStaticWrite = fi.IsStatic
                };
            }
            else if (pi != null)
            {
                MethodInfo getter = pi.GetGetMethod(true);
                MethodInfo setter = pi.GetSetMethod(true);
                MethodInfo readAcc = getter ?? setter;
                MethodInfo writeAcc = setter ?? getter;
                binding = new MemberBinding
                {
                    Property = pi,
                    MemberType = pi.PropertyType,
                    CanRead = getter != null,
                    CanWrite = setter != null,
                    IsStaticRead = readAcc != null && readAcc.IsStatic,
                    IsStaticWrite = writeAcc != null && writeAcc.IsStatic
                };
            }

            _memberCache[key] = binding;
            return binding;
        }

        private bool PrevalidateMemberPath(Type rootType, string[] segments, object rawValue, bool writing, out string error, bool checkValue = true)
        {
            MemberBinding member = GetMemberCached(rootType, segments[0]);
            if (member == null)
            {
                error = $"member '{segments[0]}' not found on '{rootType.FullName}'";
                return false;
            }

            Type curType = member.MemberType;
            for (int i = 1; i < segments.Length; i++)
            {
                if (!member.CanRead)
                {
                    error = $"member '{segments[i - 1]}' is write-only";
                    return false;
                }
                if (curType == null)
                {
                    error = null;
                    return true;
                }
                member = GetMemberCached(curType, segments[i]);
                if (member == null)
                {
                    if (!curType.IsInterface && !curType.IsAbstract && curType != typeof(object))
                    {
                        error = $"member '{segments[i]}' not found on '{curType.FullName}'";
                        return false;
                    }
                    error = null;
                    return true;
                }
                curType = member.MemberType;
            }

            if (writing)
            {
                string leafName = segments[segments.Length - 1];
                if (!member.CanWrite)
                {
                    error = $"member '{leafName}' on '{rootType.FullName}' is read-only";
                    return false;
                }
                if (checkValue)
                {
                    try
                    {
                        ConvertToType(rawValue, curType);
                    }
                    catch (Exception ex)
                    {
                        error = $"invalid value for '{leafName}': {ex.Message}";
                        return false;
                    }
                }
            }
            else if (!member.CanRead)
            {
                error = $"member '{segments[segments.Length - 1]}' is write-only";
                return false;
            }

            error = null;
            return true;
        }

        private bool TryGetFieldPath(
            Type rootType,
            string fieldPath,
            bool includeInactive,
            out object value,
            out bool missingTarget,
            out string error)
        {
            value = null;
            missingTarget = false;
            if (!SplitFieldPath(fieldPath, out string[] segments, out error))
                return false;

            MemberBinding root = GetMemberCached(rootType, segments[0]);
            if (root == null)
            {
                error = $"member '{segments[0]}' not found on '{rootType.FullName}'";
                return false;
            }
            if (!root.CanRead)
            {
                error = $"member '{segments[0]}' on '{rootType.FullName}' is write-only";
                return false;
            }

            var candidates = new List<object>();
            if (root.IsStaticRead)
            {
                object firstVal = root.Read(null);
                if (segments.Length == 1)
                {
                    value = firstVal;
                    return true;
                }
                if (!IsNullOrDestroyed(firstVal))
                    candidates.Add(firstVal);
            }
            else
            {
                List<object> roots = FindAllInstances(rootType, includeInactive);
                if (roots.Count == 0)
                {
                    missingTarget = true;
                    error = $"no live instance of '{rootType.FullName}' found in active scene";
                    return false;
                }
                if (segments.Length == 1)
                {
                    value = root.Read(roots[0]);
                    return true;
                }
                foreach (object r in roots)
                {
                    object child = root.Read(r);
                    if (!IsNullOrDestroyed(child))
                        candidates.Add(child);
                }
            }

            if (candidates.Count == 0)
            {
                missingTarget = true;
                error = $"no live target for '{rootType.FullName}.{fieldPath}' in active scene";
                return false;
            }

            foreach (object startObj in candidates)
            {
                object cur = startObj;
                bool brokenNull = false;
                for (int s = 1; s < segments.Length; s++)
                {
                    if (IsNullOrDestroyed(cur))
                    {
                        brokenNull = true;
                        break;
                    }
                    MemberBinding m = GetMemberCached(cur.GetType(), segments[s]);
                    if (m == null)
                    {
                        error = $"member '{segments[s]}' not found on '{cur.GetType().FullName}'";
                        return false;
                    }
                    if (!m.CanRead)
                    {
                        error = $"member '{segments[s]}' on '{cur.GetType().FullName}' is write-only";
                        return false;
                    }
                    cur = m.Read(cur);
                }
                if (!brokenNull)
                {
                    value = cur;
                    return true;
                }
            }

            missingTarget = true;
            error = $"no live target for '{rootType.FullName}.{fieldPath}' in active scene";
            return false;
        }

        private bool ApplySetFieldPath(
            Type rootType,
            string fieldPath,
            object rawValue,
            bool includeInactive,
            out int updated,
            out string error)
        {
            updated = 0;
            if (!SplitFieldPath(fieldPath, out string[] segments, out error))
                return false;

            MemberBinding root = GetMemberCached(rootType, segments[0]);
            if (root == null)
            {
                error = $"member '{segments[0]}' not found on '{rootType.FullName}'";
                return false;
            }

            if (segments.Length == 1)
            {
                if (!root.CanWrite)
                {
                    error = $"member '{segments[0]}' on '{rootType.FullName}' is read-only";
                    return false;
                }
                object converted;
                try
                {
                    converted = ConvertToType(rawValue, root.MemberType);
                }
                catch (Exception ex)
                {
                    error = $"invalid value for '{segments[0]}': {ex.Message}";
                    return false;
                }

                if (root.IsStaticWrite)
                {
                    root.Write(null, converted);
                    updated = 1;
                    error = null;
                    return true;
                }

                List<object> instances = FindAllInstances(rootType, includeInactive);
                foreach (object inst in instances)
                {
                    root.Write(inst, converted);
                    updated++;
                }
                error = null;
                return true;
            }

            if (!root.CanRead)
            {
                error = $"member '{segments[0]}' on '{rootType.FullName}' is write-only";
                return false;
            }

            var currentTargets = new List<object>();
            _setPathOwners.Clear();
            if (root.IsStaticRead)
            {
                object child = root.Read(null);
                if (!IsNullOrDestroyed(child))
                {
                    currentTargets.Add(child);
                    _setPathOwners.Add(null);
                }
            }
            else
            {
                List<object> roots = FindAllInstances(rootType, includeInactive);
                foreach (object r in roots)
                {
                    object child = root.Read(r);
                    if (!IsNullOrDestroyed(child))
                    {
                        currentTargets.Add(child);
                        _setPathOwners.Add(r);
                    }
                }
            }

            // Nested writes recurse because reflection hands back STRUCTS as copies: the
            // mutated copy must be written back through its owning member, and so on up the
            // chain (e.g. `vcam.m_Lens.Dutch = x` needs `vcam.m_Lens = mutatedLens`).
            // `currentTargets` holds the value of segments[0], paired with the owner that
            // produced it so a value-typed root can also be written back.
            if (currentTargets.Count == 0)
            {
                error = null;
                return true;
            }

            for (int t = 0; t < currentTargets.Count; t++)
            {
                object target = currentTargets[t];
                object owner = _setPathOwners[t];
                if (!SetNestedPath(target, segments, 1, rawValue, out string nestedError))
                {
                    error = nestedError;
                    return false;
                }
                if (root.MemberType.IsValueType && owner != null)
                {
                    try { root.Write(owner, target); } catch { }
                }
                updated++;
            }

            error = null;
            return true;
        }

        private readonly List<object> _setPathOwners = new List<object>();

        /// <summary>
        /// Walks <paramref name="segments"/> from <paramref name="index"/> into
        /// <paramref name="current"/> and writes the leaf, writing mutated value-type copies
        /// back through their owning members.
        /// </summary>
        private bool SetNestedPath(object current, string[] segments, int index, object rawValue, out string error)
        {
            error = null;
            string name = segments[index];
            MemberBinding m = GetMemberCached(current.GetType(), name);
            if (m == null)
            {
                error = $"member '{name}' not found on '{current.GetType().FullName}'";
                return false;
            }

            if (index == segments.Length - 1)
            {
                if (!m.CanWrite)
                {
                    error = $"member '{name}' on '{current.GetType().FullName}' is read-only";
                    return false;
                }
                object converted;
                try
                {
                    converted = ConvertToType(rawValue, m.MemberType);
                }
                catch (Exception ex)
                {
                    error = $"invalid value for '{name}': {ex.Message}";
                    return false;
                }
                m.Write(current, converted);
                return true;
            }

            if (!m.CanRead)
            {
                error = $"member '{name}' on '{current.GetType().FullName}' is write-only";
                return false;
            }

            object child = m.Read(current);
            if (child == null) return true;

            if (!SetNestedPath(child, segments, index + 1, rawValue, out error))
                return false;

            if (m.MemberType.IsValueType)
            {
                try { m.Write(current, child); } catch { }
            }
            return true;
        }

        private void ApplyPinnedField(PinnedField pin, long nowMs, ref bool didSceneScanThisFrame)
        {
            if (pin.Segments == null || pin.Segments.Length == 0 || pin.RootMember == null) return;
            MemberBinding root = pin.RootMember;

            if (pin.Segments.Length == 1)
            {
                if (!root.CanWrite) return;
                object val = pin.HasSingleConvertedValue
                    ? pin.SingleConvertedValue
                    : ConvertToType(pin.Value, root.MemberType);

                if (root.IsStaticWrite)
                {
                    if (root.Property != null && root.CanRead && Equals(root.Read(null), val)) return;
                    root.Write(null, val);
                    return;
                }

                List<object> instances = GetCachedInstances(pin.RootType, InstanceCacheTtlMs, nowMs, ref didSceneScanThisFrame);
                for (int i = 0; i < instances.Count; i++)
                {
                    object inst = instances[i];
                    if (IsNullOrDestroyed(inst)) continue;
                    if (root.Property != null && root.CanRead && Equals(root.Read(inst), val)) continue;
                    root.Write(inst, val);
                }
                return;
            }

            if (!root.CanRead) return;

            _pinTargetsA.Clear();
            if (root.IsStaticRead)
            {
                object child = root.Read(null);
                if (!IsNullOrDestroyed(child)) _pinTargetsA.Add(child);
            }
            else
            {
                List<object> roots = GetCachedInstances(pin.RootType, InstanceCacheTtlMs, nowMs, ref didSceneScanThisFrame);
                for (int i = 0; i < roots.Count; i++)
                {
                    object r = roots[i];
                    if (IsNullOrDestroyed(r)) continue;
                    object child = root.Read(r);
                    if (!IsNullOrDestroyed(child)) _pinTargetsA.Add(child);
                }
            }

            if (_pinTargetsA.Count == 0) return;

            List<object> curTargets = _pinTargetsA;
            List<object> nextTargets = _pinTargetsB;
            for (int s = 1; s < pin.Segments.Length - 1; s++)
            {
                nextTargets.Clear();
                string seg = pin.Segments[s];
                for (int i = 0; i < curTargets.Count; i++)
                {
                    object obj = curTargets[i];
                    if (IsNullOrDestroyed(obj)) continue;
                    MemberBinding mid = GetMemberCached(obj.GetType(), seg);
                    if (mid == null || !mid.CanRead) continue;
                    object child = mid.Read(obj);
                    if (!IsNullOrDestroyed(child)) nextTargets.Add(child);
                }
                if (nextTargets.Count == 0)
                {
                    curTargets.Clear();
                    return;
                }
                List<object> swap = curTargets;
                curTargets = nextTargets;
                nextTargets = swap;
            }

            string leafName = pin.Segments[pin.Segments.Length - 1];
            for (int i = 0; i < curTargets.Count; i++)
            {
                object leafTarget = curTargets[i];
                if (IsNullOrDestroyed(leafTarget)) continue;
                Type leafType = leafTarget.GetType();
                if (!ReferenceEquals(leafType, pin.CachedLeafType) || !pin.HasCachedLeafConvertedValue)
                {
                    MemberBinding leaf = GetMemberCached(leafType, leafName);
                    if (leaf == null || !leaf.CanWrite) continue;
                    pin.CachedLeafType = leafType;
                    pin.CachedLeafMember = leaf;
                    pin.CachedLeafConvertedValue = ConvertToType(pin.Value, leaf.MemberType);
                    pin.HasCachedLeafConvertedValue = true;
                }
                if (pin.CachedLeafMember.Property != null &&
                    pin.CachedLeafMember.CanRead &&
                    Equals(pin.CachedLeafMember.Read(leafTarget), pin.CachedLeafConvertedValue))
                {
                    continue;
                }
                pin.CachedLeafMember.Write(leafTarget, pin.CachedLeafConvertedValue);
            }

            curTargets.Clear();
            nextTargets.Clear();
        }

        private List<object> GetCachedInstances(
            Type rootType,
            long ttlMs,
            long nowMs,
            ref bool didSceneScanThisFrame)
        {
            _hasSceneCaches = true;
            if (!_instanceCache.TryGetValue(rootType, out InstanceCacheEntry entry))
            {
                entry = new InstanceCacheEntry();
                _instanceCache[rootType] = entry;
            }

            List<object> list = entry.Instances;
            int writeIdx = 0;
            for (int i = 0; i < list.Count; i++)
            {
                object item = list[i];
                if (!IsNullOrDestroyed(item))
                {
                    list[writeIdx++] = item;
                }
            }
            if (writeIdx < list.Count)
            {
                list.RemoveRange(writeIdx, list.Count - writeIdx);
            }

            if (list.Count > 0)
            {
                if ((entry.LastRefreshMs >= 0 && nowMs - entry.LastRefreshMs < ttlMs) || didSceneScanThisFrame)
                {
                    return list;
                }
                List<object> refreshed = FindAllInstances(rootType, false);
                didSceneScanThisFrame = true;
                list.Clear();
                list.AddRange(refreshed);
                entry.LastRefreshMs = nowMs;
                if (list.Count > 0) entry.HasEverFound = true;
                return list;
            }

            if (entry.LastRefreshMs >= 0 && nowMs - entry.LastRefreshMs < MissRetryMs)
            {
                return list;
            }

            List<object> scanned = FindAllInstances(rootType, false);
            didSceneScanThisFrame = true;
            list.Clear();
            list.AddRange(scanned);
            entry.LastRefreshMs = nowMs;
            if (list.Count > 0) entry.HasEverFound = true;
            return list;
        }

        private ActiveComponentListCache GetCachedActiveComponents(Type targetType, long maxAgeMs, long nowMs)
        {
            _hasSceneCaches = true;
            if (!_activeCompListCache.TryGetValue(targetType, out ActiveComponentListCache entry))
            {
                entry = new ActiveComponentListCache();
                _activeCompListCache[targetType] = entry;
            }

            if (entry.LastRefreshMs >= 0 && nowMs - entry.LastRefreshMs < maxAgeMs)
            {
                return entry;
            }

            FindActiveComponents(targetType, entry.Entries);
            entry.ColliderTransforms.Clear();
            entry.LastRefreshMs = nowMs;
            return entry;
        }

        private UnityEngine.Component GetCachedActiveComponent(
            Type type,
            ref UnityEngine.Component cached,
            ref long lastMissMs,
            long nowMs)
        {
            if (!IsNullOrDestroyed(cached))
            {
                GameObject go = cached.gameObject;
                if (go != null && go.activeInHierarchy) return cached;
            }
            cached = null;
            if (lastMissMs >= 0 && nowMs - lastMissMs < MissRetryMs) return null;

            UnityEngine.Component fresh = FindActiveComponent(type);
            if (!IsNullOrDestroyed(fresh))
            {
                cached = fresh;
                lastMissMs = -1;
                return fresh;
            }
            lastMissMs = nowMs;
            return null;
        }

        private List<object> FindAllInstances(Type targetType, bool includeInactive = false)
        {
            var results = new List<object>();

            if (typeof(UnityEngine.Object).IsAssignableFrom(targetType))
            {
                try
                {
#if IL2CPP
                    var il2cppType = Il2CppInterop.Runtime.Il2CppType.From(targetType, false);
                    if (il2cppType != null)
                    {
                        System.Collections.IEnumerable found = null;
                        if (includeInactive && _findObjectsIncludeInactive != null)
                        {
                            try
                            {
                                Interlocked.Increment(ref _sceneScanCount);
                                found = _findObjectsIncludeInactive.Invoke(null, new object[] { il2cppType, true }) as System.Collections.IEnumerable;
                            }
                            catch { found = null; }
                        }
                        if (found == null)
                        {
                            Interlocked.Increment(ref _sceneScanCount);
                            found = UnityEngine.Object.FindObjectsOfType(il2cppType);
                        }
                        if (found != null)
                        {
                            foreach (object rawObj in found)
                            {
                                UnityEngine.Object obj = rawObj as UnityEngine.Object;
                                if (obj == null) continue;
                                object wrapped = Activator.CreateInstance(targetType, new object[] { obj.Pointer });
                                if (wrapped != null) results.Add(wrapped);
                            }
                        }

                        if (includeInactive && results.Count == 0 && typeof(UnityEngine.ScriptableObject).IsAssignableFrom(targetType))
                        {
                            Interlocked.Increment(ref _sceneScanCount);
                            var assets = UnityEngine.Resources.FindObjectsOfTypeAll(il2cppType);
                            if (assets != null)
                            {
                                foreach (UnityEngine.Object obj in assets)
                                {
                                    if (obj == null) continue;
                                    object wrapped = Activator.CreateInstance(targetType, new object[] { obj.Pointer });
                                    if (wrapped != null) results.Add(wrapped);
                                }
                            }
                        }
                    }
#else
                    System.Collections.IEnumerable found = null;
                    if (includeInactive && _findObjectsIncludeInactive != null)
                    {
                        try
                        {
                            Interlocked.Increment(ref _sceneScanCount);
                            found = _findObjectsIncludeInactive.Invoke(null, new object[] { targetType, true }) as System.Collections.IEnumerable;
                        }
                        catch { found = null; }
                    }
                    if (found == null)
                    {
                        Interlocked.Increment(ref _sceneScanCount);
                        found = UnityEngine.Object.FindObjectsOfType(targetType);
                    }
                    if (found != null)
                    {
                        foreach (object rawObj in found)
                        {
                            UnityEngine.Object obj = rawObj as UnityEngine.Object;
                            if (obj != null) results.Add(obj);
                        }
                    }

                    if (includeInactive && results.Count == 0 && typeof(UnityEngine.ScriptableObject).IsAssignableFrom(targetType))
                    {
                        Interlocked.Increment(ref _sceneScanCount);
                        UnityEngine.Object[] assets = UnityEngine.Resources.FindObjectsOfTypeAll(targetType);
                        if (assets != null)
                        {
                            foreach (UnityEngine.Object obj in assets)
                            {
                                if (obj != null) results.Add(obj);
                            }
                        }
                    }
#endif
                }
                catch { }
            }

            if (results.Count == 0)
            {
                try
                {
                    MemberBinding instMember = GetMemberCached(targetType, "Instance") ?? GetMemberCached(targetType, "instance");
                    if (instMember != null && instMember.IsStaticRead && instMember.CanRead)
                    {
                        object singletonVal = instMember.Read(null);
                        if (!IsNullOrDestroyed(singletonVal))
                        {
                            results.Add(singletonVal);
                        }
                    }
                }
                catch { }
            }

            return results;
        }

        private void FindActiveComponents(Type targetType, List<ActiveComponentEntry> destination)
        {
            destination.Clear();
            if (targetType == null || !typeof(UnityEngine.Component).IsAssignableFrom(targetType)) return;
            try
            {
#if IL2CPP
                var il2cppType = Il2CppInterop.Runtime.Il2CppType.From(targetType, false);
                if (il2cppType == null) return;
                Interlocked.Increment(ref _sceneScanCount);
                var found = UnityEngine.Object.FindObjectsOfType(il2cppType);
                if (found == null) return;
                foreach (UnityEngine.Object obj in found)
                {
                    if (obj == null) continue;
                    UnityEngine.Component comp = null;
                    try { comp = obj.TryCast<UnityEngine.Component>(); } catch { comp = null; }
                    if (comp == null)
                    {
                        try { comp = Activator.CreateInstance(typeof(UnityEngine.Component), new object[] { obj.Pointer }) as UnityEngine.Component; }
                        catch { comp = null; }
                    }
                    if (IsNullOrDestroyed(comp)) continue;
                    GameObject go = comp.gameObject;
                    if (go == null || !go.activeInHierarchy) continue;
                    Transform tr = comp.transform;
                    bool isPlayerTag = false;
                    try { isPlayerTag = IsConfiguredPlayerTag(go.tag); } catch { }
                    destination.Add(new ActiveComponentEntry
                    {
                        Component = comp,
                        GameObject = go,
                        Transform = tr,
                        Root = tr != null ? tr.root : null,
                        InstanceId = comp.GetInstanceID(),
                        IsPlayerTag = isPlayerTag
                    });
                }
#else
                Interlocked.Increment(ref _sceneScanCount);
                UnityEngine.Object[] found = UnityEngine.Object.FindObjectsOfType(targetType);
                if (found == null) return;
                for (int i = 0; i < found.Length; i++)
                {
                    UnityEngine.Component comp = found[i] as UnityEngine.Component;
                    if (IsNullOrDestroyed(comp)) continue;
                    GameObject go = comp.gameObject;
                    if (go == null || !go.activeInHierarchy) continue;
                    Transform tr = comp.transform;
                    bool isPlayerTag = false;
                    try { isPlayerTag = IsConfiguredPlayerTag(go.tag); } catch { }
                    destination.Add(new ActiveComponentEntry
                    {
                        Component = comp,
                        GameObject = go,
                        Transform = tr,
                        Root = tr != null ? tr.root : null,
                        InstanceId = comp.GetInstanceID(),
                        IsPlayerTag = isPlayerTag
                    });
                }
#endif
            }
            catch { }
        }

        private static object ConvertToType(object raw, Type targetType)
        {
            if (raw == null) return null;
            Type underlying = Nullable.GetUnderlyingType(targetType) ?? targetType;
            if (underlying.IsInstanceOfType(raw)) return raw;
            if (underlying == typeof(float)) return Convert.ToSingle(raw, CultureInfo.InvariantCulture);
            if (underlying == typeof(double)) return Convert.ToDouble(raw, CultureInfo.InvariantCulture);
            if (underlying == typeof(int))
            {
                if (raw is float || raw is double || raw is decimal)
                    return Convert.ToInt32(Math.Round(Convert.ToDouble(raw, CultureInfo.InvariantCulture)));
                if (raw is string s && double.TryParse(s, NumberStyles.Float | NumberStyles.AllowThousands, CultureInfo.InvariantCulture, out double d))
                    return Convert.ToInt32(Math.Round(d));
                return Convert.ToInt32(raw, CultureInfo.InvariantCulture);
            }
            if (underlying == typeof(long))
            {
                if (raw is float || raw is double || raw is decimal)
                    return Convert.ToInt64(Math.Round(Convert.ToDouble(raw, CultureInfo.InvariantCulture)));
                if (raw is string s && double.TryParse(s, NumberStyles.Float | NumberStyles.AllowThousands, CultureInfo.InvariantCulture, out double d))
                    return Convert.ToInt64(Math.Round(d));
                return Convert.ToInt64(raw, CultureInfo.InvariantCulture);
            }
            if (underlying == typeof(bool)) return Convert.ToBoolean(raw, CultureInfo.InvariantCulture);
            if (underlying == typeof(string)) return Convert.ToString(raw, CultureInfo.InvariantCulture);
            if (underlying.IsEnum) return Enum.Parse(underlying, Convert.ToString(raw, CultureInfo.InvariantCulture), true);
            if (underlying == typeof(Vector3)) return ToVector3(raw);
            if (underlying == typeof(Vector2)) return ToVector2(raw);
            if (underlying == typeof(Quaternion)) return ToQuaternion(raw);
            return Convert.ChangeType(raw, underlying, CultureInfo.InvariantCulture);
        }

        private static bool TryReadComponent(Dictionary<string, object> src, string key, out float value)
        {
            value = 0f;
            if (!src.TryGetValue(key, out object raw) || raw == null) return false;
            try { value = Convert.ToSingle(raw, CultureInfo.InvariantCulture); return true; }
            catch { return false; }
        }

        private static Vector3 ToVector3(object raw)
        {
            if (raw is Vector3 v) return v;
            if (raw is Dictionary<string, object> d)
            {
                TryReadComponent(d, "x", out float x);
                TryReadComponent(d, "y", out float y);
                TryReadComponent(d, "z", out float z);
                return new Vector3(x, y, z);
            }
            if (raw is List<object> list && list.Count >= 2)
            {
                float x = Convert.ToSingle(list[0], CultureInfo.InvariantCulture);
                float y = Convert.ToSingle(list[1], CultureInfo.InvariantCulture);
                float z = list.Count >= 3 ? Convert.ToSingle(list[2], CultureInfo.InvariantCulture) : 0f;
                return new Vector3(x, y, z);
            }
            if (raw is string str)
            {
                // Accept the Unity ToString() shape: "(1.00, 2.00, 3.00)"
                string cleaned = str.Trim().Trim('(', ')');
                string[] parts = cleaned.Split(',');
                if (parts.Length >= 2)
                {
                    float x = float.Parse(parts[0], NumberStyles.Float, CultureInfo.InvariantCulture);
                    float y = float.Parse(parts[1], NumberStyles.Float, CultureInfo.InvariantCulture);
                    float z = parts.Length >= 3 ? float.Parse(parts[2], NumberStyles.Float, CultureInfo.InvariantCulture) : 0f;
                    return new Vector3(x, y, z);
                }
            }
            throw new InvalidOperationException("cannot convert value to UnityEngine.Vector3");
        }

        private static Vector2 ToVector2(object raw)
        {
            if (raw is Vector2 v) return v;
            if (raw is Dictionary<string, object> d)
            {
                TryReadComponent(d, "x", out float x);
                TryReadComponent(d, "y", out float y);
                return new Vector2(x, y);
            }
            if (raw is List<object> list && list.Count >= 2)
            {
                return new Vector2(
                    Convert.ToSingle(list[0], CultureInfo.InvariantCulture),
                    Convert.ToSingle(list[1], CultureInfo.InvariantCulture));
            }
            throw new InvalidOperationException("cannot convert value to UnityEngine.Vector2");
        }

        private static Quaternion ToQuaternion(object raw)
        {
            if (raw is Quaternion q) return q;
            if (raw is Dictionary<string, object> d)
            {
                if (d.ContainsKey("x") || d.ContainsKey("y") || d.ContainsKey("z") || d.ContainsKey("w"))
                {
                    TryReadComponent(d, "x", out float qx);
                    TryReadComponent(d, "y", out float qy);
                    TryReadComponent(d, "z", out float qz);
                    TryReadComponent(d, "w", out float qw);
                    if (!d.ContainsKey("w") && qx == 0f && qy == 0f && qz == 0f) qw = 1f;
                    return new Quaternion(qx, qy, qz, qw);
                }
                // Allow euler-style input: { "euler": { "x": 0, "y": 90, "z": 0 } }
                if (d.TryGetValue("euler", out object euler))
                {
                    Vector3 e = ToVector3(euler);
                    return Quaternion.Euler(e.x, e.y, e.z);
                }
            }
            if (raw is List<object> list && list.Count >= 4)
            {
                return new Quaternion(
                    Convert.ToSingle(list[0], CultureInfo.InvariantCulture),
                    Convert.ToSingle(list[1], CultureInfo.InvariantCulture),
                    Convert.ToSingle(list[2], CultureInfo.InvariantCulture),
                    Convert.ToSingle(list[3], CultureInfo.InvariantCulture));
            }
            throw new InvalidOperationException("cannot convert value to UnityEngine.Quaternion");
        }

        private void TickTargetLock(TargetLockPin pin, long nowMs)
        {
            pin.LastTickMs = nowMs;
            UnityEngine.Component playerComp = GetCachedActiveComponent(pin.PlayerType, ref pin.CachedPlayer, ref pin.LastPlayerMissMs, nowMs);
            UnityEngine.Component camComp = GetCachedActiveComponent(pin.CameraType, ref pin.CachedCamera, ref pin.LastCameraMissMs, nowMs);
            if (IsNullOrDestroyed(playerComp) || IsNullOrDestroyed(camComp))
            {
                pin.CurrentlyLocked = false;
                pin.CurrentLockedEnemy = null;
                pin.CurrentLockedTransform = null;
                return;
            }

            bool urgentRefresh = pin.CurrentLockedEnemy != null &&
                (IsNullOrDestroyed(pin.CurrentLockedEnemy) ||
                 !pin.CurrentLockedEnemy.gameObject.activeInHierarchy ||
                 !IsTargetAlive(pin.CurrentLockedEnemy, pin.AliveChecks));

            ActiveComponentListCache enemyCache = GetCachedActiveComponents(
                pin.TargetType,
                urgentRefresh ? EnemyUrgentRefreshMs : EnemyScanIntervalMs,
                nowMs);

            Vector3 playerPos = playerComp.transform.position;
            Transform playerRoot = playerComp.transform.root;
            float maxSqr = pin.MaxRange * pin.MaxRange;
            float bestSqr = maxSqr;
            UnityEngine.Component bestEnemy = null;
            Transform bestTransform = null;

            List<ActiveComponentEntry> entries = enemyCache.Entries;
            for (int i = 0; i < entries.Count; i++)
            {
                ActiveComponentEntry e = entries[i];
                UnityEngine.Component tc = e.Component;
                if (IsNullOrDestroyed(tc)) continue;
                GameObject go = e.GameObject;
                if (go == null || !go.activeInHierarchy) continue;
                if (e.Root == playerRoot) continue;
                if (e.IsPlayerTag) continue;

                float sqrDist = (e.Transform.position - playerPos).sqrMagnitude;
                if (sqrDist > bestSqr) continue;

                if (!IsTargetAlive(tc, pin.AliveChecks)) continue;

                Transform candidateTransform = e.Transform;
                if (pin.ColliderType != null)
                {
                    if (!enemyCache.ColliderTransforms.TryGetValue(e.InstanceId, out candidateTransform))
                    {
                        candidateTransform = ResolveColliderTransform(tc, pin.ColliderType);
                        enemyCache.ColliderTransforms[e.InstanceId] = candidateTransform;
                    }
                    if (candidateTransform == null) continue;
                }

                bestSqr = sqrDist;
                bestEnemy = tc;
                bestTransform = candidateTransform;
            }

            if (bestTransform != null)
            {
                bool targetChanged = bestTransform != pin.CurrentLockedTransform || !pin.CurrentlyLocked;
                if (targetChanged || (nowMs - pin.LastLockInvokeMs >= LockReassertIntervalMs))
                {
                    InvokeCameraLock(camComp, pin, bestTransform);
                    pin.CurrentlyLocked = true;
                    pin.CurrentLockedEnemy = bestEnemy;
                    pin.CurrentLockedTransform = bestTransform;
                    pin.LastLockInvokeMs = nowMs;
                }
            }
            else if (pin.CurrentlyLocked)
            {
                InvokeCameraUnlockOnComponent(camComp, pin.UnlockMethodInfo, pin.UnlockMethod);
                pin.CurrentlyLocked = false;
                pin.CurrentLockedEnemy = null;
                pin.CurrentLockedTransform = null;
            }
        }

        private static Transform ResolveColliderTransform(UnityEngine.Component tc, Type colliderType)
        {
            UnityEngine.Component rootCol = GetComponentOn(tc, colliderType, false, out _);
            if (!IsNullOrDestroyed(rootCol)) return tc.transform;
            UnityEngine.Component childCol = GetComponentOn(tc, colliderType, true, out _);
            if (!IsNullOrDestroyed(childCol)) return childCol.transform;
            return null;
        }

        private static void InvokeCameraLock(UnityEngine.Component camComp, TargetLockPin pin, Transform targetTransform)
        {
            if (pin.LockMethod3Arg != null)
            {
                pin.LockArgs3[0] = targetTransform;
                pin.LockMethod3Arg.Invoke(camComp, pin.LockArgs3);
                pin.LockArgs3[0] = null;
            }
            else if (pin.LockMethod1Arg != null)
            {
                pin.LockArgs1[0] = targetTransform;
                pin.LockMethod1Arg.Invoke(camComp, pin.LockArgs1);
                pin.LockArgs1[0] = null;
            }
        }

        private void InvokeCameraUnlock(TargetLockPin pin)
        {
            UnityEngine.Component camComp = !IsNullOrDestroyed(pin.CachedCamera) &&
                                            pin.CachedCamera.gameObject != null &&
                                            pin.CachedCamera.gameObject.activeInHierarchy
                ? pin.CachedCamera
                : FindActiveComponent(pin.CameraType);
            if (!IsNullOrDestroyed(camComp))
            {
                InvokeCameraUnlockOnComponent(camComp, pin.UnlockMethodInfo, pin.UnlockMethod);
            }
            pin.CurrentlyLocked = false;
            pin.CurrentLockedEnemy = null;
            pin.CurrentLockedTransform = null;
        }

        private static void InvokeCameraUnlockOnComponent(
            UnityEngine.Component camComp,
            MethodInfo preResolvedUnlock,
            string unlockMethod)
        {
            if (IsNullOrDestroyed(camComp)) return;
            if (preResolvedUnlock != null)
            {
                preResolvedUnlock.Invoke(camComp, null);
                return;
            }
            if (string.IsNullOrEmpty(unlockMethod)) return;
            foreach (MethodInfo m in camComp.GetType().GetMethods(InstanceMethodFlags))
            {
                if (m.Name == unlockMethod && m.GetParameters().Length == 0)
                {
                    m.Invoke(camComp, null);
                    return;
                }
            }
        }

        private int TickDamageAura(DamageAuraPin pin, long nowMs)
        {
            pin.LastTickMs = nowMs;
            if (!IsRequiredActivePresent(
                pin.RequireActiveType, ref pin.CachedRequireActive, ref pin.LastRequireActiveMissMs, nowMs))
            {
                pin.CachedPlayer = null;
                return 0;
            }

            UnityEngine.Component playerComp = GetCachedActiveComponent(pin.PlayerType, ref pin.CachedPlayer, ref pin.LastPlayerMissMs, nowMs);
            if (IsNullOrDestroyed(playerComp)) return 0;

            ActiveComponentListCache enemyCache = GetCachedActiveComponents(pin.TargetType, EnemyScanIntervalMs, nowMs);
            Vector3 playerPos = playerComp.transform.position;
            Transform playerTransform = playerComp.transform;
            Transform playerRoot = playerTransform.root;
            float maxSqr = pin.Radius * pin.Radius;
            int hits = 0;

            List<ActiveComponentEntry> entries = enemyCache.Entries;
            for (int i = 0; i < entries.Count; i++)
            {
                if (hits >= MaxAuraHitsPerTick) break;
                ActiveComponentEntry e = entries[i];
                UnityEngine.Component tc = e.Component;
                if (IsNullOrDestroyed(tc)) continue;
                GameObject go = e.GameObject;
                if (go == null || !go.activeInHierarchy) continue;
                if (e.Root == playerRoot) continue;
                if (e.IsPlayerTag) continue;

                if (!string.IsNullOrEmpty(pin.ExcludeBoolField))
                {
                    object typedTarget = tc;
#if IL2CPP
                    if (!pin.TargetType.IsInstanceOfType(tc))
                    {
                        try { typedTarget = Activator.CreateInstance(pin.TargetType, new object[] { tc.Pointer }) ?? (object)tc; }
                        catch { typedTarget = tc; }
                    }
#endif
                    if (TryReadBoolMemberCached(typedTarget, pin.ExcludeBoolMember, pin.ExcludeBoolField, out bool excluded) && excluded)
                    {
                        continue;
                    }
                }

                float sqrDist = (e.Transform.position - playerPos).sqrMagnitude;
                if (sqrDist > maxSqr) continue;

                for (int h = 0; h < pin.Handlers.Count; h++)
                {
                    DamageHandlerSpec handler = pin.Handlers[h];
                    GetComponentOn(tc, handler.ComponentType, false, out object compObj);
                    if (IsNullOrDestroyed(compObj)) continue;

                    if (!string.IsNullOrEmpty(handler.DeadField) &&
                        TryReadBoolMemberCached(compObj, handler.DeadMember, handler.DeadField, out bool isDead) &&
                        isDead)
                    {
                        break;
                    }
                    if (handler.DeadMethodInfo != null)
                    {
                        try
                        {
                            object deadRes = handler.DeadMethodInfo.Invoke(compObj, null);
                            if (deadRes is bool isDeadMethod && isDeadMethod) break;
                        }
                        catch { }
                    }
                    if (!string.IsNullOrEmpty(handler.HealthField) &&
                        TryReadDoubleMemberCached(compObj, handler.HealthMember, handler.HealthField, out double hp) &&
                        hp <= 0.0)
                    {
                        break;
                    }

                    if (handler.ZeroHealthBeforeCall && handler.HealthMember != null && handler.HealthMember.CanWrite)
                    {
                        try
                        {
                            handler.HealthMember.Write(compObj, ConvertToType(0, handler.HealthMember.MemberType));
                        }
                        catch { }
                    }

                    if (ApplyDamageHandler(handler, compObj, tc, playerTransform, playerComp, pin.DamageAmount))
                    {
                        hits++;
                        break;
                    }
                }
            }

            return hits;
        }

        /// <summary>
        /// Parses the vendor-neutral `args` array of a damage handler. When `args` is absent,
        /// a legacy `kind` shorthand (`invector_health` / `emerald_ai` / `invoke_damage_int`)
        /// is translated into the equivalent descriptor so older packages keep working.
        /// </summary>
        private List<AuraArgSpec> ParseAuraArgs(Dictionary<string, object> raw, int damageAmount)
        {
            if (raw != null && raw.TryGetValue("args", out object rawArgs) && rawArgs is List<object> argList)
            {
                var specs = new List<AuraArgSpec>();
                foreach (object item in argList)
                {
                    AuraArgSpec spec = ParseAuraArg(item);
                    if (spec == null) return null;
                    specs.Add(spec);
                }
                return specs;
            }

            string kind = GetString(raw, "kind");
            var legacy = new List<AuraArgSpec>();

            if (string.IsNullOrEmpty(kind) || kind == "invoke_damage_int")
            {
                legacy.Add(new AuraArgSpec { Bind = "damage" });
                return legacy;
            }

            if (kind == "emerald_ai")
            {
                legacy.Add(new AuraArgSpec { Bind = "damage" });
                legacy.Add(new AuraArgSpec { Bind = "constant", ConstantValue = 0 });
                legacy.Add(new AuraArgSpec { Bind = "attacker_transform" });
                legacy.Add(new AuraArgSpec { Bind = "constant", ConstantValue = 0 });
                legacy.Add(new AuraArgSpec { Bind = "constant", ConstantValue = true });
                return legacy;
            }

            if (kind == "invector_health")
            {
                string dmgClass = GetString(raw, "damage_type_class");
                if (string.IsNullOrEmpty(dmgClass)) return null;
                Type dmgType = ResolveType(dmgClass);
                if (dmgType == null) return null;

                legacy.Add(new AuraArgSpec
                {
                    Bind = "object",
                    ObjectType = dmgType,
                    CtorArgs = new List<AuraArgSpec> { new AuraArgSpec { Bind = "damage" } },
                    ObjectFields = new List<KeyValuePair<string, AuraArgSpec>>
                    {
                        new KeyValuePair<string, AuraArgSpec>("damageValue", new AuraArgSpec { Bind = "damage" }),
                        new KeyValuePair<string, AuraArgSpec>("sender", new AuraArgSpec { Bind = "attacker_transform" }),
                        new KeyValuePair<string, AuraArgSpec>("receiver", new AuraArgSpec { Bind = "target_transform" }),
                        new KeyValuePair<string, AuraArgSpec>("hitPosition", new AuraArgSpec { Bind = "target_position" }),
                        new KeyValuePair<string, AuraArgSpec>("ignoreDefense", new AuraArgSpec { Bind = "constant", ConstantValue = true }),
                        new KeyValuePair<string, AuraArgSpec>("isCritical", new AuraArgSpec { Bind = "constant", ConstantValue = true })
                    }
                });
                return legacy;
            }

            return null;
        }

        private AuraArgSpec ParseAuraArg(object item)
        {
            if (item is not Dictionary<string, object> d) return null;
            string bind = GetString(d, "bind");
            if (string.IsNullOrEmpty(bind)) return null;

            var spec = new AuraArgSpec { Bind = bind };

            if (bind == "constant")
            {
                if (!d.TryGetValue("value", out object v)) return null;
                spec.ConstantValue = v;
                return spec;
            }

            if (bind == "object")
            {
                string cls = GetString(d, "class");
                if (string.IsNullOrEmpty(cls)) return null;
                Type t = ResolveType(cls);
                if (t == null) return null;
                spec.ObjectType = t;

                if (d.TryGetValue("ctor_args", out object ctorRaw) && ctorRaw is List<object> ctorList)
                {
                    spec.CtorArgs = new List<AuraArgSpec>();
                    foreach (object c in ctorList)
                    {
                        AuraArgSpec cs = ParseAuraArg(c);
                        if (cs == null) return null;
                        spec.CtorArgs.Add(cs);
                    }
                }

                if (d.TryGetValue("fields", out object fieldsRaw) && fieldsRaw is Dictionary<string, object> fields)
                {
                    spec.ObjectFields = new List<KeyValuePair<string, AuraArgSpec>>();
                    foreach (var kv in fields)
                    {
                        AuraArgSpec fs = ParseAuraArg(kv.Value);
                        if (fs == null) return null;
                        spec.ObjectFields.Add(new KeyValuePair<string, AuraArgSpec>(kv.Key, fs));
                    }
                }
                return spec;
            }

            switch (bind)
            {
                case "damage":
                case "attacker_transform":
                case "target_transform":
                case "target_position":
                case "target_component":
                case "player_transform":
                case "player_component":
                    return spec;
                default:
                    return null;
            }
        }

        private static void BindAuraMethod(DamageHandlerSpec handler)
        {
            if (handler.Args == null) return;
            int arity = handler.Args.Count;
            foreach (MethodInfo m in handler.ComponentType.GetMethods(InstanceMethodFlags))
            {
                if (m.Name != handler.MethodName) continue;
                if (m.GetParameters().Length != arity) continue;
                handler.DamageMethod = m;
                handler.InvokeArgs = new object[arity];
                return;
            }
        }

        private object ResolveAuraArg(
            AuraArgSpec spec,
            int damageAmount,
            UnityEngine.Component targetComp,
            Transform attackerTransform,
            UnityEngine.Component playerComp)
        {
            switch (spec.Bind)
            {
                case "damage":
                    return damageAmount;
                case "attacker_transform":
                    return attackerTransform;
                case "player_transform":
                    return playerComp != null ? (object)playerComp.transform : attackerTransform;
                case "player_component":
                    return playerComp;
                case "target_transform":
                    return targetComp != null ? (object)targetComp.transform : null;
                case "target_position":
                    return targetComp != null ? (object)targetComp.transform.position : Vector3.zero;
                case "target_component":
                    return targetComp;
                case "constant":
                    return spec.ConstantValue;
                case "object":
                {
                    object built = null;
                    if (spec.CtorArgs != null && spec.CtorArgs.Count > 0)
                    {
                        var ctorArgs = new object[spec.CtorArgs.Count];
                        for (int i = 0; i < ctorArgs.Length; i++)
                        {
                            ctorArgs[i] = ResolveAuraArg(spec.CtorArgs[i], damageAmount, targetComp, attackerTransform, playerComp);
                        }
                        foreach (ConstructorInfo ci in spec.ObjectType.GetConstructors())
                        {
                            ParameterInfo[] ps = ci.GetParameters();
                            if (ps.Length != ctorArgs.Length) continue;
                            var converted = new object[ctorArgs.Length];
                            bool ok = true;
                            for (int i = 0; i < ctorArgs.Length; i++)
                            {
                                try { converted[i] = ctorArgs[i] == null ? null : ConvertToType(ctorArgs[i], ps[i].ParameterType); }
                                catch { ok = false; break; }
                            }
                            if (!ok) continue;
                            try { built = ci.Invoke(converted); break; }
                            catch { built = null; }
                        }
                    }
                    if (built == null)
                    {
                        try { built = Activator.CreateInstance(spec.ObjectType); }
                        catch { return null; }
                    }

                    if (spec.ObjectFields != null)
                    {
                        foreach (var kv in spec.ObjectFields)
                        {
                            MemberBinding mb = GetMemberCached(spec.ObjectType, kv.Key);
                            if (mb == null || !mb.CanWrite) continue;
                            object fv = ResolveAuraArg(kv.Value, damageAmount, targetComp, attackerTransform, playerComp);
                            if (fv == null) continue;
                            try { mb.Write(built, ConvertToType(fv, mb.MemberType)); } catch { }
                        }
                    }
                    return built;
                }
                default:
                    return null;
            }
        }

        private bool ApplyDamageHandler(
            DamageHandlerSpec handler,
            object compObj,
            UnityEngine.Component targetComp,
            Transform attackerTransform,
            UnityEngine.Component playerComp,
            int damageAmount)
        {
            if (handler.DamageMethod == null || handler.InvokeArgs == null) return false;

            ParameterInfo[] ps = handler.DamageMethod.GetParameters();
            for (int i = 0; i < handler.Args.Count; i++)
            {
                object v = ResolveAuraArg(handler.Args[i], damageAmount, targetComp, attackerTransform, playerComp);
                if (v != null)
                {
                    try { v = ConvertToType(v, ps[i].ParameterType); } catch { }
                }
                else if (ps[i].ParameterType.IsValueType)
                {
                    try { v = Activator.CreateInstance(ps[i].ParameterType); } catch { }
                }
                handler.InvokeArgs[i] = v;
            }

            handler.DamageMethod.Invoke(compObj, handler.InvokeArgs);
            for (int i = 0; i < handler.InvokeArgs.Length; i++) handler.InvokeArgs[i] = null;
            return true;
        }

        private UnityEngine.Component FindActiveComponent(Type type)
        {
            foreach (object obj in FindAllInstances(type, false))
            {
                UnityEngine.Component c = obj as UnityEngine.Component;
                if (IsNullOrDestroyed(c)) continue;
                GameObject go = c.gameObject;
                if (go != null && go.activeInHierarchy) return c;
            }
            return null;
        }

        private static UnityEngine.Component GetComponentOn(
            UnityEngine.Component host,
            Type compType,
            bool includeChildren,
            out object typedObj)
        {
            typedObj = null;
            if (IsNullOrDestroyed(host) || compType == null) return null;

#if IL2CPP
            var il2cppType = Il2CppInterop.Runtime.Il2CppType.From(compType, false);
            if (il2cppType == null) return null;
            UnityEngine.Component raw = includeChildren
                ? host.GetComponentInChildren(il2cppType)
                : host.GetComponent(il2cppType);
            if (raw == null) return null;
            if (compType == typeof(UnityEngine.Component))
            {
                typedObj = raw;
                return raw;
            }
            try
            {
                typedObj = Activator.CreateInstance(compType, new object[] { raw.Pointer }) ?? raw;
            }
            catch
            {
                typedObj = raw;
            }
            return raw;
#else
            UnityEngine.Component raw = includeChildren
                ? host.GetComponentInChildren(compType)
                : host.GetComponent(compType);
            typedObj = raw;
            return raw;
#endif
        }

        private bool IsTargetAlive(UnityEngine.Component targetComp, List<AliveCheckSpec> checks)
        {
            if (checks == null || checks.Count == 0) return true;
            bool matchedAny = false;
            for (int i = 0; i < checks.Count; i++)
            {
                AliveCheckSpec check = checks[i];
                GetComponentOn(targetComp, check.ComponentType, false, out object compObj);
                if (IsNullOrDestroyed(compObj)) continue;
                matchedAny = true;

                if (!string.IsNullOrEmpty(check.DeadField) &&
                    TryReadBoolMemberCached(compObj, check.DeadMember, check.DeadField, out bool isDead) &&
                    isDead)
                {
                    return false;
                }
                if (!string.IsNullOrEmpty(check.HealthField) &&
                    TryReadDoubleMemberCached(compObj, check.HealthMember, check.HealthField, out double hp) &&
                    hp <= 0.0)
                {
                    return false;
                }
            }
            return matchedAny;
        }

        private bool TryReadBoolMemberCached(object target, MemberBinding cachedMember, string memberName, out bool result)
        {
            result = false;
            try
            {
                MemberBinding m = cachedMember ?? GetMemberCached(target.GetType(), memberName);
                if (m == null || !m.CanRead) return false;
                object raw = m.Read(target);
                if (raw == null) return false;
                result = Convert.ToBoolean(raw, CultureInfo.InvariantCulture);
                return true;
            }
            catch
            {
                return false;
            }
        }

        private bool TryReadDoubleMemberCached(object target, MemberBinding cachedMember, string memberName, out double result)
        {
            result = 0.0;
            try
            {
                MemberBinding m = cachedMember ?? GetMemberCached(target.GetType(), memberName);
                if (m == null || !m.CanRead) return false;
                object raw = m.Read(target);
                if (raw == null) return false;
                result = Convert.ToDouble(raw, CultureInfo.InvariantCulture);
                return true;
            }
            catch
            {
                return false;
            }
        }

        private static object TryConvertMemberValue(MemberBinding member, object raw)
        {
            if (member == null || !member.CanWrite) return null;
            try { return ConvertToType(raw, member.MemberType); }
            catch { return null; }
        }

        private static float GetFloat(Dictionary<string, object> dict, string key, float fallback)
        {
            if (dict == null || !dict.TryGetValue(key, out object v) || v == null) return fallback;
            try { return Convert.ToSingle(v, CultureInfo.InvariantCulture); } catch { return fallback; }
        }

        private static int GetInt(Dictionary<string, object> dict, string key, int fallback)
        {
            if (dict == null || !dict.TryGetValue(key, out object v) || v == null) return fallback;
            try { return Convert.ToInt32(Math.Round(Convert.ToDouble(v, CultureInfo.InvariantCulture))); } catch { return fallback; }
        }

        private static long GetLong(Dictionary<string, object> dict, string key, long fallback)
        {
            if (dict == null || !dict.TryGetValue(key, out object v) || v == null) return fallback;
            try { return Convert.ToInt64(Math.Round(Convert.ToDouble(v, CultureInfo.InvariantCulture))); } catch { return fallback; }
        }

        private static bool GetBool(Dictionary<string, object> dict, string key, bool fallback)
        {
            if (dict == null || !dict.TryGetValue(key, out object v) || v == null) return fallback;
            try { return Convert.ToBoolean(v, CultureInfo.InvariantCulture); } catch { return fallback; }
        }

        private static List<Dictionary<string, object>> GetDictList(Dictionary<string, object> dict, string key)
        {
            var list = new List<Dictionary<string, object>>();
            if (dict == null || !dict.TryGetValue(key, out object rawObj) || !(rawObj is List<object> rawList))
                return list;
            foreach (object item in rawList)
            {
                if (item is Dictionary<string, object> entry) list.Add(entry);
            }
            return list;
        }

        private static float ClampFloat(float v, float min, float max) => v < min ? min : (v > max ? max : v);
        private static int ClampInt(int v, int min, int max) => v < min ? min : (v > max ? max : v);
        private static long ClampLong(long v, long min, long max) => v < min ? min : (v > max ? max : v);

        private void PublishDiscoveryFile()
        {
            try
            {
                string baseDir = Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData);
                if (string.IsNullOrEmpty(baseDir)) baseDir = Path.GetTempPath();
                string dir = Path.Combine(Path.Combine(baseDir, "Nekod"), "bridges");
                Directory.CreateDirectory(dir);
                _discoveryPath = Path.Combine(dir, $"unity-{_pid}.json");

                var record = new Dictionary<string, object>
                {
                    { "pid", _pid },
                    { "port", Port },
                    { "flavor", Flavor },
                    { "loader", LoaderKind },
                    { "auth_token", AuthToken },
                    { "unity_version", _unityVersion },
                    { "bridge_version", BridgeVersion }
                };
                string tmp = _discoveryPath + ".tmp";
                File.WriteAllText(tmp, MiniJson.Serialize(record), new UTF8Encoding(false));
                if (File.Exists(_discoveryPath)) File.Delete(_discoveryPath);
                File.Move(tmp, _discoveryPath);
            }
            catch (Exception ex)
            {
                _log("[NekodUnityBridge] Failed to publish discovery file: " + ex.Message);
            }
        }

        private void UnpublishDiscoveryFile()
        {
            try
            {
                if (!string.IsNullOrEmpty(_discoveryPath) && File.Exists(_discoveryPath))
                    File.Delete(_discoveryPath);
            }
            catch { }
        }

        private static string GenerateRandomToken()
        {
            byte[] bytes = new byte[16];
            using (var rng = RandomNumberGenerator.Create())
            {
                rng.GetBytes(bytes);
            }
            var sb = new StringBuilder(32);
            foreach (byte b in bytes) sb.Append(b.ToString("x2", CultureInfo.InvariantCulture));
            return sb.ToString();
        }

        private static bool ConstantTimeEquals(string a, string b)
        {
            if (a == null || b == null || a.Length != b.Length) return false;
            int diff = 0;
            for (int i = 0; i < a.Length; i++) diff |= a[i] ^ b[i];
            return diff == 0;
        }

        private static string GetString(Dictionary<string, object> dict, string key)
        {
            if (dict == null || !dict.TryGetValue(key, out object v) || v == null) return null;
            return v as string ?? Convert.ToString(v, CultureInfo.InvariantCulture);
        }

        private static string OkJson(Dictionary<string, object> data)
        {
            return MiniJson.Serialize(new Dictionary<string, object>
            {
                { "ok", true },
                { "data", data }
            });
        }

        private static string ErrorJson(string message)
        {
            return MiniJson.Serialize(new Dictionary<string, object>
            {
                { "ok", false },
                { "error", message }
            });
        }
    }
}
