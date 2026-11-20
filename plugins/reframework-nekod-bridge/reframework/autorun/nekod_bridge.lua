-- Nekod Companion Bridge for REFramework (`managed_framework_v1`)
-- Staged safely by Nekod DeploymentLedger into <Game>/reframework/autorun/nekod_bridge.lua.
-- Exposes deterministic state toggles and clean revert on mod unload.

local NEKOD_STATE = {
    time_scale_enabled = false,
    original_time_scale = 1.0,
    target_time_scale = 1.0,
    freecam_enabled = false,
    last_event = "Nekod REFramework Companion ready",
}

local function set_timescale(scale)
    local app = sdk and sdk.get_native_singleton("via.Application")
    if not app then
        return false
    end
    local app_type = sdk.find_type_definition("via.Application")
    if app_type and sdk.call_native_func then
        pcall(function()
            sdk.call_native_func(app, app_type, "set_GlobalSpeed", scale)
        end)
        return true
    end
    return false
end

local function nekod_cleanup()
    if NEKOD_STATE.time_scale_enabled then
        set_timescale(NEKOD_STATE.original_time_scale)
        NEKOD_STATE.time_scale_enabled = false
    end
    NEKOD_STATE.freecam_enabled = false
end

if re and re.on_draw_ui then
    re.on_draw_ui(function()
        if imgui and imgui.tree_node("Nekod Companion Bridge") then
            imgui.text("Status: " .. NEKOD_STATE.last_event)
            local changed, val = imgui.slider_float(
                "Global Speed",
                NEKOD_STATE.target_time_scale,
                0.25,
                4.0
            )
            if changed then
                NEKOD_STATE.time_scale_enabled = true
                NEKOD_STATE.target_time_scale = val
                set_timescale(val)
            end
            if imgui.button("Restore Defaults (LIFO Rollback)") then
                nekod_cleanup()
                NEKOD_STATE.last_event = "Restored original REFramework state"
            end
            imgui.tree_pop()
        end
    end)
end

if re and re.on_script_reset then
    re.on_script_reset(nekod_cleanup)
end
