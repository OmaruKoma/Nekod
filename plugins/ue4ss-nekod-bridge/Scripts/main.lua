-- Nekod Companion Bridge v0.7.3 for RE-UE4SS (`ue4ss_bridge_v1` + MCP introspection + UI localization + sticky properties + TargetLock + AutoFire + ThirdPerson SpinBot + DamageAura + FillItems + TriggerGoal)
-- Deployed automatically by Nekod into <Game>\<Project>\Binaries\Win64\ue4ss\Mods\NekodBridge\Scripts\main.lua
-- Zero external dependencies (pure-Lua JSON + single-threaded GameThread tick pump on stock RE-UE4SS; optional LuaSocket TCP).

local BRIDGE_VERSION = "0.7.3"
local PREFERRED_PORT = 19541
local PORT_SCAN_SPAN = 20

local json_ok, dkjson = pcall(require, "dkjson")
local socket_ok, socket = pcall(require, "socket")

-- ---------------------------------------------------------------------------
-- Pure-Lua JSON (RFC 8259 subset, zero external dependencies)
-- ---------------------------------------------------------------------------
local Json = {}
do
    local math_floor = math.floor
    local string_format = string.format
    local string_byte = string.byte
    local string_char = string.char
    local string_sub = string.sub
    local table_concat = table.concat

    local ESCAPES = {
        ['"'] = '\\"',
        ['\\'] = '\\\\',
        ['\b'] = '\\b',
        ['\f'] = '\\f',
        ['\n'] = '\\n',
        ['\r'] = '\\r',
        ['\t'] = '\\t',
    }

    local function encode_str(s)
        return '"' .. s:gsub('[%z\1-\31\\"]', function(c)
            return ESCAPES[c] or string_format("\\u%04x", string_byte(c))
        end) .. '"'
    end

    local function is_array(tbl)
        local count = 0
        local max_idx = 0
        for k, _ in pairs(tbl) do
            if type(k) ~= "number" or k < 1 or math_floor(k) ~= k then
                return false, 0
            end
            count = count + 1
            if k > max_idx then max_idx = k end
        end
        if max_idx ~= count then
            return false, 0
        end
        return true, max_idx
    end

    local function encode_val(v, seen)
        local t = type(v)
        if v == nil then
            return "null"
        elseif t == "boolean" then
            return v and "true" or "false"
        elseif t == "number" then
            if v ~= v or v == math.huge or v == -math.huge then
                return "null"
            end
            return tostring(v)
        elseif t == "string" then
            return encode_str(v)
        elseif t == "table" then
            if seen[v] then
                return '"[circular]"'
            end
            seen[v] = true
            local parts = {}
            local arr, arr_len = is_array(v)
            local out
            if arr then
                for i = 1, arr_len do
                    parts[#parts + 1] = encode_val(v[i], seen)
                end
                out = "[" .. table_concat(parts, ",") .. "]"
            else
                for k, val in pairs(v) do
                    if type(k) == "string" or type(k) == "number" then
                        parts[#parts + 1] = encode_str(tostring(k)) .. ":" .. encode_val(val, seen)
                    end
                end
                out = "{" .. table_concat(parts, ",") .. "}"
            end
            seen[v] = nil
            return out
        else
            return encode_str(tostring(v))
        end
    end

    function Json.encode(v)
        if json_ok and dkjson and dkjson.encode then
            local ok, res = pcall(dkjson.encode, v)
            if ok and type(res) == "string" then
                return res
            end
        end
        return encode_val(v, {})
    end

    local function utf8_from_codepoint(cp)
        if cp <= 0x7F then
            return string_char(cp)
        elseif cp <= 0x7FF then
            return string_char(
                0xC0 + math_floor(cp / 64),
                0x80 + (cp % 64)
            )
        elseif cp <= 0xFFFF then
            return string_char(
                0xE0 + math_floor(cp / 4096),
                0x80 + (math_floor(cp / 64) % 64),
                0x80 + (cp % 64)
            )
        else
            return "?"
        end
    end

    local function skip_ws(str, pos)
        local _, next_pos = str:find("^[ \t\r\n]*", pos)
        return (next_pos or (pos - 1)) + 1
    end

    local parse_val

    local function parse_str(str, pos)
        pos = pos + 1
        local parts = {}
        local len = #str
        while pos <= len do
            local ch = string_sub(str, pos, pos)
            if ch == '"' then
                return table_concat(parts), pos + 1
            elseif ch == '\\' then
                local esc = string_sub(str, pos + 1, pos + 1)
                if esc == '"' or esc == '\\' or esc == '/' then
                    parts[#parts + 1] = esc
                    pos = pos + 2
                elseif esc == 'b' then parts[#parts + 1] = '\b'; pos = pos + 2
                elseif esc == 'f' then parts[#parts + 1] = '\f'; pos = pos + 2
                elseif esc == 'n' then parts[#parts + 1] = '\n'; pos = pos + 2
                elseif esc == 'r' then parts[#parts + 1] = '\r'; pos = pos + 2
                elseif esc == 't' then parts[#parts + 1] = '\t'; pos = pos + 2
                elseif esc == 'u' then
                    local hex = string_sub(str, pos + 2, pos + 5)
                    local cp = tonumber(hex, 16)
                    if not cp then return nil, pos, "invalid unicode escape" end
                    parts[#parts + 1] = utf8_from_codepoint(cp)
                    pos = pos + 6
                else
                    return nil, pos, "invalid escape"
                end
            else
                local next_special = str:find('[\\"]', pos)
                if not next_special then
                    return nil, pos, "unterminated string"
                end
                parts[#parts + 1] = string_sub(str, pos, next_special - 1)
                pos = next_special
            end
        end
        return nil, pos, "unterminated string"
    end

    local function parse_num(str, pos)
        local s, e = str:find("^-?%d+%.?%d*[eE]?[+-]?%d*", pos)
        if not s then return nil, pos, "invalid number" end
        local n = tonumber(string_sub(str, s, e))
        if n == nil then return nil, pos, "invalid number" end
        return n, e + 1
    end

    local function parse_arr(str, pos)
        pos = skip_ws(str, pos + 1)
        local arr = {}
        if string_sub(str, pos, pos) == ']' then
            return arr, pos + 1
        end
        local idx = 1
        while true do
            local val, npos, err = parse_val(str, pos)
            if err then return nil, npos, err end
            arr[idx] = val
            idx = idx + 1
            pos = skip_ws(str, npos)
            local ch = string_sub(str, pos, pos)
            if ch == ']' then
                return arr, pos + 1
            elseif ch == ',' then
                pos = skip_ws(str, pos + 1)
            else
                return nil, pos, "expected ',' or ']'"
            end
        end
    end

    local function parse_obj(str, pos)
        pos = skip_ws(str, pos + 1)
        local obj = {}
        if string_sub(str, pos, pos) == '}' then
            return obj, pos + 1
        end
        while true do
            if string_sub(str, pos, pos) ~= '"' then
                return nil, pos, "expected string key"
            end
            local key, kpos, kerr = parse_str(str, pos)
            if kerr then return nil, kpos, kerr end
            pos = skip_ws(str, kpos)
            if string_sub(str, pos, pos) ~= ':' then
                return nil, pos, "expected ':'"
            end
            pos = skip_ws(str, pos + 1)
            local val, vpos, verr = parse_val(str, pos)
            if verr then return nil, vpos, verr end
            obj[key] = val
            pos = skip_ws(str, vpos)
            local ch = string_sub(str, pos, pos)
            if ch == '}' then
                return obj, pos + 1
            elseif ch == ',' then
                pos = skip_ws(str, pos + 1)
            else
                return nil, pos, "expected ',' or '}'"
            end
        end
    end

    parse_val = function(str, pos)
        pos = skip_ws(str, pos)
        local ch = string_sub(str, pos, pos)
        if ch == '{' then return parse_obj(str, pos)
        elseif ch == '[' then return parse_arr(str, pos)
        elseif ch == '"' then return parse_str(str, pos)
        elseif ch == 't' and string_sub(str, pos, pos + 3) == "true" then return true, pos + 4
        elseif ch == 'f' and string_sub(str, pos, pos + 4) == "false" then return false, pos + 5
        elseif ch == 'n' and string_sub(str, pos, pos + 3) == "null" then return nil, pos + 4
        elseif ch == '-' or (ch >= '0' and ch <= '9') then return parse_num(str, pos)
        else return nil, pos, "unexpected character in JSON" end
    end

    function Json.decode(str)
        if type(str) ~= "string" or str == "" then return nil end
        if json_ok and dkjson and dkjson.decode then
            local ok, res = pcall(dkjson.decode, str)
            if ok and type(res) == "table" then
                return res
            end
        end
        local ok, val, _, err = pcall(parse_val, str, 1)
        if not ok or err then return nil end
        return val
    end
end

-- ---------------------------------------------------------------------------
-- Token, discovery directory, and atomic file helpers (no cmd.exe spawning)
-- ---------------------------------------------------------------------------
local function generate_token()
    local env_tok = os.getenv("NEKOD_UE4SS_TOKEN")
    if type(env_tok) == "string" and #env_tok >= 16 and #env_tok <= 128 then
        return env_tok
    end
    local seed = (os.time() or 1) * 1664525
        + math.floor((os.clock() or 0) * 1000000)
        + tonumber(tostring({}):match("0x(%x+)") or "1337", 16)
        + tonumber(tostring({}):match("0x(%x+)") or "7331", 16)
    local x = (seed % 4294967295) + 1
    local parts = {}
    for i = 1, 4 do
        x = (x * 1103515245 + 12345 + i * 7919) % 4294967296
        parts[i] = string.format("%08x", math.floor(x))
    end
    return table.concat(parts)
end

local AUTH_TOKEN = generate_token()

local function get_bridge_dir()
    local custom = os.getenv("NEKOD_UE4SS_BRIDGE_DIR")
    if type(custom) == "string" and custom ~= "" then
        return custom:gsub("[\\/]+$", "")
    end
    local base = os.getenv("LOCALAPPDATA") or os.getenv("TEMP") or "."
    return base:gsub("[\\/]+$", "") .. "\\Nekod\\bridges"
end

local BRIDGE_DIR = get_bridge_dir()
local DISCOVERY_ACTIVE_PATH = BRIDGE_DIR .. "\\ue4ss-active.json"
local REQUEST_PATH = BRIDGE_DIR .. "\\ue4ss-request.json"

local function write_file_atomic(target_path, content)
    local tmp_path = target_path .. ".tmp"
    local f, err = io.open(tmp_path, "wb")
    if not f then
        return false, tostring(err)
    end
    f:write(content)
    f:flush()
    f:close()
    pcall(os.remove, target_path)
    local ok_ren, ren_err = os.rename(tmp_path, target_path)
    if not ok_ren then
        -- Fallback direct write if rename is blocked briefly by AV scanner
        local direct = io.open(target_path, "wb")
        if not direct then
            pcall(os.remove, tmp_path)
            return false, tostring(ren_err)
        end
        direct:write(content)
        direct:flush()
        direct:close()
        pcall(os.remove, tmp_path)
    end
    return true
end

local function safe_engine_version()
    local ok, major, minor = pcall(function()
        if UEHelpers and UEHelpers.GetUnrealVersion then
            return UEHelpers.GetUnrealVersion()
        end
        return nil
    end)
    if ok and major then
        return tostring(major) .. "." .. tostring(minor or 0)
    end
    return "4.27"
end

local function safe_to_json_value(val)
    local t = type(val)
    if val == nil or t == "boolean" or t == "number" or t == "string" then
        return val, t
    end
    if t == "userdata" then
        local ok_valid, valid = pcall(function() return val.IsValid and val:IsValid() end)
        if ok_valid and not valid then
            return nil, "userdata(invalid)"
        end
        local ok_name, name = pcall(function()
            if val.GetFullName then return val:GetFullName() end
            if val.ToString then return val:ToString() end
            return tostring(val)
        end)
        return ok_name and name or tostring(val), "userdata"
    end
    return tostring(val), t
end

local function sanitize_request_id(raw_id)
    if type(raw_id) ~= "string" then return nil end
    local clean = raw_id:gsub("[^%w_%-]", "")
    if clean == "" or #clean > 64 then return nil end
    return clean
end

local function find_target_instances(class_name, limit)
    local found = {}
    local ok_all, list = pcall(FindAllOf, class_name)
    if ok_all and type(list) == "table" then
        for _, obj in ipairs(list) do
            local ok_v, valid = pcall(function() return obj and obj:IsValid() end)
            if ok_v and valid then
                found[#found + 1] = obj
                if limit and #found >= limit then break end
            end
        end
    end
    if #found == 0 then
        local ok_first, single = pcall(FindFirstOf, class_name)
        if ok_first and single then
            local ok_v, valid = pcall(function() return single:IsValid() end)
            if ok_v and valid then
                found[1] = single
            end
        end
    end
    return found
end

local function resolve_uclass(object_class)
    local insts = find_target_instances(object_class, 1)
    if #insts > 0 then
        local ok_c, cls = pcall(function() return insts[1]:GetClass() end)
        if ok_c and cls and cls:IsValid() then
            return cls, insts[1]
        end
    end
    local ok_find, obj = pcall(StaticFindObject, object_class)
    if ok_find and obj then
        local ok_v, valid = pcall(function() return obj:IsValid() end)
        if ok_v and valid then
            local ok_cls, cls = pcall(function() return obj.ForEachProperty and obj or obj:GetClass() end)
            if ok_cls and cls then return cls, nil end
        end
    end
    return nil, nil
end

-- ---------------------------------------------------------------------------
-- Runtime UMG UI Localization Engine (TextBlock / RichTextBlock + CJK fallback)
-- ---------------------------------------------------------------------------
local ACTIVE_LOCALIZATIONS = {}
local ORIGINAL_TEXT_RECORDS = {}
local UNTRANSLATED_TEXTS = {}
local UNTRANSLATED_LIST = {}
local CACHED_FALLBACK_FONTS = {}
local LOCALIZATION_HOOK_REGISTERED = false
local LOCALIZATION_SCAN_INTERVAL_MS = 500
local IN_LOCALIZATION_SET_TEXT = false
local LOCALIZATION_DIRTY = false

local function is_archetype_name(full_name)
    if type(full_name) ~= "string" then return true end
    if full_name:find("Default__", 1, true) ~= nil then return true end
    if full_name:find("_C:WidgetTree.", 1, true) ~= nil then return true end
    if full_name:find("_GEN_VARIABLE", 1, true) ~= nil then return true end
    if full_name:find("SKEL_", 1, true) ~= nil then return true end
    if full_name:find("REINST_", 1, true) ~= nil then return true end
    return false
end

local is_archetype_widget_name = is_archetype_name

local function find_live_instances(class_name, limit)
    local found = {}
    if type(class_name) ~= "string" or class_name == "" then return found end
    local ok_all, list = pcall(FindAllOf, class_name)
    if ok_all and type(list) == "table" then
        for _, obj in ipairs(list) do
            local ok_v, valid = pcall(function() return obj and obj:IsValid() end)
            if ok_v and valid then
                local ok_fn, fname = pcall(function() return obj:GetFullName() end)
                if ok_fn and not is_archetype_name(fname) then
                    found[#found + 1] = obj
                    if limit and #found >= limit then break end
                end
            end
        end
    end
    return found
end

local function resolve_owner_and_leaf(root_obj, dotted_prop)
    if not root_obj or type(dotted_prop) ~= "string" or dotted_prop == "" then
        return nil, nil
    end
    local parts = {}
    for seg in dotted_prop:gmatch("[^%.]+") do
        parts[#parts + 1] = seg
    end
    if #parts == 0 then return nil, nil end
    local cur = root_obj
    for i = 1, #parts - 1 do
        local seg = parts[i]
        local ok_get, next_obj = pcall(function() return cur[seg] end)
        if not ok_get or not next_obj then
            return nil, nil
        end
        if type(next_obj) == "userdata" and next_obj.IsValid then
            local ok_v, valid = pcall(function() return next_obj:IsValid() end)
            if not ok_v or not valid then
                return nil, nil
            end
        end
        cur = next_obj
    end
    return cur, parts[#parts]
end

local function object_matches_filter(obj, filter_prop)
    if type(filter_prop) ~= "table" or type(filter_prop.name) ~= "string" or filter_prop.name == "" then
        return true
    end
    local ok_read, actual = pcall(function() return obj[filter_prop.name] end)
    if not ok_read then
        return false
    end
    return actual == filter_prop.equals
end

local function resolve_player_controller_and_pawn(player_class)
    local pc = nil
    local pcs = find_live_instances("PlayerController", 8)
    for _, cand in ipairs(pcs) do
        local ok_local, is_local = pcall(function() return cand:IsLocalController() end)
        if ok_local and is_local then
            pc = cand
            break
        end
    end
    if not pc then pc = pcs[1] end
    if not pc then return nil, nil end

    local player = nil
    if type(player_class) == "string" and player_class ~= "" then
        player = find_live_instances(player_class, 1)[1]
    else
        local ok_p, p = pcall(function() return pc:K2_GetPawn() end)
        if ok_p and p and p:IsValid() then
            player = p
        else
            pcall(function()
                if pc.Pawn and pc.Pawn:IsValid() then player = pc.Pawn end
            end)
        end
    end
    if not player then return pc, nil end
    local ok_pv, pvalid = pcall(function() return player:IsValid() end)
    if not ok_pv or not pvalid then return pc, nil end
    return pc, player
end

local function contains_cjk_utf8(s)
    if type(s) ~= "string" then return false end
    local i = 1
    local len = #s
    while i <= len - 2 do
        local b1 = s:byte(i)
        if b1 >= 0xE4 and b1 <= 0xE9 then
            local b2, b3 = s:byte(i + 1, i + 2)
            if b2 and b3 and b2 >= 0x80 and b2 <= 0xBF and b3 >= 0x80 and b3 <= 0xBF then
                return true
            end
        end
        i = i + 1
    end
    return false
end

local function has_non_ascii(s)
    return type(s) == "string" and s:find("[\128-\255]") ~= nil
end

local function record_untranslated_sample(core_text)
    if type(core_text) ~= "string" or core_text == "" then return end
    if #core_text > 240 or not core_text:find("[%a]") then return end
    if contains_cjk_utf8(core_text) then return end
    if UNTRANSLATED_TEXTS[core_text] then return end
    UNTRANSLATED_TEXTS[core_text] = true
    if #UNTRANSLATED_LIST < 200 then
        UNTRANSLATED_LIST[#UNTRANSLATED_LIST + 1] = core_text
    end
end

local function read_text_block_string(tb)
    if not tb then return nil end
    if tb.GetText then
        local ok_gt, ft = pcall(function() return tb:GetText() end)
        if ok_gt and ft ~= nil then
            local ok_ts, s = pcall(function()
                if type(ft) == "string" then return ft end
                if ft.ToString then return ft:ToString() end
                return tostring(ft)
            end)
            if ok_ts and type(s) == "string" then return s end
        end
    end
    local ok_p, val = pcall(function() return tb.Text end)
    if ok_p and val ~= nil then
        local ok_ts, s = pcall(function()
            if type(val) == "string" then return val end
            if val.ToString then return val:ToString() end
            return tostring(val)
        end)
        if ok_ts and type(s) == "string" then return s end
    end
    return nil
end

local function read_font_summary(tb)
    local ok, desc = pcall(function()
        local f = tb.Font
        if not f then return nil end
        local fo = f.FontObject
        local fo_name = (fo and fo.IsValid and fo:IsValid() and fo.GetFullName) and fo:GetFullName() or tostring(fo)
        local tf = f.TypefaceFontName and f.TypefaceFontName.ToString and f.TypefaceFontName:ToString() or ""
        if tf ~= "" and tf ~= "None" then
            return fo_name .. " (" .. tf .. ")"
        end
        return fo_name
    end)
    return ok and desc or nil
end

local function resolve_fallback_font(fallback_path)
    if type(fallback_path) ~= "string" or fallback_path == "" then return nil end
    local cached = CACHED_FALLBACK_FONTS[fallback_path]
    if cached then
        local ok_v, valid = pcall(function() return cached:IsValid() end)
        if ok_v and valid then return cached end
        CACHED_FALLBACK_FONTS[fallback_path] = nil
    end
    local ok_find, obj = pcall(StaticFindObject, fallback_path)
    if ok_find and obj then
        local ok_v, valid = pcall(function() return obj:IsValid() end)
        if ok_v and valid then
            CACHED_FALLBACK_FONTS[fallback_path] = obj
            return obj
        end
    end
    return nil
end

local function translate_text_via_specs(raw)
    if type(raw) ~= "string" or raw == "" then return nil, nil end
    local lead, core, trail = raw:match("^(%s*)(.-)(%s*)$")
    if not core or core == "" then return nil, nil end

    for _, spec in pairs(ACTIVE_LOCALIZATIONS) do
        if spec.target_outputs[core] then
            return nil, nil
        end
    end

    local matched = nil
    local font_path = nil

    for _, spec in pairs(ACTIVE_LOCALIZATIONS) do
        local hit = spec.entries_exact[core]
        if not hit and spec.case_insensitive then
            hit = spec.entries_ci[core:lower()]
        end
        if hit then
            matched = hit
            font_path = spec.font_fallback
            break
        end
    end

    if not matched and not contains_cjk_utf8(core) then
        for _, spec in pairs(ACTIVE_LOCALIZATIONS) do
            for _, rule in ipairs(spec.prefix_rules) do
                local plen = #rule.prefix
                if plen > 0 and #core >= plen then
                    local head = core:sub(1, plen)
                    local eq = (head == rule.prefix) or (spec.case_insensitive and head:lower() == rule.prefix_lower)
                    if eq then
                        matched = rule.replacement .. core:sub(plen + 1)
                        font_path = spec.font_fallback
                        break
                    end
                end
            end
            if matched then break end
        end
    end

    if matched then
        return lead .. matched .. trail, font_path
    end

    record_untranslated_sample(core)
    return nil, nil
end

local function set_widget_text_safe(tb, text_str)
    if IN_LOCALIZATION_SET_TEXT then return false end
    IN_LOCALIZATION_SET_TEXT = true
    local ok_all, res = pcall(function()
        local ftext_val = FText and FText(text_str) or text_str
        if tb.SetText then
            tb:SetText(ftext_val)
            return true
        end
        tb.Text = ftext_val
        return true
    end)
    IN_LOCALIZATION_SET_TEXT = false
    return ok_all and res == true
end

local function apply_cjk_font_fallback(tb, fallback_path, rec)
    if type(fallback_path) ~= "string" or fallback_path == "" then return false end
    local fallback_font = resolve_fallback_font(fallback_path)
    if not fallback_font then return false end

    local ok, changed = pcall(function()
        local font_info = tb.Font
        if not font_info then return false end
        local cur_obj = font_info.FontObject
        local cur_name = (cur_obj and cur_obj.IsValid and cur_obj:IsValid() and cur_obj.GetFullName)
            and cur_obj:GetFullName() or nil
        local fb_name = fallback_font.GetFullName and fallback_font:GetFullName() or nil
        if cur_name and fb_name and cur_name == fb_name then
            return false
        end
        if not rec.font_patched then
            rec.orig_font_object = cur_obj
            pcall(function() rec.orig_typeface_name = font_info.TypefaceFontName end)
            rec.font_patched = true
        end
        font_info.FontObject = fallback_font
        if FName then
            pcall(function() font_info.TypefaceFontName = FName("Regular") end)
        end
        if tb.SetFont then
            local ok_sf = pcall(function() tb:SetFont(font_info) end)
            if not ok_sf then
                tb.Font = font_info
            end
        else
            tb.Font = font_info
        end
        return true
    end)
    return ok and changed == true
end

local function apply_localization_to_widget(tb)
    if next(ACTIVE_LOCALIZATIONS) == nil then return false end
    local ok_v, valid = pcall(function() return tb and tb:IsValid() end)
    if not ok_v or not valid then return false end
    local ok_fn, full_name = pcall(function() return tb:GetFullName() end)
    if not ok_fn or is_archetype_widget_name(full_name) then return false end

    local raw_text = read_text_block_string(tb)
    if type(raw_text) ~= "string" or raw_text == "" then return false end

    local translated, font_path = translate_text_via_specs(raw_text)
    if not translated or translated == raw_text then return false end

    local rec = ORIGINAL_TEXT_RECORDS[full_name]
    if not rec then
        rec = {
            obj = tb,
            original_text = raw_text,
            translated_text = translated,
            font_patched = false,
            orig_font_object = nil,
            orig_typeface_name = nil,
        }
        ORIGINAL_TEXT_RECORDS[full_name] = rec
    else
        rec.obj = tb
        rec.translated_text = translated
    end

    if font_path and has_non_ascii(translated) then
        apply_cjk_font_fallback(tb, font_path, rec)
    end

    return set_widget_text_safe(tb, translated)
end

local function sweep_live_text_blocks()
    if next(ACTIVE_LOCALIZATIONS) == nil then return 0 end
    local translated_count = 0
    for _, cls_name in ipairs({ "TextBlock", "RichTextBlock" }) do
        local ok_all, list = pcall(FindAllOf, cls_name)
        if ok_all and type(list) == "table" then
            for _, tb in ipairs(list) do
                if apply_localization_to_widget(tb) then
                    translated_count = translated_count + 1
                end
            end
        end
    end
    return translated_count
end

local function ensure_localization_runtime_hooks()
    if not LOCALIZATION_HOOK_REGISTERED and RegisterHook then
        LOCALIZATION_HOOK_REGISTERED = true
        pcall(RegisterHook, "/Script/UMG.TextBlock:SetText", function()
            if IN_LOCALIZATION_SET_TEXT or next(ACTIVE_LOCALIZATIONS) == nil then return end
            LOCALIZATION_DIRTY = true
        end)
    end
end

local function restore_all_localized_widgets()
    local restored = 0
    local skipped = 0
    for _, rec in pairs(ORIGINAL_TEXT_RECORDS) do
        local tb = rec.obj
        local ok_v, valid = pcall(function() return tb and tb:IsValid() end)
        if ok_v and valid then
            local cur = read_text_block_string(tb)
            if cur == rec.translated_text and type(rec.original_text) == "string" then
                if set_widget_text_safe(tb, rec.original_text) then
                    restored = restored + 1
                end
            end
            if rec.font_patched and rec.orig_font_object then
                pcall(function()
                    local font_info = tb.Font
                    if font_info then
                        font_info.FontObject = rec.orig_font_object
                        if rec.orig_typeface_name ~= nil then
                            font_info.TypefaceFontName = rec.orig_typeface_name
                        end
                        if tb.SetFont then
                            local ok_sf = pcall(function() tb:SetFont(font_info) end)
                            if not ok_sf then tb.Font = font_info end
                        else
                            tb.Font = font_info
                        end
                    end
                end)
            end
        else
            skipped = skipped + 1
        end
    end
    ORIGINAL_TEXT_RECORDS = {}
    return restored, skipped
end

-- ---------------------------------------------------------------------------
-- Sticky UProperty Persistence Engine (owner filter, TArray<UStruct>, multiplier)
-- ---------------------------------------------------------------------------
local ACTIVE_PERSISTS = {}
local PERSIST_ORIGINALS = {}

local function is_ue_callable(val)
    if val == nil then return false end
    local t = type(val)
    if t == "function" then return true end
    if t == "userdata" then
        if val.IsValid then
            local ok_v, valid = pcall(function() return val:IsValid() end)
            if ok_v and not valid then return false end
        end
        if val.type then
            local ok_t, ty = pcall(function() return val:type() end)
            if ok_t and type(ty) == "string" and (ty == "UFunction" or ty:find("Function", 1, true) ~= nil) then
                return true
            end
        end
        local ok_fn, full = pcall(function()
            if val.GetFullName then return val:GetFullName() end
            return tostring(val)
        end)
        if ok_fn and type(full) == "string" and full:sub(1, 9) == "Function " then
            return true
        end
        return false
    end
    return false
end

local function null_uobject()
    if CreateInvalidObject ~= nil then
        local ok, invalid = pcall(CreateInvalidObject)
        if ok then return invalid end
    end
    return nil
end

local unpack_args = table.unpack or unpack

local function invoke_method_safe(obj, fn_name, ...)
    if not obj or type(fn_name) ~= "string" or fn_name == "" then return false end
    local ok_v, valid = pcall(function() return obj:IsValid() end)
    if not ok_v or not valid then return false end
    local ok_fn, fn_val = pcall(function() return obj[fn_name] end)
    if not ok_fn or not is_ue_callable(fn_val) then return false end
    local argc = select("#", ...)
    if argc == 0 then
        return pcall(fn_val, obj)
    end
    local args = { ... }
    return pcall(function()
        return fn_val(obj, unpack_args(args, 1, argc))
    end)
end

local function make_fname_safe(str)
    if FName ~= nil then
        local ok, fn = pcall(FName, str)
        if ok and fn ~= nil then return fn end
    end
    return str
end

local function fname_to_string_safe(val)
    if val == nil then return nil end
    if type(val) == "string" then return val end
    local v = val
    if type(v) == "userdata" and v.get then
        local ok_g, inner = pcall(function() return v:get() end)
        if ok_g and inner ~= nil then v = inner end
    end
    if type(v) == "string" then return v end
    if type(v) == "userdata" and v.ToString then
        local ok_s, s = pcall(function() return v:ToString() end)
        if ok_s and type(s) == "string" then return s end
    end
    local ok_t, s = pcall(tostring, v)
    return ok_t and s or nil
end

local function invoke_post_call_safe(root_obj, fn_name)
    invoke_method_safe(root_obj, fn_name)
end

local function apply_single_target_spec(feature_id, spec)
    if type(spec) ~= "table" or type(spec.object_class) ~= "string" or spec.object_class == "" then
        return 0
    end
    local orig_map = PERSIST_ORIGINALS[feature_id]
    if not orig_map then
        orig_map = {}
        PERSIST_ORIGINALS[feature_id] = orig_map
    end

    local insts = find_live_instances(spec.object_class, 64)
    local updated = 0

    for _, obj in ipairs(insts) do
        if object_matches_filter(obj, spec.filter_property) then
            local ok_fn, full_name = pcall(function() return obj:GetFullName() end)
            local fname = ok_fn and full_name or tostring(obj)
            local instance_changed = false
            local post_calls_needed = false

            if type(spec.array_struct_fields) == "table" and #spec.array_struct_fields > 0 then
                local arr_owner, arr_leaf = resolve_owner_and_leaf(obj, spec.property_name)
                if arr_owner and arr_leaf then
                    local ok_arr, arr = pcall(function() return arr_owner[arr_leaf] end)
                    if ok_arr and arr and arr.ForEach then
                        pcall(function()
                            arr:ForEach(function(elem_idx, elem_param)
                                local elem = (type(elem_param) == "userdata" and elem_param.get)
                                    and elem_param:get() or elem_param
                                if not elem then return end
                                for _, fspec in ipairs(spec.array_struct_fields) do
                                    local target_field = fspec.field
                                    if type(target_field) == "string" and target_field ~= "" then
                                        local key = fname .. "::" .. spec.property_name .. "[" .. tostring(elem_idx) .. "]." .. target_field
                                        local ok_cur, cur_val = pcall(function() return elem[target_field] end)
                                        if ok_cur then
                                            if orig_map[key] == nil then
                                                orig_map[key] = {
                                                    kind = "array_field",
                                                    obj = obj,
                                                    array_prop = spec.property_name,
                                                    elem_index = elem_idx,
                                                    field = target_field,
                                                    original = cur_val,
                                                    rollback_value = fspec.rollback_value,
                                                    post_call_function = spec.post_call_function,
                                                }
                                            end
                                            local desired = fspec.value
                                            if type(fspec.copy_from) == "string" and fspec.copy_from ~= "" then
                                                local ok_src, src_val = pcall(function() return elem[fspec.copy_from] end)
                                                if ok_src then desired = src_val end
                                            end
                                            if desired ~= nil and cur_val ~= desired then
                                                local ok_w = pcall(function() elem[target_field] = desired end)
                                                if ok_w then
                                                    instance_changed = true
                                                    post_calls_needed = true
                                                end
                                            end
                                        end
                                    end
                                end
                            end)
                        end)
                    end
                end
            else
                local owner, leaf = resolve_owner_and_leaf(obj, spec.property_name)
                if owner and leaf then
                    local ok_cur, cur_val = pcall(function() return owner[leaf] end)
                    if ok_cur then
                        local owner_fname = fname
                        if owner ~= obj and type(owner) == "userdata" and owner.GetFullName then
                            local ok_of, ofn = pcall(function() return owner:GetFullName() end)
                            if ok_of and type(ofn) == "string" and ofn ~= "" then
                                owner_fname = ofn
                            end
                        end
                        local key = fname .. "::" .. owner_fname .. "::" .. spec.property_name
                        if orig_map[key] == nil then
                            orig_map[key] = {
                                kind = "scalar",
                                obj = obj,
                                owner_obj = owner,
                                leaf_name = leaf,
                                property_name = spec.property_name,
                                original = cur_val,
                                rollback_value = spec.rollback_value,
                                post_call_function = spec.post_call_function,
                            }
                        end
                        local desired = nil
                        if type(spec.copy_from_property) == "string" and spec.copy_from_property ~= "" then
                            local src_owner, src_leaf = resolve_owner_and_leaf(obj, spec.copy_from_property)
                            if src_owner and src_leaf then
                                local ok_src, src_val = pcall(function() return src_owner[src_leaf] end)
                                if ok_src then desired = src_val end
                            end
                        elseif spec.multiply_original then
                            local base_val = orig_map[key].original
                            if type(base_val) ~= "number" or (base_val == 0 and spec.fallback_base ~= nil) then
                                base_val = tonumber(spec.fallback_base)
                            end
                            local mult = tonumber(spec.multiplier) or 1.0
                            if type(base_val) == "number" and mult > 0 then
                                local raw = spec.reciprocal and (base_val / mult) or (base_val * mult)
                                desired = spec.as_int and math.floor(raw + 0.5) or raw
                            end
                        else
                            desired = spec.value
                            if spec.as_int and type(desired) == "number" then
                                desired = math.floor(desired + 0.5)
                            end
                        end

                        if desired ~= nil and cur_val ~= desired then
                            local ok_w = pcall(function() owner[leaf] = desired end)
                            if ok_w then
                                instance_changed = true
                                post_calls_needed = true
                            end
                        end
                    end
                end
            end

            if post_calls_needed then
                invoke_post_call_safe(obj, spec.post_call_function)
            end
            if instance_changed then
                updated = updated + 1
            end
            if not spec.apply_all then
                break
            end
        end
    end
    return updated
end

local function sweep_active_persists()
    if next(ACTIVE_PERSISTS) == nil then return 0 end
    local total_updated = 0
    for feature_id, targets in pairs(ACTIVE_PERSISTS) do
        for _, spec in ipairs(targets) do
            total_updated = total_updated + apply_single_target_spec(feature_id, spec)
        end
    end
    return total_updated
end

local function restore_persist_feature(feature_id)
    local orig_map = PERSIST_ORIGINALS[feature_id]
    PERSIST_ORIGINALS[feature_id] = nil
    if type(orig_map) ~= "table" then return 0 end

    local restored = 0
    local pending_post = {}
    for _, rec in pairs(orig_map) do
        local obj = rec.obj
        local ok_v, valid = pcall(function() return obj and obj:IsValid() end)
        if ok_v and valid then
            local restore_val = rec.original
            if restore_val == nil then restore_val = rec.rollback_value end
            if restore_val ~= nil then
                if rec.kind == "array_field" then
                    local arr_owner, arr_leaf = resolve_owner_and_leaf(obj, rec.array_prop)
                    if arr_owner and arr_leaf then
                        pcall(function()
                            local arr = arr_owner[arr_leaf]
                            if arr and arr.ForEach then
                                arr:ForEach(function(elem_idx, elem_param)
                                    if elem_idx == rec.elem_index then
                                        local elem = (type(elem_param) == "userdata" and elem_param.get)
                                            and elem_param:get() or elem_param
                                        if elem then
                                            local ok_w = pcall(function() elem[rec.field] = restore_val end)
                                            if ok_w then
                                                restored = restored + 1
                                                if rec.post_call_function then
                                                    pending_post[obj] = rec.post_call_function
                                                end
                                            end
                                        end
                                    end
                                end)
                            end
                        end)
                    end
                else
                    local owner, leaf = nil, nil
                    if rec.owner_obj and type(rec.owner_obj) == "userdata" and rec.owner_obj.IsValid then
                        local ok_ov, ovalid = pcall(function() return rec.owner_obj:IsValid() end)
                        if ok_ov and ovalid and rec.leaf_name then
                            owner = rec.owner_obj
                            leaf = rec.leaf_name
                        end
                    end
                    if not owner or not leaf then
                        owner, leaf = resolve_owner_and_leaf(obj, rec.property_name)
                    end
                    if owner and leaf then
                        local ok_w = pcall(function() owner[leaf] = restore_val end)
                        if ok_w then
                            restored = restored + 1
                            if rec.post_call_function then
                                pending_post[obj] = rec.post_call_function
                            end
                        end
                    end
                end
            end
        end
    end
    for obj, fn_name in pairs(pending_post) do
        invoke_post_call_safe(obj, fn_name)
    end
    return restored
end

-- ---------------------------------------------------------------------------
-- Auto Lock-On, Auto-Fire & Kill Aura Engines (Single-Thread GameTick)
-- ---------------------------------------------------------------------------
local ACTIVE_TARGET_LOCKS = {}
local TARGET_LOCK_CACHE = {}
local LAST_LOCKED_TARGET = nil
local LAST_LOCKED_AT = 0.0

local ACTIVE_AUTO_FIRES = {}
local AUTO_FIRE_STATE = {}

local ACTIVE_SPINBOTS = {}
local SPINBOT_STATE = {}

local ACTIVE_DAMAGE_AURAS = {}
local DAMAGE_AURA_LAST_TICK = {}
local GAMEPLAY_STATICS_CDO = nil
local ENEMY_HEAD_BONE_CACHE = {}
local ENEMY_HEAD_BONE_CACHE_COUNT = 0

local atan2_rad = math.atan2 or math.atan

local function clamp_num(v, lo, hi)
    if v < lo then return lo end
    if v > hi then return hi end
    return v
end

local function wrap_angle_180(deg)
    return ((deg + 180.0) % 360.0) - 180.0
end

local function is_enemy_alive_and_valid(enemy, dead_prop, hp_prop)
    if not enemy then return false end
    local ok_v, valid = pcall(function() return enemy:IsValid() end)
    if not ok_v or not valid then return false end
    local ok_fn, fname = pcall(function() return enemy:GetFullName() end)
    if not ok_fn or is_archetype_name(fname) then return false end

    if is_ue_callable(enemy.IsActorBeingDestroyed) then
        local ok_bd, being_destroyed = pcall(function() return enemy:IsActorBeingDestroyed() end)
        if ok_bd and being_destroyed == true then return false end
    end

    if is_ue_callable(enemy.HasActorBegunPlay) then
        local ok_bp, begun_play = pcall(function() return enemy:HasActorBegunPlay() end)
        if ok_bp and begun_play == false then return false end
    end

    local ok_bh, bhidden = pcall(function() return enemy.bHidden end)
    if ok_bh and bhidden == true then return false end

    if is_ue_callable(enemy.IsHidden) then
        local ok_ih, is_hid = pcall(function() return enemy:IsHidden() end)
        if ok_ih and is_hid == true then return false end
    end

    if type(dead_prop) == "string" and dead_prop ~= "" then
        local ok_d, dead = pcall(function() return enemy[dead_prop] end)
        if ok_d and dead == true then return false end
    end
    if type(hp_prop) == "string" and hp_prop ~= "" then
        local ok_h, hp = pcall(function() return enemy[hp_prop] end)
        if ok_h and type(hp) == "number" and hp <= 0 then return false end
    end

    if is_ue_callable(enemy.IsAlive) then
        local out_a = {}
        local ok_a, ret_a = pcall(function() return enemy:IsAlive(out_a) end)
        if ok_a then
            if ret_a == false then return false end
            if type(out_a) == "table" and out_a.Alive == false then return false end
        end
    end
    return true
end

local function is_valid_head_point(pt, mesh_loc)
    if not pt then return false end
    local ok_xyz, px, py, pz = pcall(function() return pt.X, pt.Y, pt.Z end)
    if not ok_xyz or type(px) ~= "number" or type(py) ~= "number" or type(pz) ~= "number" then
        return false
    end
    if px == 0 and py == 0 and pz == 0 then
        return false
    end
    if mesh_loc then
        local ok_m, mx, my, mz = pcall(function() return mesh_loc.X, mesh_loc.Y, mesh_loc.Z end)
        if ok_m and type(mx) == "number" and type(my) == "number" and type(mz) == "number" then
            local dx = px - mx
            local dy = py - my
            local dz = pz - mz
            if (dx * dx + dy * dy + dz * dz) <= 9.0 then
                return false
            end
            if dz < -20.0 or dz > 260.0 then
                return false
            end
        end
    end
    return true
end

local function is_excluded_aim_bone_name(bn)
    if type(bn) ~= "string" or bn == "" or bn == "None" then return true end
    local low = bn:lower()
    return (low:find("end", 1, true) ~= nil)
        or (low:find("ik", 1, true) ~= nil)
        or (low:find("root", 1, true) ~= nil)
        or (low:find("top", 1, true) ~= nil)
        or (low:find("hair", 1, true) ~= nil)
        or (low:find("hat", 1, true) ~= nil)
        or (low:find("crown", 1, true) ~= nil)
end

local function try_mesh_bone(mesh, bone_name, mesh_loc)
    if type(bone_name) ~= "string" or bone_name == "" or bone_name == "None" then
        return nil
    end
    local fn = make_fname_safe(bone_name)
    if type(fn) == "string" then
        return nil
    end
    if mesh.DoesSocketExist then
        local ok_ex, exists = pcall(function() return mesh:DoesSocketExist(fn) end)
        if ok_ex and exists ~= true then
            return nil
        end
    end
    local ok_loc, p = pcall(function() return mesh:GetSocketLocation(fn) end)
    if ok_loc and is_valid_head_point(p, mesh_loc) then
        return p
    end
    return nil
end

local function scan_mesh_head_bone(mesh, prefer_neck)
    if not mesh or not mesh.GetNumBones or not mesh.GetBoneName then
        return nil
    end
    local ok_n, count = pcall(function() return mesh:GetNumBones() end)
    if not ok_n or type(count) ~= "number" or count <= 0 or count > 512 then
        return nil
    end
    local best_name = nil
    local best_rank = 999
    for i = 0, count - 1 do
        local ok_b, bfn = pcall(function() return mesh:GetBoneName(i) end)
        local bn = ok_b and fname_to_string_safe(bfn) or nil
        if bn and not is_excluded_aim_bone_name(bn) then
            local low = bn:lower()
            local rank = nil
            if prefer_neck then
                if low == "neck_01" or low == "neck_m" or low == "neck" then
                    rank = 1
                elseif low:find("neck", 1, true) ~= nil then
                    rank = 2
                elseif low == "spine_03" or low == "spine2" then
                    rank = 3
                elseif low == "head" or low == "head_m" then
                    rank = 4
                elseif low:find("head", 1, true) ~= nil then
                    rank = 5
                end
            else
                if low == "head" or low == "head_m" then
                    rank = 1
                elseif low:match("[_%-]head$") then
                    rank = 2
                elseif low:find("head", 1, true) ~= nil then
                    rank = 3
                elseif low == "neck_01" or low == "neck_m" or low == "neck" then
                    rank = 4
                elseif low:find("neck", 1, true) ~= nil then
                    rank = 5
                end
            end
            if rank and rank < best_rank then
                best_rank = rank
                best_name = bn
                if rank == 1 then break end
            end
        end
    end
    return best_name
end

local function finalize_aim_point(pt, mesh_loc, apply_z_offset, cfg)
    if not pt then return nil end
    local ok_xyz, px, py, pz = pcall(function() return pt.X, pt.Y, pt.Z end)
    if not ok_xyz or type(px) ~= "number" or type(py) ~= "number" or type(pz) ~= "number" then
        return nil
    end
    local z = pz
    if apply_z_offset then
        local z_off = clamp_num(tonumber(cfg.aim_z_offset) or 0.0, -100.0, 100.0)
        z = z + z_off
    end
    if mesh_loc then
        local ok_mz, mz = pcall(function() return mesh_loc.Z end)
        if ok_mz and type(mz) == "number" and z < (mz + 12.0) then
            z = mz + 12.0
        end
    end
    return { X = px, Y = py, Z = z }
end

local function resolve_enemy_aim_point(enemy, cfg)
    local aim_pt = nil
    local from_bone_or_core = false
    local mesh = nil
    pcall(function() mesh = enemy.Mesh end)

    local mesh_loc = nil
    local cls_key = nil
    pcall(function()
        local c = enemy:GetClass()
        if c and c:IsValid() then cls_key = c:GetFullName() end
    end)

    local prefer_neck = false
    if type(cfg.aim_bone_names) == "table" and type(cfg.aim_bone_names[1]) == "string" then
        local first_b = cfg.aim_bone_names[1]:lower()
        if first_b:find("neck", 1, true) ~= nil or first_b:find("spine", 1, true) ~= nil then
            prefer_neck = true
        end
    end
    local cache_key = cls_key and (cls_key .. (prefer_neck and "::neck" or "::head")) or nil

    if mesh then
        local ok_mv, mvalid = pcall(function() return mesh:IsValid() end)
        if ok_mv and mvalid and mesh.GetSocketLocation then
            pcall(function() mesh_loc = mesh:K2_GetComponentLocation() end)

            if cache_key then
                local cached_bone = ENEMY_HEAD_BONE_CACHE[cache_key]
                if type(cached_bone) == "string" and cached_bone ~= "" then
                    aim_pt = try_mesh_bone(mesh, cached_bone, mesh_loc)
                    if aim_pt then from_bone_or_core = true end
                end
            end

            if not aim_pt and type(cfg.aim_bone_names) == "table" then
                for _, bname in ipairs(cfg.aim_bone_names) do
                    local pt = try_mesh_bone(mesh, bname, mesh_loc)
                    if pt then
                        aim_pt = pt
                        from_bone_or_core = true
                        if cache_key and ENEMY_HEAD_BONE_CACHE_COUNT < 64 and not ENEMY_HEAD_BONE_CACHE[cache_key] then
                            ENEMY_HEAD_BONE_CACHE[cache_key] = bname
                            ENEMY_HEAD_BONE_CACHE_COUNT = ENEMY_HEAD_BONE_CACHE_COUNT + 1
                        end
                        break
                    end
                end
            end

            if not aim_pt and (not cache_key or ENEMY_HEAD_BONE_CACHE[cache_key] == nil) then
                local discovered = scan_mesh_head_bone(mesh, prefer_neck)
                if discovered then
                    aim_pt = try_mesh_bone(mesh, discovered, mesh_loc)
                    if aim_pt then from_bone_or_core = true end
                end
                if cache_key and ENEMY_HEAD_BONE_CACHE_COUNT < 64 then
                    ENEMY_HEAD_BONE_CACHE[cache_key] = (aim_pt and discovered) or false
                    ENEMY_HEAD_BONE_CACHE_COUNT = ENEMY_HEAD_BONE_CACHE_COUNT + 1
                end
            end

            if not aim_pt then
                pcall(function()
                    local crit_bones = enemy.CriticalHitBones
                    if crit_bones and crit_bones.ForEach then
                        crit_bones:ForEach(function(_, bone_param)
                            if aim_pt then return end
                            local bn_str = fname_to_string_safe(bone_param)
                            if not is_excluded_aim_bone_name(bn_str) then
                                local pt = try_mesh_bone(mesh, bn_str, mesh_loc)
                                if pt then
                                    aim_pt = pt
                                    from_bone_or_core = true
                                end
                            end
                        end)
                    end
                end)
            end

            if not aim_pt then
                local ok_cb, cb_name = pcall(function() return enemy.CriticalFXAttatchBone end)
                if ok_cb and cb_name then
                    local cb_str = fname_to_string_safe(cb_name)
                    if not is_excluded_aim_bone_name(cb_str) then
                        aim_pt = try_mesh_bone(mesh, cb_str, mesh_loc)
                        if aim_pt then from_bone_or_core = true end
                    end
                end
            end
        end
    end

    if not aim_pt and is_ue_callable(enemy.GetTargetCoreVector) then
        local out_vec = {}
        local out_mesh = {}
        local ok_cv, ret_cv = pcall(function() return enemy:GetTargetCoreVector(out_vec, out_mesh) end)
        if ok_cv then
            local cand = (type(out_vec) == "table" and out_vec.TargetCoreVector) or ret_cv
            if is_valid_head_point(cand, mesh_loc) then
                aim_pt = cand
                from_bone_or_core = true
            end
        end
    end

    if aim_pt then
        return finalize_aim_point(aim_pt, mesh_loc, from_bone_or_core, cfg)
    end

    local ok_el, el = pcall(function() return enemy:K2_GetActorLocation() end)
    if ok_el and el then
        return {
            X = el.X,
            Y = el.Y,
            Z = el.Z + (tonumber(cfg.aim_height_offset) or 42.0),
        }
    end
    return nil
end

local function dist_3d(a, b)
    local dx = (b.X or 0.0) - (a.X or 0.0)
    local dy = (b.Y or 0.0) - (a.Y or 0.0)
    local dz = (b.Z or 0.0) - (a.Z or 0.0)
    return math.sqrt(dx * dx + dy * dy + dz * dz)
end

local function compute_aim_angles(cam_pos, cam_rot, tgt_pos)
    local dx = tgt_pos.X - cam_pos.X
    local dy = tgt_pos.Y - cam_pos.Y
    local dz = tgt_pos.Z - cam_pos.Z
    local horiz = math.sqrt(dx * dx + dy * dy)
    local dist = math.sqrt(dx * dx + dy * dy + dz * dz)
    if dist < 1.0 then
        return dist, 0.0, 0.0, 0.0
    end
    local desired_yaw = math.deg(atan2_rad(dy, dx))
    local desired_pitch = math.deg(atan2_rad(dz, horiz))
    local dyaw = wrap_angle_180(desired_yaw - (cam_rot.Yaw or 0.0))
    local dpitch = wrap_angle_180(desired_pitch - (cam_rot.Pitch or 0.0))
    local ang = math.sqrt(dyaw * dyaw + dpitch * dpitch)
    return dist, ang, dyaw, dpitch
end

local function get_camera_view(pc, player)
    local cam_pos, cam_rot = nil, nil
    pcall(function()
        local cm = pc.PlayerCameraManager
        if cm and cm:IsValid() then
            cam_pos = cm:GetCameraLocation()
            cam_rot = cm:GetCameraRotation()
        end
    end)
    if not cam_pos then
        local ok_l, loc = pcall(function() return player:K2_GetActorLocation() end)
        if ok_l and loc then
            local ok_m = pcall(function() loc.Z = loc.Z + 65.0 end)
            if ok_m then
                cam_pos = loc
            else
                cam_pos = { X = loc.X, Y = loc.Y, Z = loc.Z + 65.0 }
            end
        end
    end
    if not cam_rot then
        local ok_r, cr = pcall(function() return pc:GetControlRotation() end)
        if ok_r and cr then cam_rot = cr end
    end
    return cam_pos, cam_rot
end

local function is_gameplay_input_blocked(pc, player)
    local ok_cur, cursor = pcall(function() return pc.bShowMouseCursor end)
    if ok_cur and cursor == true then
        return true
    end
    if player then
        local ok_hp, hp = pcall(function() return player.CurrentHP end)
        if ok_hp and type(hp) == "number" and hp <= 0 then
            return true
        end
    end
    return false
end

local function get_player_eye_pos(player)
    if not player then return nil end
    local ok_l, loc = pcall(function() return player:K2_GetActorLocation() end)
    if ok_l and loc then
        return { X = loc.X, Y = loc.Y, Z = loc.Z + 64.0 }
    end
    return nil
end

local function get_spinbot_aim_origin(player)
    if not player then return nil end
    local ok_l, loc = pcall(function() return player:K2_GetActorLocation() end)
    if ok_l and loc then
        return { X = loc.X, Y = loc.Y, Z = loc.Z + 80.0 }
    end
    return nil
end

local function check_line_of_sight(pc, enemy, trace_pos)
    if not pc or not enemy or not trace_pos then return true end
    if not pc.LineOfSightTo then return true end
    local ok, visible = pcall(function() return pc:LineOfSightTo(enemy, trace_pos, false) end)
    if ok and type(visible) == "boolean" then
        return visible
    end
    return true
end

local function has_target_los(pc, enemy, cam_pos, los_pos)
    if check_line_of_sight(pc, enemy, cam_pos) then
        return true
    end
    if los_pos and los_pos ~= cam_pos and check_line_of_sight(pc, enemy, los_pos) then
        return true
    end
    return false
end

local function select_best_enemy_target(pc, player, aim_origin, aim_rot, cfg, cache, now, cam_pos, los_pos)
    local max_dist = tonumber(cfg.max_distance) or 6000.0
    local max_fov = tonumber(cfg.max_fov_deg) or 65.0
    local need_los = (cfg.require_line_of_sight ~= false)
    local priority = (type(cfg.target_priority) == "string" and cfg.target_priority ~= "") and cfg.target_priority or "nearest"
    local los_cam = cam_pos or aim_origin

    local player_pos = aim_origin
    if player then
        local ok_p, ploc = pcall(function() return player:K2_GetActorLocation() end)
        if ok_p and ploc then
            player_pos = ploc
        end
    end

    local chosen = nil
    local chosen_pt = nil
    local cached_score = 1e18

    if cache and is_enemy_alive_and_valid(cache.target, cfg.dead_bool_property, cfg.health_property) then
        local pt = resolve_enemy_aim_point(cache.target, cfg)
        if pt then
            local _, ang = compute_aim_angles(aim_origin, aim_rot, pt)
            local pdist = dist_3d(player_pos, pt)
            if pdist <= max_dist and ang <= max_fov and (not need_los or has_target_los(pc, cache.target, los_cam, los_pos)) then
                chosen = cache.target
                chosen_pt = pt
                cached_score = (priority == "crosshair") and (ang * 35.0 + pdist * 0.015) or pdist
            end
        end
    end

    local scanned_at = (cache and cache.scanned_at) or 0.0
    if not chosen or not cache or (now - scanned_at) >= 0.05 then
        scanned_at = now
        local best_target = nil
        local best_pt = nil
        local best_score = 1e18
        local enemies = find_live_instances(cfg.enemy_class, 128)
        for _, enemy in ipairs(enemies) do
            if is_enemy_alive_and_valid(enemy, cfg.dead_bool_property, cfg.health_property) then
                local pt = resolve_enemy_aim_point(enemy, cfg)
                if pt then
                    local _, ang = compute_aim_angles(aim_origin, aim_rot, pt)
                    local pdist = dist_3d(player_pos, pt)
                    if pdist <= max_dist and ang <= max_fov then
                        if not need_los or has_target_los(pc, enemy, los_cam, los_pos) then
                            local score = (priority == "crosshair") and (ang * 35.0 + pdist * 0.015) or pdist
                            if score < best_score then
                                best_score = score
                                best_target = enemy
                                best_pt = pt
                            end
                        end
                    end
                end
            end
        end
        if best_target then
            if not chosen then
                chosen = best_target
                chosen_pt = best_pt
            elseif best_target == chosen then
                chosen_pt = best_pt
            elseif priority == "nearest" then
                if best_score < (cached_score - 80.0) then
                    chosen = best_target
                    chosen_pt = best_pt
                end
            elseif best_score < cached_score then
                chosen = best_target
                chosen_pt = best_pt
            end
        end
    end

    return chosen, chosen_pt, scanned_at
end

local function tick_target_locks(now)
    if next(ACTIVE_TARGET_LOCKS) == nil then return end
    now = now or os.clock()
    local spinbot_on = (next(ACTIVE_SPINBOTS) ~= nil)

    for feature_id, cfg in pairs(ACTIVE_TARGET_LOCKS) do
        local pc, player = resolve_player_controller_and_pawn(cfg.player_class)
        if not pc or not player or is_gameplay_input_blocked(pc, player) then
            TARGET_LOCK_CACHE[feature_id] = nil
        else
            local aim_ok = true
            if type(cfg.require_aim_bool) == "string" and cfg.require_aim_bool ~= "" then
                local ok_ab, ab = pcall(function() return player[cfg.require_aim_bool] end)
                aim_ok = ok_ab and ab == true
            end
            if not aim_ok then
                TARGET_LOCK_CACHE[feature_id] = nil
            else
                local cam_pos, cam_rot = get_camera_view(pc, player)
                local ok_cr, ctrl_rot = pcall(function() return pc:GetControlRotation() end)
                local base_rot = (ok_cr and ctrl_rot) and ctrl_rot or cam_rot
                local aim_origin = (spinbot_on and get_spinbot_aim_origin(player)) or cam_pos
                if cam_pos and aim_origin and base_rot then
                    local los_pos = (spinbot_on and get_player_eye_pos(player)) or cam_pos
                    local cache = TARGET_LOCK_CACHE[feature_id]
                    local dt = clamp_num(now - ((cache and cache.last_tick) or (now - 0.016)), 0.005, 0.05)
                    local chosen, pt, scanned_at = select_best_enemy_target(pc, player, aim_origin, base_rot, cfg, cache, now, cam_pos, los_pos)
                    TARGET_LOCK_CACHE[feature_id] = {
                        target = chosen,
                        scanned_at = scanned_at,
                        last_tick = now,
                    }
                    if chosen and pt then
                        LAST_LOCKED_TARGET = chosen
                        LAST_LOCKED_AT = now
                        local smooth = clamp_num(tonumber(cfg.smooth) or 0.42, 0.05, 1.0)
                        local _, _, dyaw, dpitch = compute_aim_angles(aim_origin, base_rot, pt)
                        local max_step = clamp_num(dt * 420.0, 2.5, 18.0)
                        local step_yaw = clamp_num(dyaw * smooth, -max_step, max_step)
                        local step_pitch = clamp_num(dpitch * smooth, -max_step, max_step)
                        local base_pitch = wrap_angle_180(base_rot.Pitch or 0.0)
                        local base_yaw = base_rot.Yaw or 0.0
                        local new_yaw = base_yaw + step_yaw
                        local new_pitch = clamp_num(base_pitch + step_pitch, -85.0, 85.0)
                        local applied = false
                        if ok_cr and ctrl_rot and type(ctrl_rot) == "userdata" then
                            applied = pcall(function()
                                ctrl_rot.Pitch = new_pitch
                                ctrl_rot.Yaw = new_yaw
                                ctrl_rot.Roll = 0.0
                                pc:SetControlRotation(ctrl_rot)
                            end)
                        end
                        if not applied then
                            applied = pcall(function()
                                pc:SetControlRotation({
                                    Pitch = new_pitch,
                                    Yaw = new_yaw,
                                    Roll = 0.0,
                                })
                            end)
                        end
                        if not applied then
                            pcall(function()
                                pc.ControlRotation.Pitch = new_pitch
                                pc.ControlRotation.Yaw = new_yaw
                                pc.ControlRotation.Roll = 0.0
                            end)
                        end
                    end
                end
            end
        end
    end
end

local function stop_auto_fire_state(feature_id)
    local st = AUTO_FIRE_STATE[feature_id]
    local cfg = ACTIVE_AUTO_FIRES[feature_id]
    if st and st.is_firing then
        st.is_firing = false
        if cfg then
            local _, player = resolve_player_controller_and_pawn(cfg.player_class)
            if player then
                if type(cfg.cancel_function) == "string" and cfg.cancel_function ~= "" then
                    invoke_method_safe(player, cfg.cancel_function)
                end
                if type(cfg.end_fire_function) == "string" and cfg.end_fire_function ~= "" then
                    invoke_method_safe(player, cfg.end_fire_function)
                end
            end
        end
    end
    if st then
        st.target = nil
    end
end

local function tick_auto_fire(now)
    if next(ACTIVE_AUTO_FIRES) == nil then return end
    now = now or os.clock()
    local spinbot_on = (next(ACTIVE_SPINBOTS) ~= nil)

    for feature_id, cfg in pairs(ACTIVE_AUTO_FIRES) do
        local st = AUTO_FIRE_STATE[feature_id]
        if not st then
            st = { is_firing = false, last_fired_at = 0.0, target = nil, scanned_at = 0.0 }
            AUTO_FIRE_STATE[feature_id] = st
        end

        local pc, player = resolve_player_controller_and_pawn(cfg.player_class)
        if not pc or not player or is_gameplay_input_blocked(pc, player) then
            stop_auto_fire_state(feature_id)
        else
            local aim_ok = true
            if type(cfg.require_aim_bool) == "string" and cfg.require_aim_bool ~= "" then
                local ok_ab, ab = pcall(function() return player[cfg.require_aim_bool] end)
                aim_ok = ok_ab and ab == true
            end
            local reloading = false
            if aim_ok and is_ue_callable(player.IsExecReload) then
                local out_r = {}
                local ok_r, ret_r = pcall(function() return player:IsExecReload(out_r) end)
                if ok_r and (ret_r == true or (type(out_r) == "table" and out_r.CheckOK == true)) then
                    reloading = true
                end
            end

            if not aim_ok or reloading then
                stop_auto_fire_state(feature_id)
            else
                local cam_pos, cam_rot = get_camera_view(pc, player)
                local ok_cr, ctrl_rot = pcall(function() return pc:GetControlRotation() end)
                local base_rot = (ok_cr and ctrl_rot) and ctrl_rot or cam_rot
                local aim_origin = (spinbot_on and get_spinbot_aim_origin(player)) or cam_pos
                if cam_pos and aim_origin and base_rot then
                    local los_pos = (spinbot_on and get_player_eye_pos(player)) or cam_pos
                    local player_pos = aim_origin
                    local ok_p, ploc = pcall(function() return player:K2_GetActorLocation() end)
                    if ok_p and ploc then player_pos = ploc end

                    local chosen = nil
                    if LAST_LOCKED_TARGET and (now - LAST_LOCKED_AT) <= 0.06
                        and is_enemy_alive_and_valid(LAST_LOCKED_TARGET, cfg.dead_bool_property, cfg.health_property) then
                        local pt = resolve_enemy_aim_point(LAST_LOCKED_TARGET, cfg)
                        if pt then
                            local max_dist = tonumber(cfg.max_distance) or 5500.0
                            local max_fov = tonumber(cfg.max_fov_deg) or 18.0
                            local need_los = (cfg.require_line_of_sight ~= false)
                            local _, ang = compute_aim_angles(aim_origin, base_rot, pt)
                            local pdist = dist_3d(player_pos, pt)
                            if pdist <= max_dist and ang <= max_fov
                                and (not need_los or has_target_los(pc, LAST_LOCKED_TARGET, cam_pos, los_pos)) then
                                chosen = LAST_LOCKED_TARGET
                            end
                        end
                    end
                    local scanned_at = st.scanned_at or 0.0
                    if not chosen then
                        local best_tgt, _, new_scan = select_best_enemy_target(pc, player, aim_origin, base_rot, cfg, st, now, cam_pos, los_pos)
                        chosen = best_tgt
                        scanned_at = new_scan
                    end

                    st.target = chosen
                    st.scanned_at = scanned_at
                    if chosen then
                        local interval_sec = math.max(16, math.min(5000, tonumber(cfg.interval_ms) or 35)) / 1000.0
                        if (now - st.last_fired_at) >= interval_sec then
                            local has_direct = (type(cfg.direct_fire_function) == "string" and cfg.direct_fire_function ~= "")
                            local fired = false
                            if has_direct then
                                local ok_pf, pf = pcall(function() return player.PlayingFire end)
                                if not (ok_pf and pf == true) then
                                    fired = invoke_method_safe(player, cfg.fire_function) or fired
                                end
                                fired = invoke_method_safe(player, cfg.direct_fire_function) or fired
                            else
                                fired = invoke_method_safe(player, cfg.fire_function)
                            end
                            if fired then
                                st.is_firing = true
                                st.last_fired_at = now
                            end
                        end
                    else
                        stop_auto_fire_state(feature_id)
                    end
                else
                    stop_auto_fire_state(feature_id)
                end
            end
        end
    end
end

-- ---------------------------------------------------------------------------
-- Third-Person CS SpinBot Engine (stabilized TPS camera + straight movement)
-- ---------------------------------------------------------------------------
local function clone_vec_xyz(v)
    if not v then return nil end
    local ok, x, y, z = pcall(function() return v.X, v.Y, v.Z end)
    if ok and type(x) == "number" and type(y) == "number" and type(z) == "number" then
        return { X = x, Y = y, Z = z }
    end
    return nil
end

local function clone_rot_pyr(r)
    if not r then return nil end
    local ok, p, y, rl = pcall(function() return r.Pitch, r.Yaw, r.Roll end)
    if ok and type(p) == "number" and type(y) == "number" and type(rl) == "number" then
        return { Pitch = p, Yaw = y, Roll = rl }
    end
    return nil
end

local function safe_set_rel_rot(comp, pitch, yaw, roll)
    if not comp then return false end
    local applied = false
    pcall(function()
        local cur = comp.RelativeRotation
        if cur and type(cur) == "userdata" then
            cur.Pitch = pitch
            cur.Yaw = yaw
            cur.Roll = roll
            if comp.K2_SetRelativeRotation then
                comp:K2_SetRelativeRotation(cur, false, {}, true)
            end
            applied = true
        end
    end)
    if applied then return true end
    if comp.K2_SetRelativeRotation then
        applied = pcall(function()
            comp:K2_SetRelativeRotation({ Pitch = pitch, Yaw = yaw, Roll = roll }, false, {}, true)
        end)
    end
    return applied
end

local function safe_set_rel_loc(comp, x, y, z)
    if not comp then return false end
    local applied = false
    pcall(function()
        local cur = comp.RelativeLocation
        if cur and type(cur) == "userdata" then
            cur.X = x
            cur.Y = y
            cur.Z = z
            if comp.K2_SetRelativeLocation then
                comp:K2_SetRelativeLocation(cur, false, {}, true)
            end
            applied = true
        end
    end)
    if applied then return true end
    if comp.K2_SetRelativeLocation then
        applied = pcall(function()
            comp:K2_SetRelativeLocation({ X = x, Y = y, Z = z }, false, {}, true)
        end)
    end
    return applied
end

local function safe_set_vec_field(obj, field_name, x, y, z)
    if not obj or type(field_name) ~= "string" then return false end
    local ok_set = pcall(function()
        local cur = obj[field_name]
        if cur and type(cur) == "userdata" then
            cur.X = x
            cur.Y = y
            cur.Z = z
        else
            obj[field_name] = { X = x, Y = y, Z = z }
        end
    end)
    if ok_set then return true end
    return pcall(function()
        obj[field_name] = { X = x, Y = y, Z = z }
    end)
end

local function safe_get_comp(player, name, fallback_name)
    local comp = nil
    local key = (type(name) == "string" and name ~= "") and name or fallback_name
    if not player or type(key) ~= "string" or key == "" then return nil end
    local ok_c, cand = pcall(function() return player[key] end)
    if ok_c and cand then
        local ok_v, valid = pcall(function() return cand:IsValid() end)
        if ok_v and valid then comp = cand end
    end
    return comp
end

local function set_comp_active_if_needed(comp, active, reset_flag)
    if not comp then return end
    local is_act = nil
    if comp.IsActive then
        local ok_i, res = pcall(function() return comp:IsActive() end)
        if ok_i and type(res) == "boolean" then is_act = res end
    end
    if is_act == active then return end
    if active then
        if comp.SetActive then
            pcall(function() comp:SetActive(true, reset_flag == true) end)
        elseif comp.Activate then
            pcall(function() comp:Activate(reset_flag == true) end)
        end
    else
        if comp.SetActive then
            pcall(function() comp:SetActive(false, false) end)
        elseif comp.Deactivate then
            pcall(function() comp:Deactivate() end)
        end
    end
end

local function same_uobject(a, b)
    if a == nil or b == nil then return a == b end
    if a == b then return true end
    if type(a) == "userdata" and type(b) == "userdata" and a.GetAddress and b.GetAddress then
        local ok_a, addr_a = pcall(function() return a:GetAddress() end)
        local ok_b, addr_b = pcall(function() return b:GetAddress() end)
        if ok_a and ok_b and addr_a ~= nil and addr_b ~= nil then
            return addr_a == addr_b
        end
    end
    return false
end

local function safe_set_world_loc_rot(comp, x, y, z, pitch, yaw, roll)
    if not comp then return false end
    local loc_tbl = { X = x, Y = y, Z = z }
    local rot_tbl = { Pitch = pitch, Yaw = yaw, Roll = roll }
    if comp.K2_SetWorldLocationAndRotation then
        local ok_lr = pcall(function()
            comp:K2_SetWorldLocationAndRotation(loc_tbl, rot_tbl, false, {}, true)
        end)
        if ok_lr then return true end
    end
    local ok_any = false
    if comp.K2_SetWorldLocation then
        ok_any = pcall(function() comp:K2_SetWorldLocation(loc_tbl, false, {}, true) end) or ok_any
    end
    if comp.K2_SetWorldRotation then
        ok_any = pcall(function() comp:K2_SetWorldRotation(rot_tbl, false, {}, true) end) or ok_any
    end
    return ok_any
end

local function capture_spinbot_originals(player, cfg)
    local orig = {}
    local mesh_comp = safe_get_comp(player, cfg.mesh_component, "FPS_Mesh")
    local fps_cam = safe_get_comp(player, cfg.fps_camera_component, "FPS_Camera")
    local tps_cam = safe_get_comp(player, cfg.tps_camera_component, "TPSCamera")
    local spring_arm = safe_get_comp(player, cfg.spring_arm_component, "SpringArm")
    local tps_base = safe_get_comp(player, cfg.tps_base_component, "TPS_Base")

    if mesh_comp then
        pcall(function()
            orig.mesh_rel_rot = clone_rot_pyr(mesh_comp.RelativeRotation)
            orig.mesh_only_owner_see = mesh_comp.bOnlyOwnerSee
            orig.mesh_owner_no_see = mesh_comp.bOwnerNoSee
        end)
    end
    if fps_cam then
        pcall(function()
            orig.fps_cam_use_ctrl = fps_cam.bUsePawnControlRotation
            orig.fps_cam_rel_rot = clone_rot_pyr(fps_cam.RelativeRotation)
        end)
    end
    if tps_base then
        pcall(function()
            orig.tps_base_rel_loc = clone_vec_xyz(tps_base.RelativeLocation)
        end)
    end
    if spring_arm then
        pcall(function()
            if spring_arm.GetAttachParent then
                orig.sa_parent = spring_arm:GetAttachParent()
            end
            orig.sa_abs_loc = spring_arm.bAbsoluteLocation
            orig.sa_abs_rot = spring_arm.bAbsoluteRotation
            orig.sa_abs_scale = spring_arm.bAbsoluteScale
            orig.sa_rel_loc = clone_vec_xyz(spring_arm.RelativeLocation)
            orig.sa_rel_rot = clone_rot_pyr(spring_arm.RelativeRotation)
            orig.sa_arm_length = spring_arm.TargetArmLength
            orig.sa_socket_offset = clone_vec_xyz(spring_arm.SocketOffset)
            orig.sa_target_offset = clone_vec_xyz(spring_arm.TargetOffset)
            orig.sa_use_ctrl = spring_arm.bUsePawnControlRotation
            orig.sa_inherit_pitch = spring_arm.bInheritPitch
            orig.sa_inherit_yaw = spring_arm.bInheritYaw
            orig.sa_inherit_roll = spring_arm.bInheritRoll
            orig.sa_do_collision = spring_arm.bDoCollisionTest
        end)
    end
    if tps_cam then
        pcall(function()
            orig.tps_cam_fov = tps_cam.FieldOfView
            orig.tps_cam_rel_loc = clone_vec_xyz(tps_cam.RelativeLocation)
            orig.tps_cam_rel_rot = clone_rot_pyr(tps_cam.RelativeRotation)
            orig.tps_cam_use_ctrl = tps_cam.bUsePawnControlRotation
        end)
    end
    return orig
end

local function activate_spinbot_on_player(st, player, cfg, pc)
    local ok_fn, p_name = pcall(function() return player:GetFullName() end)
    local pname = ok_fn and p_name or tostring(player)
    if st.player_name ~= pname or not st.originals then
        st.originals = capture_spinbot_originals(player, cfg)
        st.player_name = pname
        st.player_obj = player
    end

    local switch_fn = (type(cfg.switch_mesh_function) == "string" and cfg.switch_mesh_function ~= "")
        and cfg.switch_mesh_function or "SwitchSkeletalMesh"
    local tps_idx = tonumber(cfg.tps_mesh_index)
    if tps_idx == nil then tps_idx = 1 end
    invoke_method_safe(player, switch_fn, math.floor(tps_idx))

    local mesh_comp = safe_get_comp(player, cfg.mesh_component, "FPS_Mesh")
    local fps_cam = safe_get_comp(player, cfg.fps_camera_component, "FPS_Camera")
    local tps_cam = safe_get_comp(player, cfg.tps_camera_component, "TPSCamera")
    local spring_arm = safe_get_comp(player, cfg.spring_arm_component, "SpringArm")
    local tps_base = safe_get_comp(player, cfg.tps_base_component, "TPS_Base")
    local root_comp = safe_get_comp(player, "RootComponent", nil)

    if mesh_comp then
        pcall(function()
            mesh_comp.bOnlyOwnerSee = false
            mesh_comp.bOwnerNoSee = false
            if mesh_comp.SetVisibility then
                mesh_comp:SetVisibility(true, true)
            end
        end)
    end
    if fps_cam then
        pcall(function() fps_cam.bUsePawnControlRotation = false end)
        safe_set_rel_rot(fps_cam, 0.0, 0.0, 0.0)
        set_comp_active_if_needed(fps_cam, false, false)
    end
    if tps_base then
        safe_set_rel_loc(tps_base, 0.0, 0.0, 0.0)
    end
    if spring_arm then
        -- Detach from FPS_Mesh/TPS_Base and switch SpringArm to world-space absolute transform so FPS_Mesh spin cannot whip camera.
        if root_comp and spring_arm.K2_AttachToComponent then
            local par = nil
            if spring_arm.GetAttachParent then
                local ok_p, cur_par = pcall(function() return spring_arm:GetAttachParent() end)
                if ok_p then par = cur_par end
            end
            if not same_uobject(par, root_comp) then
                pcall(function()
                    spring_arm:K2_AttachToComponent(root_comp, make_fname_safe("None"), 0, 0, 0, false)
                end)
            end
        end
        if spring_arm.SetAbsolute then
            pcall(function() spring_arm:SetAbsolute(true, true, false) end)
        end
        pcall(function()
            spring_arm.bAbsoluteLocation = true
            spring_arm.bAbsoluteRotation = true
            spring_arm.TargetArmLength = tonumber(cfg.arm_length) or 260.0
            spring_arm.bUsePawnControlRotation = false
            spring_arm.bInheritPitch = true
            spring_arm.bInheritYaw = true
            spring_arm.bInheritRoll = false
            spring_arm.bDoCollisionTest = true
        end)
        safe_set_vec_field(spring_arm, "SocketOffset", 0.0, 0.0, 0.0)
        safe_set_vec_field(spring_arm, "TargetOffset", 0.0, 0.0, 16.0)

        local ok_al, actor_loc = pcall(function() return player:K2_GetActorLocation() end)
        if ok_al and actor_loc then
            local ctrl_pitch = 0.0
            local ctrl_yaw = 0.0
            local pctrl = pc
            if not pctrl then
                local ok_c, c_val = pcall(function() return player.Controller end)
                if ok_c then pctrl = c_val end
            end
            if pctrl then
                local ok_cr, ctrl_rot = pcall(function() return pctrl:GetControlRotation() end)
                if ok_cr and ctrl_rot then
                    ctrl_pitch = clamp_num(wrap_angle_180(ctrl_rot.Pitch or 0.0), -80.0, 80.0)
                    ctrl_yaw = ctrl_rot.Yaw or 0.0
                end
            end
            safe_set_world_loc_rot(spring_arm, actor_loc.X, actor_loc.Y, actor_loc.Z + 64.0, ctrl_pitch, ctrl_yaw, 0.0)
        end
    end
    if tps_cam then
        safe_set_rel_loc(tps_cam, 0.0, 0.0, 0.0)
        safe_set_rel_rot(tps_cam, 0.0, 0.0, 0.0)
        pcall(function()
            tps_cam.bUsePawnControlRotation = false
            tps_cam.FieldOfView = tonumber(cfg.tps_fov) or 85.0
        end)
        set_comp_active_if_needed(tps_cam, true, true)
    end
end

local function tick_spinbot(now)
    if next(ACTIVE_SPINBOTS) == nil then return end
    now = now or os.clock()

    for feature_id, cfg in pairs(ACTIVE_SPINBOTS) do
        local st = SPINBOT_STATE[feature_id]
        if not st then
            st = { yaw = 0.0, last_tick = now, player_name = nil, player_obj = nil, originals = nil }
            SPINBOT_STATE[feature_id] = st
        end
        local dt = clamp_num(now - (st.last_tick or now), 0.0, 0.05)
        st.last_tick = now

        local pc, player = resolve_player_controller_and_pawn(cfg.player_class)
        if pc and player then
            local ok_fn, p_name = pcall(function() return player:GetFullName() end)
            local pname = ok_fn and p_name or tostring(player)
            if pname ~= st.player_name then
                activate_spinbot_on_player(st, player, cfg, pc)
            end

            local mesh_comp = safe_get_comp(player, cfg.mesh_component, "FPS_Mesh")
            local fps_cam = safe_get_comp(player, cfg.fps_camera_component, "FPS_Camera")
            local tps_cam = safe_get_comp(player, cfg.tps_camera_component, "TPSCamera")
            local spring_arm = safe_get_comp(player, cfg.spring_arm_component, "SpringArm")
            local root_comp = safe_get_comp(player, "RootComponent", nil)

            if fps_cam then
                pcall(function() fps_cam.bUsePawnControlRotation = false end)
                safe_set_rel_rot(fps_cam, 0.0, 0.0, 0.0)
                set_comp_active_if_needed(fps_cam, false, false)
            end
            if spring_arm then
                if root_comp and spring_arm.GetAttachParent and spring_arm.K2_AttachToComponent then
                    local ok_p, par = pcall(function() return spring_arm:GetAttachParent() end)
                    if not ok_p or not same_uobject(par, root_comp) then
                        pcall(function()
                            spring_arm:K2_AttachToComponent(root_comp, make_fname_safe("None"), 0, 0, 0, false)
                        end)
                    end
                end
                if spring_arm.SetAbsolute then
                    pcall(function() spring_arm:SetAbsolute(true, true, false) end)
                end
                pcall(function()
                    spring_arm.bAbsoluteLocation = true
                    spring_arm.bAbsoluteRotation = true
                    spring_arm.TargetArmLength = tonumber(cfg.arm_length) or 260.0
                    spring_arm.bUsePawnControlRotation = false
                    spring_arm.bInheritPitch = true
                    spring_arm.bInheritYaw = true
                    spring_arm.bInheritRoll = false
                end)
                safe_set_vec_field(spring_arm, "SocketOffset", 0.0, 0.0, 0.0)
                safe_set_vec_field(spring_arm, "TargetOffset", 0.0, 0.0, 16.0)

                local ok_al, actor_loc = pcall(function() return player:K2_GetActorLocation() end)
                local ok_cr, ctrl_rot = pcall(function() return pc:GetControlRotation() end)
                if ok_al and actor_loc and ok_cr and ctrl_rot then
                    local ctrl_pitch = clamp_num(wrap_angle_180(ctrl_rot.Pitch or 0.0), -80.0, 80.0)
                    local ctrl_yaw = ctrl_rot.Yaw or 0.0
                    safe_set_world_loc_rot(spring_arm, actor_loc.X, actor_loc.Y, actor_loc.Z + 64.0, ctrl_pitch, ctrl_yaw, 0.0)
                end
            end
            if tps_cam then
                safe_set_rel_loc(tps_cam, 0.0, 0.0, 0.0)
                safe_set_rel_rot(tps_cam, 0.0, 0.0, 0.0)
                pcall(function() tps_cam.bUsePawnControlRotation = false end)
                set_comp_active_if_needed(tps_cam, true, false)
            end

            local speed = tonumber(cfg.spin_speed_deg_per_sec) or 1440.0
            local pitch = tonumber(cfg.spin_pitch_deg) or 0.0
            st.yaw = (st.yaw + speed * dt) % 360.0
            if mesh_comp then
                safe_set_rel_rot(mesh_comp, pitch, st.yaw, 0.0)
            end
        end
    end
end

local function restore_spinbot(feature_id)
    local st = SPINBOT_STATE[feature_id]
    local cfg = ACTIVE_SPINBOTS[feature_id]
    SPINBOT_STATE[feature_id] = nil
    if not st or not st.originals then return end

    local player = nil
    if st.player_obj then
        local ok_v, valid = pcall(function() return st.player_obj:IsValid() end)
        if ok_v and valid then player = st.player_obj end
    end
    if not player and cfg then
        local _, p = resolve_player_controller_and_pawn(cfg.player_class)
        player = p
    end
    if not player then return end

    local orig = st.originals
    local c = cfg or {}
    local mesh_comp = safe_get_comp(player, c.mesh_component, "FPS_Mesh")
    local fps_cam = safe_get_comp(player, c.fps_camera_component, "FPS_Camera")
    local tps_cam = safe_get_comp(player, c.tps_camera_component, "TPSCamera")
    local spring_arm = safe_get_comp(player, c.spring_arm_component, "SpringArm")
    local tps_base = safe_get_comp(player, c.tps_base_component, "TPS_Base")

    if mesh_comp then
        if orig.mesh_rel_rot then
            safe_set_rel_rot(mesh_comp, orig.mesh_rel_rot.Pitch, orig.mesh_rel_rot.Yaw, orig.mesh_rel_rot.Roll)
        end
        pcall(function()
            if orig.mesh_only_owner_see ~= nil then mesh_comp.bOnlyOwnerSee = orig.mesh_only_owner_see end
            if orig.mesh_owner_no_see ~= nil then mesh_comp.bOwnerNoSee = orig.mesh_owner_no_see end
        end)
    end

    local switch_fn = (type(c.switch_mesh_function) == "string" and c.switch_mesh_function ~= "")
        and c.switch_mesh_function or "SwitchSkeletalMesh"
    local fps_idx = tonumber(c.fps_mesh_index)
    if fps_idx == nil then fps_idx = 0 end
    invoke_method_safe(player, switch_fn, math.floor(fps_idx))

    if tps_base and orig.tps_base_rel_loc then
        safe_set_rel_loc(tps_base, orig.tps_base_rel_loc.X, orig.tps_base_rel_loc.Y, orig.tps_base_rel_loc.Z)
    end
    if spring_arm then
        if spring_arm.SetAbsolute then
            pcall(function()
                spring_arm:SetAbsolute(orig.sa_abs_loc == true, orig.sa_abs_rot == true, orig.sa_abs_scale == true)
            end)
        end
        pcall(function()
            spring_arm.bAbsoluteLocation = (orig.sa_abs_loc == true)
            spring_arm.bAbsoluteRotation = (orig.sa_abs_rot == true)
            spring_arm.bAbsoluteScale = (orig.sa_abs_scale == true)
        end)
        local restore_parent = nil
        if orig.sa_parent then
            local ok_v, valid = pcall(function() return orig.sa_parent:IsValid() end)
            if ok_v and valid then restore_parent = orig.sa_parent end
        end
        if not restore_parent then restore_parent = tps_base end
        if restore_parent and spring_arm.K2_AttachToComponent then
            pcall(function()
                spring_arm:K2_AttachToComponent(restore_parent, make_fname_safe("None"), 0, 0, 0, false)
            end)
        end
        if orig.sa_rel_loc then
            safe_set_rel_loc(spring_arm, orig.sa_rel_loc.X, orig.sa_rel_loc.Y, orig.sa_rel_loc.Z)
        end
        if orig.sa_rel_rot then
            safe_set_rel_rot(spring_arm, orig.sa_rel_rot.Pitch, orig.sa_rel_rot.Yaw, orig.sa_rel_rot.Roll)
        end
        pcall(function()
            if orig.sa_arm_length ~= nil then spring_arm.TargetArmLength = orig.sa_arm_length end
            if orig.sa_use_ctrl ~= nil then spring_arm.bUsePawnControlRotation = orig.sa_use_ctrl end
            if orig.sa_inherit_pitch ~= nil then spring_arm.bInheritPitch = orig.sa_inherit_pitch end
            if orig.sa_inherit_yaw ~= nil then spring_arm.bInheritYaw = orig.sa_inherit_yaw end
            if orig.sa_inherit_roll ~= nil then spring_arm.bInheritRoll = orig.sa_inherit_roll end
            if orig.sa_do_collision ~= nil then spring_arm.bDoCollisionTest = orig.sa_do_collision end
        end)
        if orig.sa_socket_offset then
            safe_set_vec_field(spring_arm, "SocketOffset", orig.sa_socket_offset.X, orig.sa_socket_offset.Y, orig.sa_socket_offset.Z)
        end
        if orig.sa_target_offset then
            safe_set_vec_field(spring_arm, "TargetOffset", orig.sa_target_offset.X, orig.sa_target_offset.Y, orig.sa_target_offset.Z)
        end
    end
    if tps_cam then
        if orig.tps_cam_rel_loc then
            safe_set_rel_loc(tps_cam, orig.tps_cam_rel_loc.X, orig.tps_cam_rel_loc.Y, orig.tps_cam_rel_loc.Z)
        end
        if orig.tps_cam_rel_rot then
            safe_set_rel_rot(tps_cam, orig.tps_cam_rel_rot.Pitch, orig.tps_cam_rel_rot.Yaw, orig.tps_cam_rel_rot.Roll)
        end
        pcall(function()
            if orig.tps_cam_use_ctrl ~= nil then tps_cam.bUsePawnControlRotation = orig.tps_cam_use_ctrl end
            if orig.tps_cam_fov ~= nil then tps_cam.FieldOfView = orig.tps_cam_fov end
        end)
        set_comp_active_if_needed(tps_cam, false, false)
    end
    if fps_cam then
        pcall(function()
            if orig.fps_cam_use_ctrl ~= nil then
                fps_cam.bUsePawnControlRotation = orig.fps_cam_use_ctrl
            else
                fps_cam.bUsePawnControlRotation = true
            end
        end)
        if orig.fps_cam_rel_rot then
            safe_set_rel_rot(fps_cam, orig.fps_cam_rel_rot.Pitch, orig.fps_cam_rel_rot.Yaw, orig.fps_cam_rel_rot.Roll)
        end
        set_comp_active_if_needed(fps_cam, true, true)
    end
end

local DAMAGE_NONE_CDO = nil

local function resolve_damage_type_cdo(path)
    if DAMAGE_NONE_CDO then
        local ok_v, valid = pcall(function() return DAMAGE_NONE_CDO:IsValid() end)
        if ok_v and valid then return DAMAGE_NONE_CDO end
        DAMAGE_NONE_CDO = nil
    end
    for _, cand_path in ipairs({ path, "/Script/Engine.Default__DamageType" }) do
        if type(cand_path) == "string" and cand_path ~= "" then
            local ok_dt, dt = pcall(StaticFindObject, cand_path)
            if ok_dt and dt then
                local ok_v, valid = pcall(function() return dt:IsValid() end)
                if ok_v and valid then
                    DAMAGE_NONE_CDO = dt
                    return dt
                end
            end
        end
    end
    return nil
end

local function resolve_gameplay_statics()
    if GAMEPLAY_STATICS_CDO then
        local ok_v, valid = pcall(function() return GAMEPLAY_STATICS_CDO:IsValid() end)
        if ok_v and valid then return GAMEPLAY_STATICS_CDO end
        GAMEPLAY_STATICS_CDO = nil
    end
    local ok_gs, gs = pcall(StaticFindObject, "/Script/Engine.Default__GameplayStatics")
    if ok_gs and gs then
        local ok_v, valid = pcall(function() return gs:IsValid() end)
        if ok_v and valid then
            GAMEPLAY_STATICS_CDO = gs
            return gs
        end
    end
    return nil
end

local function resolve_damage_type_class()
    for _, cls_path in ipairs({
        "/Game/00_RoD/03_Weapon/00_Common/Blueprints/Damage/BP_DamageNone.BP_DamageNone_C",
        "/Script/Engine.DamageType",
    }) do
        local ok_c, cls = pcall(StaticFindObject, cls_path)
        if ok_c and cls then
            local ok_v, valid = pcall(function() return cls:IsValid() end)
            if ok_v and valid then return cls end
        end
    end
    return null_uobject()
end

local function tick_damage_auras(now)
    if next(ACTIVE_DAMAGE_AURAS) == nil then return end
    now = now or os.clock()
    local gs = resolve_gameplay_statics()
    local dmg_cdo = resolve_damage_type_cdo("/Game/00_RoD/03_Weapon/00_Common/Blueprints/Damage/BP_DamageNone.Default__BP_DamageNone_C")
    local dmg_cls = resolve_damage_type_class()

    for feature_id, cfg in pairs(ACTIVE_DAMAGE_AURAS) do
        local interval_sec = (tonumber(cfg.interval_ms) or 250) / 1000.0
        local last = DAMAGE_AURA_LAST_TICK[feature_id] or 0.0
        if (now - last) >= interval_sec then
            DAMAGE_AURA_LAST_TICK[feature_id] = now
            local pc, player = resolve_player_controller_and_pawn(cfg.player_class)
            if pc and player and not is_gameplay_input_blocked(pc, player) then
                local ok_pl, ploc = pcall(function() return player:K2_GetActorLocation() end)
                if ok_pl and ploc then
                    local radius = tonumber(cfg.radius) or 3500.0
                    local damage = tonumber(cfg.damage) or 99999.0
                    local max_per_tick = math.max(1, math.min(64, math.floor(tonumber(cfg.max_targets_per_tick) or 8)))
                    local candidates = {}
                    local enemies = find_live_instances(cfg.enemy_class, 128)
                    for _, enemy in ipairs(enemies) do
                        if is_enemy_alive_and_valid(enemy, cfg.dead_bool_property, cfg.health_property) then
                            local ok_el, eloc = pcall(function() return enemy:K2_GetActorLocation() end)
                            if ok_el and eloc then
                                local dx = eloc.X - ploc.X
                                local dy = eloc.Y - ploc.Y
                                local dz = eloc.Z - ploc.Z
                                local d = math.sqrt(dx * dx + dy * dy + dz * dz)
                                if d <= radius then
                                    candidates[#candidates + 1] = { enemy = enemy, dist = d, loc = eloc }
                                end
                            end
                        end
                    end
                    table.sort(candidates, function(a, b) return a.dist < b.dist end)
                    for i = 1, math.min(#candidates, max_per_tick) do
                        local enemy = candidates[i].enemy
                        local eloc = candidates[i].loc
                        if dmg_cdo and is_ue_callable(enemy.DamageEvent) then
                            pcall(function()
                                enemy:DamageEvent(
                                    damage,
                                    dmg_cdo,
                                    eloc,
                                    { X = 0.0, Y = 0.0, Z = 1.0 },
                                    player,
                                    make_fname_safe("None"),
                                    true
                                )
                            end)
                        end
                        if is_enemy_alive_and_valid(enemy, cfg.dead_bool_property, cfg.health_property)
                            and gs and gs.ApplyDamage
                        then
                            pcall(function()
                                gs:ApplyDamage(enemy, damage, pc, player, dmg_cls)
                            end)
                        end
                        if is_enemy_alive_and_valid(enemy, cfg.dead_bool_property, cfg.health_property) then
                            if type(cfg.fallback_set_health_function) == "string" and cfg.fallback_set_health_function ~= "" then
                                invoke_method_safe(enemy, cfg.fallback_set_health_function, 0.0)
                            end
                            if type(cfg.fallback_death_function) == "string" and cfg.fallback_death_function ~= "" then
                                invoke_method_safe(enemy, cfg.fallback_death_function)
                            end
                        end
                    end
                end
            end
        end
    end
end

-- ---------------------------------------------------------------------------
-- Storage Fill & Stage Goal Completion Helpers (v0.5.1)
-- ---------------------------------------------------------------------------
local function find_helper_or_cdo(class_name, object_path)
    if type(object_path) == "string" and object_path ~= "" then
        local ok_sf, obj = pcall(StaticFindObject, object_path)
        if ok_sf and obj then
            local ok_v, valid = pcall(function() return obj:IsValid() end)
            if ok_v and valid then return obj end
        end
        if LoadAsset ~= nil then
            local pkg_cls = object_path:gsub("%.Default__([^%.]+)$", ".%1")
            pcall(LoadAsset, pkg_cls)
            local ok_sf2, obj2 = pcall(StaticFindObject, object_path)
            if ok_sf2 and obj2 then
                local ok_v2, valid2 = pcall(function() return obj2:IsValid() end)
                if ok_v2 and valid2 then return obj2 end
            end
        end
    end
    if type(class_name) == "string" and class_name ~= "" then
        local ok_f, first = pcall(FindFirstOf, class_name)
        if ok_f and first then
            local ok_v, valid = pcall(function() return first:IsValid() end)
            if ok_v and valid then return first end
        end
        local ok_all, list = pcall(FindAllOf, class_name)
        if ok_all and type(list) == "table" then
            for _, cand in ipairs(list) do
                local ok_v, valid = pcall(function() return cand and cand:IsValid() end)
                if ok_v and valid then return cand end
            end
        end
    end
    return nil
end

local function read_inventory_counts(save_class, map_prop)
    if type(save_class) ~= "string" or save_class == "" then
        return nil, {}, nil
    end
    local saves = find_live_instances(save_class, 4)
    local save_obj = saves[1]
    if not save_obj then
        return nil, {}, nil
    end
    if type(map_prop) ~= "string" or map_prop == "" then
        return save_obj, {}, nil
    end
    local ok_m, raw_map = pcall(function() return save_obj[map_prop] end)
    if not ok_m or not raw_map then
        return save_obj, {}, nil
    end
    local counts = {}
    if type(raw_map) == "table" then
        for k, v in pairs(raw_map) do
            local ks = fname_to_string_safe(k)
            local n = tonumber(v)
            if ks and ks ~= "" and n then
                counts[ks] = n
            end
        end
    elseif raw_map.ForEach then
        pcall(function()
            raw_map:ForEach(function(k_param, v_param)
                local ks = fname_to_string_safe(k_param)
                local v = v_param
                if type(v) == "userdata" and v.get then
                    local ok_v, inner = pcall(function() return v:get() end)
                    if ok_v then v = inner end
                end
                local n = tonumber(v)
                if ks and ks ~= "" and n then
                    counts[ks] = n
                end
            end)
        end)
    end
    return save_obj, counts, raw_map
end

local function extract_array_entries(container)
    local out = {}
    if container == nil then return out end
    if type(container) == "table" then
        for _, elem in ipairs(container) do
            out[#out + 1] = elem
        end
        return out
    end
    if type(container) == "userdata" then
        if container.ForEach then
            pcall(function()
                container:ForEach(function(_, elem_param)
                    local elem = (type(elem_param) == "userdata" and elem_param.get)
                        and elem_param:get() or elem_param
                    out[#out + 1] = elem
                end)
            end)
            if #out > 0 then return out end
        end
        if container.GetArrayNum then
            pcall(function()
                local n = container:GetArrayNum() or 0
                for i = 1, n do
                    out[#out + 1] = container[i]
                end
            end)
        end
    end
    return out
end

local function collect_dynamic_item_targets(helper_obj, world_ctx, p)
    local tbl_fn_name = type(p.table_function) == "string" and p.table_function or ""
    if tbl_fn_name == "" then return {} end
    local ok_fn, fn = pcall(function() return helper_obj[tbl_fn_name] end)
    if not ok_fn or not is_ue_callable(fn) then return {} end

    local out_items = {}
    local out_names = {}
    local ok_call, r1, r2 = pcall(fn, helper_obj, world_ctx, out_items, out_names)
    if not ok_call then return {} end

    local names_arr = extract_array_entries(out_names)
    local items_arr = extract_array_entries(out_items)
    if #names_arr == 0 then
        local r2_arr = extract_array_entries(r2)
        local r1_arr = extract_array_entries(r1)
        if #r2_arr > 0 then
            names_arr = r2_arr
            if #r1_arr > 0 then items_arr = r1_arr end
        elseif type(r1) == "table" then
            if type(r1.ItemRowName) ~= "nil" then
                names_arr = extract_array_entries(r1.ItemRowName)
                if type(r1.ItemList) ~= "nil" then
                    items_arr = extract_array_entries(r1.ItemList)
                end
            elseif #r1_arr > 0 and #items_arr == 0 then
                names_arr = r1_arr
            end
        end
    end

    local default_amt = math.max(1, math.min(99999, math.floor(tonumber(p.default_amount) or 99)))
    local max_field = type(p.max_field) == "string" and p.max_field or ""
    local targets = {}
    local seen = {}
    for i, raw_name in ipairs(names_arr) do
        local key = fname_to_string_safe(raw_name)
        if key and key ~= "" and key ~= "None" and not seen[key] then
            local cap = default_amt
            local info = items_arr[i]
            if info and max_field ~= "" then
                local ok_mx, mx_val = pcall(function() return info[max_field] end)
                local mx_num = ok_mx and tonumber(mx_val) or nil
                if mx_num and mx_num >= 1 then
                    cap = math.min(99999, math.floor(mx_num))
                end
            end
            seen[key] = true
            targets[#targets + 1] = { key = key, amount = cap }
        end
    end
    return targets
end

local function handle_fill_items(p)
    local helper_class = type(p.helper_class) == "string" and p.helper_class or ""
    local add_fn_name = type(p.add_function) == "string" and p.add_function or ""
    if helper_class == "" or add_fn_name == "" then
        return { ok = false, error = "fill_items requires non-empty helper_class and add_function" }
    end

    local pc, pawn = resolve_player_controller_and_pawn(nil)
    local save_class = type(p.save_class) == "string" and p.save_class or ""
    local save_obj, counts, raw_map = read_inventory_counts(save_class, p.inventory_map_property)
    if save_class ~= "" and not save_obj then
        return { ok = false, error = "no active SaveGame found; load a save before filling storage" }
    end

    local world_ctx = pawn or pc or save_obj
    local helper_obj = find_helper_or_cdo(helper_class, p.helper_object_path)
    if not helper_obj then
        return { ok = false, error = "helper class '" .. helper_class .. "' not loaded in active scene" }
    end

    local ok_add, add_fn = pcall(function() return helper_obj[add_fn_name] end)
    if not ok_add or not is_ue_callable(add_fn) then
        return { ok = false, error = "helper '" .. helper_class .. "' missing function '" .. add_fn_name .. "'" }
    end

    local source = "dynamic"
    local targets = collect_dynamic_item_targets(helper_obj, world_ctx, p)
    if #targets == 0 and type(p.fallback_items) == "table" then
        source = "fallback"
        local seen = {}
        for _, entry in ipairs(p.fallback_items) do
            if type(entry) == "table" and type(entry.key) == "string" and entry.key ~= "" and not seen[entry.key] then
                local cap = math.max(1, math.min(99999, math.floor(tonumber(entry.amount) or tonumber(p.default_amount) or 99)))
                seen[entry.key] = true
                targets[#targets + 1] = { key = entry.key, amount = cap }
            end
        end
    end

    if #targets == 0 then
        return { ok = false, error = "fill_items resolved 0 target items" }
    end

    local has_counts = next(counts) ~= nil
    local filled = 0
    local already_max = 0
    local failed = 0
    for _, item in ipairs(targets) do
        local cur = counts[item.key]
        if cur == nil and not has_counts and raw_map and raw_map.Find then
            local ok_f, found_val = pcall(function() return raw_map:Find(make_fname_safe(item.key)) end)
            if ok_f and found_val ~= nil then
                local fv = found_val
                if type(fv) == "userdata" and fv.get then
                    local ok_g, inner = pcall(function() return fv:get() end)
                    if ok_g and inner ~= nil then fv = inner end
                end
                if tonumber(fv) then
                    cur = tonumber(fv)
                end
            end
        end
        local delta = math.floor(item.amount - (cur or 0))
        if delta <= 0 then
            already_max = already_max + 1
        else
            local ok_call = pcall(add_fn, helper_obj, make_fname_safe(item.key), delta, world_ctx or null_uobject())
            if ok_call then
                filled = filled + 1
            else
                failed = failed + 1
            end
        end
    end

    if filled == 0 and already_max == 0 and failed > 0 then
        return { ok = false, error = "fill_items failed adding all " .. tostring(failed) .. " items" }
    end

    print(string.format("[NekodUE4SS] fill_items done: total=%d filled=%d already_max=%d failed=%d source=%s",
        #targets, filled, already_max, failed, source))

    return {
        ok = true,
        data = {
            total = #targets,
            filled = filled,
            already_max = already_max,
            failed = failed,
            source = source,
        },
    }
end

local function handle_trigger_goal(p)
    local goal_class = type(p.goal_class) == "string" and p.goal_class or ""
    local goal_fn_name = type(p.goal_function) == "string" and p.goal_function or ""
    if goal_class == "" or goal_fn_name == "" then
        return { ok = false, error = "trigger_goal requires non-empty goal_class and goal_function" }
    end

    local req_cls = type(p.require_live_class) == "string" and p.require_live_class or ""
    if req_cls ~= "" and #find_live_instances(req_cls, 4) == 0 then
        return { ok = false, error = "not in an active combat stage (enter a battle or wave stage before using instant stage clear)" }
    end

    local pc, pawn = resolve_player_controller_and_pawn(req_cls)

    -- Strategy 1: trigger an existing live goal actor in the level
    local goal = find_live_instances(goal_class, 8)[1]
    if not goal and type(p.wave_manager_class) == "string" and p.wave_manager_class ~= "" then
        local wms = find_live_instances(p.wave_manager_class, 4)
        for _, wm in ipairs(wms) do
            local ok_g, cand = pcall(function() return wm.BP_Goal end)
            if ok_g and cand then
                local ok_v, valid = pcall(function() return cand:IsValid() end)
                if ok_v and valid then
                    goal = cand
                    break
                end
            end
        end
    end
    if goal then
        local ok_ig, is_goal = pcall(function() return goal.IsGoal end)
        if ok_ig and is_goal == true then
            return { ok = true, data = { mode = "already_cleared" } }
        end
        local ok_fn, fn = pcall(function() return goal[goal_fn_name] end)
        if ok_fn and is_ue_callable(fn) then
            local ok_call = pcall(fn, goal)
            if ok_call then
                print("[NekodUE4SS] trigger_goal succeeded via existing_goal")
                return { ok = true, data = { mode = "existing_goal" } }
            end
        end
    end

    -- Strategy 2: spawn BP_Goal_C at the player transform when in streamed sublevel or Wave arena
    if p.spawn_if_missing ~= false and type(p.goal_class_path) == "string" and p.goal_class_path ~= "" then
        local ok_c, goal_uclass = pcall(StaticFindObject, p.goal_class_path)
        if (not ok_c or not goal_uclass) and LoadAsset ~= nil then
            pcall(LoadAsset, p.goal_class_path)
            ok_c, goal_uclass = pcall(StaticFindObject, p.goal_class_path)
        end
        local gs = resolve_gameplay_statics()
        local ok_gc, gvalid = pcall(function() return goal_uclass and goal_uclass:IsValid() end)
        local ok_pv, pvalid = pcall(function() return pawn and pawn:IsValid() end)
        if ok_c and ok_gc and gvalid and ok_pv and pvalid and gs and gs.BeginDeferredActorSpawnFromClass and gs.FinishSpawningActor then
            local ok_x, xform = pcall(function() return pawn:GetTransform() end)
            if ok_x and xform then
                local ok_sp, actor = pcall(function()
                    return gs:BeginDeferredActorSpawnFromClass(pawn, goal_uclass, xform, 1, pawn)
                end)
                if ok_sp and actor then
                    local ok_v, valid = pcall(function() return actor:IsValid() end)
                    if ok_v and valid then
                        pcall(function() gs:FinishSpawningActor(actor, xform) end)
                        local ok_fn, fn = pcall(function() return actor[goal_fn_name] end)
                        if ok_fn and is_ue_callable(fn) then
                            local ok_call = pcall(fn, actor)
                            if ok_call then
                                print("[NekodUE4SS] trigger_goal succeeded via spawned_goal")
                                return { ok = true, data = { mode = "spawned_goal" } }
                            end
                        end
                    end
                end
            end
        end
    end

    -- Strategy 3: clean up combat state via manager component and invoke result helper directly
    if type(p.result_helper_class) == "string" and p.result_helper_class ~= ""
        and type(p.result_function) == "string" and p.result_function ~= ""
    then
        if type(p.manager_class) == "string" and p.manager_class ~= "" and type(p.pre_clear_functions) == "table" then
            local mgrs = find_live_instances(p.manager_class, 8)
            for _, mgr in ipairs(mgrs) do
                for _, fn_name in ipairs(p.pre_clear_functions) do
                    if type(fn_name) == "string" and fn_name ~= "" then
                        invoke_method_safe(mgr, fn_name)
                    end
                end
            end
        end
        local res_helper = find_helper_or_cdo(p.result_helper_class, p.result_helper_path)
        local world_ctx = pc or pawn
        if res_helper and world_ctx then
            local ok_rf, rfn = pcall(function() return res_helper[p.result_function] end)
            if ok_rf and is_ue_callable(rfn) then
                local out_wb = {}
                local ok_call = pcall(rfn, res_helper, true, null_uobject(), world_ctx, out_wb)
                if ok_call then
                    print("[NekodUE4SS] trigger_goal succeeded via result_fallback")
                    return { ok = true, data = { mode = "result_fallback" } }
                end
            end
        end
    end

    return { ok = false, error = "stage clear failed: no live/spawnable goal or result helper available" }
end

local function dispatch_on_game_thread(req, transport_name)
    local op = type(req.op) == "string" and req.op or ""
    local p = type(req.params) == "table" and req.params or {}

    if op == "ping" then
        return {
            ok = true,
            data = {
                bridge_version = BRIDGE_VERSION,
                engine_version = safe_engine_version(),
                transport = transport_name,
                capabilities = {
                    "find_objects",
                    "dump_ui_texts",
                    "get_uproperty",
                    "list_properties",
                    "list_functions",
                    "cheat_manager_exec",
                    "set_uproperty",
                    "call_ufunction",
                    "apply_localization",
                    "clear_localization",
                    "persist_uproperty",
                    "clear_persist_uproperty",
                    "add_uproperty",
                    "set_target_lock",
                    "clear_target_lock",
                    "set_auto_fire",
                    "clear_auto_fire",
                    "set_spinbot",
                    "clear_spinbot",
                    "set_damage_aura",
                    "clear_damage_aura",
                    "fill_items",
                    "trigger_goal",
                },
            },
        }
    end

    if op == "find_objects" then
        local class_name = p.class_name
        if type(class_name) ~= "string" or class_name == "" then
            return { ok = false, error = "find_objects requires non-empty class_name" }
        end
        local limit = math.max(1, math.min(200, math.floor(tonumber(p.limit) or 25)))
        local filter = type(p.filter) == "string" and p.filter:lower() or ""
        local scan_limit = (filter == "") and limit or 2000
        local list = find_target_instances(class_name, scan_limit)
        local entries = {}
        for _, obj in ipairs(list) do
            local ok_full, full_name = pcall(function() return obj:GetFullName() end)
            local name_str = ok_full and full_name or tostring(obj)
            if filter == "" or name_str:lower():find(filter, 1, true) then
                local ok_cls, cls_name = pcall(function()
                    local c = obj:GetClass()
                    return (c and c:IsValid()) and c:GetFName():ToString() or class_name
                end)
                entries[#entries + 1] = {
                    index = #entries + 1,
                    full_name = name_str,
                    class_name = ok_cls and cls_name or class_name,
                    valid = true,
                }
                if #entries >= limit then break end
            end
        end
        return {
            ok = true,
            data = {
                class_name = class_name,
                count = #entries,
                objects = entries,
            },
        }
    end

    if op == "dump_ui_texts" then
        local live_only = (p.live_only ~= false)
        local unique_only = (p.unique_only == true)
        local filter = type(p.filter) == "string" and p.filter:lower() or ""
        local limit = math.max(1, math.min(500, math.floor(tonumber(p.limit) or 200)))
        local total_scanned = 0
        local texts = {}
        local seen_texts = {}
        local untranslated_seen = {}
        local untranslated_sample = {}
        local untranslated_count = 0

        for _, cls_name in ipairs({ "TextBlock", "RichTextBlock" }) do
            local ok_all, list = pcall(FindAllOf, cls_name)
            if ok_all and type(list) == "table" then
                for _, tb in ipairs(list) do
                    local ok_v, valid = pcall(function() return tb and tb:IsValid() end)
                    if ok_v and valid then
                        total_scanned = total_scanned + 1
                        local ok_fn, full_name = pcall(function() return tb:GetFullName() end)
                        local fname = ok_fn and full_name or tostring(tb)
                        local is_arch = is_archetype_widget_name(fname)
                        if not (live_only and is_arch) then
                            local txt = read_text_block_string(tb)
                            if type(txt) == "string" and txt ~= "" then
                                local lead_trim = txt:match("^%s*(.-)%s*$") or txt
                                local matches_filter = (filter == "")
                                    or fname:lower():find(filter, 1, true)
                                    or txt:lower():find(filter, 1, true)
                                if lead_trim ~= "" then
                                    if lead_trim:find("[%a]") and not contains_cjk_utf8(lead_trim) then
                                        untranslated_count = untranslated_count + 1
                                        if not untranslated_seen[lead_trim] then
                                            untranslated_seen[lead_trim] = true
                                            if #untranslated_sample < 30 then
                                                untranslated_sample[#untranslated_sample + 1] = lead_trim
                                            end
                                        end
                                    end
                                    if matches_filter and #texts < limit then
                                        if not unique_only or not seen_texts[txt] then
                                            seen_texts[txt] = true
                                            texts[#texts + 1] = {
                                                index = #texts + 1,
                                                full_name = fname,
                                                text = txt,
                                                font = read_font_summary(tb),
                                                is_transient = fname:find("Transient", 1, true) ~= nil,
                                            }
                                        end
                                    end
                                end
                            end
                        end
                    end
                end
            end
        end

        return {
            ok = true,
            data = {
                total_scanned = total_scanned,
                count = #texts,
                texts = texts,
                untranslated_count = math.max(untranslated_count, #UNTRANSLATED_LIST),
                untranslated_sample = (#untranslated_sample > 0) and untranslated_sample or UNTRANSLATED_LIST,
            },
        }
    end

    if op == "get_uproperty" then
        local cls = p.object_class or p.class_name
        local prop = p.property_name
        if type(cls) ~= "string" or cls == "" or type(prop) ~= "string" or prop == "" then
            return { ok = false, error = "get_uproperty requires object_class and property_name" }
        end
        local idx = math.max(1, math.floor(tonumber(p.instance_index) or 1))
        local insts = find_target_instances(cls, idx)
        local target = insts[idx] or insts[1]
        if not target then
            return { ok = false, error = "no live UObject instance of '" .. cls .. "'" }
        end
        local ok_read, val = pcall(function() return target[prop] end)
        if not ok_read then
            return { ok = false, error = "failed reading '" .. cls .. "." .. prop .. "': " .. tostring(val) }
        end
        local clean_val, val_type = safe_to_json_value(val)
        return {
            ok = true,
            data = {
                object_class = cls,
                property_name = prop,
                value = clean_val,
                value_type = val_type,
            },
        }
    end

    if op == "list_properties" then
        local cls_name = p.object_class or p.class_name
        if type(cls_name) ~= "string" or cls_name == "" then
            return { ok = false, error = "list_properties requires object_class" }
        end
        local uclass = resolve_uclass(cls_name)
        if not uclass then
            return { ok = false, error = "no UClass found for '" .. cls_name .. "'" }
        end
        local filter = type(p.filter) == "string" and p.filter:lower() or ""
        local limit = math.max(1, math.min(200, math.floor(tonumber(p.limit) or 80)))
        local results = {}
        local cur = uclass
        local depth = 0
        while cur and cur:IsValid() and depth < 8 and #results < limit do
            local owner_name = cls_name
            pcall(function() owner_name = cur:GetFName():ToString() end)
            pcall(function()
                cur:ForEachProperty(function(prop)
                    if #results >= limit then return true end
                    pcall(function()
                        local pname = prop:GetFName():ToString()
                        if filter == "" or pname:lower():find(filter, 1, true) then
                            local ptype = "Unknown"
                            pcall(function() ptype = prop:GetClass():GetFName():ToString() end)
                            results[#results + 1] = {
                                name = pname,
                                type = ptype,
                                owner = owner_name,
                            }
                        end
                    end)
                end)
            end)
            local ok_super, super_struct = pcall(function() return cur:GetSuperStruct() end)
            cur = ok_super and super_struct or nil
            depth = depth + 1
        end
        return {
            ok = true,
            data = {
                object_class = cls_name,
                count = #results,
                properties = results,
            },
        }
    end

    if op == "list_functions" then
        local cls_name = p.object_class or p.class_name
        if type(cls_name) ~= "string" or cls_name == "" then
            return { ok = false, error = "list_functions requires object_class" }
        end
        local uclass = resolve_uclass(cls_name)
        if not uclass then
            return { ok = false, error = "no UClass found for '" .. cls_name .. "'" }
        end
        local filter = type(p.filter) == "string" and p.filter:lower() or ""
        local limit = math.max(1, math.min(200, math.floor(tonumber(p.limit) or 80)))
        local results = {}
        local cur = uclass
        local depth = 0
        while cur and cur:IsValid() and depth < 8 and #results < limit do
            local owner_name = cls_name
            pcall(function() owner_name = cur:GetFName():ToString() end)
            pcall(function()
                cur:ForEachFunction(function(fn_obj)
                    if #results >= limit then return true end
                    pcall(function()
                        local fname = fn_obj:GetFName():ToString()
                        if filter == "" or fname:lower():find(filter, 1, true) then
                            local full_fn = fname
                            pcall(function() full_fn = fn_obj:GetFullName() end)
                            results[#results + 1] = {
                                name = fname,
                                full_name = full_fn,
                                owner = owner_name,
                            }
                        end
                    end)
                end)
            end)
            local ok_super, super_struct = pcall(function() return cur:GetSuperStruct() end)
            cur = ok_super and super_struct or nil
            depth = depth + 1
        end
        return {
            ok = true,
            data = {
                object_class = cls_name,
                count = #results,
                functions = results,
            },
        }
    end

    if op == "cheat_manager_exec" or op == "cheat_manager_command" then
        local cmd = p.command
        if type(cmd) ~= "string" or cmd == "" then
            return { ok = false, error = "missing cheat manager command" }
        end
        local pc = nil
        local ok_pc, found_pc = pcall(FindFirstOf, "PlayerController")
        if ok_pc and found_pc then
            local ok_v, valid = pcall(function() return found_pc:IsValid() end)
            if ok_v and valid then pc = found_pc end
        end
        if not pc then
            return { ok = false, error = "no active PlayerController in scene" }
        end
        local ok_exec, exec_err = pcall(function()
            pc:ConsoleCommand(cmd, true)
        end)
        if not ok_exec then
            return { ok = false, error = "ConsoleCommand failed: " .. tostring(exec_err) }
        end
        return { ok = true, data = { executed = cmd } }
    end

    if op == "set_uproperty" then
        local cls = p.object_class or p.class_name
        local prop = p.property_name
        local val = p.value
        if type(cls) ~= "string" or cls == "" or type(prop) ~= "string" or prop == "" then
            return { ok = false, error = "missing object_class or property_name" }
        end
        local insts = find_target_instances(cls, p.apply_all and 64 or 1)
        if #insts == 0 then
            return { ok = false, error = "no live UObject instance of '" .. cls .. "'" }
        end
        local updated = 0
        for _, obj in ipairs(insts) do
            local ok_set, err_set = pcall(function() obj[prop] = val end)
            if not ok_set then
                return { ok = false, error = "failed setting '" .. cls .. "." .. prop .. "': " .. tostring(err_set) }
            end
            updated = updated + 1
            if not p.apply_all then break end
        end
        return { ok = true, data = { updated = updated } }
    end

    if op == "call_ufunction" then
        local cls = p.object_class or p.class_name
        local fn_name = p.function_name
        if type(cls) ~= "string" or cls == "" or type(fn_name) ~= "string" or fn_name == "" then
            return { ok = false, error = "missing object_class or function_name" }
        end
        local insts = find_target_instances(cls, 1)
        local obj = insts[1]
        if not obj then
            return { ok = false, error = "no live UObject instance of '" .. cls .. "'" }
        end
        local ok_fn, fn_val = pcall(function() return obj[fn_name] end)
        if not ok_fn or not is_ue_callable(fn_val) then
            return { ok = false, error = "UFunction '" .. cls .. "::" .. fn_name .. "' not callable" }
        end
        local ok_call, call_err = pcall(fn_val, obj)
        if not ok_call then
            return { ok = false, error = "UFunction call failed: " .. tostring(call_err) }
        end
        return { ok = true, data = { invoked = 1 } }
    end

    if op == "apply_localization" then
        local feature_id = type(p.feature_id) == "string" and p.feature_id or ""
        if feature_id == "" then
            return { ok = false, error = "apply_localization requires non-empty feature_id" }
        end
        local entries_in = type(p.entries) == "table" and p.entries or {}
        local prefix_in = type(p.prefix_rules) == "table" and p.prefix_rules or {}
        local case_ins = (p.case_insensitive ~= false)
        local font_fb = (type(p.font_fallback) == "string" and p.font_fallback ~= "") and p.font_fallback or nil
        local scan_ms = math.max(200, math.min(5000, math.floor(tonumber(p.scan_interval_ms) or 500)))

        local entries_exact = {}
        local entries_ci = {}
        local target_outputs = {}
        local entries_loaded = 0
        for src, dst in pairs(entries_in) do
            if type(src) == "string" and src ~= "" and type(dst) == "string" and dst ~= "" then
                entries_exact[src] = dst
                if case_ins then
                    entries_ci[src:lower()] = dst
                end
                target_outputs[dst] = true
                entries_loaded = entries_loaded + 1
            end
        end

        local prefix_rules = {}
        for _, rule in ipairs(prefix_in) do
            if type(rule) == "table" and type(rule.prefix) == "string" and rule.prefix ~= ""
                and type(rule.replacement) == "string"
            then
                prefix_rules[#prefix_rules + 1] = {
                    prefix = rule.prefix,
                    prefix_lower = rule.prefix:lower(),
                    replacement = rule.replacement,
                }
                entries_loaded = entries_loaded + 1
            end
        end

        if entries_loaded == 0 then
            return { ok = false, error = "apply_localization requires at least one entry or prefix_rule" }
        end

        ACTIVE_LOCALIZATIONS[feature_id] = {
            entries_exact = entries_exact,
            entries_ci = entries_ci,
            prefix_rules = prefix_rules,
            target_outputs = target_outputs,
            font_fallback = font_fb,
            scan_interval_ms = scan_ms,
            case_insensitive = case_ins,
        }
        LOCALIZATION_SCAN_INTERVAL_MS = math.min(LOCALIZATION_SCAN_INTERVAL_MS, scan_ms)

        local fallback_loaded = false
        if font_fb then
            fallback_loaded = resolve_fallback_font(font_fb) ~= nil
        end

        ensure_localization_runtime_hooks()
        local translated_widgets = sweep_live_text_blocks()

        return {
            ok = true,
            data = {
                feature_id = feature_id,
                entries_loaded = entries_loaded,
                translated_widgets = translated_widgets,
                font_fallback_loaded = fallback_loaded,
            },
        }
    end

    if op == "clear_localization" then
        local feature_id = type(p.feature_id) == "string" and p.feature_id or ""
        if feature_id == "" then
            return { ok = false, error = "clear_localization requires non-empty feature_id" }
        end
        ACTIVE_LOCALIZATIONS[feature_id] = nil
        local restored_widgets = 0
        local skipped_invalid = 0
        if next(ACTIVE_LOCALIZATIONS) == nil then
            restored_widgets, skipped_invalid = restore_all_localized_widgets()
        end
        return {
            ok = true,
            data = {
                feature_id = feature_id,
                restored_widgets = restored_widgets,
                skipped_invalid = skipped_invalid,
            },
        }
    end

    if op == "persist_uproperty" then
        local feature_id = type(p.feature_id) == "string" and p.feature_id or ""
        local targets = type(p.targets) == "table" and p.targets or {}
        if feature_id == "" or #targets == 0 then
            return { ok = false, error = "persist_uproperty requires non-empty feature_id and targets" }
        end
        ACTIVE_PERSISTS[feature_id] = targets
        local updated = 0
        for _, spec in ipairs(targets) do
            updated = updated + apply_single_target_spec(feature_id, spec)
        end
        return {
            ok = true,
            data = {
                feature_id = feature_id,
                updated = updated,
                pending = (updated == 0),
            },
        }
    end

    if op == "clear_persist_uproperty" then
        local feature_id = type(p.feature_id) == "string" and p.feature_id or ""
        if feature_id == "" then
            return { ok = false, error = "clear_persist_uproperty requires non-empty feature_id" }
        end
        ACTIVE_PERSISTS[feature_id] = nil
        local restored = restore_persist_feature(feature_id)
        return {
            ok = true,
            data = {
                feature_id = feature_id,
                restored = restored,
            },
        }
    end

    if op == "add_uproperty" then
        local cls = p.object_class or p.class_name
        local prop = p.property_name
        local delta = tonumber(p.delta)
        if type(cls) ~= "string" or cls == "" or type(prop) ~= "string" or prop == "" or delta == nil then
            return { ok = false, error = "add_uproperty requires object_class, property_name, and numeric delta" }
        end
        local insts = find_live_instances(cls, 64)
        local updated = 0
        local last_val = nil
        for _, obj in ipairs(insts) do
            if object_matches_filter(obj, p.filter_property) then
                local owner, leaf = resolve_owner_and_leaf(obj, prop)
                if owner and leaf then
                    local ok_cur, cur_val = pcall(function() return owner[leaf] end)
                    if ok_cur and type(cur_val) == "number" then
                        local next_val = cur_val + delta
                        if p.min_value ~= nil then
                            next_val = math.max(tonumber(p.min_value), next_val)
                        end
                        if p.max_value ~= nil then
                            next_val = math.min(tonumber(p.max_value), next_val)
                        end
                        if p.as_int then
                            next_val = math.floor(next_val + 0.5)
                        end
                        local ok_set, err_set = pcall(function() owner[leaf] = next_val end)
                        if not ok_set then
                            return { ok = false, error = "failed setting '" .. cls .. "." .. prop .. "': " .. tostring(err_set) }
                        end
                        invoke_post_call_safe(obj, p.post_call_function)
                        updated = updated + 1
                        last_val = next_val
                        if not p.apply_all then break end
                    end
                end
            end
        end
        if updated == 0 then
            return { ok = false, error = "no live matching UObject instance of '" .. cls .. "'" }
        end
        return {
            ok = true,
            data = {
                updated = updated,
                new_value = last_val,
            },
        }
    end

    if op == "set_target_lock" then
        local feature_id = type(p.feature_id) == "string" and p.feature_id or ""
        local enemy_class = type(p.enemy_class) == "string" and p.enemy_class or ""
        if feature_id == "" or enemy_class == "" then
            return { ok = false, error = "set_target_lock requires feature_id and enemy_class" }
        end
        ENEMY_HEAD_BONE_CACHE = {}
        ENEMY_HEAD_BONE_CACHE_COUNT = 0
        ACTIVE_TARGET_LOCKS[feature_id] = p
        TARGET_LOCK_CACHE[feature_id] = nil
        tick_target_locks(os.clock())
        print(string.format("[NekodUE4SS] set_target_lock enabled (%s, fov=%s, los=%s, priority=%s, z_off=%s)",
            feature_id, tostring(p.max_fov_deg), tostring(p.require_line_of_sight ~= false),
            tostring(p.target_priority or "nearest"), tostring(p.aim_z_offset or 0)))
        return {
            ok = true,
            data = {
                feature_id = feature_id,
                active = true,
            },
        }
    end

    if op == "clear_target_lock" then
        local feature_id = type(p.feature_id) == "string" and p.feature_id or ""
        if feature_id == "" then
            return { ok = false, error = "clear_target_lock requires non-empty feature_id" }
        end
        ACTIVE_TARGET_LOCKS[feature_id] = nil
        TARGET_LOCK_CACHE[feature_id] = nil
        if next(ACTIVE_TARGET_LOCKS) == nil then
            LAST_LOCKED_TARGET = nil
            LAST_LOCKED_AT = 0.0
        end
        return {
            ok = true,
            data = {
                feature_id = feature_id,
                active = false,
            },
        }
    end

    if op == "set_auto_fire" then
        local feature_id = type(p.feature_id) == "string" and p.feature_id or ""
        local enemy_class = type(p.enemy_class) == "string" and p.enemy_class or ""
        local fire_function = type(p.fire_function) == "string" and p.fire_function or ""
        if feature_id == "" or enemy_class == "" or fire_function == "" then
            return { ok = false, error = "set_auto_fire requires feature_id, enemy_class, and fire_function" }
        end
        ENEMY_HEAD_BONE_CACHE = {}
        ENEMY_HEAD_BONE_CACHE_COUNT = 0
        ACTIVE_AUTO_FIRES[feature_id] = p
        AUTO_FIRE_STATE[feature_id] = { is_firing = false, last_fired_at = 0.0, target = nil, scanned_at = 0.0 }
        tick_auto_fire(os.clock())
        print(string.format("[NekodUE4SS] set_auto_fire enabled (%s, fn=%s, fov=%s, priority=%s, z_off=%s)",
            feature_id, fire_function, tostring(p.max_fov_deg),
            tostring(p.target_priority or "nearest"), tostring(p.aim_z_offset or 0)))
        return {
            ok = true,
            data = {
                feature_id = feature_id,
                active = true,
            },
        }
    end

    if op == "clear_auto_fire" then
        local feature_id = type(p.feature_id) == "string" and p.feature_id or ""
        if feature_id == "" then
            return { ok = false, error = "clear_auto_fire requires non-empty feature_id" }
        end
        stop_auto_fire_state(feature_id)
        ACTIVE_AUTO_FIRES[feature_id] = nil
        AUTO_FIRE_STATE[feature_id] = nil
        return {
            ok = true,
            data = {
                feature_id = feature_id,
                active = false,
            },
        }
    end

    if op == "set_spinbot" then
        local feature_id = type(p.feature_id) == "string" and p.feature_id or ""
        if feature_id == "" then
            return { ok = false, error = "set_spinbot requires non-empty feature_id" }
        end
        ACTIVE_SPINBOTS[feature_id] = p
        SPINBOT_STATE[feature_id] = { yaw = 0.0, last_tick = os.clock(), player_name = nil, player_obj = nil, originals = nil }
        tick_spinbot(os.clock())
        print(string.format("[NekodUE4SS] set_spinbot enabled (%s, speed=%sdeg/s, arm=%s)",
            feature_id, tostring(p.spin_speed_deg_per_sec), tostring(p.arm_length)))
        return {
            ok = true,
            data = {
                feature_id = feature_id,
                active = true,
            },
        }
    end

    if op == "clear_spinbot" then
        local feature_id = type(p.feature_id) == "string" and p.feature_id or ""
        if feature_id == "" then
            return { ok = false, error = "clear_spinbot requires non-empty feature_id" }
        end
        restore_spinbot(feature_id)
        ACTIVE_SPINBOTS[feature_id] = nil
        return {
            ok = true,
            data = {
                feature_id = feature_id,
                active = false,
            },
        }
    end

    if op == "set_damage_aura" then
        local feature_id = type(p.feature_id) == "string" and p.feature_id or ""
        local enemy_class = type(p.enemy_class) == "string" and p.enemy_class or ""
        if feature_id == "" or enemy_class == "" then
            return { ok = false, error = "set_damage_aura requires feature_id and enemy_class" }
        end
        ACTIVE_DAMAGE_AURAS[feature_id] = p
        DAMAGE_AURA_LAST_TICK[feature_id] = 0.0
        tick_damage_auras(os.clock())
        print(string.format("[NekodUE4SS] set_damage_aura enabled (%s, radius=%s)",
            feature_id, tostring(p.radius)))
        return {
            ok = true,
            data = {
                feature_id = feature_id,
                active = true,
            },
        }
    end

    if op == "clear_damage_aura" then
        local feature_id = type(p.feature_id) == "string" and p.feature_id or ""
        if feature_id == "" then
            return { ok = false, error = "clear_damage_aura requires non-empty feature_id" }
        end
        ACTIVE_DAMAGE_AURAS[feature_id] = nil
        DAMAGE_AURA_LAST_TICK[feature_id] = nil
        return {
            ok = true,
            data = {
                feature_id = feature_id,
                active = false,
            },
        }
    end

    if op == "fill_items" then
        return handle_fill_items(p)
    end

    if op == "trigger_goal" then
        return handle_trigger_goal(p)
    end

    return { ok = false, error = "unsupported ue4ss bridge op: " .. tostring(op) }
end

local function publish_discovery(transport_name, bound_port)
    local payload = {
        bridge_version = BRIDGE_VERSION,
        transport = transport_name,
        host = "127.0.0.1",
        port = bound_port or nil,
        ipc_dir = transport_name == "file_ipc" and BRIDGE_DIR or nil,
        auth_token = AUTH_TOKEN,
        engine_version = safe_engine_version(),
        started_at = os.time() or 0,
    }
    local ok, err = write_file_atomic(DISCOVERY_ACTIVE_PATH, Json.encode(payload))
    if not ok then
        print("[NekodUE4SS] Warning: failed publishing discovery file to " .. DISCOVERY_ACTIVE_PATH .. ": " .. tostring(err))
    end
end

-- ---------------------------------------------------------------------------
-- Start dual transport: single-threaded GameThread tick pump (zero LoopAsync races)
-- ---------------------------------------------------------------------------
local bound_server = nil
local bound_port = nil

if socket_ok and socket then
    for offset = 0, PORT_SCAN_SPAN do
        local candidate = PREFERRED_PORT + offset
        local ok_bind, srv = pcall(socket.bind, "127.0.0.1", candidate)
        if ok_bind and srv then
            bound_server = srv
            bound_port = candidate
            break
        end
    end
end

if bound_server then
    bound_server:settimeout(0)
    publish_discovery("tcp", bound_port)
else
    publish_discovery("file_ipc", nil)
end

local IN_MASTER_TICK = false
local LAST_IPC_POLL = 0.0
local LAST_AURA_TICK = 0.0
local LAST_PERSIST_TICK = 0.0
local LAST_LOC_TICK = 0.0

local function poll_ipc_on_game_thread()
    if bound_server then
        local client = bound_server:accept()
        if client then
            client:settimeout(0.5)
            local line = client:receive("*l")
            if type(line) == "string" and line ~= "" then
                local req = Json.decode(line)
                if type(req) ~= "table" or req.token ~= AUTH_TOKEN then
                    client:send('{"ok":false,"error":"unauthorized bridge token"}\n')
                else
                    local ok, res = pcall(dispatch_on_game_thread, req, "tcp")
                    local reply = ok and res or { ok = false, error = "internal bridge error: " .. tostring(res) }
                    client:send(Json.encode(reply) .. "\n")
                end
            end
            client:close()
        end
        return
    end

    local f = io.open(REQUEST_PATH, "rb")
    if not f then return end
    local raw = f:read("*a")
    f:close()
    pcall(os.remove, REQUEST_PATH)

    local req = Json.decode(raw)
    if type(req) ~= "table" then return end
    local req_id = sanitize_request_id(req.id)
    if not req_id then return end

    local resp_path = BRIDGE_DIR .. "\\ue4ss-response-" .. req_id .. ".json"
    if req.token ~= AUTH_TOKEN then
        write_file_atomic(resp_path, '{"ok":false,"error":"unauthorized bridge token"}')
        return
    end

    local ok, res = pcall(dispatch_on_game_thread, req, "file_ipc")
    local reply = ok and res or { ok = false, error = "internal bridge error: " .. tostring(res) }
    write_file_atomic(resp_path, Json.encode(reply))
end

local function master_game_thread_tick()
    if IN_MASTER_TICK then return end
    IN_MASTER_TICK = true

    local now = os.clock()
    if (now - LAST_IPC_POLL) >= 0.035 then
        LAST_IPC_POLL = now
        pcall(poll_ipc_on_game_thread)
    end
    if next(ACTIVE_TARGET_LOCKS) ~= nil then
        pcall(tick_target_locks, now)
    end
    if next(ACTIVE_SPINBOTS) ~= nil then
        pcall(tick_spinbot, now)
    end
    if next(ACTIVE_AUTO_FIRES) ~= nil then
        pcall(tick_auto_fire, now)
    end
    if next(ACTIVE_DAMAGE_AURAS) ~= nil and (now - LAST_AURA_TICK) >= 0.10 then
        LAST_AURA_TICK = now
        pcall(tick_damage_auras, now)
    end
    if next(ACTIVE_PERSISTS) ~= nil and (now - LAST_PERSIST_TICK) >= 0.033 then
        LAST_PERSIST_TICK = now
        pcall(sweep_active_persists)
    end
    if next(ACTIVE_LOCALIZATIONS) ~= nil then
        local loc_interval = math.max(0.2, (LOCALIZATION_SCAN_INTERVAL_MS or 500) / 1000.0)
        if LOCALIZATION_DIRTY or (now - LAST_LOC_TICK) >= loc_interval then
            LOCALIZATION_DIRTY = false
            LAST_LOC_TICK = now
            pcall(sweep_live_text_blocks)
        end
    end

    IN_MASTER_TICK = false
end

local TICK_DRIVER = "none"
if LoopInGameThreadAfterFrames ~= nil then
    local ok_l, handle = pcall(LoopInGameThreadAfterFrames, 1, master_game_thread_tick)
    if ok_l and handle ~= nil then
        TICK_DRIVER = "LoopInGameThreadAfterFrames"
    end
end
if TICK_DRIVER == "none" and LoopInGameThreadWithDelay ~= nil then
    local ok_d, handle = pcall(LoopInGameThreadWithDelay, 16, master_game_thread_tick)
    if ok_d and handle ~= nil then
        TICK_DRIVER = "LoopInGameThreadWithDelay"
    end
end
if TICK_DRIVER == "none" and LoopAsync ~= nil then
    TICK_DRIVER = "LoopAsync_Guarded"
    local LEGACY_TICK_IN_FLIGHT = false
    LoopAsync(33, function()
        if LEGACY_TICK_IN_FLIGHT then return false end
        LEGACY_TICK_IN_FLIGHT = true
        local ok_sched = pcall(ExecuteInGameThread, function()
            master_game_thread_tick()
            LEGACY_TICK_IN_FLIGHT = false
        end)
        if not ok_sched then
            LEGACY_TICK_IN_FLIGHT = false
        end
        return false
    end)
end

if bound_server then
    print(string.format("[NekodUE4SS] v%s TCP bridge listening on 127.0.0.1:%d (driver=%s, discovery=%s)",
        BRIDGE_VERSION, bound_port, TICK_DRIVER, DISCOVERY_ACTIVE_PATH))
else
    print(string.format("[NekodUE4SS] v%s pure-Lua File-IPC bridge active (driver=%s, dir=%s)",
        BRIDGE_VERSION, TICK_DRIVER, BRIDGE_DIR))
end
