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
        public const string BridgeVersion = "0.4.0";

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

        private sealed class PinnedField
        {
            public string Id;
            public Type RootType;
            public string FieldName;
            public object Value;
        }

        private sealed class AliveCheckSpec
        {
            public Type ComponentType;
            public string DeadField;
            public string HealthField;
        }

        private sealed class TargetLockPin
        {
            public string Id;
            public Type PlayerType;
            public Type CameraType;
            public Type TargetType;
            public string LockMethod;
            public string UnlockMethod;
            public List<AliveCheckSpec> AliveChecks;
            public float MaxRange;
            public float HeightOffset;
            public float LockSpeed;
            public bool RequireCollider;
            public long IntervalMs;
            public long LastTickMs;
            public bool CurrentlyLocked;
        }

        private sealed class DamageHandlerSpec
        {
            public string Kind;
            public Type ComponentType;
            public Type DamageType;
            public string MethodName;
            public string DeadField;
            public string HealthField;
        }

        private sealed class DamageAuraPin
        {
            public string Id;
            public Type PlayerType;
            public Type TargetType;
            public int DamageAmount;
            public float Radius;
            public long IntervalMs;
            public long LastTickMs;
            public List<DamageHandlerSpec> Handlers;
        }

        private const int PortScanSpan = 20;
        private const int MaxPendingWork = 16;
        private const int MainThreadTimeoutMs = 3000;
        private const long PinIntervalMs = 250;
        private const int MaxAuraHitsPerTick = 32;
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
        private readonly Dictionary<string, PinnedField> _pins = new Dictionary<string, PinnedField>(StringComparer.Ordinal);
        private readonly Dictionary<string, TargetLockPin> _targetLocks = new Dictionary<string, TargetLockPin>(StringComparer.Ordinal);
        private readonly Dictionary<string, DamageAuraPin> _damageAuras = new Dictionary<string, DamageAuraPin>(StringComparer.Ordinal);
        private readonly Stopwatch _pinTimer = Stopwatch.StartNew();
        private readonly Stopwatch _loopClock = Stopwatch.StartNew();
        private readonly Action<string> _log;
        private readonly string _unityVersion;
        private readonly int _pid;

        private TcpListener _listener;
        private Thread _acceptThread;
        private volatile bool _running;
        private string _discoveryPath;
        private long _mainThreadTicks;
        private int _activePinCount;

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
            Volatile.Write(ref _activePinCount, _pins.Count + _targetLocks.Count + _damageAuras.Count);
        }

        public void Stop()
        {
            _running = false;
            lock (_pins)
            {
                _pins.Clear();
                _targetLocks.Clear();
                _damageAuras.Clear();
                Volatile.Write(ref _activePinCount, 0);
            }
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

            if (Volatile.Read(ref _activePinCount) > 0)
            {
                List<PinnedField> fieldPins = null;
                List<TargetLockPin> lockPins = null;
                List<DamageAuraPin> auraPins = null;
                bool runFieldPins = _pinTimer.ElapsedMilliseconds >= PinIntervalMs;
                if (runFieldPins) _pinTimer.Restart();

                lock (_pins)
                {
                    if (runFieldPins && _pins.Count > 0)
                        fieldPins = new List<PinnedField>(_pins.Values);
                    if (_targetLocks.Count > 0)
                        lockPins = new List<TargetLockPin>(_targetLocks.Values);
                    if (_damageAuras.Count > 0)
                        auraPins = new List<DamageAuraPin>(_damageAuras.Values);
                }

                if (fieldPins != null)
                {
                    foreach (PinnedField pin in fieldPins)
                    {
                        try
                        {
                            ApplySetFieldPath(pin.RootType, pin.FieldName, pin.Value, out _, out _);
                        }
                        catch
                        {
                            // Scene transition or unloaded instance; keep pin armed until gameplay resumes.
                        }
                    }
                }

                long nowMs = _loopClock.ElapsedMilliseconds;
                if (lockPins != null)
                {
                    foreach (TargetLockPin lockPin in lockPins)
                    {
                        if (nowMs - lockPin.LastTickMs < lockPin.IntervalMs) continue;
                        try
                        {
                            TickTargetLock(lockPin);
                        }
                        catch
                        {
                            lockPin.LastTickMs = nowMs;
                        }
                    }
                }

                if (auraPins != null)
                {
                    foreach (DamageAuraPin auraPin in auraPins)
                    {
                        if (nowMs - auraPin.LastTickMs < auraPin.IntervalMs) continue;
                        try
                        {
                            TickDamageAura(auraPin);
                        }
                        catch
                        {
                            auraPin.LastTickMs = nowMs;
                        }
                    }
                }
            }
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
                                    { "active_pins", Volatile.Read(ref _activePinCount) }
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

        private string HandleGetField(Dictionary<string, object> p)
        {
            string className = GetString(p, "class_name");
            string fieldName = GetString(p, "field_name");
            if (string.IsNullOrEmpty(className) || string.IsNullOrEmpty(fieldName))
                return ErrorJson("get_field requires class_name and field_name");

            Type type = ResolveType(className);
            if (type == null) return ErrorJson("type not found: " + className);

            if (!TryGetFieldPath(type, fieldName, out object val, out string error))
                return ErrorJson(error);

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

            if (!ApplySetFieldPath(type, fieldName, p["value"], out int updated, out string error))
                return ErrorJson(error);

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

            if (!ApplySetFieldPath(type, fieldName, rawValue, out int updated, out string applyError))
                return ErrorJson(applyError);

            lock (_pins)
            {
                _pins[id] = new PinnedField
                {
                    Id = id,
                    RootType = type,
                    FieldName = fieldName,
                    Value = rawValue
                };
                UpdateActivePinCountLocked();
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
            }

            int restored = 0;
            string className = GetString(p, "class_name");
            string fieldName = GetString(p, "field_name");
            if (p != null && p.ContainsKey("restore_value") && !string.IsNullOrEmpty(className) && !string.IsNullOrEmpty(fieldName))
            {
                Type type = ResolveType(className);
                if (type != null)
                {
                    ApplySetFieldPath(type, fieldName, p["restore_value"], out restored, out _);
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
                aliveChecks.Add(new AliveCheckSpec
                {
                    ComponentType = compType,
                    DeadField = GetString(rawCheck, "dead_field"),
                    HealthField = GetString(rawCheck, "health_field")
                });
            }

            var pin = new TargetLockPin
            {
                Id = id,
                PlayerType = playerType,
                CameraType = cameraType,
                TargetType = targetType,
                LockMethod = string.IsNullOrEmpty(GetString(p, "lock_method")) ? "SetLockTarget" : GetString(p, "lock_method"),
                UnlockMethod = string.IsNullOrEmpty(GetString(p, "unlock_method")) ? "RemoveLockTarget" : GetString(p, "unlock_method"),
                AliveChecks = aliveChecks,
                MaxRange = ClampFloat(GetFloat(p, "max_range", 45f), 1f, 500f),
                HeightOffset = GetFloat(p, "height_offset", 0f),
                LockSpeed = ClampFloat(GetFloat(p, "lock_speed", 10f), 0.5f, 100f),
                RequireCollider = GetBool(p, "require_collider", true),
                IntervalMs = ClampLong(GetLong(p, "interval_ms", 80), 30, 2000),
                LastTickMs = _loopClock.ElapsedMilliseconds,
                CurrentlyLocked = false
            };

            lock (_pins)
            {
                _targetLocks[id] = pin;
                UpdateActivePinCountLocked();
            }

            try
            {
                TickTargetLock(pin);
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

            Type playerType = ResolveType(playerClass);
            if (playerType == null) return ErrorJson("type not found: " + playerClass);
            Type targetType = ResolveType(targetClass);
            if (targetType == null) return ErrorJson("type not found: " + targetClass);

            List<Dictionary<string, object>> rawHandlers = GetDictList(p, "handlers");
            if (rawHandlers.Count == 0)
                return ErrorJson("pin_damage_aura requires at least one damage handler");

            var handlers = new List<DamageHandlerSpec>(rawHandlers.Count);
            foreach (Dictionary<string, object> raw in rawHandlers)
            {
                string kind = GetString(raw, "kind");
                string compClass = GetString(raw, "component_class");
                string methodName = GetString(raw, "method_name");
                if (string.IsNullOrEmpty(kind) || string.IsNullOrEmpty(compClass) || string.IsNullOrEmpty(methodName))
                    return ErrorJson("damage handler requires kind, component_class, and method_name");

                if (kind != "invector_health" && kind != "emerald_ai" && kind != "invoke_damage_int")
                    return ErrorJson("unsupported damage handler kind: " + kind);

                Type compType = ResolveType(compClass);
                if (compType == null) return ErrorJson("type not found: " + compClass);

                Type damageType = null;
                if (kind == "invector_health")
                {
                    string dmgClass = GetString(raw, "damage_type_class");
                    if (string.IsNullOrEmpty(dmgClass))
                        return ErrorJson("invector_health handler requires damage_type_class");
                    damageType = ResolveType(dmgClass);
                    if (damageType == null) return ErrorJson("type not found: " + dmgClass);
                }

                handlers.Add(new DamageHandlerSpec
                {
                    Kind = kind,
                    ComponentType = compType,
                    DamageType = damageType,
                    MethodName = methodName,
                    DeadField = GetString(raw, "dead_field"),
                    HealthField = GetString(raw, "health_field")
                });
            }

            var pin = new DamageAuraPin
            {
                Id = id,
                PlayerType = playerType,
                TargetType = targetType,
                DamageAmount = ClampInt(GetInt(p, "damage_amount", 999999), 1, 100000000),
                Radius = ClampFloat(GetFloat(p, "radius", 25f), 1f, 200f),
                IntervalMs = ClampLong(GetLong(p, "interval_ms", 250), 100, 5000),
                LastTickMs = _loopClock.ElapsedMilliseconds,
                Handlers = handlers
            };

            lock (_pins)
            {
                _damageAuras[id] = pin;
                UpdateActivePinCountLocked();
            }

            int hits = 0;
            try
            {
                hits = TickDamageAura(pin);
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
            }

            return OkJson(new Dictionary<string, object>
            {
                { "id", id },
                { "unpinned", true }
            });
        }

        private string HandleClearPins()
        {
            int count;
            List<TargetLockPin> locksToRelease;
            lock (_pins)
            {
                count = _pins.Count + _targetLocks.Count + _damageAuras.Count;
                locksToRelease = _targetLocks.Count > 0 ? new List<TargetLockPin>(_targetLocks.Values) : null;
                _pins.Clear();
                _targetLocks.Clear();
                _damageAuras.Clear();
                Volatile.Write(ref _activePinCount, 0);
            }

            if (locksToRelease != null)
            {
                foreach (TargetLockPin lockPin in locksToRelease)
                {
                    try { InvokeCameraUnlock(lockPin); } catch { }
                }
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

                List<object> instances = FindAllInstances(type);
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

        private static void GetMember(Type t, string name, out FieldInfo fi, out PropertyInfo pi)
        {
            fi = null;
            pi = null;
            for (Type cur = t; cur != null && cur != typeof(object); cur = cur.BaseType)
            {
                fi = cur.GetField(name, MemberFlags);
                if (fi != null) return;
                pi = cur.GetProperty(name, MemberFlags);
                if (pi != null && pi.GetIndexParameters().Length == 0) return;
                pi = null;
            }
        }

        private static bool IsMemberStatic(FieldInfo fi, PropertyInfo pi, bool forWrite)
        {
            if (fi != null) return fi.IsStatic;
            MethodInfo accessor = forWrite
                ? (pi.GetSetMethod(true) ?? pi.GetGetMethod(true))
                : (pi.GetGetMethod(true) ?? pi.GetSetMethod(true));
            return accessor != null && accessor.IsStatic;
        }

        private static Type GetMemberType(FieldInfo fi, PropertyInfo pi)
        {
            return fi != null ? fi.FieldType : pi.PropertyType;
        }

        private static object ReadMember(object target, FieldInfo fi, PropertyInfo pi)
        {
            return fi != null ? fi.GetValue(target) : pi.GetValue(target, null);
        }

        private static void WriteMember(object target, FieldInfo fi, PropertyInfo pi, object value)
        {
            if (fi != null) fi.SetValue(target, value);
            else pi.SetValue(target, value, null);
        }

        private static bool PrevalidateMemberPath(Type rootType, string[] segments, object rawValue, bool writing, out string error)
        {
            GetMember(rootType, segments[0], out FieldInfo fi, out PropertyInfo pi);
            if (fi == null && pi == null)
            {
                error = $"member '{segments[0]}' not found on '{rootType.FullName}'";
                return false;
            }

            Type curType = GetMemberType(fi, pi);
            for (int i = 1; i < segments.Length; i++)
            {
                if (pi != null && pi.GetGetMethod(true) == null)
                {
                    error = $"member '{segments[i - 1]}' is write-only";
                    return false;
                }
                if (curType == null)
                {
                    error = null;
                    return true;
                }
                GetMember(curType, segments[i], out fi, out pi);
                if (fi == null && pi == null)
                {
                    if (!curType.IsInterface && !curType.IsAbstract && curType != typeof(object))
                    {
                        error = $"member '{segments[i]}' not found on '{curType.FullName}'";
                        return false;
                    }
                    error = null;
                    return true;
                }
                curType = GetMemberType(fi, pi);
            }

            if (writing)
            {
                string leafName = segments[segments.Length - 1];
                if (pi != null && pi.GetSetMethod(true) == null)
                {
                    error = $"member '{leafName}' on '{rootType.FullName}' is read-only";
                    return false;
                }
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
            else if (pi != null && pi.GetGetMethod(true) == null)
            {
                error = $"member '{segments[segments.Length - 1]}' is write-only";
                return false;
            }

            error = null;
            return true;
        }

        private static bool TryGetFieldPath(Type rootType, string fieldPath, out object value, out string error)
        {
            value = null;
            if (!SplitFieldPath(fieldPath, out string[] segments, out error))
                return false;

            GetMember(rootType, segments[0], out FieldInfo rootFi, out PropertyInfo rootPi);
            if (rootFi == null && rootPi == null)
            {
                error = $"member '{segments[0]}' not found on '{rootType.FullName}'";
                return false;
            }
            if (rootPi != null && rootPi.GetGetMethod(true) == null)
            {
                error = $"member '{segments[0]}' on '{rootType.FullName}' is write-only";
                return false;
            }

            bool firstStatic = IsMemberStatic(rootFi, rootPi, false);
            var candidates = new List<object>();
            if (firstStatic)
            {
                object firstVal = ReadMember(null, rootFi, rootPi);
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
                List<object> roots = FindAllInstances(rootType);
                if (roots.Count == 0)
                {
                    error = $"no live instance of '{rootType.FullName}' found in active scene";
                    return false;
                }
                if (segments.Length == 1)
                {
                    value = ReadMember(roots[0], rootFi, rootPi);
                    return true;
                }
                foreach (object r in roots)
                {
                    object child = ReadMember(r, rootFi, rootPi);
                    if (!IsNullOrDestroyed(child))
                        candidates.Add(child);
                }
            }

            if (candidates.Count == 0)
            {
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
                    GetMember(cur.GetType(), segments[s], out FieldInfo fi, out PropertyInfo pi);
                    if (fi == null && pi == null)
                    {
                        error = $"member '{segments[s]}' not found on '{cur.GetType().FullName}'";
                        return false;
                    }
                    if (pi != null && pi.GetGetMethod(true) == null)
                    {
                        error = $"member '{segments[s]}' on '{cur.GetType().FullName}' is write-only";
                        return false;
                    }
                    cur = ReadMember(cur, fi, pi);
                }
                if (!brokenNull)
                {
                    value = cur;
                    return true;
                }
            }

            error = $"no live target for '{rootType.FullName}.{fieldPath}' in active scene";
            return false;
        }

        private static bool ApplySetFieldPath(Type rootType, string fieldPath, object rawValue, out int updated, out string error)
        {
            updated = 0;
            if (!SplitFieldPath(fieldPath, out string[] segments, out error))
                return false;

            GetMember(rootType, segments[0], out FieldInfo rootFi, out PropertyInfo rootPi);
            if (rootFi == null && rootPi == null)
            {
                error = $"member '{segments[0]}' not found on '{rootType.FullName}'";
                return false;
            }

            if (segments.Length == 1)
            {
                if (rootPi != null && rootPi.GetSetMethod(true) == null)
                {
                    error = $"member '{segments[0]}' on '{rootType.FullName}' is read-only";
                    return false;
                }
                object converted;
                try
                {
                    converted = ConvertToType(rawValue, GetMemberType(rootFi, rootPi));
                }
                catch (Exception ex)
                {
                    error = $"invalid value for '{segments[0]}': {ex.Message}";
                    return false;
                }

                if (IsMemberStatic(rootFi, rootPi, true))
                {
                    WriteMember(null, rootFi, rootPi, converted);
                    updated = 1;
                    error = null;
                    return true;
                }

                List<object> instances = FindAllInstances(rootType);
                foreach (object inst in instances)
                {
                    WriteMember(inst, rootFi, rootPi, converted);
                    updated++;
                }
                error = null;
                return true;
            }

            if (rootPi != null && rootPi.GetGetMethod(true) == null)
            {
                error = $"member '{segments[0]}' on '{rootType.FullName}' is write-only";
                return false;
            }

            var currentTargets = new List<object>();
            if (IsMemberStatic(rootFi, rootPi, false))
            {
                object child = ReadMember(null, rootFi, rootPi);
                if (!IsNullOrDestroyed(child))
                    currentTargets.Add(child);
            }
            else
            {
                List<object> roots = FindAllInstances(rootType);
                foreach (object r in roots)
                {
                    object child = ReadMember(r, rootFi, rootPi);
                    if (!IsNullOrDestroyed(child))
                        currentTargets.Add(child);
                }
            }

            for (int s = 1; s < segments.Length - 1; s++)
            {
                if (currentTargets.Count == 0)
                {
                    error = null;
                    return true;
                }
                var nextTargets = new List<object>();
                foreach (object obj in currentTargets)
                {
                    GetMember(obj.GetType(), segments[s], out FieldInfo fi, out PropertyInfo pi);
                    if (fi == null && pi == null)
                    {
                        error = $"member '{segments[s]}' not found on '{obj.GetType().FullName}'";
                        return false;
                    }
                    if (pi != null && pi.GetGetMethod(true) == null)
                    {
                        error = $"member '{segments[s]}' on '{obj.GetType().FullName}' is write-only";
                        return false;
                    }
                    object child = ReadMember(obj, fi, pi);
                    if (!IsNullOrDestroyed(child))
                        nextTargets.Add(child);
                }
                currentTargets = nextTargets;
            }

            if (currentTargets.Count == 0)
            {
                error = null;
                return true;
            }

            string leafName = segments[segments.Length - 1];
            foreach (object leafTarget in currentTargets)
            {
                GetMember(leafTarget.GetType(), leafName, out FieldInfo leafFi, out PropertyInfo leafPi);
                if (leafFi == null && leafPi == null)
                {
                    error = $"member '{leafName}' not found on '{leafTarget.GetType().FullName}'";
                    return false;
                }
                if (leafPi != null && leafPi.GetSetMethod(true) == null)
                {
                    error = $"member '{leafName}' on '{leafTarget.GetType().FullName}' is read-only";
                    return false;
                }
                object converted;
                try
                {
                    converted = ConvertToType(rawValue, GetMemberType(leafFi, leafPi));
                }
                catch (Exception ex)
                {
                    error = $"invalid value for '{leafName}': {ex.Message}";
                    return false;
                }
                WriteMember(leafTarget, leafFi, leafPi, converted);
                updated++;
            }

            error = null;
            return true;
        }

        private static List<object> FindAllInstances(Type targetType)
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
                        if (_findObjectsIncludeInactive != null)
                        {
                            try
                            {
                                found = _findObjectsIncludeInactive.Invoke(null, new object[] { il2cppType, true }) as System.Collections.IEnumerable;
                            }
                            catch { found = null; }
                        }
                        if (found == null)
                        {
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

                        if (results.Count == 0 && typeof(UnityEngine.ScriptableObject).IsAssignableFrom(targetType))
                        {
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
                    if (_findObjectsIncludeInactive != null)
                    {
                        try
                        {
                            found = _findObjectsIncludeInactive.Invoke(null, new object[] { targetType, true }) as System.Collections.IEnumerable;
                        }
                        catch { found = null; }
                    }
                    if (found == null)
                    {
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

                    if (results.Count == 0 && typeof(UnityEngine.ScriptableObject).IsAssignableFrom(targetType))
                    {
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
                    const BindingFlags singletonFlags =
                        BindingFlags.Public | BindingFlags.NonPublic | BindingFlags.Static | BindingFlags.FlattenHierarchy;
                    object singletonVal = null;
                    PropertyInfo instProp = targetType.GetProperty("Instance", singletonFlags)
                        ?? targetType.GetProperty("instance", singletonFlags);
                    if (instProp != null && instProp.GetIndexParameters().Length == 0 && instProp.GetGetMethod(true) != null)
                    {
                        singletonVal = instProp.GetValue(null, null);
                    }
                    else
                    {
                        FieldInfo instField = targetType.GetField("Instance", singletonFlags)
                            ?? targetType.GetField("instance", singletonFlags);
                        if (instField != null)
                        {
                            singletonVal = instField.GetValue(null);
                        }
                    }
                    if (!IsNullOrDestroyed(singletonVal))
                    {
                        results.Add(singletonVal);
                    }
                }
                catch { }
            }

            return results;
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
            return Convert.ChangeType(raw, underlying, CultureInfo.InvariantCulture);
        }

        private void TickTargetLock(TargetLockPin pin)
        {
            pin.LastTickMs = _loopClock.ElapsedMilliseconds;
            UnityEngine.Component playerComp = FindActiveComponent(pin.PlayerType);
            UnityEngine.Component camComp = FindActiveComponent(pin.CameraType);
            if (IsNullOrDestroyed(playerComp) || IsNullOrDestroyed(camComp))
            {
                pin.CurrentlyLocked = false;
                return;
            }

            Type colliderType = pin.RequireCollider ? ResolveType("UnityEngine.Collider") : null;
            Vector3 playerPos = playerComp.transform.position;
            Transform playerRoot = playerComp.transform.root;
            float maxSqr = pin.MaxRange * pin.MaxRange;
            float bestSqr = maxSqr;
            Transform bestTransform = null;

            foreach (object obj in FindAllInstances(pin.TargetType))
            {
                UnityEngine.Component tc = obj as UnityEngine.Component;
                if (IsNullOrDestroyed(tc)) continue;
                GameObject go = tc.gameObject;
                if (go == null || !go.activeInHierarchy) continue;
                if (tc.transform.root == playerRoot) continue;
                if (string.Equals(go.tag, "Player", StringComparison.OrdinalIgnoreCase)) continue;
                if (!IsTargetAlive(tc, pin.AliveChecks)) continue;

                float sqrDist = (tc.transform.position - playerPos).sqrMagnitude;
                if (sqrDist > bestSqr) continue;

                Transform candidateTransform = tc.transform;
                if (colliderType != null)
                {
                    UnityEngine.Component rootCol = GetComponentOn(tc, colliderType, false, out _);
                    if (IsNullOrDestroyed(rootCol))
                    {
                        UnityEngine.Component childCol = GetComponentOn(tc, colliderType, true, out _);
                        if (IsNullOrDestroyed(childCol)) continue;
                        candidateTransform = childCol.transform;
                    }
                }

                bestSqr = sqrDist;
                bestTransform = candidateTransform;
            }

            if (bestTransform != null)
            {
                InvokeCameraLock(camComp, pin, bestTransform);
                pin.CurrentlyLocked = true;
            }
            else if (pin.CurrentlyLocked)
            {
                InvokeCameraUnlockOnComponent(camComp, pin.UnlockMethod);
                pin.CurrentlyLocked = false;
            }
        }

        private static void InvokeCameraLock(UnityEngine.Component camComp, TargetLockPin pin, Transform targetTransform)
        {
            MethodInfo threeArg = null;
            MethodInfo oneArg = null;
            foreach (MethodInfo m in camComp.GetType().GetMethods(InstanceMethodFlags))
            {
                if (m.Name != pin.LockMethod) continue;
                ParameterInfo[] ps = m.GetParameters();
                if (ps.Length == 3 && ps[0].ParameterType.IsAssignableFrom(typeof(UnityEngine.Transform)))
                {
                    threeArg = m;
                    break;
                }
                if (ps.Length == 1 && ps[0].ParameterType.IsAssignableFrom(typeof(UnityEngine.Transform)) && oneArg == null)
                {
                    oneArg = m;
                }
            }

            if (threeArg != null)
            {
                ParameterInfo[] ps = threeArg.GetParameters();
                threeArg.Invoke(camComp, new object[]
                {
                    targetTransform,
                    ConvertToType(pin.HeightOffset, ps[1].ParameterType),
                    ConvertToType(pin.LockSpeed, ps[2].ParameterType)
                });
            }
            else if (oneArg != null)
            {
                oneArg.Invoke(camComp, new object[] { targetTransform });
            }
        }

        private static void InvokeCameraUnlock(TargetLockPin pin)
        {
            UnityEngine.Component camComp = FindActiveComponent(pin.CameraType);
            if (!IsNullOrDestroyed(camComp))
            {
                InvokeCameraUnlockOnComponent(camComp, pin.UnlockMethod);
            }
            pin.CurrentlyLocked = false;
        }

        private static void InvokeCameraUnlockOnComponent(UnityEngine.Component camComp, string unlockMethod)
        {
            if (IsNullOrDestroyed(camComp) || string.IsNullOrEmpty(unlockMethod)) return;
            foreach (MethodInfo m in camComp.GetType().GetMethods(InstanceMethodFlags))
            {
                if (m.Name == unlockMethod && m.GetParameters().Length == 0)
                {
                    m.Invoke(camComp, null);
                    return;
                }
            }
        }

        private int TickDamageAura(DamageAuraPin pin)
        {
            pin.LastTickMs = _loopClock.ElapsedMilliseconds;
            UnityEngine.Component playerComp = FindActiveComponent(pin.PlayerType);
            if (IsNullOrDestroyed(playerComp)) return 0;

            Vector3 playerPos = playerComp.transform.position;
            Transform playerTransform = playerComp.transform;
            Transform playerRoot = playerTransform.root;
            float maxSqr = pin.Radius * pin.Radius;
            int hits = 0;

            foreach (object obj in FindAllInstances(pin.TargetType))
            {
                if (hits >= MaxAuraHitsPerTick) break;
                UnityEngine.Component tc = obj as UnityEngine.Component;
                if (IsNullOrDestroyed(tc)) continue;
                GameObject go = tc.gameObject;
                if (go == null || !go.activeInHierarchy) continue;
                if (tc.transform.root == playerRoot) continue;
                if (string.Equals(go.tag, "Player", StringComparison.OrdinalIgnoreCase)) continue;

                float sqrDist = (tc.transform.position - playerPos).sqrMagnitude;
                if (sqrDist > maxSqr) continue;

                foreach (DamageHandlerSpec handler in pin.Handlers)
                {
                    GetComponentOn(tc, handler.ComponentType, false, out object compObj);
                    if (IsNullOrDestroyed(compObj)) continue;

                    if (!string.IsNullOrEmpty(handler.DeadField) &&
                        TryReadBoolMember(compObj, handler.DeadField, out bool isDead) &&
                        isDead)
                    {
                        break;
                    }
                    if (!string.IsNullOrEmpty(handler.HealthField) &&
                        TryReadDoubleMember(compObj, handler.HealthField, out double hp) &&
                        hp <= 0.0)
                    {
                        break;
                    }

                    if (ApplyDamageHandler(handler, compObj, tc, playerTransform, pin.DamageAmount))
                    {
                        hits++;
                        break;
                    }
                }
            }

            return hits;
        }

        private static bool ApplyDamageHandler(
            DamageHandlerSpec handler,
            object compObj,
            UnityEngine.Component targetComp,
            Transform attackerTransform,
            int damageAmount)
        {
            if (handler.Kind == "invector_health")
            {
                object dmg;
                try
                {
                    dmg = Activator.CreateInstance(handler.DamageType, new object[] { damageAmount });
                }
                catch
                {
                    dmg = Activator.CreateInstance(handler.DamageType);
                    SetOptionalMember(dmg, "damageValue", (float)damageAmount);
                }
                if (dmg == null) return false;

                SetOptionalMember(dmg, "sender", attackerTransform);
                SetOptionalMember(dmg, "receiver", targetComp.transform);
                SetOptionalMember(dmg, "hitPosition", targetComp.transform.position);
                SetOptionalMember(dmg, "ignoreDefense", true);
                SetOptionalMember(dmg, "isCritical", true);

                foreach (MethodInfo m in handler.ComponentType.GetMethods(InstanceMethodFlags))
                {
                    if (m.Name != handler.MethodName) continue;
                    ParameterInfo[] ps = m.GetParameters();
                    if (ps.Length == 1 && ps[0].ParameterType.IsAssignableFrom(handler.DamageType))
                    {
                        m.Invoke(compObj, new object[] { dmg });
                        return true;
                    }
                }
                return false;
            }

            if (handler.Kind == "emerald_ai")
            {
                foreach (MethodInfo m in handler.ComponentType.GetMethods(InstanceMethodFlags))
                {
                    if (m.Name != handler.MethodName) continue;
                    ParameterInfo[] ps = m.GetParameters();
                    if (ps.Length == 5)
                    {
                        Type enumType = Nullable.GetUnderlyingType(ps[1].ParameterType) ?? ps[1].ParameterType;
                        object targetTypePlayer = enumType.IsEnum ? Enum.ToObject(enumType, 0) : null;
                        m.Invoke(compObj, new object[]
                        {
                            ConvertToType(damageAmount, ps[0].ParameterType),
                            targetTypePlayer,
                            attackerTransform,
                            ConvertToType(0, ps[3].ParameterType),
                            ConvertToType(true, ps[4].ParameterType)
                        });
                        return true;
                    }
                }
                return false;
            }

            if (handler.Kind == "invoke_damage_int")
            {
                foreach (MethodInfo m in handler.ComponentType.GetMethods(InstanceMethodFlags))
                {
                    if (m.Name != handler.MethodName) continue;
                    ParameterInfo[] ps = m.GetParameters();
                    if (ps.Length == 1)
                    {
                        m.Invoke(compObj, new object[] { ConvertToType(damageAmount, ps[0].ParameterType) });
                        return true;
                    }
                }
            }

            return false;
        }

        private static UnityEngine.Component FindActiveComponent(Type type)
        {
            foreach (object obj in FindAllInstances(type))
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

        private static bool IsTargetAlive(UnityEngine.Component targetComp, List<AliveCheckSpec> checks)
        {
            if (checks == null || checks.Count == 0) return true;
            bool matchedAny = false;
            foreach (AliveCheckSpec check in checks)
            {
                GetComponentOn(targetComp, check.ComponentType, false, out object compObj);
                if (IsNullOrDestroyed(compObj)) continue;
                matchedAny = true;

                if (!string.IsNullOrEmpty(check.DeadField) &&
                    TryReadBoolMember(compObj, check.DeadField, out bool isDead) &&
                    isDead)
                {
                    return false;
                }
                if (!string.IsNullOrEmpty(check.HealthField) &&
                    TryReadDoubleMember(compObj, check.HealthField, out double hp) &&
                    hp <= 0.0)
                {
                    return false;
                }
            }
            return matchedAny;
        }

        private static bool TryReadBoolMember(object target, string memberName, out bool result)
        {
            result = false;
            try
            {
                GetMember(target.GetType(), memberName, out FieldInfo fi, out PropertyInfo pi);
                if (fi == null && (pi == null || pi.GetGetMethod(true) == null)) return false;
                object raw = ReadMember(target, fi, pi);
                if (raw == null) return false;
                result = Convert.ToBoolean(raw, CultureInfo.InvariantCulture);
                return true;
            }
            catch
            {
                return false;
            }
        }

        private static bool TryReadDoubleMember(object target, string memberName, out double result)
        {
            result = 0.0;
            try
            {
                GetMember(target.GetType(), memberName, out FieldInfo fi, out PropertyInfo pi);
                if (fi == null && (pi == null || pi.GetGetMethod(true) == null)) return false;
                object raw = ReadMember(target, fi, pi);
                if (raw == null) return false;
                result = Convert.ToDouble(raw, CultureInfo.InvariantCulture);
                return true;
            }
            catch
            {
                return false;
            }
        }

        private static void SetOptionalMember(object target, string memberName, object value)
        {
            if (target == null || string.IsNullOrEmpty(memberName)) return;
            try
            {
                GetMember(target.GetType(), memberName, out FieldInfo fi, out PropertyInfo pi);
                if (fi != null)
                {
                    fi.SetValue(target, ConvertToType(value, fi.FieldType));
                }
                else if (pi != null && pi.GetSetMethod(true) != null)
                {
                    pi.SetValue(target, ConvertToType(value, pi.PropertyType), null);
                }
            }
            catch { }
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
