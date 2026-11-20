-- Nekod Companion Bridge for UE4SS (`ue4ss_bridge_v1`)
-- Drop into <Game>\Binaries\Win64\Mods\NekodBridge\Scripts\main.lua (or ue4ss\Mods\NekodBridge\Scripts\main.lua)
-- Listens on 127.0.0.1:19541 using newline-delimited JSON with auth_token verification.

local HOST = "127.0.0.1"
local PORT = 19541
local AUTH_TOKEN = os.getenv("NEKOD_UE4SS_TOKEN") or "nekod_ue4ss_change_this_token_16"

local socket_ok, socket = pcall(require, "socket")
local json_ok, json = pcall(require, "dkjson")

local function safe_engine_version()
    local ok, major, minor = pcall(function()
        return UEHelpers and UEHelpers.GetUnrealVersion and UEHelpers.GetUnrealVersion() or nil
    end)
    if ok and major then
        return tostring(major) .. "." .. tostring(minor or 0)
    end
    return "5.3"
end

local function json_encode(tbl)
    if json_ok and json and json.encode then
        return json.encode(tbl)
    end
    if tbl.ok then
        return string.format('{"ok":true,"data":{"engine_version":"%s"}}', safe_engine_version())
    end
    return string.format('{"ok":false,"error":"%s"}', tostring(tbl.error or "error"))
end

local function handle_request(req)
    if type(req) ~= "table" or req.token ~= AUTH_TOKEN then
        return { ok = false, error = "unauthorized bridge token" }
    end

    if req.op == "ping" then
        return { ok = true, data = { engine_version = safe_engine_version() } }
    elseif req.op == "cheat_manager_command" then
        local cmd = req.params and req.params.command
        if type(cmd) ~= "string" or cmd == "" then
            return { ok = false, error = "missing cheat manager command" }
        end
        ExecuteInGameThread(function()
            local pc = FindFirstOf("PlayerController")
            if pc and pc:IsValid() and pc.CheatManager and pc.CheatManager:IsValid() then
                pc:ConsoleCommand(cmd, true)
            end
        end)
        return { ok = true, data = {} }
    elseif req.op == "set_uproperty" then
        local cls = req.params and req.params.object_class
        local prop = req.params and req.params.property_name
        local val = req.params and req.params.value
        if type(cls) ~= "string" or type(prop) ~= "string" then
            return { ok = false, error = "missing object_class or property_name" }
        end
        ExecuteInGameThread(function()
            local obj = FindFirstOf(cls)
            if obj and obj:IsValid() then
                obj[prop] = val
            end
        end)
        return { ok = true, data = {} }
    elseif req.op == "call_ufunction" then
        local cls = req.params and req.params.object_class
        local fn_name = req.params and req.params.function_name
        if type(cls) ~= "string" or type(fn_name) ~= "string" then
            return { ok = false, error = "missing object_class or function_name" }
        end
        ExecuteInGameThread(function()
            local obj = FindFirstOf(cls)
            if obj and obj:IsValid() and type(obj[fn_name]) == "function" then
                obj[fn_name](obj)
            end
        end)
        return { ok = true, data = {} }
    end

    return { ok = false, error = "unsupported ue4ss bridge op: " .. tostring(req.op) }
end

if socket_ok and socket then
    local server = assert(socket.bind(HOST, PORT))
    server:settimeout(0)
    print(string.format("[NekodUE4SS] Listening on %s:%d", HOST, PORT))
    LoopAsync(50, function()
        local client = server:accept()
        if client then
            client:settimeout(0.5)
            local line = client:receive("*l")
            if line and json_ok and json and json.decode then
                local req = json.decode(line)
                client:send(json_encode(handle_request(req)) .. "\n")
            else
                client:send('{"ok":false,"error":"dkjson unavailable or empty request"}\n')
            end
            client:close()
        end
        return false
    end)
else
    print("[NekodUE4SS] LuaSocket not loaded; registered offline CheatManager helpers only.")
end
