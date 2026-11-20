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
        private sealed class PendingWork
        {
            public string RawLine;
            public string ResponseLine;
            public readonly ManualResetEventSlim Done = new ManualResetEventSlim(false);
        }

        private const int PortScanSpan = 20;
        private readonly Queue<PendingWork> _queue = new Queue<PendingWork>();
        private readonly object _queueLock = new object();
        private readonly Action<string> _log;

        private TcpListener _listener;
        private Thread _acceptThread;
        private volatile bool _running;
        private string _discoveryPath;

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

        public void Stop()
        {
            _running = false;
            try { _listener?.Stop(); } catch { }
            UnpublishDiscoveryFile();
        }

        /// <summary>Must be invoked from MonoBehaviour.Update() on the Unity main thread.</summary>
        public void PumpMainThread()
        {
            while (true)
            {
                PendingWork work;
                lock (_queueLock)
                {
                    if (_queue.Count == 0) return;
                    work = _queue.Dequeue();
                }

                try
                {
                    work.ResponseLine = ProcessRequestOnMainThread(work.RawLine);
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

                    var work = new PendingWork { RawLine = line };
                    lock (_queueLock)
                    {
                        _queue.Enqueue(work);
                    }

                    string reply = work.Done.Wait(5000)
                        ? work.ResponseLine
                        : ErrorJson("main thread dispatch timeout");
                    writer.WriteLine(reply);
                    writer.Flush();
                }
                catch
                {
                    // Connection closed or timed out cleanly.
                }
            }
        }

        private string ProcessRequestOnMainThread(string rawLine)
        {
            if (!(MiniJson.Deserialize(rawLine) is Dictionary<string, object> req))
                return ErrorJson("invalid JSON object");

            string token = GetString(req, "token");
            if (!ConstantTimeEquals(token, AuthToken))
                return ErrorJson("unauthorized token");

            string op = GetString(req, "op") ?? "";
            var p = req.TryGetValue("params", out object pObj)
                ? pObj as Dictionary<string, object>
                : null;

            switch (op)
            {
                case "ping":
                    return OkJson(new Dictionary<string, object>
                    {
                        { "flavor", Flavor },
                        { "loader", LoaderKind },
                        { "pid", Process.GetCurrentProcess().Id },
                        { "port", Port },
                        { "unity_version", Application.unityVersion },
                        { "bridge_version", "0.1.0" }
                    });

                case "list_types":
                    return HandleListTypes(p);

                case "get_field":
                    return HandleGetField(p);

                case "set_field":
                    return HandleSetField(p);

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

            const BindingFlags flags = BindingFlags.Public | BindingFlags.NonPublic | BindingFlags.Static | BindingFlags.Instance | BindingFlags.FlattenHierarchy;
            FieldInfo fi = type.GetField(fieldName, flags);
            PropertyInfo pi = fi == null ? type.GetProperty(fieldName, flags) : null;
            if (fi == null && pi == null)
                return ErrorJson($"member '{fieldName}' not found on '{type.FullName}'");

            bool isStatic = fi != null ? fi.IsStatic : (pi.GetGetMethod(true)?.IsStatic ?? false);
            object target = isStatic ? null : FindFirstInstance(type);
            if (!isStatic && target == null)
                return ErrorJson($"no live instance of '{type.FullName}' found in active scene");

            object val = fi != null ? fi.GetValue(target) : pi.GetValue(target, null);
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

            const BindingFlags flags = BindingFlags.Public | BindingFlags.NonPublic | BindingFlags.Static | BindingFlags.Instance | BindingFlags.FlattenHierarchy;
            FieldInfo fi = type.GetField(fieldName, flags);
            PropertyInfo pi = fi == null ? type.GetProperty(fieldName, flags) : null;
            if (fi == null && pi == null)
                return ErrorJson($"member '{fieldName}' not found on '{type.FullName}'");

            Type memberType = fi != null ? fi.FieldType : pi.PropertyType;
            object converted = ConvertToType(p["value"], memberType);

            bool isStatic = fi != null ? fi.IsStatic : (pi.GetSetMethod(true)?.IsStatic ?? false);
            if (isStatic)
            {
                if (fi != null) fi.SetValue(null, converted);
                else pi.SetValue(null, converted, null);
                return OkJson(new Dictionary<string, object>
                {
                    { "type", type.FullName },
                    { "member", fieldName },
                    { "updated", 1 }
                });
            }

            List<object> instances = FindAllInstances(type);
            if (instances.Count == 0)
                return ErrorJson($"no live instance of '{type.FullName}' found in active scene");

            foreach (object inst in instances)
            {
                if (fi != null) fi.SetValue(inst, converted);
                else pi.SetValue(inst, converted, null);
            }
            return OkJson(new Dictionary<string, object>
            {
                { "type", type.FullName },
                { "member", fieldName },
                { "updated", instances.Count }
            });
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

        private static Type ResolveType(string typeName)
        {
            Type direct = Type.GetType(typeName, false);
            if (direct != null) return direct;

            Type shortMatch = null;
            foreach (Assembly asm in AppDomain.CurrentDomain.GetAssemblies())
            {
                try
                {
                    Type exact = asm.GetType(typeName, false);
                    if (exact != null) return exact;
                    if (shortMatch == null)
                    {
                        foreach (Type t in asm.GetTypes())
                        {
                            if (t != null && (t.FullName == typeName || t.Name == typeName))
                            {
                                shortMatch = t;
                                break;
                            }
                        }
                    }
                }
                catch { }
            }
            return shortMatch;
        }

        private static object FindFirstInstance(Type targetType)
        {
            List<object> all = FindAllInstances(targetType);
            return all.Count > 0 ? all[0] : null;
        }

        private static List<object> FindAllInstances(Type targetType)
        {
            var results = new List<object>();
            if (!typeof(UnityEngine.Object).IsAssignableFrom(targetType)) return results;

            try
            {
#if IL2CPP
                var il2cppType = Il2CppInterop.Runtime.Il2CppType.From(targetType, false);
                if (il2cppType != null)
                {
                    var found = UnityEngine.Object.FindObjectsOfType(il2cppType);
                    if (found != null)
                    {
                        foreach (UnityEngine.Object obj in found)
                        {
                            if (obj == null) continue;
                            object wrapped = Activator.CreateInstance(targetType, new object[] { obj.Pointer });
                            if (wrapped != null) results.Add(wrapped);
                        }
                    }
                }
#else
                UnityEngine.Object[] found = UnityEngine.Object.FindObjectsOfType(targetType);
                if (found != null)
                {
                    foreach (UnityEngine.Object obj in found)
                    {
                        if (obj != null) results.Add(obj);
                    }
                }
#endif
            }
            catch { }
            return results;
        }

        private static object ConvertToType(object raw, Type targetType)
        {
            if (raw == null) return null;
            Type underlying = Nullable.GetUnderlyingType(targetType) ?? targetType;
            if (underlying.IsInstanceOfType(raw)) return raw;
            if (underlying == typeof(float)) return Convert.ToSingle(raw, CultureInfo.InvariantCulture);
            if (underlying == typeof(double)) return Convert.ToDouble(raw, CultureInfo.InvariantCulture);
            if (underlying == typeof(int)) return Convert.ToInt32(raw, CultureInfo.InvariantCulture);
            if (underlying == typeof(long)) return Convert.ToInt64(raw, CultureInfo.InvariantCulture);
            if (underlying == typeof(bool)) return Convert.ToBoolean(raw, CultureInfo.InvariantCulture);
            if (underlying == typeof(string)) return Convert.ToString(raw, CultureInfo.InvariantCulture);
            if (underlying.IsEnum) return Enum.Parse(underlying, Convert.ToString(raw, CultureInfo.InvariantCulture), true);
            return Convert.ChangeType(raw, underlying, CultureInfo.InvariantCulture);
        }

        private void PublishDiscoveryFile()
        {
            try
            {
                string baseDir = Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData);
                if (string.IsNullOrEmpty(baseDir)) baseDir = Path.GetTempPath();
                string dir = Path.Combine(Path.Combine(baseDir, "Nekod"), "bridges");
                Directory.CreateDirectory(dir);
                int pid = Process.GetCurrentProcess().Id;
                _discoveryPath = Path.Combine(dir, $"unity-{pid}.json");

                var record = new Dictionary<string, object>
                {
                    { "pid", pid },
                    { "port", Port },
                    { "flavor", Flavor },
                    { "loader", LoaderKind },
                    { "auth_token", AuthToken },
                    { "unity_version", Application.unityVersion },
                    { "bridge_version", "0.1.0" }
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
