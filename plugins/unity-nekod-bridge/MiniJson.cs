using System;
using System.Collections;
using System.Collections.Generic;
using System.Globalization;
using System.Text;

namespace Nekod.UnityBridge
{
    /// <summary>
    /// Minimal zero-dependency RFC 8259 JSON parser/serializer so NekodUnityBridge compiles
    /// cleanly across both BepInEx 5 Mono (netstandard2.0) and BepInEx 6 IL2CPP (net6.0)
    /// without UnityExplorer, UniverseLib, or Newtonsoft.Json.
    /// </summary>
    internal static class MiniJson
    {
        public static object Deserialize(string json)
        {
            if (string.IsNullOrEmpty(json)) return null;
            int index = 0;
            return ParseValue(json, ref index);
        }

        public static string Serialize(object obj)
        {
            var sb = new StringBuilder(128);
            WriteValue(obj, sb);
            return sb.ToString();
        }

        private static void SkipWhitespace(string s, ref int i)
        {
            while (i < s.Length && char.IsWhiteSpace(s[i])) i++;
        }

        private static object ParseValue(string s, ref int i)
        {
            SkipWhitespace(s, ref i);
            if (i >= s.Length) return null;
            char c = s[i];
            if (c == '{') return ParseObject(s, ref i);
            if (c == '[') return ParseArray(s, ref i);
            if (c == '"') return ParseString(s, ref i);
            if (c == 't' && s.Length - i >= 4 && s.Substring(i, 4) == "true") { i += 4; return true; }
            if (c == 'f' && s.Length - i >= 5 && s.Substring(i, 5) == "false") { i += 5; return false; }
            if (c == 'n' && s.Length - i >= 4 && s.Substring(i, 4) == "null") { i += 4; return null; }
            return ParseNumber(s, ref i);
        }

        private static Dictionary<string, object> ParseObject(string s, ref int i)
        {
            var map = new Dictionary<string, object>(StringComparer.Ordinal);
            i++; // '{'
            while (i < s.Length)
            {
                SkipWhitespace(s, ref i);
                if (i < s.Length && s[i] == '}') { i++; return map; }
                string key = ParseString(s, ref i);
                SkipWhitespace(s, ref i);
                if (i >= s.Length || s[i] != ':') throw new FormatException("Expected ':' in JSON object");
                i++;
                object val = ParseValue(s, ref i);
                map[key] = val;
                SkipWhitespace(s, ref i);
                if (i < s.Length && s[i] == ',') { i++; continue; }
                if (i < s.Length && s[i] == '}') { i++; return map; }
                throw new FormatException("Expected ',' or '}' in JSON object");
            }
            throw new FormatException("Unterminated JSON object");
        }

        private static List<object> ParseArray(string s, ref int i)
        {
            var list = new List<object>();
            i++; // '['
            while (i < s.Length)
            {
                SkipWhitespace(s, ref i);
                if (i < s.Length && s[i] == ']') { i++; return list; }
                list.Add(ParseValue(s, ref i));
                SkipWhitespace(s, ref i);
                if (i < s.Length && s[i] == ',') { i++; continue; }
                if (i < s.Length && s[i] == ']') { i++; return list; }
                throw new FormatException("Expected ',' or ']' in JSON array");
            }
            throw new FormatException("Unterminated JSON array");
        }

        private static string ParseString(string s, ref int i)
        {
            if (i >= s.Length || s[i] != '"') throw new FormatException("Expected '\"'");
            i++;
            var sb = new StringBuilder();
            while (i < s.Length)
            {
                char c = s[i++];
                if (c == '"') return sb.ToString();
                if (c == '\\' && i < s.Length)
                {
                    char esc = s[i++];
                    switch (esc)
                    {
                        case '"': sb.Append('"'); break;
                        case '\\': sb.Append('\\'); break;
                        case '/': sb.Append('/'); break;
                        case 'b': sb.Append('\b'); break;
                        case 'f': sb.Append('\f'); break;
                        case 'n': sb.Append('\n'); break;
                        case 'r': sb.Append('\r'); break;
                        case 't': sb.Append('\t'); break;
                        case 'u':
                            if (i + 4 <= s.Length)
                            {
                                string hex = s.Substring(i, 4);
                                sb.Append((char)Convert.ToInt32(hex, 16));
                                i += 4;
                            }
                            break;
                        default: sb.Append(esc); break;
                    }
                }
                else
                {
                    sb.Append(c);
                }
            }
            throw new FormatException("Unterminated JSON string");
        }

        private static object ParseNumber(string s, ref int i)
        {
            int start = i;
            bool isFloat = false;
            if (i < s.Length && (s[i] == '-' || s[i] == '+')) i++;
            while (i < s.Length)
            {
                char c = s[i];
                if (char.IsDigit(c)) { i++; continue; }
                if (c == '.' || c == 'e' || c == 'E' || c == '+' || c == '-')
                {
                    isFloat = true;
                    i++;
                    continue;
                }
                break;
            }
            string raw = s.Substring(start, i - start);
            if (!isFloat && long.TryParse(raw, NumberStyles.Integer, CultureInfo.InvariantCulture, out long l))
                return l;
            if (double.TryParse(raw, NumberStyles.Float, CultureInfo.InvariantCulture, out double d))
                return d;
            throw new FormatException("Invalid JSON number: " + raw);
        }

        private static void WriteValue(object value, StringBuilder sb)
        {
            if (value == null) { sb.Append("null"); return; }
            if (value is bool b) { sb.Append(b ? "true" : "false"); return; }
            if (value is string str) { WriteString(str, sb); return; }
            if (value is int i32) { sb.Append(i32.ToString(CultureInfo.InvariantCulture)); return; }
            if (value is long i64) { sb.Append(i64.ToString(CultureInfo.InvariantCulture)); return; }
            if (value is float f32) { sb.Append(f32.ToString("R", CultureInfo.InvariantCulture)); return; }
            if (value is double f64) { sb.Append(f64.ToString("R", CultureInfo.InvariantCulture)); return; }
            if (value is IDictionary dict)
            {
                sb.Append('{');
                bool first = true;
                foreach (DictionaryEntry entry in dict)
                {
                    if (!first) sb.Append(',');
                    first = false;
                    WriteString(Convert.ToString(entry.Key, CultureInfo.InvariantCulture) ?? "", sb);
                    sb.Append(':');
                    WriteValue(entry.Value, sb);
                }
                sb.Append('}');
                return;
            }
            if (value is IEnumerable seq)
            {
                sb.Append('[');
                bool first = true;
                foreach (object item in seq)
                {
                    if (!first) sb.Append(',');
                    first = false;
                    WriteValue(item, sb);
                }
                sb.Append(']');
                return;
            }
            WriteString(Convert.ToString(value, CultureInfo.InvariantCulture) ?? "", sb);
        }

        private static void WriteString(string str, StringBuilder sb)
        {
            sb.Append('"');
            foreach (char c in str)
            {
                switch (c)
                {
                    case '"': sb.Append("\\\""); break;
                    case '\\': sb.Append("\\\\"); break;
                    case '\b': sb.Append("\\b"); break;
                    case '\f': sb.Append("\\f"); break;
                    case '\n': sb.Append("\\n"); break;
                    case '\r': sb.Append("\\r"); break;
                    case '\t': sb.Append("\\t"); break;
                    default:
                        if (c < 0x20)
                            sb.Append("\\u").Append(((int)c).ToString("x4", CultureInfo.InvariantCulture));
                        else
                            sb.Append(c);
                        break;
                }
            }
            sb.Append('"');
        }
    }
}
