//! Unreal Engine UE4SS Reflection Bridge runtime adapter (`ue4ss_bridge_v1`).
//!
//! Communicates with an authenticated UE4SS bridge plugin over loopback JSON-line IPC
//! or zero-dependency pure-Lua File-IPC (`NekodUE4SSBridge v0.2.0`) to invoke `CheatManager`
//! commands, mutate `UObject` properties, trigger `UFunction`s, and run read-only `UObject`
//! introspection with explicit version guardrails and LIFO rollback on session stop.

use std::collections::BTreeMap;
use std::fs;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use nekod_manifest::{PermissionScope, RuntimeDependencySpec, RuntimeKind};
use nekod_runtime_api::{
    DependencyCheckResult, DependencyStatus, FeatureCommand, FeatureValue, ProcessIdentity,
    RuntimeAdapterClass, RuntimeAdapterMetadata, RuntimeDiagnostics, RuntimeError,
    RuntimeLifecycleMode, RuntimeProvider, RuntimeSessionId, SessionLifecycleState,
    StartSessionRequest,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::bridge_ipc::{
    BridgeRpcRequest, BridgeSessionStore, ManagedBridgeSession, send_bridge_rpc, send_file_ipc_rpc,
    validate_loopback_bridge, validate_loopback_host_port,
};
use crate::session::validate_session_preflight_for_kind;

fn default_loopback_host() -> String {
    "127.0.0.1".to_owned()
}

fn default_ue4ss_port() -> u16 {
    19541
}

fn default_localization_scan_interval_ms() -> u32 {
    500
}

fn default_case_insensitive() -> bool {
    true
}

fn default_scalar_scale() -> f64 {
    1.0
}

fn default_max_distance() -> f64 {
    6000.0
}

fn default_max_fov_deg() -> f64 {
    65.0
}

fn default_smooth() -> f64 {
    0.42
}

fn default_aim_height_offset() -> f64 {
    60.0
}

fn default_aura_radius() -> f64 {
    3500.0
}

fn default_aura_damage() -> f64 {
    99999.0
}

fn default_aura_interval_ms() -> u32 {
    250
}

fn default_aura_max_targets() -> u32 {
    8
}

fn default_auto_fire_fov_deg() -> f64 {
    12.0
}

fn default_auto_fire_interval_ms() -> u32 {
    75
}

fn default_auto_fire_distance() -> f64 {
    5500.0
}

fn default_spin_speed_deg_per_sec() -> f64 {
    1440.0
}

fn default_spin_arm_length() -> f64 {
    280.0
}

fn default_spin_tps_fov() -> f64 {
    85.0
}

/// Ordered prefix translation rule for dynamic UE UI strings (e.g. `FPS: `, `Stage `).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UiTextPrefixRule {
    pub prefix: String,
    pub replacement: String,
}

/// Optional boolean/scalar equality guard on UObject instances (e.g. `bHasPlayer == true`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ue4ssPropertyFilter {
    pub name: String,
    pub equals: serde_json::Value,
}

/// Field patch applied to every element of a `TArray<UStruct>` property.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ue4ssArrayStructField {
    pub field: String,
    #[serde(default)]
    pub value: Option<serde_json::Value>,
    #[serde(default)]
    pub copy_from: Option<String>,
    #[serde(default)]
    pub rollback_value: Option<serde_json::Value>,
}

/// Additional UProperty target bundled into a `toggle_uproperty` binding.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ue4ssExtraPropertyTarget {
    pub object_class: String,
    pub property_name: String,
    #[serde(default)]
    pub enabled_value: Option<serde_json::Value>,
    #[serde(default)]
    pub disabled_value: Option<serde_json::Value>,
    #[serde(default)]
    pub copy_from_property: Option<String>,
    #[serde(default)]
    pub array_struct_fields: Vec<Ue4ssArrayStructField>,
    #[serde(default)]
    pub apply_all: bool,
    #[serde(default)]
    pub filter_property: Option<Ue4ssPropertyFilter>,
    #[serde(default)]
    pub post_call_function: Option<String>,
}

/// Additional scalar target bundled into a `set_uproperty_scalar` slider.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ue4ssScalarExtraTarget {
    pub object_class: String,
    pub property_name: String,
    #[serde(default = "default_scalar_scale")]
    pub scale: f64,
    #[serde(default)]
    pub reciprocal: bool,
    #[serde(default)]
    pub as_int: bool,
    #[serde(default)]
    pub fallback_base: Option<f64>,
    #[serde(default)]
    pub reset_value: Option<serde_json::Value>,
    #[serde(default)]
    pub apply_all: bool,
    #[serde(default)]
    pub filter_property: Option<Ue4ssPropertyFilter>,
    #[serde(default)]
    pub post_call_function: Option<String>,
    #[serde(default)]
    pub copy_from_property: Option<String>,
}

/// Static fallback item row used if dynamic UE DataTable enumeration yields no rows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ue4ssFallbackItem {
    pub key: String,
    pub amount: i32,
}

fn default_fill_item_amount() -> i32 {
    99
}

fn default_true_bool() -> bool {
    true
}

/// Transport mode announced by `NekodUE4SSBridge` discovery JSON.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Ue4ssTransportKind {
    Tcp,
    FileIpc,
}

/// Discovery metadata resolved from `%LOCALAPPDATA%\Nekod\bridges\ue4ss-{pid}.json` or `ue4ss-active.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ue4ssDiscovery {
    pub transport: Ue4ssTransportKind,
    pub host: String,
    pub port: Option<u16>,
    pub ipc_dir: Option<PathBuf>,
    pub auth_token: String,
    pub engine_version: Option<String>,
    pub bridge_version: Option<String>,
}

/// Entrypoint JSON payload for `ue4ss_bridge_v1`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ue4ssBridgePayload {
    pub schema_version: String,
    pub engine_version: String,
    #[serde(default = "default_loopback_host")]
    pub host: String,
    #[serde(default = "default_ue4ss_port")]
    pub port: u16,
    #[serde(default)]
    pub auth_token: String,
    #[serde(default)]
    pub auto_discover: bool,
    #[serde(default)]
    pub discovery_dir: Option<String>,
    pub features: Vec<Ue4ssFeatureBinding>,
}

/// Declarative Unreal reflection action mapped from UI `FeatureCommand`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Ue4ssFeatureBinding {
    CheatManagerCommand {
        feature_id: String,
        enable_command: String,
        #[serde(default)]
        disable_command: Option<String>,
    },
    CheatManagerFloat {
        feature_id: String,
        command_prefix: String,
        #[serde(default)]
        reset_command: Option<String>,
    },
    #[serde(rename = "toggle_uproperty", alias = "toggle_u_property")]
    ToggleUProperty {
        feature_id: String,
        object_class: String,
        property_name: String,
        #[serde(default)]
        enabled_value: Option<serde_json::Value>,
        #[serde(default)]
        disabled_value: Option<serde_json::Value>,
        #[serde(default)]
        copy_from_property: Option<String>,
        #[serde(default)]
        array_struct_fields: Vec<Ue4ssArrayStructField>,
        #[serde(default)]
        persist: bool,
        #[serde(default)]
        apply_all: bool,
        #[serde(default)]
        filter_property: Option<Ue4ssPropertyFilter>,
        #[serde(default)]
        post_call_function: Option<String>,
        #[serde(default)]
        extra_targets: Vec<Ue4ssExtraPropertyTarget>,
    },
    #[serde(rename = "set_uproperty_scalar", alias = "set_u_property_scalar")]
    SetUPropertyScalar {
        feature_id: String,
        object_class: String,
        property_name: String,
        #[serde(default)]
        multiply_original: bool,
        #[serde(default = "default_scalar_scale")]
        scale: f64,
        #[serde(default)]
        reciprocal: bool,
        #[serde(default)]
        as_int: bool,
        #[serde(default)]
        fallback_base: Option<f64>,
        #[serde(default)]
        persist: bool,
        #[serde(default)]
        reset_value: Option<serde_json::Value>,
        #[serde(default)]
        apply_all: bool,
        #[serde(default)]
        filter_property: Option<Ue4ssPropertyFilter>,
        #[serde(default)]
        post_call_function: Option<String>,
        #[serde(default)]
        extra_targets: Vec<Ue4ssScalarExtraTarget>,
    },
    #[serde(rename = "add_uproperty", alias = "add_u_property")]
    AddUProperty {
        feature_id: String,
        object_class: String,
        property_name: String,
        delta: f64,
        #[serde(default)]
        as_int: bool,
        #[serde(default)]
        min_value: Option<f64>,
        #[serde(default)]
        max_value: Option<f64>,
        #[serde(default)]
        apply_all: bool,
        #[serde(default)]
        filter_property: Option<Ue4ssPropertyFilter>,
        #[serde(default)]
        post_call_function: Option<String>,
    },
    TargetLock {
        feature_id: String,
        enemy_class: String,
        #[serde(default)]
        player_class: Option<String>,
        #[serde(default)]
        dead_bool_property: Option<String>,
        #[serde(default)]
        health_property: Option<String>,
        #[serde(default)]
        require_aim_bool: Option<String>,
        #[serde(default)]
        aim_bone_names: Vec<String>,
        #[serde(default = "default_max_distance")]
        max_distance: f64,
        #[serde(default = "default_max_fov_deg")]
        max_fov_deg: f64,
        #[serde(default = "default_smooth")]
        smooth: f64,
        #[serde(default = "default_aim_height_offset")]
        aim_height_offset: f64,
        #[serde(default)]
        aim_z_offset: f64,
        #[serde(default)]
        target_priority: Option<String>,
        #[serde(default = "default_true_bool")]
        require_line_of_sight: bool,
    },
    #[serde(rename = "auto_fire")]
    AutoFire {
        feature_id: String,
        enemy_class: String,
        #[serde(default)]
        player_class: Option<String>,
        fire_function: String,
        #[serde(default)]
        direct_fire_function: Option<String>,
        #[serde(default)]
        cancel_function: Option<String>,
        #[serde(default)]
        end_fire_function: Option<String>,
        #[serde(default)]
        dead_bool_property: Option<String>,
        #[serde(default)]
        health_property: Option<String>,
        #[serde(default)]
        require_aim_bool: Option<String>,
        #[serde(default)]
        aim_bone_names: Vec<String>,
        #[serde(default = "default_auto_fire_distance")]
        max_distance: f64,
        #[serde(default = "default_auto_fire_fov_deg")]
        max_fov_deg: f64,
        #[serde(default = "default_auto_fire_interval_ms")]
        interval_ms: u32,
        #[serde(default = "default_aim_height_offset")]
        aim_height_offset: f64,
        #[serde(default)]
        aim_z_offset: f64,
        #[serde(default)]
        target_priority: Option<String>,
        #[serde(default = "default_true_bool")]
        require_line_of_sight: bool,
    },
    #[serde(rename = "spinbot")]
    SpinBot {
        feature_id: String,
        #[serde(default)]
        player_class: Option<String>,
        #[serde(default)]
        mesh_component: Option<String>,
        #[serde(default)]
        fps_camera_component: Option<String>,
        #[serde(default)]
        tps_camera_component: Option<String>,
        #[serde(default)]
        spring_arm_component: Option<String>,
        #[serde(default)]
        tps_base_component: Option<String>,
        #[serde(default)]
        switch_mesh_function: Option<String>,
        #[serde(default)]
        tps_mesh_index: Option<i32>,
        #[serde(default)]
        fps_mesh_index: Option<i32>,
        #[serde(default = "default_spin_speed_deg_per_sec")]
        spin_speed_deg_per_sec: f64,
        #[serde(default)]
        spin_pitch_deg: f64,
        #[serde(default = "default_spin_arm_length")]
        arm_length: f64,
        #[serde(default = "default_spin_tps_fov")]
        tps_fov: f64,
    },
    DamageAura {
        feature_id: String,
        enemy_class: String,
        #[serde(default)]
        player_class: Option<String>,
        #[serde(default)]
        dead_bool_property: Option<String>,
        #[serde(default)]
        health_property: Option<String>,
        #[serde(default = "default_aura_radius")]
        radius: f64,
        #[serde(default = "default_aura_damage")]
        damage: f64,
        #[serde(default = "default_aura_interval_ms")]
        interval_ms: u32,
        #[serde(default = "default_aura_max_targets")]
        max_targets_per_tick: u32,
        #[serde(default)]
        fallback_set_health_function: Option<String>,
        #[serde(default)]
        fallback_death_function: Option<String>,
    },
    #[serde(rename = "call_ufunction", alias = "call_u_function")]
    CallUFunction {
        feature_id: String,
        object_class: String,
        function_name: String,
    },
    UiTextLocalization {
        feature_id: String,
        #[serde(default)]
        entries: BTreeMap<String, String>,
        #[serde(default)]
        prefix_rules: Vec<UiTextPrefixRule>,
        #[serde(default)]
        font_fallback: Option<String>,
        #[serde(default = "default_localization_scan_interval_ms")]
        scan_interval_ms: u32,
        #[serde(default = "default_case_insensitive")]
        case_insensitive: bool,
    },
    #[serde(rename = "fill_items")]
    FillItems {
        feature_id: String,
        helper_class: String,
        #[serde(default)]
        helper_object_path: Option<String>,
        add_function: String,
        #[serde(default)]
        table_function: Option<String>,
        #[serde(default)]
        max_field: Option<String>,
        #[serde(default)]
        save_class: Option<String>,
        #[serde(default)]
        inventory_map_property: Option<String>,
        #[serde(default = "default_fill_item_amount")]
        default_amount: i32,
        #[serde(default)]
        fallback_items: Vec<Ue4ssFallbackItem>,
    },
    #[serde(rename = "trigger_goal")]
    TriggerGoal {
        feature_id: String,
        goal_class: String,
        #[serde(default)]
        goal_class_path: Option<String>,
        goal_function: String,
        #[serde(default)]
        require_live_class: Option<String>,
        #[serde(default)]
        wave_manager_class: Option<String>,
        #[serde(default)]
        manager_class: Option<String>,
        #[serde(default)]
        pre_clear_functions: Vec<String>,
        #[serde(default)]
        result_helper_class: Option<String>,
        #[serde(default)]
        result_helper_path: Option<String>,
        #[serde(default)]
        result_function: Option<String>,
        #[serde(default = "default_true_bool")]
        spawn_if_missing: bool,
    },
}

impl Ue4ssFeatureBinding {
    fn feature_id(&self) -> &str {
        match self {
            Self::CheatManagerCommand { feature_id, .. }
            | Self::CheatManagerFloat { feature_id, .. }
            | Self::ToggleUProperty { feature_id, .. }
            | Self::SetUPropertyScalar { feature_id, .. }
            | Self::AddUProperty { feature_id, .. }
            | Self::TargetLock { feature_id, .. }
            | Self::AutoFire { feature_id, .. }
            | Self::SpinBot { feature_id, .. }
            | Self::DamageAura { feature_id, .. }
            | Self::CallUFunction { feature_id, .. }
            | Self::UiTextLocalization { feature_id, .. }
            | Self::FillItems { feature_id, .. }
            | Self::TriggerGoal { feature_id, .. } => feature_id,
        }
    }
}

fn default_ue4ss_bridges_dir(custom_dir: Option<&str>) -> PathBuf {
    match custom_dir.map(str::trim).filter(|s| !s.is_empty()) {
        Some(custom) => PathBuf::from(custom),
        None => {
            if let Ok(local_app_data) = std::env::var("LOCALAPPDATA") {
                PathBuf::from(local_app_data).join("Nekod").join("bridges")
            } else {
                std::env::temp_dir().join("Nekod").join("bridges")
            }
        }
    }
}

fn parse_ue4ss_discovery_file(file: &Path, base_dir: &Path) -> Option<Ue4ssDiscovery> {
    let raw = fs::read_to_string(file).ok()?;
    let val: Value = serde_json::from_str(&raw).ok()?;
    let auth_token = val
        .get("auth_token")
        .or_else(|| val.get("token"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|t| (16..=128).contains(&t.len()))?
        .to_owned();

    let engine_version = val
        .get("engine_version")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    let bridge_version = val
        .get("bridge_version")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    let host = val
        .get("host")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("127.0.0.1")
        .to_owned();
    let port_opt = val
        .get("port")
        .and_then(Value::as_u64)
        .and_then(|p| u16::try_from(p).ok())
        .filter(|p| *p != 0);
    let ipc_dir_raw = val
        .get("ipc_dir")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty());

    let transport_hint = val.get("transport").and_then(Value::as_str);
    let use_file_ipc = if let Some(hint) = transport_hint {
        hint.eq_ignore_ascii_case("file_ipc")
    } else {
        port_opt.is_none() || ipc_dir_raw.is_some()
    };

    if use_file_ipc {
        let candidate_dir = ipc_dir_raw
            .map(PathBuf::from)
            .unwrap_or_else(|| base_dir.to_path_buf());
        let canon_base = fs::canonicalize(base_dir).ok()?;
        let canon_ipc = fs::canonicalize(&candidate_dir).ok()?;
        if !canon_ipc.is_dir() || !canon_ipc.starts_with(&canon_base) {
            return None;
        }
        Some(Ue4ssDiscovery {
            transport: Ue4ssTransportKind::FileIpc,
            host: "127.0.0.1".to_owned(),
            port: None,
            ipc_dir: Some(canon_ipc),
            auth_token,
            engine_version,
            bridge_version,
        })
    } else {
        let port = port_opt?;
        let addr = validate_loopback_host_port(&host, port).ok()?;
        Some(Ue4ssDiscovery {
            transport: Ue4ssTransportKind::Tcp,
            host: addr.ip().to_string(),
            port: Some(addr.port()),
            ipc_dir: None,
            auth_token,
            engine_version,
            bridge_version,
        })
    }
}

/// Resolves live `NekodUE4SSBridge` discovery from `<dir>/ue4ss-{pid}.json` or `ue4ss-active.json`.
pub fn resolve_nekod_ue4ss_discovery(
    target_pid: Option<u32>,
    custom_dir: Option<&str>,
) -> Option<Ue4ssDiscovery> {
    let base_dir = default_ue4ss_bridges_dir(custom_dir);
    let mut candidates = Vec::new();
    if let Some(pid) = target_pid
        && pid != 0
    {
        candidates.push(base_dir.join(format!("ue4ss-{pid}.json")));
    }
    candidates.push(base_dir.join("ue4ss-active.json"));

    for file in candidates {
        if file.is_file()
            && let Some(found) = parse_ue4ss_discovery_file(&file, &base_dir)
        {
            return Some(found);
        }
    }
    None
}

/// Sends a strictly read-only introspection RPC (`ping`, `find_objects`, `get_uproperty`,
/// `list_properties`, `list_functions`) to a live `NekodUE4SSBridge` instance discovered locally.
pub fn probe_nekod_ue4ss_bridge_readonly(
    target_pid: Option<u32>,
    custom_dir: Option<&str>,
    op: &str,
    params: Value,
) -> Result<Value, RuntimeError> {
    if !matches!(
        op,
        "ping"
            | "find_objects"
            | "dump_ui_texts"
            | "get_uproperty"
            | "list_properties"
            | "list_functions"
    ) {
        return Err(RuntimeError::ExecutionFailed(format!(
            "readonly UE4SS bridge probe refuses operation '{op}'"
        )));
    }

    let discovery = resolve_nekod_ue4ss_discovery(target_pid, custom_dir).ok_or_else(|| {
        RuntimeError::ExecutionFailed(
            "UE4SS companion bridge is not ready (ensure RE-UE4SS and NekodBridge are installed under <Game>/<Project>/Binaries/Win64/ and the game is running)".to_owned(),
        )
    })?;

    let req = BridgeRpcRequest {
        token: discovery.auth_token.clone(),
        op: op.to_owned(),
        params,
    };

    let resp = match discovery.transport {
        Ue4ssTransportKind::Tcp => {
            let port = discovery.port.ok_or_else(|| {
                RuntimeError::ExecutionFailed("UE4SS TCP discovery missing port".to_owned())
            })?;
            let endpoint = validate_loopback_bridge(&discovery.host, port, &discovery.auth_token)?;
            send_bridge_rpc(endpoint, &req)?
        }
        Ue4ssTransportKind::FileIpc => {
            let dir = discovery.ipc_dir.as_deref().ok_or_else(|| {
                RuntimeError::ExecutionFailed("UE4SS file IPC discovery missing ipc_dir".to_owned())
            })?;
            send_file_ipc_rpc(dir, &req)?
        }
    };

    Ok(resp.data)
}

fn send_ue4ss_rpc(
    endpoint: SocketAddr,
    file_ipc_dir: Option<&Path>,
    req: &BridgeRpcRequest,
) -> Result<crate::bridge_ipc::BridgeRpcResponse, RuntimeError> {
    let res = if let Some(dir) = file_ipc_dir {
        send_file_ipc_rpc(dir, req)
    } else {
        send_bridge_rpc(endpoint, req)
    };
    match res {
        Err(RuntimeError::ExecutionFailed(msg))
            if msg
                .to_ascii_lowercase()
                .contains("unsupported ue4ss bridge op") =>
        {
            Err(RuntimeError::ExecutionFailed(format!(
                "{msg} (installed NekodUE4SSBridge is outdated; close the game, click '更新伴生桥接' to upgrade to v0.7.3, then restart the game)"
            )))
        }
        other => other,
    }
}

fn has_ue4ss_dll_in_dir(dir: &Path) -> bool {
    if dir.join("ue4ss").join("UE4SS.dll").is_file() || dir.join("UE4SS.dll").is_file() {
        return true;
    }
    let root_win64 = dir.join("Binaries").join("Win64");
    if root_win64.join("ue4ss").join("UE4SS.dll").is_file()
        || root_win64.join("UE4SS.dll").is_file()
    {
        return true;
    }
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let sub = entry.path();
            if sub.is_dir() {
                let win64 = sub.join("Binaries").join("Win64");
                if win64.join("ue4ss").join("UE4SS.dll").is_file()
                    || win64.join("UE4SS.dll").is_file()
                {
                    return true;
                }
            }
        }
    }
    false
}

/// Controlled Unreal Engine `UE4SS` Reflection Bridge adapter (`ue4ss_bridge_v1`).
pub struct Ue4ssBridgeRuntime {
    store: BridgeSessionStore,
    bindings: Mutex<BTreeMap<String, Ue4ssBridgePayload>>,
}

impl Default for Ue4ssBridgeRuntime {
    fn default() -> Self {
        Self::new()
    }
}

fn validate_aim_tuning(
    kind: &str,
    feature_id: &str,
    aim_z_offset: f64,
    target_priority: &Option<String>,
) -> Result<(), RuntimeError> {
    if !aim_z_offset.is_finite() || !(-100.0..=100.0).contains(&aim_z_offset) {
        return Err(RuntimeError::InvalidPayload(format!(
            "{kind} feature '{feature_id}' aim_z_offset must be finite within -100..=100"
        )));
    }
    if let Some(priority) = target_priority {
        let p = priority.trim();
        if !matches!(p, "nearest" | "crosshair") {
            return Err(RuntimeError::InvalidPayload(format!(
                "{kind} feature '{feature_id}' target_priority '{priority}' must be 'nearest' or 'crosshair'"
            )));
        }
    }
    Ok(())
}

fn validate_toggle_target_spec(
    feature_id: &str,
    object_class: &str,
    property_name: &str,
    enabled_value: &Option<Value>,
    copy_from_property: &Option<String>,
    array_struct_fields: &[Ue4ssArrayStructField],
) -> Result<(), RuntimeError> {
    if object_class.trim().is_empty() || property_name.trim().is_empty() {
        return Err(RuntimeError::InvalidPayload(format!(
            "toggle_uproperty feature '{feature_id}' requires non-empty object_class and property_name"
        )));
    }
    if enabled_value.is_none()
        && copy_from_property
            .as_deref()
            .is_none_or(|s| s.trim().is_empty())
        && array_struct_fields.is_empty()
    {
        return Err(RuntimeError::InvalidPayload(format!(
            "toggle_uproperty feature '{feature_id}' target '{object_class}.{property_name}' must specify enabled_value, copy_from_property, or array_struct_fields"
        )));
    }
    if array_struct_fields
        .iter()
        .any(|f| f.field.trim().is_empty())
    {
        return Err(RuntimeError::InvalidPayload(format!(
            "toggle_uproperty feature '{feature_id}' contains empty array_struct_fields entry"
        )));
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn build_toggle_persist_target(
    object_class: &str,
    property_name: &str,
    enabled_value: &Option<Value>,
    disabled_value: &Option<Value>,
    copy_from_property: &Option<String>,
    array_struct_fields: &[Ue4ssArrayStructField],
    apply_all: bool,
    filter_property: &Option<Ue4ssPropertyFilter>,
    post_call_function: &Option<String>,
) -> Value {
    let mut map = serde_json::Map::new();
    map.insert("object_class".to_owned(), json!(object_class));
    map.insert("property_name".to_owned(), json!(property_name));
    map.insert("apply_all".to_owned(), json!(apply_all));
    if let Some(v) = enabled_value {
        map.insert("value".to_owned(), v.clone());
    }
    if let Some(rb) = disabled_value {
        map.insert("rollback_value".to_owned(), rb.clone());
    }
    if let Some(cf) = copy_from_property {
        map.insert("copy_from_property".to_owned(), json!(cf));
    }
    if !array_struct_fields.is_empty() {
        map.insert("array_struct_fields".to_owned(), json!(array_struct_fields));
    }
    if let Some(fp) = filter_property {
        map.insert("filter_property".to_owned(), json!(fp));
    }
    if let Some(post) = post_call_function {
        map.insert("post_call_function".to_owned(), json!(post));
    }
    Value::Object(map)
}

#[allow(clippy::too_many_arguments)]
fn build_scalar_persist_target(
    object_class: &str,
    property_name: &str,
    val: f64,
    multiply_original: bool,
    scale: f64,
    reciprocal: bool,
    as_int: bool,
    fallback_base: Option<f64>,
    reset_value: &Option<Value>,
    apply_all: bool,
    filter_property: &Option<Ue4ssPropertyFilter>,
    post_call_function: &Option<String>,
    copy_from_property: &Option<String>,
) -> Value {
    let mut map = serde_json::Map::new();
    map.insert("object_class".to_owned(), json!(object_class));
    map.insert("property_name".to_owned(), json!(property_name));
    map.insert("apply_all".to_owned(), json!(apply_all));
    map.insert("as_int".to_owned(), json!(as_int));
    if let Some(cf) = copy_from_property
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        map.insert("copy_from_property".to_owned(), json!(cf));
    } else if multiply_original {
        map.insert("multiply_original".to_owned(), json!(true));
        map.insert("multiplier".to_owned(), json!(val * scale));
        map.insert("reciprocal".to_owned(), json!(reciprocal));
        if let Some(fb) = fallback_base {
            map.insert("fallback_base".to_owned(), json!(fb));
        }
    } else {
        let computed = if reciprocal {
            1.0 / (val * scale)
        } else {
            val * scale
        };
        if as_int {
            map.insert("value".to_owned(), json!(computed.round() as i64));
        } else {
            map.insert("value".to_owned(), json!(computed));
        }
    }
    if let Some(rb) = reset_value {
        map.insert("rollback_value".to_owned(), rb.clone());
    }
    if let Some(fp) = filter_property {
        map.insert("filter_property".to_owned(), json!(fp));
    }
    if let Some(post) = post_call_function {
        map.insert("post_call_function".to_owned(), json!(post));
    }
    Value::Object(map)
}

impl Ue4ssBridgeRuntime {
    pub fn new() -> Self {
        Self {
            store: BridgeSessionStore::new(),
            bindings: Mutex::new(BTreeMap::new()),
        }
    }

    fn validate_payload(payload: &Ue4ssBridgePayload) -> Result<(), RuntimeError> {
        if !payload.schema_version.starts_with("1.") {
            return Err(RuntimeError::InvalidPayload(format!(
                "unsupported ue4ss_bridge_v1 schema_version '{}'",
                payload.schema_version
            )));
        }
        if payload.engine_version.trim().is_empty() {
            return Err(RuntimeError::InvalidPayload(
                "ue4ss_bridge_v1 requires explicit target engine_version (e.g. '5.3' or '*')"
                    .to_owned(),
            ));
        }
        if payload.features.is_empty() {
            return Err(RuntimeError::InvalidPayload(
                "ue4ss_bridge_v1 payload must define at least one feature".to_owned(),
            ));
        }
        for feature in &payload.features {
            match feature {
                Ue4ssFeatureBinding::CheatManagerFloat {
                    feature_id,
                    command_prefix,
                    ..
                } => {
                    if command_prefix.trim().is_empty() {
                        return Err(RuntimeError::InvalidPayload(format!(
                            "cheat_manager_float feature '{feature_id}' requires non-empty command_prefix"
                        )));
                    }
                }
                Ue4ssFeatureBinding::ToggleUProperty {
                    feature_id,
                    object_class,
                    property_name,
                    enabled_value,
                    copy_from_property,
                    array_struct_fields,
                    extra_targets,
                    ..
                } => {
                    validate_toggle_target_spec(
                        feature_id,
                        object_class,
                        property_name,
                        enabled_value,
                        copy_from_property,
                        array_struct_fields,
                    )?;
                    for extra in extra_targets {
                        validate_toggle_target_spec(
                            feature_id,
                            &extra.object_class,
                            &extra.property_name,
                            &extra.enabled_value,
                            &extra.copy_from_property,
                            &extra.array_struct_fields,
                        )?;
                    }
                }
                Ue4ssFeatureBinding::SetUPropertyScalar {
                    feature_id,
                    object_class,
                    property_name,
                    scale,
                    extra_targets,
                    ..
                } => {
                    if object_class.trim().is_empty() || property_name.trim().is_empty() {
                        return Err(RuntimeError::InvalidPayload(format!(
                            "set_uproperty_scalar feature '{feature_id}' requires non-empty object_class and property_name"
                        )));
                    }
                    if !scale.is_finite() || *scale <= 0.0 {
                        return Err(RuntimeError::InvalidPayload(format!(
                            "set_uproperty_scalar feature '{feature_id}' requires finite positive scale"
                        )));
                    }
                    for extra in extra_targets {
                        if extra.object_class.trim().is_empty()
                            || extra.property_name.trim().is_empty()
                            || !extra.scale.is_finite()
                            || extra.scale <= 0.0
                        {
                            return Err(RuntimeError::InvalidPayload(format!(
                                "set_uproperty_scalar feature '{feature_id}' has invalid extra_target"
                            )));
                        }
                    }
                }
                Ue4ssFeatureBinding::AddUProperty {
                    feature_id,
                    object_class,
                    property_name,
                    delta,
                    ..
                } => {
                    if object_class.trim().is_empty() || property_name.trim().is_empty() {
                        return Err(RuntimeError::InvalidPayload(format!(
                            "add_uproperty feature '{feature_id}' requires non-empty object_class and property_name"
                        )));
                    }
                    if !delta.is_finite() || *delta == 0.0 {
                        return Err(RuntimeError::InvalidPayload(format!(
                            "add_uproperty feature '{feature_id}' requires finite non-zero delta"
                        )));
                    }
                }
                Ue4ssFeatureBinding::TargetLock {
                    feature_id,
                    enemy_class,
                    max_distance,
                    max_fov_deg,
                    smooth,
                    aim_z_offset,
                    target_priority,
                    ..
                } => {
                    if enemy_class.trim().is_empty() {
                        return Err(RuntimeError::InvalidPayload(format!(
                            "target_lock feature '{feature_id}' requires non-empty enemy_class"
                        )));
                    }
                    if !(100.0..=50000.0).contains(max_distance)
                        || !(5.0..=180.0).contains(max_fov_deg)
                        || !(0.05..=1.0).contains(smooth)
                    {
                        return Err(RuntimeError::InvalidPayload(format!(
                            "target_lock feature '{feature_id}' bounds must satisfy max_distance 100..=50000, max_fov_deg 5..=180, smooth 0.05..=1.0"
                        )));
                    }
                    validate_aim_tuning("target_lock", feature_id, *aim_z_offset, target_priority)?;
                }
                Ue4ssFeatureBinding::AutoFire {
                    feature_id,
                    enemy_class,
                    fire_function,
                    aim_bone_names,
                    max_distance,
                    max_fov_deg,
                    interval_ms,
                    aim_z_offset,
                    target_priority,
                    ..
                } => {
                    if enemy_class.trim().is_empty() || fire_function.trim().is_empty() {
                        return Err(RuntimeError::InvalidPayload(format!(
                            "auto_fire feature '{feature_id}' requires non-empty enemy_class and fire_function"
                        )));
                    }
                    if !max_distance.is_finite()
                        || *max_distance <= 0.0
                        || *max_distance > 100_000.0
                        || !max_fov_deg.is_finite()
                        || *max_fov_deg <= 0.5
                        || *max_fov_deg > 180.0
                        || !(16..=5000).contains(interval_ms)
                    {
                        return Err(RuntimeError::InvalidPayload(format!(
                            "auto_fire feature '{feature_id}' bounds must satisfy max_distance (0..=100000], max_fov_deg (0.5..=180], interval_ms 16..=5000"
                        )));
                    }
                    if aim_bone_names.iter().any(|b| b.trim().is_empty()) {
                        return Err(RuntimeError::InvalidPayload(format!(
                            "auto_fire feature '{feature_id}' aim_bone_names must not contain empty entries"
                        )));
                    }
                    validate_aim_tuning("auto_fire", feature_id, *aim_z_offset, target_priority)?;
                }
                Ue4ssFeatureBinding::SpinBot {
                    feature_id,
                    spin_speed_deg_per_sec,
                    spin_pitch_deg,
                    arm_length,
                    tps_fov,
                    ..
                } => {
                    if !spin_speed_deg_per_sec.is_finite()
                        || !(30.0..=7200.0).contains(spin_speed_deg_per_sec)
                        || !spin_pitch_deg.is_finite()
                        || !(-89.0..=89.0).contains(spin_pitch_deg)
                        || !arm_length.is_finite()
                        || !(80.0..=800.0).contains(arm_length)
                        || !tps_fov.is_finite()
                        || !(40.0..=130.0).contains(tps_fov)
                    {
                        return Err(RuntimeError::InvalidPayload(format!(
                            "spinbot feature '{feature_id}' bounds must satisfy spin_speed 30..=7200, pitch -89..=89, arm_length 80..=800, tps_fov 40..=130"
                        )));
                    }
                }
                Ue4ssFeatureBinding::DamageAura {
                    feature_id,
                    enemy_class,
                    radius,
                    damage,
                    interval_ms,
                    max_targets_per_tick,
                    ..
                } => {
                    if enemy_class.trim().is_empty() {
                        return Err(RuntimeError::InvalidPayload(format!(
                            "damage_aura feature '{feature_id}' requires non-empty enemy_class"
                        )));
                    }
                    if !(100.0..=50000.0).contains(radius)
                        || !(1.0..=10_000_000.0).contains(damage)
                        || !(100..=5000).contains(interval_ms)
                        || !(1..=64).contains(max_targets_per_tick)
                    {
                        return Err(RuntimeError::InvalidPayload(format!(
                            "damage_aura feature '{feature_id}' bounds out of range (radius 100..=50000, damage 1..=1e7, interval_ms 100..=5000, max_targets_per_tick 1..=64)"
                        )));
                    }
                }
                Ue4ssFeatureBinding::UiTextLocalization {
                    feature_id,
                    entries,
                    prefix_rules,
                    font_fallback,
                    scan_interval_ms,
                    ..
                } => {
                    if entries.is_empty() && prefix_rules.is_empty() {
                        return Err(RuntimeError::InvalidPayload(format!(
                            "ui_text_localization feature '{feature_id}' must define at least one entry or prefix_rule"
                        )));
                    }
                    if entries.len() > 5000 {
                        return Err(RuntimeError::InvalidPayload(format!(
                            "ui_text_localization feature '{feature_id}' exceeds max 5000 entries"
                        )));
                    }
                    if prefix_rules.len() > 256 {
                        return Err(RuntimeError::InvalidPayload(format!(
                            "ui_text_localization feature '{feature_id}' exceeds max 256 prefix_rules"
                        )));
                    }
                    if prefix_rules.iter().any(|r| r.prefix.trim().is_empty()) {
                        return Err(RuntimeError::InvalidPayload(format!(
                            "ui_text_localization feature '{feature_id}' contains empty prefix rule"
                        )));
                    }
                    if !(200..=5000).contains(scan_interval_ms) {
                        return Err(RuntimeError::InvalidPayload(format!(
                            "ui_text_localization feature '{feature_id}' scan_interval_ms must be 200..=5000"
                        )));
                    }
                    if let Some(fb) = font_fallback.as_deref()
                        && (fb.len() > 256
                            || !(fb.starts_with("/Engine/") || fb.starts_with("/Game/"))
                            || fb.chars().any(|c| c.is_whitespace() || c.is_control()))
                    {
                        return Err(RuntimeError::InvalidPayload(format!(
                            "ui_text_localization feature '{feature_id}' font_fallback '{fb}' must be a valid /Engine/ or /Game/ asset path without whitespace"
                        )));
                    }
                }
                Ue4ssFeatureBinding::FillItems {
                    feature_id,
                    helper_class,
                    add_function,
                    default_amount,
                    fallback_items,
                    ..
                } => {
                    if helper_class.trim().is_empty() || add_function.trim().is_empty() {
                        return Err(RuntimeError::InvalidPayload(format!(
                            "fill_items feature '{feature_id}' requires non-empty helper_class and add_function"
                        )));
                    }
                    if !(1..=99_999).contains(default_amount) {
                        return Err(RuntimeError::InvalidPayload(format!(
                            "fill_items feature '{feature_id}' default_amount must be in 1..=99999"
                        )));
                    }
                    if fallback_items.len() > 2000 {
                        return Err(RuntimeError::InvalidPayload(format!(
                            "fill_items feature '{feature_id}' exceeds max 2000 fallback_items"
                        )));
                    }
                    if fallback_items.iter().any(|item| {
                        item.key.trim().is_empty() || !(1..=99_999).contains(&item.amount)
                    }) {
                        return Err(RuntimeError::InvalidPayload(format!(
                            "fill_items feature '{feature_id}' fallback_items must have non-empty key and amount in 1..=99999"
                        )));
                    }
                }
                Ue4ssFeatureBinding::TriggerGoal {
                    feature_id,
                    goal_class,
                    goal_function,
                    pre_clear_functions,
                    ..
                } => {
                    if goal_class.trim().is_empty() || goal_function.trim().is_empty() {
                        return Err(RuntimeError::InvalidPayload(format!(
                            "trigger_goal feature '{feature_id}' requires non-empty goal_class and goal_function"
                        )));
                    }
                    if pre_clear_functions.len() > 16
                        || pre_clear_functions.iter().any(|f| f.trim().is_empty())
                    {
                        return Err(RuntimeError::InvalidPayload(format!(
                            "trigger_goal feature '{feature_id}' pre_clear_functions must contain at most 16 non-empty names"
                        )));
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }
}

impl RuntimeProvider for Ue4ssBridgeRuntime {
    fn kind(&self) -> RuntimeKind {
        RuntimeKind::Ue4ssBridgeV1
    }

    fn adapter_metadata(&self) -> RuntimeAdapterMetadata {
        RuntimeAdapterMetadata {
            adapter_id: "ue4ss_bridge_v1".to_owned(),
            adapter_class: RuntimeAdapterClass::UnrealBridge,
            runtime_kind: RuntimeKind::Ue4ssBridgeV1,
            lifecycle_mode: RuntimeLifecycleMode::ManagedLoaderDeployment,
            manages_disk_files: true,
            requires_process_attach: false,
        }
    }

    fn check_dependencies(
        &self,
        dependencies: &[RuntimeDependencySpec],
        game_install_dir: Option<&Path>,
    ) -> Vec<DependencyCheckResult> {
        dependencies
            .iter()
            .map(|dep| {
                let satisfied = game_install_dir.and_then(|dir| {
                    if dep.dependency_id.contains("ue4ss") && has_ue4ss_dll_in_dir(dir) {
                        Some(dir.display().to_string())
                    } else {
                        None
                    }
                });
                DependencyCheckResult {
                    dependency_id: dep.dependency_id.clone(),
                    optional: dep.optional,
                    outcome: match satisfied {
                        Some(path) => DependencyStatus::Satisfied {
                            resolved_location: path,
                        },
                        None => DependencyStatus::Missing {
                            suggested_action: format!(
                                "Stage or install UE4SS '{}' ({}) into <Project>/Binaries/Win64/",
                                dep.dependency_id, dep.version_req
                            ),
                        },
                    },
                }
            })
            .collect()
    }

    fn discover_matching_processes(
        &self,
        _expected_process_names: &[String],
    ) -> Result<Vec<ProcessIdentity>, RuntimeError> {
        Ok(Vec::new())
    }

    fn start_session(&self, req: StartSessionRequest) -> Result<RuntimeSessionId, RuntimeError> {
        validate_session_preflight_for_kind(&req, RuntimeKind::Ue4ssBridgeV1)?;

        if !req
            .granted_permissions
            .allows(PermissionScope::ProcessWriteMemory)
        {
            return Err(RuntimeError::PermissionDenied(
                PermissionScope::ProcessWriteMemory,
            ));
        }

        let raw = req.declarative_payload_json.as_deref().ok_or_else(|| {
            RuntimeError::InvalidPayload("missing ue4ss_bridge_v1 JSON entrypoint".to_owned())
        })?;
        let payload: Ue4ssBridgePayload = serde_json::from_str(raw).map_err(|e| {
            RuntimeError::InvalidPayload(format!("invalid ue4ss_bridge_v1 JSON: {e}"))
        })?;
        Self::validate_payload(&payload)?;

        let (endpoint, effective_token, file_ipc_dir, transport_desc) = if payload.auto_discover {
            let discovery = resolve_nekod_ue4ss_discovery(
                Some(req.target_process.pid),
                payload.discovery_dir.as_deref(),
            )
            .ok_or_else(|| {
                RuntimeError::ExecutionFailed(
                    "UE4SS companion bridge is not ready (ensure RE-UE4SS + NekodBridge are installed into <Game>/<Project>/Binaries/Win64/ and the game is running)"
                        .to_owned(),
                )
            })?;
            match discovery.transport {
                Ue4ssTransportKind::Tcp => {
                    let port = discovery.port.ok_or_else(|| {
                        RuntimeError::ExecutionFailed("UE4SS discovery missing TCP port".to_owned())
                    })?;
                    let addr =
                        validate_loopback_bridge(&discovery.host, port, &discovery.auth_token)?;
                    (addr, discovery.auth_token, None, format!("TCP {addr}"))
                }
                Ue4ssTransportKind::FileIpc => {
                    let dir = discovery.ipc_dir.ok_or_else(|| {
                        RuntimeError::ExecutionFailed("UE4SS discovery missing ipc_dir".to_owned())
                    })?;
                    let placeholder = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0);
                    (
                        placeholder,
                        discovery.auth_token,
                        Some(dir.clone()),
                        format!("File-IPC {}", dir.display()),
                    )
                }
            }
        } else {
            let addr = validate_loopback_bridge(&payload.host, payload.port, &payload.auth_token)?;
            (
                addr,
                payload.auth_token.clone(),
                None,
                format!("TCP {addr}"),
            )
        };

        let ping = send_ue4ss_rpc(
            endpoint,
            file_ipc_dir.as_deref(),
            &BridgeRpcRequest {
                token: effective_token.clone(),
                op: "ping".to_owned(),
                params: json!({ "expected_engine_version": payload.engine_version }),
            },
        )?;
        let reported_ver = ping
            .data
            .get("engine_version")
            .and_then(Value::as_str)
            .unwrap_or("");
        if payload.engine_version != "*" && reported_ver != payload.engine_version {
            return Err(RuntimeError::ProcessIdentityMismatch(format!(
                "UE4SS engine version mismatch: expected '{}', bridge reported '{reported_ver}'",
                payload.engine_version
            )));
        }

        let session = ManagedBridgeSession {
            endpoint,
            auth_token: effective_token,
            target_process: req.target_process,
            runtime_kind: RuntimeKind::Ue4ssBridgeV1,
            use_unity_explorer_http: false,
            file_ipc_dir,
            state: SessionLifecycleState::Attached,
            rollback_stack: Vec::new(),
            rollback_failures: 0,
            recent_events: vec![format!(
                "Authenticated UE4SS {} bridge ({transport_desc})",
                if payload.engine_version == "*" {
                    reported_ver
                } else {
                    payload.engine_version.as_str()
                }
            )],
        };

        let sid = self.store.insert("ue4ss", session);
        self.bindings.lock().unwrap().insert(sid.0.clone(), payload);
        Ok(sid)
    }

    fn apply_feature(
        &self,
        session_id: &RuntimeSessionId,
        command: FeatureCommand,
    ) -> Result<(), RuntimeError> {
        let binding = {
            let guard = self.bindings.lock().unwrap();
            let payload = guard
                .get(&session_id.0)
                .ok_or_else(|| RuntimeError::SessionNotFound(session_id.clone()))?;
            payload
                .features
                .iter()
                .find(|f| f.feature_id() == command.feature_id)
                .cloned()
                .ok_or_else(|| RuntimeError::UnknownFeature(command.feature_id.clone()))?
        };

        let (endpoint, auth_token, file_ipc_dir) = self.store.session_transport(session_id)?;

        match (binding, command.value) {
            (
                Ue4ssFeatureBinding::CheatManagerCommand {
                    enable_command,
                    disable_command,
                    ..
                },
                FeatureValue::Toggle(enable),
            ) => {
                let cmd = if enable {
                    enable_command
                } else {
                    disable_command.clone().unwrap_or(enable_command)
                };
                send_ue4ss_rpc(
                    endpoint,
                    file_ipc_dir.as_deref(),
                    &BridgeRpcRequest {
                        token: auth_token.clone(),
                        op: "cheat_manager_exec".to_owned(),
                        params: json!({ "command": cmd }),
                    },
                )?;
                self.store.with_session_mut(session_id, |session| {
                    if enable && let Some(dis) = disable_command {
                        session
                            .rollback_stack
                            .retain(|(fid, _)| fid != &command.feature_id);
                        session.rollback_stack.push((
                            command.feature_id.clone(),
                            BridgeRpcRequest {
                                token: auth_token,
                                op: "cheat_manager_exec".to_owned(),
                                params: json!({ "command": dis }),
                            },
                        ));
                    } else if !enable {
                        session
                            .rollback_stack
                            .retain(|(fid, _)| fid != &command.feature_id);
                    }
                    session
                        .recent_events
                        .push(format!("UE4SS CheatManager '{cmd}' (enable={enable})"));
                    Ok(())
                })
            }
            (
                Ue4ssFeatureBinding::CheatManagerFloat {
                    command_prefix,
                    reset_command,
                    ..
                },
                FeatureValue::Float(val),
            ) => {
                if !val.is_finite() {
                    return Err(RuntimeError::InvalidPayload(format!(
                        "non-finite float value for UE4SS feature '{}'",
                        command.feature_id
                    )));
                }
                let cmd = format!("{} {val}", command_prefix.trim());
                send_ue4ss_rpc(
                    endpoint,
                    file_ipc_dir.as_deref(),
                    &BridgeRpcRequest {
                        token: auth_token.clone(),
                        op: "cheat_manager_exec".to_owned(),
                        params: json!({ "command": cmd }),
                    },
                )?;
                self.store.with_session_mut(session_id, |session| {
                    if let Some(reset) = reset_command {
                        session
                            .rollback_stack
                            .retain(|(fid, _)| fid != &command.feature_id);
                        session.rollback_stack.push((
                            command.feature_id.clone(),
                            BridgeRpcRequest {
                                token: auth_token,
                                op: "cheat_manager_exec".to_owned(),
                                params: json!({ "command": reset }),
                            },
                        ));
                    }
                    session
                        .recent_events
                        .push(format!("UE4SS CheatManager '{cmd}'"));
                    Ok(())
                })
            }
            (
                Ue4ssFeatureBinding::ToggleUProperty {
                    feature_id,
                    object_class,
                    property_name,
                    enabled_value,
                    disabled_value,
                    copy_from_property,
                    array_struct_fields,
                    persist,
                    apply_all,
                    filter_property,
                    post_call_function,
                    extra_targets,
                },
                FeatureValue::Toggle(enable),
            ) => {
                let use_sticky = persist
                    || !extra_targets.is_empty()
                    || copy_from_property.is_some()
                    || !array_struct_fields.is_empty()
                    || filter_property.is_some()
                    || post_call_function.is_some();

                if use_sticky {
                    if enable {
                        let mut targets = vec![build_toggle_persist_target(
                            &object_class,
                            &property_name,
                            &enabled_value,
                            &disabled_value,
                            &copy_from_property,
                            &array_struct_fields,
                            apply_all,
                            &filter_property,
                            &post_call_function,
                        )];
                        for extra in &extra_targets {
                            targets.push(build_toggle_persist_target(
                                &extra.object_class,
                                &extra.property_name,
                                &extra.enabled_value,
                                &extra.disabled_value,
                                &extra.copy_from_property,
                                &extra.array_struct_fields,
                                extra.apply_all,
                                &extra.filter_property,
                                &extra.post_call_function,
                            ));
                        }
                        let target_count = targets.len();
                        send_ue4ss_rpc(
                            endpoint,
                            file_ipc_dir.as_deref(),
                            &BridgeRpcRequest {
                                token: auth_token.clone(),
                                op: "persist_uproperty".to_owned(),
                                params: json!({
                                    "feature_id": feature_id,
                                    "targets": targets,
                                }),
                            },
                        )?;
                        self.store.with_session_mut(session_id, |session| {
                            session
                                .rollback_stack
                                .retain(|(fid, _)| fid != &command.feature_id);
                            session.rollback_stack.push((
                                command.feature_id.clone(),
                                BridgeRpcRequest {
                                    token: auth_token,
                                    op: "clear_persist_uproperty".to_owned(),
                                    params: json!({ "feature_id": feature_id }),
                                },
                            ));
                            session.recent_events.push(format!(
                                "UE4SS sticky UProperty '{feature_id}' enabled ({target_count} targets)"
                            ));
                            Ok(())
                        })
                    } else {
                        send_ue4ss_rpc(
                            endpoint,
                            file_ipc_dir.as_deref(),
                            &BridgeRpcRequest {
                                token: auth_token,
                                op: "clear_persist_uproperty".to_owned(),
                                params: json!({ "feature_id": feature_id }),
                            },
                        )?;
                        self.store.with_session_mut(session_id, |session| {
                            session
                                .rollback_stack
                                .retain(|(fid, _)| fid != &command.feature_id);
                            session.recent_events.push(format!(
                                "UE4SS sticky UProperty '{feature_id}' cleared & restored"
                            ));
                            Ok(())
                        })
                    }
                } else {
                    let val = if enable {
                        enabled_value.ok_or_else(|| {
                            RuntimeError::InvalidPayload(format!(
                                "toggle_uproperty '{feature_id}' missing enabled_value"
                            ))
                        })?
                    } else {
                        disabled_value.clone().ok_or_else(|| {
                            RuntimeError::InvalidPayload(format!(
                                "toggle_uproperty '{feature_id}' missing disabled_value"
                            ))
                        })?
                    };
                    send_ue4ss_rpc(
                        endpoint,
                        file_ipc_dir.as_deref(),
                        &BridgeRpcRequest {
                            token: auth_token.clone(),
                            op: "set_uproperty".to_owned(),
                            params: json!({
                                "object_class": object_class,
                                "property_name": property_name,
                                "value": val
                            }),
                        },
                    )?;
                    self.store.with_session_mut(session_id, |session| {
                        if enable {
                            session
                                .rollback_stack
                                .retain(|(fid, _)| fid != &command.feature_id);
                            session.rollback_stack.push((
                                command.feature_id.clone(),
                                BridgeRpcRequest {
                                    token: auth_token,
                                    op: "set_uproperty".to_owned(),
                                    params: json!({
                                        "object_class": object_class,
                                        "property_name": property_name,
                                        "value": disabled_value
                                    }),
                                },
                            ));
                        } else {
                            session
                                .rollback_stack
                                .retain(|(fid, _)| fid != &command.feature_id);
                        }
                        session.recent_events.push(format!(
                            "UE4SS UProperty '{object_class}.{property_name}' updated"
                        ));
                        Ok(())
                    })
                }
            }
            (
                Ue4ssFeatureBinding::SetUPropertyScalar {
                    feature_id,
                    object_class,
                    property_name,
                    multiply_original,
                    scale,
                    reciprocal,
                    as_int,
                    fallback_base,
                    persist,
                    reset_value,
                    apply_all,
                    filter_property,
                    post_call_function,
                    extra_targets,
                },
                cmd_val @ (FeatureValue::Float(_) | FeatureValue::Integer(_)),
            ) => {
                let val = match cmd_val {
                    FeatureValue::Float(v) => v,
                    FeatureValue::Integer(n) => n as f64,
                    _ => unreachable!(),
                };
                if !val.is_finite() || ((multiply_original || reciprocal) && val <= 0.0) {
                    return Err(RuntimeError::InvalidPayload(format!(
                        "invalid scalar value {val} for UE4SS feature '{feature_id}'"
                    )));
                }
                let use_persist = persist || multiply_original || !extra_targets.is_empty();
                if use_persist {
                    let mut targets = vec![build_scalar_persist_target(
                        &object_class,
                        &property_name,
                        val,
                        multiply_original,
                        scale,
                        reciprocal,
                        as_int,
                        fallback_base,
                        &reset_value,
                        apply_all,
                        &filter_property,
                        &post_call_function,
                        &None,
                    )];
                    for extra in &extra_targets {
                        targets.push(build_scalar_persist_target(
                            &extra.object_class,
                            &extra.property_name,
                            val,
                            multiply_original,
                            extra.scale,
                            extra.reciprocal,
                            extra.as_int,
                            extra.fallback_base,
                            &extra.reset_value,
                            extra.apply_all,
                            &extra.filter_property,
                            &extra.post_call_function,
                            &extra.copy_from_property,
                        ));
                    }
                    send_ue4ss_rpc(
                        endpoint,
                        file_ipc_dir.as_deref(),
                        &BridgeRpcRequest {
                            token: auth_token.clone(),
                            op: "persist_uproperty".to_owned(),
                            params: json!({
                                "feature_id": feature_id,
                                "targets": targets,
                            }),
                        },
                    )?;
                    self.store.with_session_mut(session_id, |session| {
                        session
                            .rollback_stack
                            .retain(|(fid, _)| fid != &command.feature_id);
                        session.rollback_stack.push((
                            command.feature_id.clone(),
                            BridgeRpcRequest {
                                token: auth_token,
                                op: "clear_persist_uproperty".to_owned(),
                                params: json!({ "feature_id": feature_id }),
                            },
                        ));
                        session.recent_events.push(format!(
                            "UE4SS scalar '{feature_id}' set to {val} (multiply_original={multiply_original})"
                        ));
                        Ok(())
                    })
                } else {
                    let computed = if reciprocal {
                        1.0 / (val * scale)
                    } else {
                        val * scale
                    };
                    let json_val = if as_int {
                        json!(computed.round() as i64)
                    } else {
                        json!(computed)
                    };
                    send_ue4ss_rpc(
                        endpoint,
                        file_ipc_dir.as_deref(),
                        &BridgeRpcRequest {
                            token: auth_token.clone(),
                            op: "set_uproperty".to_owned(),
                            params: json!({
                                "object_class": object_class,
                                "property_name": property_name,
                                "value": json_val,
                                "apply_all": apply_all,
                            }),
                        },
                    )?;
                    self.store.with_session_mut(session_id, |session| {
                        if let Some(rb) = reset_value {
                            session
                                .rollback_stack
                                .retain(|(fid, _)| fid != &command.feature_id);
                            session.rollback_stack.push((
                                command.feature_id.clone(),
                                BridgeRpcRequest {
                                    token: auth_token,
                                    op: "set_uproperty".to_owned(),
                                    params: json!({
                                        "object_class": object_class,
                                        "property_name": property_name,
                                        "value": rb,
                                        "apply_all": apply_all,
                                    }),
                                },
                            ));
                        }
                        session.recent_events.push(format!(
                            "UE4SS scalar '{object_class}.{property_name}' set to {json_val}"
                        ));
                        Ok(())
                    })
                }
            }
            (
                Ue4ssFeatureBinding::AddUProperty {
                    object_class,
                    property_name,
                    delta,
                    as_int,
                    min_value,
                    max_value,
                    apply_all,
                    filter_property,
                    post_call_function,
                    ..
                },
                FeatureValue::Action,
            ) => {
                send_ue4ss_rpc(
                    endpoint,
                    file_ipc_dir.as_deref(),
                    &BridgeRpcRequest {
                        token: auth_token,
                        op: "add_uproperty".to_owned(),
                        params: json!({
                            "object_class": object_class,
                            "property_name": property_name,
                            "delta": delta,
                            "as_int": as_int,
                            "min_value": min_value,
                            "max_value": max_value,
                            "apply_all": apply_all,
                            "filter_property": filter_property,
                            "post_call_function": post_call_function,
                        }),
                    },
                )?;
                self.store.with_session_mut(session_id, |session| {
                    session.recent_events.push(format!(
                        "UE4SS added {delta} to '{object_class}.{property_name}'"
                    ));
                    Ok(())
                })
            }
            (
                Ue4ssFeatureBinding::TargetLock {
                    feature_id,
                    enemy_class,
                    player_class,
                    dead_bool_property,
                    health_property,
                    require_aim_bool,
                    aim_bone_names,
                    max_distance,
                    max_fov_deg,
                    smooth,
                    aim_height_offset,
                    aim_z_offset,
                    target_priority,
                    require_line_of_sight,
                },
                FeatureValue::Toggle(enable),
            ) => {
                if enable {
                    send_ue4ss_rpc(
                        endpoint,
                        file_ipc_dir.as_deref(),
                        &BridgeRpcRequest {
                            token: auth_token.clone(),
                            op: "set_target_lock".to_owned(),
                            params: json!({
                                "feature_id": feature_id,
                                "enemy_class": enemy_class,
                                "player_class": player_class,
                                "dead_bool_property": dead_bool_property,
                                "health_property": health_property,
                                "require_aim_bool": require_aim_bool,
                                "aim_bone_names": aim_bone_names,
                                "max_distance": max_distance,
                                "max_fov_deg": max_fov_deg,
                                "smooth": smooth,
                                "aim_height_offset": aim_height_offset,
                                "aim_z_offset": aim_z_offset,
                                "target_priority": target_priority,
                                "require_line_of_sight": require_line_of_sight,
                            }),
                        },
                    )?;
                    self.store.with_session_mut(session_id, |session| {
                        session
                            .rollback_stack
                            .retain(|(fid, _)| fid != &command.feature_id);
                        session.rollback_stack.push((
                            command.feature_id.clone(),
                            BridgeRpcRequest {
                                token: auth_token,
                                op: "clear_target_lock".to_owned(),
                                params: json!({ "feature_id": feature_id }),
                            },
                        ));
                        session.recent_events.push(format!(
                            "UE4SS TargetLock '{feature_id}' enabled (enemy={enemy_class}, fov={max_fov_deg})"
                        ));
                        Ok(())
                    })
                } else {
                    send_ue4ss_rpc(
                        endpoint,
                        file_ipc_dir.as_deref(),
                        &BridgeRpcRequest {
                            token: auth_token,
                            op: "clear_target_lock".to_owned(),
                            params: json!({ "feature_id": feature_id }),
                        },
                    )?;
                    self.store.with_session_mut(session_id, |session| {
                        session
                            .rollback_stack
                            .retain(|(fid, _)| fid != &command.feature_id);
                        session
                            .recent_events
                            .push(format!("UE4SS TargetLock '{feature_id}' disabled"));
                        Ok(())
                    })
                }
            }
            (
                Ue4ssFeatureBinding::AutoFire {
                    feature_id,
                    enemy_class,
                    player_class,
                    fire_function,
                    direct_fire_function,
                    cancel_function,
                    end_fire_function,
                    dead_bool_property,
                    health_property,
                    require_aim_bool,
                    aim_bone_names,
                    max_distance,
                    max_fov_deg,
                    interval_ms,
                    aim_height_offset,
                    aim_z_offset,
                    target_priority,
                    require_line_of_sight,
                },
                FeatureValue::Toggle(enable),
            ) => {
                if enable {
                    send_ue4ss_rpc(
                        endpoint,
                        file_ipc_dir.as_deref(),
                        &BridgeRpcRequest {
                            token: auth_token.clone(),
                            op: "set_auto_fire".to_owned(),
                            params: json!({
                                "feature_id": feature_id,
                                "enemy_class": enemy_class,
                                "player_class": player_class,
                                "fire_function": fire_function,
                                "direct_fire_function": direct_fire_function,
                                "cancel_function": cancel_function,
                                "end_fire_function": end_fire_function,
                                "dead_bool_property": dead_bool_property,
                                "health_property": health_property,
                                "require_aim_bool": require_aim_bool,
                                "aim_bone_names": aim_bone_names,
                                "max_distance": max_distance,
                                "max_fov_deg": max_fov_deg,
                                "interval_ms": interval_ms,
                                "aim_height_offset": aim_height_offset,
                                "aim_z_offset": aim_z_offset,
                                "target_priority": target_priority,
                                "require_line_of_sight": require_line_of_sight,
                            }),
                        },
                    )?;
                    self.store.with_session_mut(session_id, |session| {
                        session
                            .rollback_stack
                            .retain(|(fid, _)| fid != &command.feature_id);
                        session.rollback_stack.push((
                            command.feature_id.clone(),
                            BridgeRpcRequest {
                                token: auth_token,
                                op: "clear_auto_fire".to_owned(),
                                params: json!({ "feature_id": feature_id }),
                            },
                        ));
                        session.recent_events.push(format!(
                            "UE4SS AutoFire '{feature_id}' enabled (enemy={enemy_class}, fov={max_fov_deg}, interval_ms={interval_ms})"
                        ));
                        Ok(())
                    })
                } else {
                    send_ue4ss_rpc(
                        endpoint,
                        file_ipc_dir.as_deref(),
                        &BridgeRpcRequest {
                            token: auth_token,
                            op: "clear_auto_fire".to_owned(),
                            params: json!({ "feature_id": feature_id }),
                        },
                    )?;
                    self.store.with_session_mut(session_id, |session| {
                        session
                            .rollback_stack
                            .retain(|(fid, _)| fid != &command.feature_id);
                        session
                            .recent_events
                            .push(format!("UE4SS AutoFire '{feature_id}' disabled"));
                        Ok(())
                    })
                }
            }
            (
                Ue4ssFeatureBinding::SpinBot {
                    feature_id,
                    player_class,
                    mesh_component,
                    fps_camera_component,
                    tps_camera_component,
                    spring_arm_component,
                    tps_base_component,
                    switch_mesh_function,
                    tps_mesh_index,
                    fps_mesh_index,
                    spin_speed_deg_per_sec,
                    spin_pitch_deg,
                    arm_length,
                    tps_fov,
                },
                FeatureValue::Toggle(enable),
            ) => {
                if enable {
                    send_ue4ss_rpc(
                        endpoint,
                        file_ipc_dir.as_deref(),
                        &BridgeRpcRequest {
                            token: auth_token.clone(),
                            op: "set_spinbot".to_owned(),
                            params: json!({
                                "feature_id": feature_id,
                                "player_class": player_class,
                                "mesh_component": mesh_component,
                                "fps_camera_component": fps_camera_component,
                                "tps_camera_component": tps_camera_component,
                                "spring_arm_component": spring_arm_component,
                                "tps_base_component": tps_base_component,
                                "switch_mesh_function": switch_mesh_function,
                                "tps_mesh_index": tps_mesh_index,
                                "fps_mesh_index": fps_mesh_index,
                                "spin_speed_deg_per_sec": spin_speed_deg_per_sec,
                                "spin_pitch_deg": spin_pitch_deg,
                                "arm_length": arm_length,
                                "tps_fov": tps_fov,
                            }),
                        },
                    )?;
                    self.store.with_session_mut(session_id, |session| {
                        session
                            .rollback_stack
                            .retain(|(fid, _)| fid != &command.feature_id);
                        session.rollback_stack.push((
                            command.feature_id.clone(),
                            BridgeRpcRequest {
                                token: auth_token,
                                op: "clear_spinbot".to_owned(),
                                params: json!({ "feature_id": feature_id }),
                            },
                        ));
                        session.recent_events.push(format!(
                            "UE4SS SpinBot '{feature_id}' enabled (speed={spin_speed_deg_per_sec}deg/s, arm={arm_length})"
                        ));
                        Ok(())
                    })
                } else {
                    send_ue4ss_rpc(
                        endpoint,
                        file_ipc_dir.as_deref(),
                        &BridgeRpcRequest {
                            token: auth_token,
                            op: "clear_spinbot".to_owned(),
                            params: json!({ "feature_id": feature_id }),
                        },
                    )?;
                    self.store.with_session_mut(session_id, |session| {
                        session
                            .rollback_stack
                            .retain(|(fid, _)| fid != &command.feature_id);
                        session
                            .recent_events
                            .push(format!("UE4SS SpinBot '{feature_id}' disabled"));
                        Ok(())
                    })
                }
            }
            (
                Ue4ssFeatureBinding::DamageAura {
                    feature_id,
                    enemy_class,
                    player_class,
                    dead_bool_property,
                    health_property,
                    radius,
                    damage,
                    interval_ms,
                    max_targets_per_tick,
                    fallback_set_health_function,
                    fallback_death_function,
                },
                FeatureValue::Toggle(enable),
            ) => {
                if enable {
                    send_ue4ss_rpc(
                        endpoint,
                        file_ipc_dir.as_deref(),
                        &BridgeRpcRequest {
                            token: auth_token.clone(),
                            op: "set_damage_aura".to_owned(),
                            params: json!({
                                "feature_id": feature_id,
                                "enemy_class": enemy_class,
                                "player_class": player_class,
                                "dead_bool_property": dead_bool_property,
                                "health_property": health_property,
                                "radius": radius,
                                "damage": damage,
                                "interval_ms": interval_ms,
                                "max_targets_per_tick": max_targets_per_tick,
                                "fallback_set_health_function": fallback_set_health_function,
                                "fallback_death_function": fallback_death_function,
                            }),
                        },
                    )?;
                    self.store.with_session_mut(session_id, |session| {
                        session
                            .rollback_stack
                            .retain(|(fid, _)| fid != &command.feature_id);
                        session.rollback_stack.push((
                            command.feature_id.clone(),
                            BridgeRpcRequest {
                                token: auth_token,
                                op: "clear_damage_aura".to_owned(),
                                params: json!({ "feature_id": feature_id }),
                            },
                        ));
                        session.recent_events.push(format!(
                            "UE4SS DamageAura '{feature_id}' enabled (enemy={enemy_class}, radius={radius}, damage={damage})"
                        ));
                        Ok(())
                    })
                } else {
                    send_ue4ss_rpc(
                        endpoint,
                        file_ipc_dir.as_deref(),
                        &BridgeRpcRequest {
                            token: auth_token,
                            op: "clear_damage_aura".to_owned(),
                            params: json!({ "feature_id": feature_id }),
                        },
                    )?;
                    self.store.with_session_mut(session_id, |session| {
                        session
                            .rollback_stack
                            .retain(|(fid, _)| fid != &command.feature_id);
                        session
                            .recent_events
                            .push(format!("UE4SS DamageAura '{feature_id}' disabled"));
                        Ok(())
                    })
                }
            }
            (
                Ue4ssFeatureBinding::CallUFunction {
                    object_class,
                    function_name,
                    ..
                },
                FeatureValue::Action,
            ) => {
                send_ue4ss_rpc(
                    endpoint,
                    file_ipc_dir.as_deref(),
                    &BridgeRpcRequest {
                        token: auth_token,
                        op: "call_ufunction".to_owned(),
                        params: json!({
                            "object_class": object_class,
                            "function_name": function_name
                        }),
                    },
                )?;
                self.store.with_session_mut(session_id, |session| {
                    session
                        .recent_events
                        .push(format!("UE4SS called '{object_class}::{function_name}'"));
                    Ok(())
                })
            }
            (
                Ue4ssFeatureBinding::UiTextLocalization {
                    feature_id,
                    entries,
                    prefix_rules,
                    font_fallback,
                    scan_interval_ms,
                    case_insensitive,
                },
                FeatureValue::Toggle(enable),
            ) => {
                if enable {
                    let entries_len = entries.len();
                    let prefix_len = prefix_rules.len();
                    send_ue4ss_rpc(
                        endpoint,
                        file_ipc_dir.as_deref(),
                        &BridgeRpcRequest {
                            token: auth_token.clone(),
                            op: "apply_localization".to_owned(),
                            params: json!({
                                "feature_id": feature_id,
                                "entries": entries,
                                "prefix_rules": prefix_rules,
                                "font_fallback": font_fallback,
                                "scan_interval_ms": scan_interval_ms,
                                "case_insensitive": case_insensitive,
                            }),
                        },
                    )?;
                    self.store.with_session_mut(session_id, |session| {
                        session
                            .rollback_stack
                            .retain(|(fid, _)| fid != &command.feature_id);
                        session.rollback_stack.push((
                            command.feature_id.clone(),
                            BridgeRpcRequest {
                                token: auth_token,
                                op: "clear_localization".to_owned(),
                                params: json!({ "feature_id": feature_id }),
                            },
                        ));
                        session.recent_events.push(format!(
                            "UE4SS UI localization '{feature_id}' activated ({entries_len} entries, {prefix_len} prefix rules)"
                        ));
                        Ok(())
                    })
                } else {
                    send_ue4ss_rpc(
                        endpoint,
                        file_ipc_dir.as_deref(),
                        &BridgeRpcRequest {
                            token: auth_token,
                            op: "clear_localization".to_owned(),
                            params: json!({ "feature_id": feature_id }),
                        },
                    )?;
                    self.store.with_session_mut(session_id, |session| {
                        session
                            .rollback_stack
                            .retain(|(fid, _)| fid != &command.feature_id);
                        session
                            .recent_events
                            .push(format!("UE4SS UI localization '{feature_id}' cleared"));
                        Ok(())
                    })
                }
            }
            (
                Ue4ssFeatureBinding::FillItems {
                    feature_id,
                    helper_class,
                    helper_object_path,
                    add_function,
                    table_function,
                    max_field,
                    save_class,
                    inventory_map_property,
                    default_amount,
                    fallback_items,
                },
                FeatureValue::Action,
            ) => {
                send_ue4ss_rpc(
                    endpoint,
                    file_ipc_dir.as_deref(),
                    &BridgeRpcRequest {
                        token: auth_token,
                        op: "fill_items".to_owned(),
                        params: json!({
                            "feature_id": feature_id,
                            "helper_class": helper_class,
                            "helper_object_path": helper_object_path,
                            "add_function": add_function,
                            "table_function": table_function,
                            "max_field": max_field,
                            "save_class": save_class,
                            "inventory_map_property": inventory_map_property,
                            "default_amount": default_amount,
                            "fallback_items": fallback_items,
                        }),
                    },
                )?;
                self.store.with_session_mut(session_id, |session| {
                    session.recent_events.push(format!(
                        "UE4SS FillItems '{feature_id}' executed ({helper_class}::{add_function})"
                    ));
                    Ok(())
                })
            }
            (
                Ue4ssFeatureBinding::TriggerGoal {
                    feature_id,
                    goal_class,
                    goal_class_path,
                    goal_function,
                    require_live_class,
                    wave_manager_class,
                    manager_class,
                    pre_clear_functions,
                    result_helper_class,
                    result_helper_path,
                    result_function,
                    spawn_if_missing,
                },
                FeatureValue::Action,
            ) => {
                send_ue4ss_rpc(
                    endpoint,
                    file_ipc_dir.as_deref(),
                    &BridgeRpcRequest {
                        token: auth_token,
                        op: "trigger_goal".to_owned(),
                        params: json!({
                            "feature_id": feature_id,
                            "goal_class": goal_class,
                            "goal_class_path": goal_class_path,
                            "goal_function": goal_function,
                            "require_live_class": require_live_class,
                            "wave_manager_class": wave_manager_class,
                            "manager_class": manager_class,
                            "pre_clear_functions": pre_clear_functions,
                            "result_helper_class": result_helper_class,
                            "result_helper_path": result_helper_path,
                            "result_function": result_function,
                            "spawn_if_missing": spawn_if_missing,
                        }),
                    },
                )?;
                self.store.with_session_mut(session_id, |session| {
                    session.recent_events.push(format!(
                        "UE4SS TriggerGoal '{feature_id}' executed ({goal_class}::{goal_function})"
                    ));
                    Ok(())
                })
            }
            _ => Err(RuntimeError::InvalidPayload(format!(
                "control type mismatch for UE4SS feature '{}'",
                command.feature_id
            ))),
        }
    }

    fn stop_session(
        &self,
        session_id: &RuntimeSessionId,
    ) -> Result<RuntimeDiagnostics, RuntimeError> {
        self.bindings.lock().unwrap().remove(&session_id.0);
        self.store.stop_and_rollback(session_id)
    }

    fn diagnostics(
        &self,
        session_id: &RuntimeSessionId,
    ) -> Result<RuntimeDiagnostics, RuntimeError> {
        self.store.diagnostics(session_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};
    use std::thread;

    use nekod_manifest::{
        AuthorInfo, BinaryArch, CompatibilityStatus, FeatureCategory, FeatureControlType,
        FeatureRiskLevel, PermissionDeclaration, SupportedGameVersion, TargetGameRef,
        TargetPlatform, TrainerArtifact, TrainerCompatibility, TrainerFeature, TrainerManifest,
        TrustTier,
    };
    use nekod_runtime_api::GrantedPermissions;

    use crate::CompositeRuntimeRouter;

    fn ue4ss_request(engine_ver: &str, port: u16) -> StartSessionRequest {
        let payload = serde_json::json!({
            "schema_version": "1.0.0",
            "engine_version": engine_ver,
            "host": "127.0.0.1",
            "port": port,
            "auth_token": "ue4ss_secret_token_9876543210",
            "features": [{
                "kind": "cheat_manager_command",
                "feature_id": "god-mode",
                "enable_command": "God",
                "disable_command": "God"
            }]
        })
        .to_string();

        let digest = "b".repeat(64);
        let manifest = TrainerManifest {
            schema_version: "1.0.0".parse().unwrap(),
            id: "dev.example.ue4ss-bridge".to_owned(),
            name: "UE4SS Bridge Mod".to_owned(),
            version: "1.0.0".parse().unwrap(),
            description: "UE4SS reflection bridge test".to_owned(),
            authors: vec![AuthorInfo {
                name: "Nekod".to_owned(),
                github: None,
                url: None,
            }],
            license: "MIT".to_owned(),
            source_repository: "https://github.com/OmaruKoma/Nekod".to_owned(),
            homepage: None,
            target_game: TargetGameRef {
                game_id: "sample-unreal-game".to_owned(),
                game_name: "Sample Unreal Game".to_owned(),
                store_ids: None,
            },
            supported_versions: vec![SupportedGameVersion {
                platform: TargetPlatform::WindowsX64,
                version_pattern: "*".to_owned(),
                executable_sha256: None,
                status: CompatibilityStatus::Verified,
            }],
            runtime: nekod_manifest::RuntimeRequirement {
                kind: RuntimeKind::Ue4ssBridgeV1,
                api_version: "1.0.0".parse().unwrap(),
                min_nekod_version: "0.1.0-alpha.0".parse().unwrap(),
                entrypoint: "ue4ss_bridge.json".to_owned(),
                supported_platforms: vec![TargetPlatform::WindowsX64],
            },
            compatibility: TrainerCompatibility {
                singleplayer_only: true,
                requires_offline_mode: false,
                anti_cheat_incompatible: vec![],
                last_verified_at: None,
            },
            permissions: vec![
                PermissionDeclaration {
                    scope: PermissionScope::ProcessDiscover,
                    reason: "Discover UE process".to_owned(),
                    required: true,
                    constraints: None,
                },
                PermissionDeclaration {
                    scope: PermissionScope::ProcessWriteMemory,
                    reason: "Invoke CheatManager and mutate UProperties via UE4SS".to_owned(),
                    required: true,
                    constraints: None,
                },
            ],
            features: vec![TrainerFeature {
                id: "god-mode".to_owned(),
                name: "God Mode".to_owned(),
                description: "Toggle CheatManager God command".to_owned(),
                category: FeatureCategory::Player,
                control_type: FeatureControlType::Toggle,
                default_hotkey: None,
                mutates_memory: true,
                supports_rollback: true,
                risk_level: FeatureRiskLevel::Low,
                value_range: None,
            }],
            artifacts: vec![TrainerArtifact {
                filename: "ue4ss_bridge.json".to_owned(),
                sha256: digest.clone(),
                size_bytes: payload.len() as u64,
                platform: None,
            }],
            extensions: None,
        };

        let mut granted_scopes = BTreeSet::new();
        granted_scopes.insert(PermissionScope::ProcessDiscover);
        granted_scopes.insert(PermissionScope::ProcessWriteMemory);

        StartSessionRequest {
            trainer: manifest.clone(),
            target_process: ProcessIdentity {
                pid: 6200,
                process_name: "SampleUE5-Win64-Shipping.exe".to_owned(),
                executable_path: "C:\\Games\\SampleUE5.exe".to_owned(),
                platform: TargetPlatform::WindowsX64,
                architecture: BinaryArch::X86_64,
                executable_sha256: None,
                detected_version: None,
                process_creation_time_100ns: None,
            },
            granted_permissions: GrantedPermissions {
                trainer_id: manifest.id,
                trainer_version: "1.0.0".to_owned(),
                manifest_sha256: digest,
                trust_tier: TrustTier::CommunityVerified,
                granted_scopes,
                scope_constraints: BTreeMap::new(),
            },
            declarative_payload_json: Some(payload),
            verified_package_assets: BTreeMap::new(),
        }
    }

    #[test]
    fn ue4ss_bridge_verifies_engine_version_executes_cheat_manager_and_rolls_back() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let ops = Arc::new(Mutex::new(Vec::<String>::new()));
        let ops_clone = ops.clone();

        let thread_handle = thread::spawn(move || {
            for stream in listener.incoming().take(4) {
                let mut stream = stream.unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                let req: serde_json::Value = serde_json::from_str(&line).unwrap();
                let op = req["op"].as_str().unwrap().to_owned();
                ops_clone.lock().unwrap().push(op.clone());
                if op == "ping" {
                    writeln!(
                        stream,
                        "{{\"ok\":true,\"data\":{{\"engine_version\":\"5.3\"}}}}"
                    )
                    .unwrap();
                } else {
                    writeln!(stream, "{{\"ok\":true,\"data\":{{}}}}").unwrap();
                }
            }
        });

        let router = CompositeRuntimeRouter::with_default_providers();

        // Version mismatch fails before creating session.
        assert!(matches!(
            router.start_session(ue4ss_request("4.27", port)),
            Err(RuntimeError::ProcessIdentityMismatch(_))
        ));

        let sid = router.start_session(ue4ss_request("5.3", port)).unwrap();
        router
            .apply_feature(
                &sid,
                FeatureCommand {
                    feature_id: "god-mode".to_owned(),
                    value: FeatureValue::Toggle(true),
                },
            )
            .unwrap();
        let diag = router.stop_session(&sid).unwrap();
        assert_eq!(diag.runtime_kind, RuntimeKind::Ue4ssBridgeV1);
        assert_eq!(diag.rollback_failures, 0);

        thread_handle.join().unwrap();
        let recorded = ops.lock().unwrap();
        assert_eq!(
            recorded.as_slice(),
            &["ping", "ping", "cheat_manager_exec", "cheat_manager_exec"]
        );
    }

    #[test]
    fn ue4ss_file_ipc_auto_discover_executes_cheat_manager_float_and_rolls_back() {
        let dir = std::env::temp_dir().join(format!(
            "nekod_ue4ss_file_ipc_test_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        let token = "ue4ss_file_ipc_test_token_123456";
        fs::write(
            dir.join("ue4ss-active.json"),
            serde_json::json!({
                "bridge_version": "0.2.0",
                "transport": "file_ipc",
                "ipc_dir": dir.to_string_lossy(),
                "auth_token": token,
                "engine_version": "4.27"
            })
            .to_string(),
        )
        .unwrap();

        let captured = Arc::new(Mutex::new(Vec::<(String, serde_json::Value)>::new()));
        let captured_clone = Arc::clone(&captured);
        let dir_clone = dir.clone();
        let worker = thread::spawn(move || {
            let mut handled = 0usize;
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while handled < 4 && std::time::Instant::now() < deadline {
                let req_path = dir_clone.join("ue4ss-request.json");
                if req_path.is_file() {
                    let raw = match fs::read_to_string(&req_path) {
                        Ok(s) => s,
                        Err(_) => {
                            thread::sleep(std::time::Duration::from_millis(10));
                            continue;
                        }
                    };
                    let _ = fs::remove_file(&req_path);
                    let req: serde_json::Value = serde_json::from_str(&raw).unwrap();
                    assert_eq!(req["token"], token);
                    let id = req["id"].as_str().unwrap();
                    let op = req["op"].as_str().unwrap().to_owned();
                    let params = req["params"].clone();
                    captured_clone.lock().unwrap().push((op.clone(), params));

                    let resp = if op == "ping" {
                        serde_json::json!({
                            "ok": true,
                            "data": { "bridge_version": "0.2.0", "engine_version": "4.27", "transport": "file_ipc" }
                        })
                    } else {
                        serde_json::json!({ "ok": true, "data": { "executed": true } })
                    };
                    let resp_path = dir_clone.join(format!("ue4ss-response-{id}.json"));
                    fs::write(&resp_path, resp.to_string()).unwrap();
                    handled += 1;
                } else {
                    thread::sleep(std::time::Duration::from_millis(10));
                }
            }
        });

        let payload = serde_json::json!({
            "schema_version": "1.0.0",
            "engine_version": "*",
            "auto_discover": true,
            "discovery_dir": dir.to_string_lossy(),
            "features": [
                {
                    "kind": "cheat_manager_float",
                    "feature_id": "game-speed",
                    "command_prefix": "Slomo",
                    "reset_command": "Slomo 1.0"
                }
            ]
        })
        .to_string();

        let mut base = ue4ss_request("5.3", 19541);
        base.declarative_payload_json = Some(payload);

        let router = CompositeRuntimeRouter::with_default_providers();
        let sid = router.start_session(base).unwrap();
        // Repeat float changes twice and ensure rollback collapses to single LIFO reset.
        router
            .apply_feature(
                &sid,
                FeatureCommand {
                    feature_id: "game-speed".to_owned(),
                    value: FeatureValue::Float(2.0),
                },
            )
            .unwrap();
        router
            .apply_feature(
                &sid,
                FeatureCommand {
                    feature_id: "game-speed".to_owned(),
                    value: FeatureValue::Float(3.5),
                },
            )
            .unwrap();
        let diag = router.stop_session(&sid).unwrap();
        assert_eq!(diag.rollback_failures, 0);

        worker.join().unwrap();
        let ops = captured.lock().unwrap();
        assert_eq!(ops.len(), 4);
        assert_eq!(ops[0].0, "ping");
        assert_eq!(ops[1].1["command"], "Slomo 2");
        assert_eq!(ops[2].1["command"], "Slomo 3.5");
        assert_eq!(ops[3].1["command"], "Slomo 1.0");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn ue4ss_readonly_probe_allows_introspection_and_refuses_mutations() {
        let dir = std::env::temp_dir().join(format!(
            "nekod_ue4ss_probe_test_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        let dir_str = dir.to_string_lossy().into_owned();
        let token = "ue4ss_probe_token_abcdef123456";

        // 1. Refuse mutating ops immediately before any transport call.
        for forbidden in ["cheat_manager_exec", "set_uproperty", "call_ufunction"] {
            let err = probe_nekod_ue4ss_bridge_readonly(
                None,
                Some(&dir_str),
                forbidden,
                serde_json::json!({}),
            )
            .unwrap_err();
            assert!(err.to_string().contains("refuses operation"));
        }

        // 2. Test TCP discovery probe.
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        fs::write(
            dir.join("ue4ss-active.json"),
            serde_json::json!({
                "bridge_version": "0.2.0",
                "transport": "tcp",
                "host": "127.0.0.1",
                "port": port,
                "auth_token": token,
                "engine_version": "5.3"
            })
            .to_string(),
        )
        .unwrap();

        let tcp_server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            let req: serde_json::Value = serde_json::from_str(&line).unwrap();
            assert_eq!(req["op"], "find_objects");
            writeln!(
                stream,
                "{{\"ok\":true,\"data\":{{\"class_name\":\"PlayerController\",\"count\":1}}}}"
            )
            .unwrap();
        });

        let tcp_data = probe_nekod_ue4ss_bridge_readonly(
            None,
            Some(&dir_str),
            "find_objects",
            serde_json::json!({ "class_name": "PlayerController" }),
        )
        .unwrap();
        assert_eq!(tcp_data["count"], 1);
        tcp_server.join().unwrap();

        // 3. Test File-IPC discovery probe.
        fs::write(
            dir.join("ue4ss-active.json"),
            serde_json::json!({
                "bridge_version": "0.2.0",
                "transport": "file_ipc",
                "ipc_dir": dir_str,
                "auth_token": token,
                "engine_version": "4.27"
            })
            .to_string(),
        )
        .unwrap();

        let dir_clone = dir.clone();
        let file_worker = thread::spawn(move || {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
            while std::time::Instant::now() < deadline {
                let req_path = dir_clone.join("ue4ss-request.json");
                if req_path.is_file() {
                    let raw = fs::read_to_string(&req_path).unwrap();
                    let _ = fs::remove_file(&req_path);
                    let req: serde_json::Value = serde_json::from_str(&raw).unwrap();
                    assert_eq!(req["op"], "list_properties");
                    let id = req["id"].as_str().unwrap();
                    fs::write(
                        dir_clone.join(format!("ue4ss-response-{id}.json")),
                        serde_json::json!({
                            "ok": true,
                            "data": { "object_class": "PlayerController", "count": 1, "properties": [] }
                        })
                        .to_string(),
                    )
                    .unwrap();
                    break;
                }
                thread::sleep(std::time::Duration::from_millis(10));
            }
        });

        let ipc_data = probe_nekod_ue4ss_bridge_readonly(
            None,
            Some(&dir_str),
            "list_properties",
            serde_json::json!({ "object_class": "PlayerController" }),
        )
        .unwrap();
        assert_eq!(ipc_data["object_class"], "PlayerController");
        file_worker.join().unwrap();

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn ue4ss_ui_text_localization_validates_applies_and_rolls_back_via_file_ipc() {
        let dir = std::env::temp_dir().join(format!(
            "nekod_ue4ss_loc_test_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        let dir_str = dir.to_string_lossy().into_owned();
        let token = "ue4ss_loc_test_token_abcdef123456";

        // 1. Payload validation must reject empty localization dictionaries, out-of-range
        //    scan intervals, and unsafe font_fallback paths.
        let mut bad_empty = ue4ss_request("*", 19541);
        bad_empty.declarative_payload_json = Some(
            serde_json::json!({
                "schema_version": "1.0.0",
                "engine_version": "*",
                "auto_discover": true,
                "discovery_dir": dir_str,
                "features": [{
                    "kind": "ui_text_localization",
                    "feature_id": "zh-cn-ui",
                    "entries": {},
                    "prefix_rules": []
                }]
            })
            .to_string(),
        );
        let rt = Ue4ssBridgeRuntime::new();
        assert!(matches!(
            rt.start_session(bad_empty),
            Err(RuntimeError::InvalidPayload(_))
        ));

        let mut bad_interval = ue4ss_request("*", 19541);
        bad_interval.declarative_payload_json = Some(
            serde_json::json!({
                "schema_version": "1.0.0",
                "engine_version": "*",
                "auto_discover": true,
                "discovery_dir": dir_str,
                "features": [{
                    "kind": "ui_text_localization",
                    "feature_id": "zh-cn-ui",
                    "entries": { "RESUME": "继续" },
                    "scan_interval_ms": 50
                }]
            })
            .to_string(),
        );
        assert!(matches!(
            rt.start_session(bad_interval),
            Err(RuntimeError::InvalidPayload(_))
        ));

        let mut bad_font = ue4ss_request("*", 19541);
        bad_font.declarative_payload_json = Some(
            serde_json::json!({
                "schema_version": "1.0.0",
                "engine_version": "*",
                "auto_discover": true,
                "discovery_dir": dir_str,
                "features": [{
                    "kind": "ui_text_localization",
                    "feature_id": "zh-cn-ui",
                    "entries": { "RESUME": "继续" },
                    "font_fallback": "C:/Windows/Fonts/msyh.ttc"
                }]
            })
            .to_string(),
        );
        assert!(matches!(
            rt.start_session(bad_font),
            Err(RuntimeError::InvalidPayload(_))
        ));

        // 2. Readonly probe must allow `dump_ui_texts` and refuse `apply_localization`.
        assert!(
            probe_nekod_ue4ss_bridge_readonly(
                None,
                Some(&dir_str),
                "apply_localization",
                serde_json::json!({}),
            )
            .unwrap_err()
            .to_string()
            .contains("refuses operation")
        );

        fs::write(
            dir.join("ue4ss-active.json"),
            serde_json::json!({
                "bridge_version": "0.3.0",
                "transport": "file_ipc",
                "ipc_dir": dir_str,
                "auth_token": token,
                "engine_version": "4.27"
            })
            .to_string(),
        )
        .unwrap();

        let dir_clone = dir.clone();
        let observed_ops = Arc::new(Mutex::new(Vec::<String>::new()));
        let observed_clone = Arc::clone(&observed_ops);
        let worker = thread::spawn(move || {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            let mut handled = 0usize;
            while handled < 4 && std::time::Instant::now() < deadline {
                let req_path = dir_clone.join("ue4ss-request.json");
                if req_path.is_file() {
                    let raw = fs::read_to_string(&req_path).unwrap();
                    let _ = fs::remove_file(&req_path);
                    let req: serde_json::Value = serde_json::from_str(&raw).unwrap();
                    let id = req["id"].as_str().unwrap();
                    let op = req["op"].as_str().unwrap().to_owned();
                    observed_clone.lock().unwrap().push(op.clone());
                    let data = match op.as_str() {
                        "dump_ui_texts" => serde_json::json!({
                            "total_scanned": 1,
                            "count": 1,
                            "texts": [{ "index": 1, "text": "RESUME" }]
                        }),
                        "ping" => serde_json::json!({
                            "bridge_version": "0.3.0",
                            "engine_version": "4.27",
                            "transport": "file_ipc"
                        }),
                        "apply_localization" => {
                            assert_eq!(req["params"]["feature_id"], "zh-cn-ui");
                            assert_eq!(req["params"]["entries"]["RESUME"], "继续游戏");
                            assert_eq!(
                                req["params"]["font_fallback"],
                                "/Engine/EngineFonts/Roboto.Roboto"
                            );
                            serde_json::json!({
                                "feature_id": "zh-cn-ui",
                                "entries_loaded": 2,
                                "translated_widgets": 1,
                                "font_fallback_loaded": true
                            })
                        }
                        "clear_localization" => {
                            assert_eq!(req["params"]["feature_id"], "zh-cn-ui");
                            serde_json::json!({
                                "feature_id": "zh-cn-ui",
                                "restored_widgets": 1,
                                "skipped_invalid": 0
                            })
                        }
                        other => panic!("unexpected op: {other}"),
                    };
                    fs::write(
                        dir_clone.join(format!("ue4ss-response-{id}.json")),
                        serde_json::json!({ "ok": true, "data": data }).to_string(),
                    )
                    .unwrap();
                    handled += 1;
                } else {
                    thread::sleep(std::time::Duration::from_millis(10));
                }
            }
        });

        let dump = probe_nekod_ue4ss_bridge_readonly(
            None,
            Some(&dir_str),
            "dump_ui_texts",
            serde_json::json!({ "live_only": true, "limit": 50 }),
        )
        .unwrap();
        assert_eq!(dump["texts"][0]["text"], "RESUME");

        let mut req = ue4ss_request("*", 19541);
        req.declarative_payload_json = Some(
            serde_json::json!({
                "schema_version": "1.0.0",
                "engine_version": "*",
                "auto_discover": true,
                "discovery_dir": dir_str,
                "features": [{
                    "kind": "ui_text_localization",
                    "feature_id": "zh-cn-ui",
                    "entries": { "RESUME": "继续游戏" },
                    "prefix_rules": [{ "prefix": "Stage ", "replacement": "关卡 " }],
                    "font_fallback": "/Engine/EngineFonts/Roboto.Roboto",
                    "scan_interval_ms": 500,
                    "case_insensitive": true
                }]
            })
            .to_string(),
        );
        let sid = rt.start_session(req).unwrap();
        rt.apply_feature(
            &sid,
            FeatureCommand {
                feature_id: "zh-cn-ui".to_owned(),
                value: FeatureValue::Toggle(true),
            },
        )
        .unwrap();
        let diag = rt.stop_session(&sid).unwrap();
        assert_eq!(diag.rollback_failures, 0);

        worker.join().unwrap();
        assert_eq!(
            *observed_ops.lock().unwrap(),
            vec![
                "dump_ui_texts",
                "ping",
                "apply_localization",
                "clear_localization"
            ]
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn ue4ss_ui_localization_and_cheat_manager_payload_validates() {
        let raw = r#"{
            "schema_version": "1.0.0",
            "engine_version": "*",
            "auto_discover": true,
            "features": [
                {
                    "kind": "ui_text_localization",
                    "feature_id": "zh-cn-ui",
                    "font_fallback": "/Engine/EngineFonts/Roboto.Roboto",
                    "entries": { "Start Game": "开始游戏" },
                    "prefix_rules": [{ "prefix": "Settings", "replacement": "设置" }]
                },
                {
                    "kind": "cheat_manager_command",
                    "feature_id": "time-pause",
                    "enable_command": "Pause",
                    "disable_command": "Pause"
                },
                {
                    "kind": "cheat_manager_command",
                    "feature_id": "debug-camera",
                    "enable_command": "ToggleDebugCamera",
                    "disable_command": "ToggleDebugCamera"
                }
            ]
        }"#;
        let payload: Ue4ssBridgePayload =
            serde_json::from_str(raw).expect("ui localization payload parses");
        Ue4ssBridgeRuntime::validate_payload(&payload).expect("ui localization payload validates");
        assert!(payload.auto_discover);
        assert_eq!(payload.features.len(), 3);
    }

    #[test]
    fn ue4ss_sticky_target_lock_damage_aura_and_add_uproperty_roundtrip_and_rollback() {
        let dir = std::env::temp_dir().join(format!(
            "nekod_ue4ss_v04_test_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        let dir_str = dir.to_string_lossy().into_owned();
        let token = "ue4ss_v04_test_token_abcdef123456";

        // 1. Validation rejections for new bindings.
        let rt = Ue4ssBridgeRuntime::new();
        for bad_feature in [
            serde_json::json!({
                "kind": "target_lock",
                "feature_id": "bad-fov",
                "enemy_class": "BP_NewEnemy_Base_C",
                "max_fov_deg": 240.0
            }),
            serde_json::json!({
                "kind": "damage_aura",
                "feature_id": "bad-interval",
                "enemy_class": "BP_NewEnemy_Base_C",
                "interval_ms": 20
            }),
            serde_json::json!({
                "kind": "add_uproperty",
                "feature_id": "bad-zero-delta",
                "object_class": "MyEMSPersistentSaveGame_C",
                "property_name": "Money",
                "delta": 0.0
            }),
        ] {
            let mut bad_req = ue4ss_request("*", 19541);
            bad_req.declarative_payload_json = Some(
                serde_json::json!({
                    "schema_version": "1.0.0",
                    "engine_version": "*",
                    "auto_discover": true,
                    "discovery_dir": dir_str,
                    "features": [bad_feature]
                })
                .to_string(),
            );
            assert!(matches!(
                rt.start_session(bad_req),
                Err(RuntimeError::InvalidPayload(_))
            ));
        }

        // 2. Readonly probe must refuse all new mutating ops.
        for forbidden in [
            "persist_uproperty",
            "clear_persist_uproperty",
            "add_uproperty",
            "set_target_lock",
            "clear_target_lock",
            "set_damage_aura",
            "clear_damage_aura",
        ] {
            let err = probe_nekod_ue4ss_bridge_readonly(
                None,
                Some(&dir_str),
                forbidden,
                serde_json::json!({}),
            )
            .unwrap_err();
            assert!(err.to_string().contains("refuses operation"));
        }

        // 3. Round-trip sticky persist, scalar multiplier, add_uproperty, target_lock,
        //    damage_aura, and verify LIFO rollback on stop_session.
        fs::write(
            dir.join("ue4ss-active.json"),
            serde_json::json!({
                "bridge_version": "0.4.0",
                "transport": "file_ipc",
                "ipc_dir": dir_str,
                "auth_token": token,
                "engine_version": "4.27"
            })
            .to_string(),
        )
        .unwrap();

        let dir_clone = dir.clone();
        let observed_ops = Arc::new(Mutex::new(Vec::<String>::new()));
        let observed_clone = Arc::clone(&observed_ops);
        let worker = thread::spawn(move || {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(6);
            let mut handled = 0usize;
            while handled < 10 && std::time::Instant::now() < deadline {
                let req_path = dir_clone.join("ue4ss-request.json");
                if req_path.is_file() {
                    let raw = fs::read_to_string(&req_path).unwrap();
                    let _ = fs::remove_file(&req_path);
                    let req: serde_json::Value = serde_json::from_str(&raw).unwrap();
                    let id = req["id"].as_str().unwrap();
                    let op = req["op"].as_str().unwrap().to_owned();
                    observed_clone.lock().unwrap().push(op.clone());
                    let data = match op.as_str() {
                        "ping" => serde_json::json!({
                            "bridge_version": "0.4.0",
                            "engine_version": "4.27",
                            "transport": "file_ipc"
                        }),
                        "persist_uproperty" => {
                            let targets = req["params"]["targets"].as_array().unwrap();
                            assert!(!targets.is_empty());
                            serde_json::json!({
                                "feature_id": req["params"]["feature_id"],
                                "updated": 1,
                                "pending": false
                            })
                        }
                        "add_uproperty" => {
                            assert_eq!(req["params"]["delta"], 100000.0);
                            serde_json::json!({ "updated": 1, "new_value": 100000 })
                        }
                        "set_target_lock" => {
                            assert_eq!(req["params"]["enemy_class"], "BP_NewEnemy_Base_C");
                            serde_json::json!({ "feature_id": "auto-lock-enemy", "active": true })
                        }
                        "set_damage_aura" => {
                            assert_eq!(req["params"]["radius"], 3500.0);
                            serde_json::json!({ "feature_id": "kill-aura", "active": true })
                        }
                        "clear_damage_aura" | "clear_target_lock" | "clear_persist_uproperty" => {
                            serde_json::json!({ "cleared": true })
                        }
                        other => panic!("unexpected op: {other}"),
                    };
                    fs::write(
                        dir_clone.join(format!("ue4ss-response-{id}.json")),
                        serde_json::json!({ "ok": true, "data": data }).to_string(),
                    )
                    .unwrap();
                    handled += 1;
                } else {
                    thread::sleep(std::time::Duration::from_millis(10));
                }
            }
        });

        let mut req = ue4ss_request("*", 19541);
        req.declarative_payload_json = Some(
            serde_json::json!({
                "schema_version": "1.0.0",
                "engine_version": "*",
                "auto_discover": true,
                "discovery_dir": dir_str,
                "features": [
                    {
                        "kind": "toggle_uproperty",
                        "feature_id": "god-mode",
                        "object_class": "BP_PL_Player_C",
                        "property_name": "bInvincible",
                        "enabled_value": true,
                        "disabled_value": false,
                        "persist": true
                    },
                    {
                        "kind": "set_uproperty_scalar",
                        "feature_id": "move-speed",
                        "object_class": "BP_PL_Player_C",
                        "property_name": "CharacterMovement.MaxWalkSpeed",
                        "multiply_original": true,
                        "persist": true
                    },
                    {
                        "kind": "add_uproperty",
                        "feature_id": "add-money",
                        "object_class": "MyEMSPersistentSaveGame_C",
                        "property_name": "Money",
                        "delta": 100000.0,
                        "as_int": true
                    },
                    {
                        "kind": "target_lock",
                        "feature_id": "auto-lock-enemy",
                        "enemy_class": "BP_NewEnemy_Base_C",
                        "player_class": "BP_PL_Player_C",
                        "dead_bool_property": "IsDead",
                        "health_property": "Health"
                    },
                    {
                        "kind": "damage_aura",
                        "feature_id": "kill-aura",
                        "enemy_class": "BP_NewEnemy_Base_C",
                        "player_class": "BP_PL_Player_C",
                        "dead_bool_property": "IsDead",
                        "health_property": "Health",
                        "radius": 3500.0,
                        "damage": 99999.0,
                        "interval_ms": 250
                    }
                ]
            })
            .to_string(),
        );

        let sid = rt.start_session(req).unwrap();
        rt.apply_feature(
            &sid,
            FeatureCommand {
                feature_id: "god-mode".to_owned(),
                value: FeatureValue::Toggle(true),
            },
        )
        .unwrap();
        rt.apply_feature(
            &sid,
            FeatureCommand {
                feature_id: "move-speed".to_owned(),
                value: FeatureValue::Float(2.0),
            },
        )
        .unwrap();
        rt.apply_feature(
            &sid,
            FeatureCommand {
                feature_id: "add-money".to_owned(),
                value: FeatureValue::Action,
            },
        )
        .unwrap();
        rt.apply_feature(
            &sid,
            FeatureCommand {
                feature_id: "auto-lock-enemy".to_owned(),
                value: FeatureValue::Toggle(true),
            },
        )
        .unwrap();
        rt.apply_feature(
            &sid,
            FeatureCommand {
                feature_id: "kill-aura".to_owned(),
                value: FeatureValue::Toggle(true),
            },
        )
        .unwrap();

        let diag = rt.stop_session(&sid).unwrap();
        assert_eq!(diag.rollback_failures, 0);

        worker.join().unwrap();
        assert_eq!(
            *observed_ops.lock().unwrap(),
            vec![
                "ping",
                "persist_uproperty",
                "persist_uproperty",
                "add_uproperty",
                "set_target_lock",
                "set_damage_aura",
                "clear_damage_aura",
                "clear_target_lock",
                "clear_persist_uproperty",
                "clear_persist_uproperty",
            ]
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn ue4ss_combat_spinbot_and_persist_payload_validates() {
        let raw = r#"{
            "schema_version": "1.0.0",
            "engine_version": "4.27",
            "auto_discover": true,
            "features": [
                {
                    "kind": "toggle_uproperty",
                    "feature_id": "no-move-slowdown",
                    "object_class": "CharacterMovementComponent",
                    "property_name": "MaxWalkSpeed",
                    "enabled_value": 650.0,
                    "disabled_value": 400.0,
                    "persist": true
                },
                {
                    "kind": "target_lock",
                    "feature_id": "auto-lock-enemy",
                    "enemy_class": "BP_EnemyBase_C",
                    "max_distance": 5000.0,
                    "max_fov_deg": 90.0,
                    "aim_bone_names": ["neck_01", "head"]
                },
                {
                    "kind": "auto_fire",
                    "feature_id": "auto-fire",
                    "enemy_class": "BP_EnemyBase_C",
                    "fire_function": "FireShot",
                    "interval_ms": 60
                },
                {
                    "kind": "spinbot",
                    "feature_id": "spinbot-third-person",
                    "spin_speed_deg_per_sec": 720.0,
                    "spring_arm_component": "SpringArm",
                    "tps_camera_component": "TPSCamera"
                },
                {
                    "kind": "damage_aura",
                    "feature_id": "kill-aura",
                    "enemy_class": "BP_EnemyBase_C",
                    "interval_ms": 200,
                    "radius": 2500.0,
                    "damage": 9999.0
                },
                {
                    "kind": "fill_items",
                    "feature_id": "fill-all-items",
                    "helper_class": "BP_InventoryHelper_C",
                    "add_function": "AddItem"
                },
                {
                    "kind": "trigger_goal",
                    "feature_id": "instant-win",
                    "goal_class": "BP_Goal_C",
                    "goal_function": "ClearStage"
                }
            ]
        }"#;
        let payload: Ue4ssBridgePayload =
            serde_json::from_str(raw).expect("combat spinbot payload parses");
        Ue4ssBridgeRuntime::validate_payload(&payload).expect("combat spinbot payload validates");
        assert!(payload.auto_discover);
        assert_eq!(payload.features.len(), 7);
    }

    #[test]
    fn ue4ss_legacy_bridge_unsupported_op_reports_upgrade_hint() {
        let dir = std::env::temp_dir().join(format!(
            "nekod_ue4ss_legacy_hint_test_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        let dir_str = dir.to_string_lossy().into_owned();
        let token = "ue4ss_legacy_hint_token_123456";

        fs::write(
            dir.join("ue4ss-active.json"),
            serde_json::json!({
                "bridge_version": "0.3.0",
                "transport": "file_ipc",
                "ipc_dir": dir_str,
                "auth_token": token,
                "engine_version": "4.27"
            })
            .to_string(),
        )
        .unwrap();

        let dir_clone = dir.clone();
        let worker = thread::spawn(move || {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            let mut handled = 0usize;
            while handled < 2 && std::time::Instant::now() < deadline {
                let req_path = dir_clone.join("ue4ss-request.json");
                if req_path.is_file() {
                    let raw = match fs::read_to_string(&req_path) {
                        Ok(s) => s,
                        Err(_) => {
                            thread::sleep(std::time::Duration::from_millis(10));
                            continue;
                        }
                    };
                    let _ = fs::remove_file(&req_path);
                    let req: serde_json::Value = serde_json::from_str(&raw).unwrap();
                    let id = req["id"].as_str().unwrap();
                    let op = req["op"].as_str().unwrap();
                    let resp = match op {
                        "ping" => serde_json::json!({
                            "ok": true,
                            "data": {
                                "bridge_version": "0.3.0",
                                "engine_version": "4.27",
                                "transport": "file_ipc"
                            }
                        }),
                        "add_uproperty" => serde_json::json!({
                            "ok": false,
                            "error": "unsupported ue4ss bridge op: add_uproperty"
                        }),
                        other => panic!("unexpected op: {other}"),
                    };
                    fs::write(
                        dir_clone.join(format!("ue4ss-response-{id}.json")),
                        resp.to_string(),
                    )
                    .unwrap();
                    handled += 1;
                } else {
                    thread::sleep(std::time::Duration::from_millis(10));
                }
            }
        });

        let rt = Ue4ssBridgeRuntime::new();
        let mut req = ue4ss_request("*", 19541);
        req.declarative_payload_json = Some(
            serde_json::json!({
                "schema_version": "1.0.0",
                "engine_version": "*",
                "auto_discover": true,
                "discovery_dir": dir_str,
                "features": [
                    {
                        "kind": "add_uproperty",
                        "feature_id": "add-money",
                        "object_class": "MyEMSPersistentSaveGame_C",
                        "property_name": "Money",
                        "delta": 100000.0,
                        "as_int": true
                    }
                ]
            })
            .to_string(),
        );

        let sid = rt.start_session(req).unwrap();
        let err = rt
            .apply_feature(
                &sid,
                FeatureCommand {
                    feature_id: "add-money".to_owned(),
                    value: FeatureValue::Action,
                },
            )
            .expect_err("legacy v0.3.0 bridge must return error for add_uproperty");

        match err {
            RuntimeError::ExecutionFailed(msg) => {
                assert!(
                    msg.contains("unsupported ue4ss bridge op: add_uproperty"),
                    "unexpected msg: {msg}"
                );
                assert!(msg.contains("v0.7.3"), "missing upgrade hint in msg: {msg}");
            }
            other => panic!("unexpected error variant: {other:?}"),
        }

        let _ = rt.stop_session(&sid);
        worker.join().unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn ue4ss_fill_items_and_trigger_goal_validate_refuse_readonly_and_roundtrip() {
        let dir = std::env::temp_dir().join(format!(
            "nekod_ue4ss_v05_test_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        let dir_str = dir.to_string_lossy().into_owned();
        let token = "ue4ss_v05_test_token_abcdef123456";

        // 1. Validation rejections.
        let rt = Ue4ssBridgeRuntime::new();
        for bad_feature in [
            serde_json::json!({
                "kind": "fill_items",
                "feature_id": "bad-helper",
                "helper_class": "",
                "add_function": "AddItemToPlayer"
            }),
            serde_json::json!({
                "kind": "fill_items",
                "feature_id": "bad-amount",
                "helper_class": "BF_ItemUtils_C",
                "add_function": "AddItemToPlayer",
                "default_amount": 0
            }),
            serde_json::json!({
                "kind": "fill_items",
                "feature_id": "bad-fallback",
                "helper_class": "BF_ItemUtils_C",
                "add_function": "AddItemToPlayer",
                "fallback_items": [{ "key": "", "amount": 5 }]
            }),
            serde_json::json!({
                "kind": "trigger_goal",
                "feature_id": "bad-goal",
                "goal_class": "BP_Goal_C",
                "goal_function": ""
            }),
            serde_json::json!({
                "kind": "trigger_goal",
                "feature_id": "bad-pre-clear",
                "goal_class": "BP_Goal_C",
                "goal_function": "E_Goal",
                "pre_clear_functions": [""]
            }),
        ] {
            let mut bad_req = ue4ss_request("*", 19541);
            bad_req.declarative_payload_json = Some(
                serde_json::json!({
                    "schema_version": "1.0.0",
                    "engine_version": "*",
                    "auto_discover": true,
                    "discovery_dir": dir_str,
                    "features": [bad_feature]
                })
                .to_string(),
            );
            assert!(matches!(
                rt.start_session(bad_req),
                Err(RuntimeError::InvalidPayload(_))
            ));
        }

        // 2. Readonly probe must refuse the new mutating ops.
        for forbidden in ["fill_items", "trigger_goal"] {
            let err = probe_nekod_ue4ss_bridge_readonly(
                None,
                Some(&dir_str),
                forbidden,
                serde_json::json!({}),
            )
            .expect_err("readonly probe must refuse mutating op");
            assert!(
                matches!(err, RuntimeError::ExecutionFailed(ref m) if m.contains(forbidden)),
                "unexpected error for {forbidden}: {err:?}"
            );
        }

        // 3. File-IPC round trip.
        fs::write(
            dir.join("ue4ss-active.json"),
            serde_json::json!({
                "bridge_version": "0.5.0",
                "transport": "file_ipc",
                "ipc_dir": dir_str,
                "auth_token": token,
                "engine_version": "4.27"
            })
            .to_string(),
        )
        .unwrap();

        let dir_clone = dir.clone();
        let observed_ops = Arc::new(Mutex::new(Vec::<String>::new()));
        let observed_clone = Arc::clone(&observed_ops);
        let worker = thread::spawn(move || {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(6);
            let mut handled = 0usize;
            while handled < 3 && std::time::Instant::now() < deadline {
                let req_path = dir_clone.join("ue4ss-request.json");
                if req_path.is_file() {
                    let raw = match fs::read_to_string(&req_path) {
                        Ok(s) => s,
                        Err(_) => {
                            thread::sleep(std::time::Duration::from_millis(10));
                            continue;
                        }
                    };
                    let _ = fs::remove_file(&req_path);
                    let req: serde_json::Value = serde_json::from_str(&raw).unwrap();
                    let id = req["id"].as_str().unwrap();
                    let op = req["op"].as_str().unwrap().to_owned();
                    observed_clone.lock().unwrap().push(op.clone());
                    let data = match op.as_str() {
                        "ping" => serde_json::json!({
                            "bridge_version": "0.5.0",
                            "engine_version": "4.27",
                            "transport": "file_ipc"
                        }),
                        "fill_items" => {
                            assert_eq!(req["params"]["helper_class"], "BF_ItemUtils_C");
                            assert_eq!(req["params"]["add_function"], "AddItemToPlayer");
                            assert_eq!(req["params"]["fallback_items"][0]["key"], "IT_MT_C01");
                            serde_json::json!({ "total": 1, "filled": 1, "already_max": 0, "failed": 0 })
                        }
                        "trigger_goal" => {
                            assert_eq!(req["params"]["goal_class"], "BP_Goal_C");
                            assert_eq!(req["params"]["goal_function"], "E_Goal");
                            serde_json::json!({ "mode": "existing_goal" })
                        }
                        other => panic!("unexpected op: {other}"),
                    };
                    fs::write(
                        dir_clone.join(format!("ue4ss-response-{id}.json")),
                        serde_json::json!({ "ok": true, "data": data }).to_string(),
                    )
                    .unwrap();
                    handled += 1;
                } else {
                    thread::sleep(std::time::Duration::from_millis(10));
                }
            }
        });

        let mut req = ue4ss_request("*", 19541);
        req.declarative_payload_json = Some(
            serde_json::json!({
                "schema_version": "1.0.0",
                "engine_version": "*",
                "auto_discover": true,
                "discovery_dir": dir_str,
                "features": [
                    {
                        "kind": "fill_items",
                        "feature_id": "fill-storage-items",
                        "helper_class": "BF_ItemUtils_C",
                        "add_function": "AddItemToPlayer",
                        "fallback_items": [{ "key": "IT_MT_C01", "amount": 999 }]
                    },
                    {
                        "kind": "trigger_goal",
                        "feature_id": "instant-stage-clear",
                        "goal_class": "BP_Goal_C",
                        "goal_function": "E_Goal"
                    }
                ]
            })
            .to_string(),
        );

        let sid = rt.start_session(req).unwrap();
        for fid in ["fill-storage-items", "instant-stage-clear"] {
            rt.apply_feature(
                &sid,
                FeatureCommand {
                    feature_id: fid.to_owned(),
                    value: FeatureValue::Action,
                },
            )
            .unwrap();
        }
        let diag = rt.stop_session(&sid).unwrap();
        assert_eq!(diag.rollback_failures, 0);

        worker.join().unwrap();
        assert_eq!(
            *observed_ops.lock().unwrap(),
            vec!["ping", "fill_items", "trigger_goal"]
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn ue4ss_auto_fire_validates_refuses_readonly_and_roundtrips() {
        let dir = std::env::temp_dir().join(format!(
            "nekod_ue4ss_v06_autofire_test_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        let dir_str = dir.to_string_lossy().into_owned();
        let token = "ue4ss_v06_autofire_token_123456";

        // 1. Validation rejections for AutoFire.
        let rt = Ue4ssBridgeRuntime::new();
        for bad_feature in [
            serde_json::json!({
                "kind": "auto_fire",
                "feature_id": "bad-fire-fn",
                "enemy_class": "BP_NewEnemy_Base_C",
                "fire_function": ""
            }),
            serde_json::json!({
                "kind": "auto_fire",
                "feature_id": "bad-fov",
                "enemy_class": "BP_NewEnemy_Base_C",
                "fire_function": "PressLeftButton",
                "max_fov_deg": 0.2
            }),
            serde_json::json!({
                "kind": "auto_fire",
                "feature_id": "bad-interval",
                "enemy_class": "BP_NewEnemy_Base_C",
                "fire_function": "PressLeftButton",
                "interval_ms": 5
            }),
            serde_json::json!({
                "kind": "auto_fire",
                "feature_id": "bad-bone",
                "enemy_class": "BP_NewEnemy_Base_C",
                "fire_function": "PressLeftButton",
                "aim_bone_names": [""]
            }),
            serde_json::json!({
                "kind": "auto_fire",
                "feature_id": "bad-z-offset",
                "enemy_class": "BP_NewEnemy_Base_C",
                "fire_function": "PressLeftButton",
                "aim_z_offset": 200.0
            }),
            serde_json::json!({
                "kind": "auto_fire",
                "feature_id": "bad-priority",
                "enemy_class": "BP_NewEnemy_Base_C",
                "fire_function": "PressLeftButton",
                "target_priority": "random"
            }),
        ] {
            let mut bad_req = ue4ss_request("*", 19541);
            bad_req.declarative_payload_json = Some(
                serde_json::json!({
                    "schema_version": "1.0.0",
                    "engine_version": "*",
                    "auto_discover": true,
                    "discovery_dir": dir_str,
                    "features": [bad_feature]
                })
                .to_string(),
            );
            assert!(matches!(
                rt.start_session(bad_req),
                Err(RuntimeError::InvalidPayload(_))
            ));
        }

        // 2. Readonly probe must refuse mutating auto_fire ops.
        for forbidden in ["set_auto_fire", "clear_auto_fire"] {
            let err = probe_nekod_ue4ss_bridge_readonly(
                None,
                Some(&dir_str),
                forbidden,
                serde_json::json!({}),
            )
            .expect_err("readonly probe must refuse mutating auto_fire op");
            assert!(
                matches!(err, RuntimeError::ExecutionFailed(ref m) if m.contains(forbidden)),
                "unexpected error for {forbidden}: {err:?}"
            );
        }

        // 3. File-IPC round trip: enable -> set_auto_fire, stop_session rollback -> clear_auto_fire.
        fs::write(
            dir.join("ue4ss-active.json"),
            serde_json::json!({
                "bridge_version": "0.7.3",
                "transport": "file_ipc",
                "ipc_dir": dir_str,
                "auth_token": token,
                "engine_version": "4.27"
            })
            .to_string(),
        )
        .unwrap();

        let dir_clone = dir.clone();
        let observed_ops = Arc::new(Mutex::new(Vec::<String>::new()));
        let observed_clone = Arc::clone(&observed_ops);
        let worker = thread::spawn(move || {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(6);
            let mut handled = 0usize;
            while handled < 3 && std::time::Instant::now() < deadline {
                let req_path = dir_clone.join("ue4ss-request.json");
                if req_path.is_file() {
                    let raw = match fs::read_to_string(&req_path) {
                        Ok(s) => s,
                        Err(_) => {
                            thread::sleep(std::time::Duration::from_millis(10));
                            continue;
                        }
                    };
                    let _ = fs::remove_file(&req_path);
                    let req: serde_json::Value = serde_json::from_str(&raw).unwrap();
                    let id = req["id"].as_str().unwrap();
                    let op = req["op"].as_str().unwrap().to_owned();
                    observed_clone.lock().unwrap().push(op.clone());
                    let data = match op.as_str() {
                        "ping" => serde_json::json!({
                            "bridge_version": "0.7.3",
                            "engine_version": "4.27",
                            "transport": "file_ipc"
                        }),
                        "set_auto_fire" => {
                            assert_eq!(req["params"]["feature_id"], "auto-fire");
                            assert_eq!(req["params"]["enemy_class"], "BP_NewEnemy_Base_C");
                            assert_eq!(req["params"]["fire_function"], "PressLeftButton");
                            assert_eq!(req["params"]["direct_fire_function"], "WeaponFire");
                            assert_eq!(req["params"]["cancel_function"], "OnFireCancel");
                            assert_eq!(req["params"]["end_fire_function"], "ContinutieFireEnd");
                            assert_eq!(req["params"]["aim_z_offset"], -6.0);
                            assert_eq!(req["params"]["target_priority"], "nearest");
                            assert_eq!(req["params"]["require_line_of_sight"], true);
                            serde_json::json!({ "feature_id": "auto-fire", "active": true })
                        }
                        "clear_auto_fire" => {
                            assert_eq!(req["params"]["feature_id"], "auto-fire");
                            serde_json::json!({ "feature_id": "auto-fire", "active": false })
                        }
                        other => panic!("unexpected op: {other}"),
                    };
                    fs::write(
                        dir_clone.join(format!("ue4ss-response-{id}.json")),
                        serde_json::json!({ "ok": true, "data": data }).to_string(),
                    )
                    .unwrap();
                    handled += 1;
                } else {
                    thread::sleep(std::time::Duration::from_millis(10));
                }
            }
        });

        let mut req = ue4ss_request("*", 19541);
        req.declarative_payload_json = Some(
            serde_json::json!({
                "schema_version": "1.0.0",
                "engine_version": "*",
                "auto_discover": true,
                "discovery_dir": dir_str,
                "features": [
                    {
                        "kind": "auto_fire",
                        "feature_id": "auto-fire",
                        "enemy_class": "BP_NewEnemy_Base_C",
                        "player_class": "BP_PL_Player_C",
                        "fire_function": "PressLeftButton",
                        "direct_fire_function": "WeaponFire",
                        "cancel_function": "OnFireCancel",
                        "end_fire_function": "ContinutieFireEnd",
                        "dead_bool_property": "IsDead",
                        "health_property": "Health",
                        "aim_bone_names": ["neck_01", "Neck_M", "head", "Head"],
                        "max_distance": 5500.0,
                        "max_fov_deg": 12.0,
                        "interval_ms": 75,
                        "aim_z_offset": -6.0,
                        "target_priority": "nearest",
                        "require_line_of_sight": true
                    }
                ]
            })
            .to_string(),
        );

        let sid = rt.start_session(req).unwrap();
        rt.apply_feature(
            &sid,
            FeatureCommand {
                feature_id: "auto-fire".to_owned(),
                value: FeatureValue::Toggle(true),
            },
        )
        .unwrap();
        let diag = rt.stop_session(&sid).unwrap();
        assert_eq!(diag.rollback_failures, 0);

        worker.join().unwrap();
        assert_eq!(
            *observed_ops.lock().unwrap(),
            vec!["ping", "set_auto_fire", "clear_auto_fire"]
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn ue4ss_spinbot_validates_refuses_readonly_and_roundtrips() {
        let dir = std::env::temp_dir().join(format!(
            "nekod_ue4ss_v07_spinbot_test_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        let dir_str = dir.to_string_lossy().into_owned();
        let token = "ue4ss_v07_spinbot_token_123456";

        // 1. Validation rejections for SpinBot bounds.
        let rt = Ue4ssBridgeRuntime::new();
        for bad_feature in [
            serde_json::json!({
                "kind": "spinbot",
                "feature_id": "bad-speed",
                "spin_speed_deg_per_sec": 10.0
            }),
            serde_json::json!({
                "kind": "spinbot",
                "feature_id": "bad-pitch",
                "spin_pitch_deg": 95.0
            }),
            serde_json::json!({
                "kind": "spinbot",
                "feature_id": "bad-arm",
                "arm_length": 20.0
            }),
            serde_json::json!({
                "kind": "spinbot",
                "feature_id": "bad-fov",
                "tps_fov": 25.0
            }),
        ] {
            let mut bad_req = ue4ss_request("*", 19541);
            bad_req.declarative_payload_json = Some(
                serde_json::json!({
                    "schema_version": "1.0.0",
                    "engine_version": "*",
                    "auto_discover": true,
                    "discovery_dir": dir_str,
                    "features": [bad_feature]
                })
                .to_string(),
            );
            assert!(matches!(
                rt.start_session(bad_req),
                Err(RuntimeError::InvalidPayload(_))
            ));
        }

        // 2. Readonly probe must refuse set_spinbot / clear_spinbot.
        for forbidden in ["set_spinbot", "clear_spinbot"] {
            let err = probe_nekod_ue4ss_bridge_readonly(
                None,
                Some(&dir_str),
                forbidden,
                serde_json::json!({}),
            )
            .expect_err("readonly probe must refuse mutating spinbot op");
            assert!(
                matches!(err, RuntimeError::ExecutionFailed(ref m) if m.contains(forbidden)),
                "unexpected error for {forbidden}: {err:?}"
            );
        }

        // 3. File-IPC round trip: enable -> set_spinbot, stop_session rollback -> clear_spinbot.
        fs::write(
            dir.join("ue4ss-active.json"),
            serde_json::json!({
                "bridge_version": "0.7.0",
                "transport": "file_ipc",
                "ipc_dir": dir_str,
                "auth_token": token,
                "engine_version": "4.27"
            })
            .to_string(),
        )
        .unwrap();

        let dir_clone = dir.clone();
        let observed_ops = Arc::new(Mutex::new(Vec::<String>::new()));
        let observed_clone = Arc::clone(&observed_ops);
        let worker = thread::spawn(move || {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(6);
            let mut handled = 0usize;
            while handled < 3 && std::time::Instant::now() < deadline {
                let req_path = dir_clone.join("ue4ss-request.json");
                if req_path.is_file() {
                    let raw = match fs::read_to_string(&req_path) {
                        Ok(s) => s,
                        Err(_) => {
                            thread::sleep(std::time::Duration::from_millis(10));
                            continue;
                        }
                    };
                    let _ = fs::remove_file(&req_path);
                    let req: serde_json::Value = serde_json::from_str(&raw).unwrap();
                    let id = req["id"].as_str().unwrap();
                    let op = req["op"].as_str().unwrap().to_owned();
                    observed_clone.lock().unwrap().push(op.clone());
                    let data = match op.as_str() {
                        "ping" => serde_json::json!({
                            "bridge_version": "0.7.0",
                            "engine_version": "4.27",
                            "transport": "file_ipc"
                        }),
                        "set_spinbot" => {
                            assert_eq!(req["params"]["feature_id"], "spinbot-third-person");
                            assert_eq!(req["params"]["tps_mesh_index"], 1);
                            assert_eq!(req["params"]["spin_speed_deg_per_sec"], 1440.0);
                            assert_eq!(req["params"]["arm_length"], 280.0);
                            serde_json::json!({ "feature_id": "spinbot-third-person", "active": true })
                        }
                        "clear_spinbot" => {
                            assert_eq!(req["params"]["feature_id"], "spinbot-third-person");
                            serde_json::json!({ "feature_id": "spinbot-third-person", "active": false })
                        }
                        other => panic!("unexpected op: {other}"),
                    };
                    fs::write(
                        dir_clone.join(format!("ue4ss-response-{id}.json")),
                        serde_json::json!({ "ok": true, "data": data }).to_string(),
                    )
                    .unwrap();
                    handled += 1;
                } else {
                    thread::sleep(std::time::Duration::from_millis(10));
                }
            }
        });

        let mut req = ue4ss_request("*", 19541);
        req.declarative_payload_json = Some(
            serde_json::json!({
                "schema_version": "1.0.0",
                "engine_version": "*",
                "auto_discover": true,
                "discovery_dir": dir_str,
                "features": [
                    {
                        "kind": "spinbot",
                        "feature_id": "spinbot-third-person",
                        "player_class": "BP_PL_Player_C",
                        "mesh_component": "FPS_Mesh",
                        "fps_camera_component": "FPS_Camera",
                        "tps_camera_component": "TPSCamera",
                        "spring_arm_component": "SpringArm",
                        "tps_base_component": "TPS_Base",
                        "switch_mesh_function": "SwitchSkeletalMesh",
                        "tps_mesh_index": 1,
                        "fps_mesh_index": 0,
                        "spin_speed_deg_per_sec": 1440.0,
                        "spin_pitch_deg": 0.0,
                        "arm_length": 280.0,
                        "tps_fov": 85.0
                    }
                ]
            })
            .to_string(),
        );

        let sid = rt.start_session(req).unwrap();
        rt.apply_feature(
            &sid,
            FeatureCommand {
                feature_id: "spinbot-third-person".to_owned(),
                value: FeatureValue::Toggle(true),
            },
        )
        .unwrap();
        let diag = rt.stop_session(&sid).unwrap();
        assert_eq!(diag.rollback_failures, 0);

        worker.join().unwrap();
        assert_eq!(
            *observed_ops.lock().unwrap(),
            vec!["ping", "set_spinbot", "clear_spinbot"]
        );

        let _ = fs::remove_dir_all(&dir);
    }
}
