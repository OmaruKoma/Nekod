//! Unity Reflection Bridge runtime adapter (`unity_bridge_v1`).
//!
//! Communicates with an authenticated managed loader bridge (BepInEx / MelonLoader)
//! over loopback JSON-line IPC, strictly separating `Mono` vs. `IL2CPP` runtime flavors
//! and restoring modified class fields in LIFO order on session teardown.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use nekod_manifest::{PermissionScope, RuntimeDependencySpec, RuntimeKind, validate_relative_path};
use nekod_runtime_api::{
    DependencyCheckResult, DependencyStatus, FeatureCommand, FeatureValue, ProcessIdentity,
    RuntimeAdapterClass, RuntimeAdapterMetadata, RuntimeDiagnostics, RuntimeError,
    RuntimeLifecycleMode, RuntimeProvider, RuntimeSessionId, SessionLifecycleState,
    StartSessionRequest,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::bridge_ipc::{
    BridgeRpcRequest, BridgeSessionStore, ManagedBridgeSession, send_bridge_rpc,
    send_unity_explorer_http_rpc, validate_loopback_bridge, validate_loopback_host_port,
};
use crate::session::validate_session_preflight_for_kind;

/// Wire protocol spoken between Nekod and an in-game Unity companion plugin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum UnityBridgeProtocol {
    /// Nekod default authenticated newline-delimited JSON RPC.
    #[default]
    NekodJsonLineV1,
    /// Native compatibility with `OmaruKoma/UnityExplorerMCP` (`MCPBridge.dll`) HTTP POST `/`.
    UnityExplorerMcpHttpV1,
}

/// Explicit Unity scripting backend flavor (Mono and IL2CPP are never assumed identical).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnityFlavor {
    Mono,
    Il2cpp,
}

impl UnityFlavor {
    fn as_str(self) -> &'static str {
        match self {
            Self::Mono => "mono",
            Self::Il2cpp => "il2cpp",
        }
    }
}

/// Supported Unity loader family hosting the controlled bridge plugin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnityLoaderKind {
    BepinexMono,
    BepinexIl2cpp,
    Melonloader,
}

/// Entrypoint JSON payload for `unity_bridge_v1`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UnityBridgePayload {
    pub schema_version: String,
    #[serde(default)]
    pub protocol: UnityBridgeProtocol,
    pub flavor: UnityFlavor,
    pub loader: UnityLoaderKind,
    pub host: String,
    #[serde(default)]
    pub port: u16,
    #[serde(default)]
    pub auto_discover_by_pid: bool,
    #[serde(default)]
    pub discovery_dir: Option<String>,
    #[serde(default)]
    pub auth_token: String,
    pub features: Vec<UnityFeatureBinding>,
}

/// Additional Unity field target synchronized alongside a primary `ToggleField` binding.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UnityToggleTarget {
    pub class_name: String,
    pub field_name: String,
    pub enabled_value: Value,
    pub disabled_value: Value,
}

/// Additional Unity field target updated alongside a primary `SetField` binding.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UnitySetTarget {
    pub class_name: String,
    pub field_name: String,
    #[serde(default)]
    pub scale: Option<f64>,
    #[serde(default)]
    pub reciprocal: bool,
}

/// Optional component liveness filter checked when selecting targets for `TargetLock`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UnityAliveCheck {
    pub component_class: String,
    #[serde(default)]
    pub dead_field: Option<String>,
    #[serde(default)]
    pub health_field: Option<String>,
}

fn default_damage_handler_kind() -> String {
    "call".to_owned()
}

/// Native Unity damage method specification executed per enemy hit in `DamageAura`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UnityDamageHandler {
    #[serde(default = "default_damage_handler_kind")]
    pub kind: String,
    pub component_class: String,
    #[serde(default)]
    pub damage_type_class: Option<String>,
    pub method_name: String,
    #[serde(default)]
    pub dead_field: Option<String>,
    #[serde(default)]
    pub health_field: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dead_method: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub zero_health_before_call: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub args: Option<Vec<Value>>,
}

fn default_lock_method() -> String {
    "SetLockTarget".to_owned()
}

fn default_unlock_method() -> String {
    "RemoveLockTarget".to_owned()
}

fn default_lock_range() -> f64 {
    45.0
}

fn default_lock_speed() -> f64 {
    10.0
}

fn default_true() -> bool {
    true
}

fn default_lock_interval_ms() -> u64 {
    80
}

fn default_aura_radius() -> f64 {
    25.0
}

fn default_aura_interval_ms() -> u64 {
    250
}

fn default_spin_roll_speed() -> f64 {
    360.0
}

fn default_spin_interval_ms() -> u64 {
    16
}

/// Single field write executed inside a lifecycle method hook (`HookSetField`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UnityHookFieldWrite {
    pub class_name: String,
    pub field_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub restore_value: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub copy_from_class: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub copy_from_field: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ensure_component: Option<String>,
    #[serde(default)]
    pub init_null_lists: bool,
    #[serde(default)]
    pub only_if_null: bool,
    #[serde(default)]
    pub include_inactive: bool,
    #[serde(default)]
    pub clone_under_owner: bool,
}

/// Ordered setup step executed before invoking an `InvokeMethod` action feature.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum UnityActionStep {
    SetField {
        class_name: String,
        field_name: String,
        value: Value,
    },
    CopyField {
        from_class: String,
        from_field: String,
        to_class: String,
        to_field: String,
    },
    InvokeMethod {
        class_name: String,
        method_name: String,
        #[serde(default)]
        args: Vec<Value>,
    },
}

fn default_hook_timing() -> String {
    "prefix".to_owned()
}

/// Declarative Unity class/field/method binding mapped from a UI `FeatureCommand`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum UnityFeatureBinding {
    ToggleField {
        feature_id: String,
        class_name: String,
        field_name: String,
        enabled_value: Value,
        disabled_value: Value,
        #[serde(default)]
        persist: bool,
        #[serde(default)]
        extra_targets: Vec<UnityToggleTarget>,
    },
    SetField {
        feature_id: String,
        class_name: String,
        field_name: String,
        #[serde(default)]
        persist: bool,
        #[serde(default)]
        scale: Option<f64>,
        #[serde(default)]
        reciprocal: bool,
        #[serde(default)]
        extra_targets: Vec<UnitySetTarget>,
    },
    InvokeMethod {
        feature_id: String,
        class_name: String,
        method_name: String,
        #[serde(default)]
        args: Vec<Value>,
        #[serde(default)]
        include_inactive: bool,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        pre_steps: Vec<UnityActionStep>,
    },
    TargetLock {
        feature_id: String,
        player_class: String,
        camera_class: String,
        target_class: String,
        #[serde(default = "default_lock_method")]
        lock_method: String,
        #[serde(default = "default_unlock_method")]
        unlock_method: String,
        #[serde(default)]
        alive_checks: Vec<UnityAliveCheck>,
        #[serde(default = "default_lock_range")]
        max_range: f64,
        #[serde(default)]
        height_offset: f64,
        #[serde(default = "default_lock_speed")]
        lock_speed: f64,
        #[serde(default = "default_true")]
        require_collider: bool,
        #[serde(default = "default_lock_interval_ms")]
        interval_ms: u64,
        #[serde(default)]
        player_tags: Vec<String>,
    },
    DamageAura {
        feature_id: String,
        player_class: String,
        target_class: String,
        #[serde(default)]
        require_active_class: Option<String>,
        #[serde(default)]
        exclude_bool_field: Option<String>,
        damage_amount: i64,
        #[serde(default = "default_aura_radius")]
        radius: f64,
        #[serde(default = "default_aura_interval_ms")]
        interval_ms: u64,
        handlers: Vec<UnityDamageHandler>,
        #[serde(default)]
        player_tags: Vec<String>,
    },
    SpinBot {
        feature_id: String,
        player_class: String,
        #[serde(default)]
        target_class: Option<String>,
        #[serde(default)]
        require_active_class: Option<String>,
        #[serde(default)]
        spin_node_path: Option<String>,
        #[serde(default = "default_true")]
        spin_local: bool,
        #[serde(default = "default_true")]
        late_update: bool,
        #[serde(default)]
        roll_field: Option<String>,
        #[serde(default = "default_spin_roll_speed")]
        roll_speed_deg: f64,
        #[serde(default)]
        orbit_radius: f64,
        #[serde(default)]
        orbit_speed_deg: f64,
        #[serde(default)]
        height_offset: f64,
        #[serde(default)]
        aim_at_target: bool,
        #[serde(default = "default_lock_range")]
        max_range: f64,
        #[serde(default = "default_true")]
        require_alive: bool,
        #[serde(default)]
        alive_checks: Vec<UnityAliveCheck>,
        #[serde(default = "default_spin_interval_ms")]
        interval_ms: u64,
        #[serde(default)]
        player_tags: Vec<String>,
    },
    SetGameObjectActive {
        feature_id: String,
        #[serde(default)]
        game_object_name: Option<String>,
        #[serde(default)]
        game_object_prefix: Option<String>,
        #[serde(default)]
        game_object_names: Vec<String>,
        #[serde(default)]
        game_object_prefixes: Vec<String>,
        enabled_active: bool,
        #[serde(default)]
        disabled_active: Option<bool>,
        #[serde(default = "default_true")]
        include_inactive: bool,
    },
    CopyField {
        feature_id: String,
        from_class: String,
        from_field: String,
        to_class: String,
        to_field: String,
    },
    ReplaceTextureMaterial {
        feature_id: String,
        #[serde(default)]
        renderer_name_prefix: Option<String>,
        #[serde(default)]
        renderer_name_prefixes: Vec<String>,
        #[serde(default)]
        material_name_prefix: Option<String>,
        #[serde(default)]
        material_name_prefixes: Vec<String>,
        asset_path: String,
        #[serde(default)]
        shader_name: Option<String>,
        #[serde(default)]
        color: Option<[f64; 4]>,
    },
    HookSetField {
        feature_id: String,
        hook_class: String,
        hook_method: String,
        #[serde(default = "default_hook_timing")]
        timing: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        param_count: Option<u8>,
        #[serde(default)]
        apply_immediately: bool,
        writes: Vec<UnityHookFieldWrite>,
    },
}

fn build_invoke_method_call(
    class_name: &str,
    method_name: &str,
    args: &[Value],
) -> (&'static str, Value) {
    if args.is_empty() {
        (
            "invoke_method",
            json!({ "class_name": class_name, "method_name": method_name }),
        )
    } else {
        (
            "invoke_method_args",
            json!({ "class_name": class_name, "method_name": method_name, "args": args }),
        )
    }
}

impl UnityFeatureBinding {
    fn feature_id(&self) -> &str {
        match self {
            Self::ToggleField { feature_id, .. }
            | Self::SetField { feature_id, .. }
            | Self::InvokeMethod { feature_id, .. }
            | Self::TargetLock { feature_id, .. }
            | Self::DamageAura { feature_id, .. }
            | Self::SpinBot { feature_id, .. }
            | Self::SetGameObjectActive { feature_id, .. }
            | Self::CopyField { feature_id, .. }
            | Self::ReplaceTextureMaterial { feature_id, .. }
            | Self::HookSetField { feature_id, .. } => feature_id,
        }
    }
}

struct ActiveUnitySession {
    payload: UnityBridgePayload,
    verified_assets: BTreeMap<String, String>,
}

/// Controlled Unity Reflection Bridge adapter (`unity_bridge_v1`).
pub struct UnityBridgeRuntime {
    store: BridgeSessionStore,
    bindings: std::sync::Mutex<BTreeMap<String, ActiveUnitySession>>,
}

impl Default for UnityBridgeRuntime {
    fn default() -> Self {
        Self::new()
    }
}

impl UnityBridgeRuntime {
    pub fn new() -> Self {
        Self {
            store: BridgeSessionStore::new(),
            bindings: std::sync::Mutex::new(std::collections::BTreeMap::new()),
        }
    }

    fn validate_payload(payload: &UnityBridgePayload) -> Result<(), RuntimeError> {
        if !payload.schema_version.starts_with("1.") {
            return Err(RuntimeError::InvalidPayload(format!(
                "unsupported unity_bridge_v1 schema_version '{}'",
                payload.schema_version
            )));
        }
        // Enforce strict pairing between Unity runtime flavor and BepInEx loader target.
        if payload.flavor == UnityFlavor::Mono && payload.loader == UnityLoaderKind::BepinexIl2cpp {
            return Err(RuntimeError::InvalidPayload(
                "Unity Mono flavor cannot use bepinex_il2cpp loader".to_owned(),
            ));
        }
        if payload.flavor == UnityFlavor::Il2cpp && payload.loader == UnityLoaderKind::BepinexMono {
            return Err(RuntimeError::InvalidPayload(
                "Unity IL2CPP flavor cannot use bepinex_mono loader".to_owned(),
            ));
        }
        if payload.features.is_empty() {
            return Err(RuntimeError::InvalidPayload(
                "unity_bridge_v1 payload must declare at least one feature".to_owned(),
            ));
        }

        for feature in &payload.features {
            match feature {
                UnityFeatureBinding::InvokeMethod {
                    feature_id,
                    class_name,
                    method_name,
                    pre_steps,
                    ..
                } => {
                    if feature_id.trim().is_empty()
                        || class_name.trim().is_empty()
                        || method_name.trim().is_empty()
                    {
                        return Err(RuntimeError::InvalidPayload(format!(
                            "invoke_method feature '{feature_id}' requires non-empty class_name and method_name"
                        )));
                    }
                    for (idx, step) in pre_steps.iter().enumerate() {
                        match step {
                            UnityActionStep::SetField {
                                class_name: step_class,
                                field_name: step_field,
                                ..
                            } => {
                                if step_class.trim().is_empty() || step_field.trim().is_empty() {
                                    return Err(RuntimeError::InvalidPayload(format!(
                                        "invoke_method feature '{feature_id}' pre_steps[{idx}] requires non-empty class_name and field_name"
                                    )));
                                }
                            }
                            UnityActionStep::CopyField {
                                from_class,
                                from_field,
                                to_class,
                                to_field,
                            } => {
                                if from_class.trim().is_empty()
                                    || from_field.trim().is_empty()
                                    || to_class.trim().is_empty()
                                    || to_field.trim().is_empty()
                                {
                                    return Err(RuntimeError::InvalidPayload(format!(
                                        "invoke_method feature '{feature_id}' pre_steps[{idx}] copy_field requires non-empty source and target class/field"
                                    )));
                                }
                            }
                            UnityActionStep::InvokeMethod {
                                class_name: step_class,
                                method_name: step_method,
                                ..
                            } => {
                                if step_class.trim().is_empty() || step_method.trim().is_empty() {
                                    return Err(RuntimeError::InvalidPayload(format!(
                                        "invoke_method feature '{feature_id}' pre_steps[{idx}] invoke_method requires non-empty class_name and method_name"
                                    )));
                                }
                            }
                        }
                    }
                }
                UnityFeatureBinding::TargetLock {
                    feature_id,
                    player_class,
                    camera_class,
                    target_class,
                    lock_method,
                    unlock_method,
                    alive_checks,
                    max_range,
                    height_offset,
                    lock_speed,
                    interval_ms,
                    ..
                } => {
                    if player_class.trim().is_empty()
                        || camera_class.trim().is_empty()
                        || target_class.trim().is_empty()
                        || lock_method.trim().is_empty()
                        || unlock_method.trim().is_empty()
                    {
                        return Err(RuntimeError::InvalidPayload(format!(
                            "target_lock feature '{feature_id}' requires non-empty class and method names"
                        )));
                    }
                    if !max_range.is_finite() || *max_range <= 0.0 {
                        return Err(RuntimeError::InvalidPayload(format!(
                            "target_lock feature '{feature_id}' requires positive finite max_range"
                        )));
                    }
                    if !height_offset.is_finite() || !lock_speed.is_finite() || *lock_speed <= 0.0 {
                        return Err(RuntimeError::InvalidPayload(format!(
                            "target_lock feature '{feature_id}' requires finite height_offset and positive lock_speed"
                        )));
                    }
                    if !(30..=2000).contains(interval_ms) {
                        return Err(RuntimeError::InvalidPayload(format!(
                            "target_lock feature '{feature_id}' interval_ms must be 30..=2000"
                        )));
                    }
                    if alive_checks
                        .iter()
                        .any(|c| c.component_class.trim().is_empty())
                    {
                        return Err(RuntimeError::InvalidPayload(format!(
                            "target_lock feature '{feature_id}' alive_checks entry requires component_class"
                        )));
                    }
                }
                UnityFeatureBinding::DamageAura {
                    feature_id,
                    player_class,
                    target_class,
                    damage_amount,
                    radius,
                    interval_ms,
                    handlers,
                    ..
                } => {
                    if player_class.trim().is_empty() || target_class.trim().is_empty() {
                        return Err(RuntimeError::InvalidPayload(format!(
                            "damage_aura feature '{feature_id}' requires player_class and target_class"
                        )));
                    }
                    if *damage_amount <= 0 {
                        return Err(RuntimeError::InvalidPayload(format!(
                            "damage_aura feature '{feature_id}' requires damage_amount > 0"
                        )));
                    }
                    if !radius.is_finite() || *radius <= 0.0 {
                        return Err(RuntimeError::InvalidPayload(format!(
                            "damage_aura feature '{feature_id}' requires positive finite radius"
                        )));
                    }
                    if !(100..=5000).contains(interval_ms) {
                        return Err(RuntimeError::InvalidPayload(format!(
                            "damage_aura feature '{feature_id}' interval_ms must be 100..=5000"
                        )));
                    }
                    if handlers.is_empty() {
                        return Err(RuntimeError::InvalidPayload(format!(
                            "damage_aura feature '{feature_id}' requires at least one handler"
                        )));
                    }
                    for h in handlers {
                        if h.component_class.trim().is_empty() || h.method_name.trim().is_empty() {
                            return Err(RuntimeError::InvalidPayload(format!(
                                "damage_aura feature '{feature_id}' handler requires component_class and method_name"
                            )));
                        }
                        match h.kind.as_str() {
                            "invector_health" => {
                                if h.damage_type_class
                                    .as_deref()
                                    .map(str::trim)
                                    .unwrap_or("")
                                    .is_empty()
                                {
                                    return Err(RuntimeError::InvalidPayload(format!(
                                        "damage_aura feature '{feature_id}' invector_health handler requires damage_type_class"
                                    )));
                                }
                            }
                            "emerald_ai" | "invoke_damage_int" => {}
                            "call" => {
                                if h.args.is_none() {
                                    return Err(RuntimeError::InvalidPayload(format!(
                                        "damage_aura feature '{feature_id}' 'call' handler requires explicit 'args' array"
                                    )));
                                }
                            }
                            other => {
                                return Err(RuntimeError::InvalidPayload(format!(
                                    "unsupported damage_aura handler kind '{other}'"
                                )));
                            }
                        }
                    }
                }
                UnityFeatureBinding::SpinBot {
                    feature_id,
                    player_class,
                    target_class,
                    spin_node_path,
                    roll_field,
                    roll_speed_deg,
                    orbit_radius,
                    orbit_speed_deg,
                    height_offset,
                    aim_at_target,
                    max_range,
                    alive_checks,
                    interval_ms,
                    ..
                } => {
                    if feature_id.trim().is_empty() || player_class.trim().is_empty() {
                        return Err(RuntimeError::InvalidPayload(format!(
                            "spin_bot feature '{feature_id}' requires non-empty feature_id and player_class"
                        )));
                    }
                    if !roll_speed_deg.is_finite()
                        || !orbit_radius.is_finite()
                        || !orbit_speed_deg.is_finite()
                        || !height_offset.is_finite()
                        || !max_range.is_finite()
                        || *max_range <= 0.0
                        || *orbit_radius < 0.0
                    {
                        return Err(RuntimeError::InvalidPayload(format!(
                            "spin_bot feature '{feature_id}' contains invalid numeric parameter"
                        )));
                    }
                    if !(8..=1000).contains(interval_ms) {
                        return Err(RuntimeError::InvalidPayload(format!(
                            "spin_bot feature '{feature_id}' interval_ms must be 8..=1000"
                        )));
                    }
                    let needs_target = *orbit_radius > 0.0 || *aim_at_target;
                    if needs_target
                        && target_class
                            .as_deref()
                            .map(str::trim)
                            .unwrap_or("")
                            .is_empty()
                    {
                        return Err(RuntimeError::InvalidPayload(format!(
                            "spin_bot feature '{feature_id}' requires target_class when orbit_radius > 0 or aim_at_target is enabled"
                        )));
                    }
                    let has_spin_channel = spin_node_path
                        .as_deref()
                        .map(str::trim)
                        .is_some_and(|s| !s.is_empty())
                        || roll_field
                            .as_deref()
                            .map(str::trim)
                            .is_some_and(|s| !s.is_empty())
                        || *orbit_radius > 0.0;
                    if !has_spin_channel {
                        return Err(RuntimeError::InvalidPayload(format!(
                            "spin_bot feature '{feature_id}' requires spin_node_path, roll_field, or orbit_radius > 0"
                        )));
                    }
                    if alive_checks
                        .iter()
                        .any(|c| c.component_class.trim().is_empty())
                    {
                        return Err(RuntimeError::InvalidPayload(format!(
                            "spin_bot feature '{feature_id}' alive_checks entry requires component_class"
                        )));
                    }
                }
                UnityFeatureBinding::SetGameObjectActive {
                    feature_id,
                    game_object_name,
                    game_object_prefix,
                    game_object_names,
                    game_object_prefixes,
                    ..
                } => {
                    let has_selector = game_object_name
                        .as_deref()
                        .map(str::trim)
                        .is_some_and(|s| !s.is_empty())
                        || game_object_prefix
                            .as_deref()
                            .map(str::trim)
                            .is_some_and(|s| !s.is_empty())
                        || game_object_names.iter().any(|s| !s.trim().is_empty())
                        || game_object_prefixes.iter().any(|s| !s.trim().is_empty());
                    if !has_selector {
                        return Err(RuntimeError::InvalidPayload(format!(
                            "set_game_object_active feature '{feature_id}' requires at least one GameObject name or prefix"
                        )));
                    }
                }
                UnityFeatureBinding::CopyField {
                    feature_id,
                    from_class,
                    from_field,
                    to_class,
                    to_field,
                } => {
                    if from_class.trim().is_empty()
                        || from_field.trim().is_empty()
                        || to_class.trim().is_empty()
                        || to_field.trim().is_empty()
                    {
                        return Err(RuntimeError::InvalidPayload(format!(
                            "copy_field feature '{feature_id}' requires non-empty from/to class and field names"
                        )));
                    }
                }
                UnityFeatureBinding::ReplaceTextureMaterial {
                    feature_id,
                    material_name_prefix,
                    material_name_prefixes,
                    asset_path,
                    color,
                    ..
                } => {
                    let has_mat_prefix = material_name_prefix
                        .as_deref()
                        .map(str::trim)
                        .is_some_and(|s| !s.is_empty())
                        || material_name_prefixes.iter().any(|s| !s.trim().is_empty());
                    if !has_mat_prefix {
                        return Err(RuntimeError::InvalidPayload(format!(
                            "replace_texture_material feature '{feature_id}' requires material_name_prefix or material_name_prefixes"
                        )));
                    }
                    validate_relative_path(asset_path).map_err(|e| {
                        RuntimeError::InvalidPayload(format!(
                            "replace_texture_material feature '{feature_id}' invalid asset_path '{asset_path}': {e}"
                        ))
                    })?;
                    let ext = Path::new(asset_path)
                        .extension()
                        .and_then(|s| s.to_str())
                        .map(str::to_ascii_lowercase)
                        .unwrap_or_default();
                    if !matches!(ext.as_str(), "png" | "jpg" | "jpeg") {
                        return Err(RuntimeError::InvalidPayload(format!(
                            "replace_texture_material feature '{feature_id}' asset_path '{asset_path}' must be .png, .jpg, or .jpeg"
                        )));
                    }
                    if let Some(rgba) = color
                        && rgba.iter().any(|c| !c.is_finite() || *c < 0.0 || *c > 1.0)
                    {
                        return Err(RuntimeError::InvalidPayload(format!(
                            "replace_texture_material feature '{feature_id}' color components must be 0.0..=1.0"
                        )));
                    }
                }
                UnityFeatureBinding::HookSetField {
                    feature_id,
                    hook_class,
                    hook_method,
                    timing,
                    writes,
                    ..
                } => {
                    if feature_id.trim().is_empty()
                        || hook_class.trim().is_empty()
                        || hook_method.trim().is_empty()
                    {
                        return Err(RuntimeError::InvalidPayload(format!(
                            "hook_set_field feature '{feature_id}' requires non-empty hook_class and hook_method"
                        )));
                    }
                    let normalized_timing = timing.trim().to_ascii_lowercase();
                    if !matches!(normalized_timing.as_str(), "prefix" | "postfix") {
                        return Err(RuntimeError::InvalidPayload(format!(
                            "hook_set_field feature '{feature_id}' timing '{timing}' must be 'prefix' or 'postfix'"
                        )));
                    }
                    if writes.is_empty() {
                        return Err(RuntimeError::InvalidPayload(format!(
                            "hook_set_field feature '{feature_id}' requires at least one entry in writes"
                        )));
                    }
                    for (idx, w) in writes.iter().enumerate() {
                        if w.class_name.trim().is_empty() || w.field_name.trim().is_empty() {
                            return Err(RuntimeError::InvalidPayload(format!(
                                "hook_set_field feature '{feature_id}' writes[{idx}] requires non-empty class_name and field_name"
                            )));
                        }
                        let has_value = w.value.is_some();
                        let has_copy = w.copy_from_class.is_some() || w.copy_from_field.is_some();
                        let has_ensure = w.ensure_component.is_some();
                        let mode_count = usize::from(has_value)
                            + usize::from(has_copy)
                            + usize::from(has_ensure);
                        if mode_count != 1 {
                            return Err(RuntimeError::InvalidPayload(format!(
                                "hook_set_field feature '{feature_id}' writes[{idx}] must set exactly one of value, copy_from_class/copy_from_field, or ensure_component"
                            )));
                        }
                        if has_copy {
                            let src_cls = w.copy_from_class.as_deref().map(str::trim).unwrap_or("");
                            let src_fld = w.copy_from_field.as_deref().map(str::trim).unwrap_or("");
                            if src_cls.is_empty() || src_fld.is_empty() {
                                return Err(RuntimeError::InvalidPayload(format!(
                                    "hook_set_field feature '{feature_id}' writes[{idx}] requires both non-empty copy_from_class and copy_from_field"
                                )));
                            }
                        }
                        if has_ensure
                            && w.ensure_component
                                .as_deref()
                                .map(str::trim)
                                .unwrap_or("")
                                .is_empty()
                        {
                            return Err(RuntimeError::InvalidPayload(format!(
                                "hook_set_field feature '{feature_id}' writes[{idx}] ensure_component must be non-empty"
                            )));
                        }
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }
}

/// Resolves a live `UnityExplorerMCP` loopback port from `<dir>/bridge-<pid>.json` written by
/// `MCPBridge.dll` (`OmaruKoma/UnityExplorerMCP`).
pub fn resolve_unity_explorer_discovery_port(
    target_pid: u32,
    custom_dir: Option<&str>,
) -> Option<u16> {
    let dir: PathBuf = match custom_dir {
        Some(custom) => PathBuf::from(custom),
        None => {
            if let Ok(local_app_data) = std::env::var("LOCALAPPDATA") {
                PathBuf::from(local_app_data)
                    .join("UnityExplorerMCP")
                    .join("bridges")
            } else {
                std::env::temp_dir()
                    .join("UnityExplorerMCP")
                    .join("bridges")
            }
        }
    };
    let file = dir.join(format!("bridge-{target_pid}.json"));
    let raw = fs::read_to_string(file).ok()?;
    let val: Value = serde_json::from_str(&raw).ok()?;
    let port_u64 = val.get("port").or_else(|| val.get("Port"))?.as_u64()?;
    u16::try_from(port_u64).ok().filter(|p| *p != 0)
}

/// Discovery record written by standalone `NekodUnityBridge` (`plugins/unity-nekod-bridge/`)
/// under `%LOCALAPPDATA%\Nekod\bridges\unity-<pid>.json` (zero UnityExplorer dependency).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NekodUnityDiscovery {
    pub port: u16,
    pub auth_token: Option<String>,
    pub flavor: Option<String>,
}

/// Resolves port and per-launch auth token from `<dir>/unity-<pid>.json` written by
/// standalone `NekodUnityBridge.IL2CPP.dll` / `NekodUnityBridge.Mono.dll`.
pub fn resolve_nekod_unity_discovery(
    target_pid: u32,
    custom_dir: Option<&str>,
) -> Option<NekodUnityDiscovery> {
    let dir: PathBuf = match custom_dir {
        Some(custom) => PathBuf::from(custom),
        None => {
            if let Ok(local_app_data) = std::env::var("LOCALAPPDATA") {
                PathBuf::from(local_app_data).join("Nekod").join("bridges")
            } else {
                std::env::temp_dir().join("Nekod").join("bridges")
            }
        }
    };
    let file = dir.join(format!("unity-{target_pid}.json"));
    let raw = fs::read_to_string(file).ok()?;
    let val: Value = serde_json::from_str(&raw).ok()?;
    let port_u64 = val.get("port")?.as_u64()?;
    let port = u16::try_from(port_u64).ok().filter(|p| *p != 0)?;
    let auth_token = val
        .get("auth_token")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let flavor = val.get("flavor").and_then(Value::as_str).map(str::to_owned);
    Some(NekodUnityDiscovery {
        port,
        auth_token,
        flavor,
    })
}

/// Executes a strictly read-only inspection RPC (`ping`, `list_types`, `list_members`,
/// `find_instances`, `dump_hierarchy`, `get_field`) against a live `NekodUnityBridge`
/// loopback instance discovered by target PID.
pub fn probe_nekod_unity_bridge_readonly(
    target_pid: u32,
    discovery_dir: Option<&str>,
    op: &str,
    params: Value,
) -> Result<Value, RuntimeError> {
    if !matches!(
        op,
        "ping" | "list_types" | "list_members" | "find_instances" | "dump_hierarchy" | "get_field"
    ) {
        return Err(RuntimeError::InvalidPayload(format!(
            "readonly bridge probe refuses operation '{op}'; allowed ops: ping, list_types, list_members, find_instances, dump_hierarchy, get_field"
        )));
    }
    let disc = resolve_nekod_unity_discovery(target_pid, discovery_dir).ok_or_else(|| {
        RuntimeError::InvalidPayload(format!(
            "no NekodUnityBridge discovery record found for PID {target_pid} (ensure game is running with NekodUnityBridge deployed)"
        ))
    })?;
    let token = disc.auth_token.as_deref().unwrap_or("");
    let endpoint = validate_loopback_bridge("127.0.0.1", disc.port, token)?;
    send_bridge_rpc(
        endpoint,
        &BridgeRpcRequest {
            token: token.to_owned(),
            op: op.to_owned(),
            params,
        },
    )
    .map(|resp| resp.data)
}

fn find_plugin_dll_matching(plugins_dir: &Path, needle: &str) -> Option<PathBuf> {
    let entries = fs::read_dir(plugins_dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file()
            && let Some(name) = path.file_name().and_then(|n| n.to_str())
        {
            let lower = name.to_ascii_lowercase();
            if lower.contains(needle) && lower.ends_with(".dll") {
                return Some(path);
            }
        } else if path.is_dir()
            && let Some(found) = fs::read_dir(&path).ok().and_then(|sub| {
                sub.flatten().map(|e| e.path()).find(|p| {
                    p.is_file()
                        && p.file_name().and_then(|n| n.to_str()).is_some_and(|n| {
                            let lower = n.to_ascii_lowercase();
                            lower.contains(needle) && lower.ends_with(".dll")
                        })
                })
            })
        {
            return Some(found);
        }
    }
    None
}

fn send_unity_rpc(
    use_http: bool,
    endpoint: std::net::SocketAddr,
    token: &str,
    op: &str,
    params: Value,
) -> Result<Value, RuntimeError> {
    if use_http {
        send_unity_explorer_http_rpc(endpoint, op, &params)
    } else {
        send_bridge_rpc(
            endpoint,
            &BridgeRpcRequest {
                token: token.to_owned(),
                op: op.to_owned(),
                params,
            },
        )
        .map(|resp| resp.data)
    }
}

fn transform_numeric_value(
    value: &FeatureValue,
    scale: Option<f64>,
    reciprocal: bool,
) -> Result<Value, RuntimeError> {
    let numeric = match *value {
        FeatureValue::Integer(v) if scale.is_none() && !reciprocal => return Ok(json!(v)),
        FeatureValue::Float(v) if !v.is_finite() => {
            return Err(RuntimeError::InvalidPayload(
                "numeric feature value must be finite".to_owned(),
            ));
        }
        FeatureValue::Float(v) if scale.is_none() && !reciprocal => return Ok(json!(v)),
        FeatureValue::Integer(v) => v as f64,
        FeatureValue::Float(v) => v,
        _ => {
            return Err(RuntimeError::InvalidPayload(
                "expected integer or float value".to_owned(),
            ));
        }
    };

    if let Some(s) = scale
        && !s.is_finite()
    {
        return Err(RuntimeError::InvalidPayload(
            "unity_bridge_v1 target scale must be finite".to_owned(),
        ));
    }

    let base = if reciprocal {
        if numeric <= 0.0 || !numeric.is_finite() {
            return Err(RuntimeError::InvalidPayload(
                "reciprocal numeric value must be > 0".to_owned(),
            ));
        }
        1.0 / numeric
    } else {
        numeric
    };

    let scaled = base * scale.unwrap_or(1.0);
    if !scaled.is_finite() {
        return Err(RuntimeError::InvalidPayload(
            "transformed numeric value must be finite".to_owned(),
        ));
    }
    Ok(json!(scaled))
}

fn map_bridge_rpc_error(err: RuntimeError) -> RuntimeError {
    if let RuntimeError::ExecutionFailed(msg) = &err {
        if msg.contains("unsupported op: pin_method_hook")
            || msg.contains("unsupported op: unpin_method_hook")
        {
            return RuntimeError::ExecutionFailed(
                "companion NekodUnityBridge is outdated (missing HookSetField / Harmony method hook support); close the game and update NekodUnityBridge to v0.5.5".to_owned(),
            );
        }
        if msg.contains("unsupported op: pin_spinbot")
            || msg.contains("unsupported op: unpin_spinbot")
            || msg.contains("unsupported op: set_gameobject_active")
            || msg.contains("unsupported op: copy_field")
            || msg.contains("unsupported op: invoke_method_args")
            || msg.contains("unsupported op: replace_texture_material")
            || msg.contains("unsupported op: restore_texture_material")
        {
            return RuntimeError::ExecutionFailed(
                "companion NekodUnityBridge is outdated (missing SpinBot / GameObject / asset material support); close the game and update NekodUnityBridge to v0.5.0".to_owned(),
            );
        }
        if msg.contains("unsupported op: pin_target_lock")
            || msg.contains("unsupported op: unpin_target_lock")
            || msg.contains("unsupported op: pin_damage_aura")
            || msg.contains("unsupported op: unpin_damage_aura")
        {
            return RuntimeError::ExecutionFailed(
                "companion NekodUnityBridge is outdated (missing target_lock/damage_aura support); close the game and update NekodUnityBridge to v0.4.0".to_owned(),
            );
        }
        if msg.contains("unsupported op: pin_field") || msg.contains("unsupported op: unpin_field")
        {
            return RuntimeError::ExecutionFailed(
                "companion NekodUnityBridge is outdated (missing pin_field support); close the game and update NekodUnityBridge to v0.3.0".to_owned(),
            );
        }
    }
    err
}

impl RuntimeProvider for UnityBridgeRuntime {
    fn kind(&self) -> RuntimeKind {
        RuntimeKind::UnityBridgeV1
    }

    fn adapter_metadata(&self) -> RuntimeAdapterMetadata {
        RuntimeAdapterMetadata {
            adapter_id: "unity_bridge_v1".to_owned(),
            adapter_class: RuntimeAdapterClass::UnityBridge,
            runtime_kind: RuntimeKind::UnityBridgeV1,
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
                let low = dep.dependency_id.to_ascii_lowercase();
                let outcome = match game_install_dir {
                    Some(dir) if low.contains("nekod-unity-bridge") => {
                        if let Some(dll) = find_plugin_dll_matching(
                            &dir.join("BepInEx").join("plugins"),
                            "nekodunitybridge",
                        ) {
                            DependencyStatus::Satisfied {
                                resolved_location: dll.display().to_string(),
                            }
                        } else {
                            DependencyStatus::Missing {
                                suggested_action: format!(
                                    "Stage standalone '{}' ({}) from Plugin Bucket into BepInEx/plugins",
                                    dep.dependency_id, dep.version_req
                                ),
                            }
                        }
                    }
                    Some(dir)
                        if (low.contains("unity-explorer-mcp") || low.contains("mcpbridge")) =>
                    {
                        if let Some(dll) = find_plugin_dll_matching(
                            &dir.join("BepInEx").join("plugins"),
                            "mcpbridge",
                        ) {
                            DependencyStatus::Satisfied {
                                resolved_location: dll.display().to_string(),
                            }
                        } else {
                            DependencyStatus::Missing {
                                suggested_action: format!(
                                    "Stage '{}' ({}) from Plugin Bucket via RuntimeDependencyManager",
                                    dep.dependency_id, dep.version_req
                                ),
                            }
                        }
                    }
                    Some(dir)
                        if low.contains("bepinex") && dir.join("BepInEx").join("core").is_dir() =>
                    {
                        DependencyStatus::Satisfied {
                            resolved_location: dir
                                .join("BepInEx")
                                .join("core")
                                .display()
                                .to_string(),
                        }
                    }
                    Some(dir)
                        if low.contains("melonloader") && dir.join("MelonLoader").is_dir() =>
                    {
                        DependencyStatus::Satisfied {
                            resolved_location: dir.join("MelonLoader").display().to_string(),
                        }
                    }
                    _ => DependencyStatus::Missing {
                        suggested_action: format!(
                            "Stage or install '{}' ({}) via RuntimeDependencyManager",
                            dep.dependency_id, dep.version_req
                        ),
                    },
                };
                DependencyCheckResult {
                    dependency_id: dep.dependency_id.clone(),
                    optional: dep.optional,
                    outcome,
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
        validate_session_preflight_for_kind(&req, RuntimeKind::UnityBridgeV1)?;

        if !req
            .granted_permissions
            .allows(PermissionScope::ProcessWriteMemory)
        {
            return Err(RuntimeError::PermissionDenied(
                PermissionScope::ProcessWriteMemory,
            ));
        }

        let raw = req.declarative_payload_json.as_deref().ok_or_else(|| {
            RuntimeError::InvalidPayload("missing unity_bridge_v1 JSON entrypoint".to_owned())
        })?;
        let payload: UnityBridgePayload = serde_json::from_str(raw).map_err(|e| {
            RuntimeError::InvalidPayload(format!("invalid unity_bridge_v1 JSON: {e}"))
        })?;
        Self::validate_payload(&payload)?;

        let use_http = payload.protocol == UnityBridgeProtocol::UnityExplorerMcpHttpV1;
        let (resolved_port, effective_token) = if payload.auto_discover_by_pid {
            if use_http {
                let discovered_port = resolve_unity_explorer_discovery_port(
                    req.target_process.pid,
                    payload.discovery_dir.as_deref(),
                )
                .unwrap_or(payload.port);
                (discovered_port, payload.auth_token.clone())
            } else if let Some(disc) = resolve_nekod_unity_discovery(
                req.target_process.pid,
                payload.discovery_dir.as_deref(),
            ) {
                let tok = if payload.auth_token.trim().is_empty() {
                    let discovered_tok = disc.auth_token.unwrap_or_default();
                    if discovered_tok.trim().is_empty() {
                        return Err(RuntimeError::ExecutionFailed(format!(
                            "NekodUnityBridge discovery record unity-{}.json is missing auth_token",
                            req.target_process.pid
                        )));
                    }
                    discovered_tok
                } else {
                    payload.auth_token.clone()
                };
                (disc.port, tok)
            } else {
                if payload.auth_token.trim().is_empty() {
                    return Err(RuntimeError::ExecutionFailed(format!(
                        "NekodUnityBridge not ready for PID {}: discovery record unity-{}.json not found yet (wait for game/BepInEx to finish loading, or deploy NekodUnityBridge first)",
                        req.target_process.pid, req.target_process.pid
                    )));
                }
                (payload.port, payload.auth_token.clone())
            }
        } else {
            (payload.port, payload.auth_token.clone())
        };

        let endpoint = if use_http {
            validate_loopback_host_port(&payload.host, resolved_port)?
        } else {
            validate_loopback_bridge(&payload.host, resolved_port, &effective_token)?
        };

        // Perform handshake verifying reported Unity flavor (and optional PID) matches manifest.
        let ping_data = send_unity_rpc(
            use_http,
            endpoint,
            &effective_token,
            "ping",
            if use_http {
                json!({})
            } else {
                json!({ "expected_flavor": payload.flavor.as_str() })
            },
        )?;

        let reported_flavor = ping_data
            .get("flavor")
            .or_else(|| ping_data.get("Backend"))
            .or_else(|| ping_data.get("backend"))
            .and_then(Value::as_str)
            .unwrap_or("");
        if !reported_flavor.eq_ignore_ascii_case(payload.flavor.as_str()) {
            return Err(RuntimeError::ProcessIdentityMismatch(format!(
                "unity bridge flavor mismatch: expected '{}', bridge reported '{reported_flavor}'",
                payload.flavor.as_str()
            )));
        }

        if let Some(reported_pid) = ping_data
            .get("Pid")
            .or_else(|| ping_data.get("pid"))
            .and_then(Value::as_u64)
            && reported_pid != 0
            && reported_pid != u64::from(req.target_process.pid)
        {
            return Err(RuntimeError::ProcessIdentityMismatch(format!(
                "unity bridge PID mismatch: target pid {}, bridge reported {reported_pid}",
                req.target_process.pid
            )));
        }

        let session = ManagedBridgeSession {
            endpoint,
            auth_token: effective_token,
            target_process: req.target_process,
            runtime_kind: RuntimeKind::UnityBridgeV1,
            use_unity_explorer_http: use_http,
            file_ipc_dir: None,
            state: SessionLifecycleState::Attached,
            rollback_stack: Vec::new(),
            rollback_failures: 0,
            recent_events: vec![format!(
                "Authenticated Unity {} ({:?}) bridge on {endpoint}",
                payload.flavor.as_str(),
                payload.protocol
            )],
        };

        let sid = self.store.insert("unity", session);
        self.bindings.lock().unwrap().insert(
            sid.0.clone(),
            ActiveUnitySession {
                payload,
                verified_assets: req.verified_package_assets,
            },
        );
        Ok(sid)
    }

    fn apply_feature(
        &self,
        session_id: &RuntimeSessionId,
        command: FeatureCommand,
    ) -> Result<(), RuntimeError> {
        enum RollbackUpdate {
            Replace(Vec<BridgeRpcRequest>),
            Append(Vec<BridgeRpcRequest>),
            RemoveFeature,
            None,
        }

        let (binding, verified_assets) = {
            let guard = self.bindings.lock().unwrap();
            let active = guard
                .get(&session_id.0)
                .ok_or_else(|| RuntimeError::SessionNotFound(session_id.clone()))?;
            let found = active
                .payload
                .features
                .iter()
                .find(|f| f.feature_id() == command.feature_id)
                .cloned()
                .ok_or_else(|| RuntimeError::UnknownFeature(command.feature_id.clone()))?;
            (found, active.verified_assets.clone())
        };

        // Snapshot endpoint & token without holding the session lock while blocking on TCP I/O.
        let (endpoint, auth_token, use_http) = self.store.session_connection(session_id)?;

        let (calls, rollback_update, event_msg) = match (binding, command.value) {
            (
                UnityFeatureBinding::ToggleField {
                    class_name,
                    field_name,
                    enabled_value,
                    disabled_value,
                    persist,
                    extra_targets,
                    ..
                },
                FeatureValue::Toggle(enable),
            ) => {
                let mut targets = Vec::with_capacity(1 + extra_targets.len());
                targets.push((
                    class_name.clone(),
                    field_name.clone(),
                    enabled_value,
                    disabled_value,
                ));
                for extra in extra_targets {
                    targets.push((
                        extra.class_name,
                        extra.field_name,
                        extra.enabled_value,
                        extra.disabled_value,
                    ));
                }

                let target_count = targets.len();
                let mut calls = Vec::with_capacity(target_count);
                let mut rollback_entries = Vec::new();
                for (idx, (cls, fld, en_val, dis_val)) in targets.into_iter().enumerate() {
                    if persist && !use_http {
                        let pin_id = format!("{}:{idx}", command.feature_id);
                        if enable {
                            calls.push((
                                "pin_field",
                                json!({
                                    "id": pin_id,
                                    "class_name": cls,
                                    "field_name": fld,
                                    "value": en_val
                                }),
                            ));
                            rollback_entries.push(BridgeRpcRequest {
                                token: auth_token.clone(),
                                op: "unpin_field".to_owned(),
                                params: json!({
                                    "id": pin_id,
                                    "class_name": cls,
                                    "field_name": fld,
                                    "restore_value": dis_val
                                }),
                            });
                        } else {
                            calls.push((
                                "unpin_field",
                                json!({
                                    "id": pin_id,
                                    "class_name": cls,
                                    "field_name": fld,
                                    "restore_value": dis_val
                                }),
                            ));
                        }
                    } else {
                        let target_val = if enable { en_val } else { dis_val.clone() };
                        let (op, params, undo_params) = if use_http {
                            (
                                "set_static",
                                json!({ "type": cls, "member": fld, "value": target_val }),
                                json!({ "type": cls, "member": fld, "value": dis_val }),
                            )
                        } else {
                            (
                                "set_field",
                                json!({ "class_name": cls, "field_name": fld, "value": target_val }),
                                json!({ "class_name": cls, "field_name": fld, "value": dis_val }),
                            )
                        };
                        calls.push((op, params));
                        if enable {
                            rollback_entries.push(BridgeRpcRequest {
                                token: auth_token.clone(),
                                op: op.to_owned(),
                                params: undo_params,
                            });
                        }
                    }
                }

                let rollback_update = if enable {
                    RollbackUpdate::Replace(rollback_entries)
                } else {
                    RollbackUpdate::RemoveFeature
                };
                (
                    calls,
                    rollback_update,
                    format!(
                        "Unity field '{class_name}.{field_name}' {} (enable={enable}, targets={target_count})",
                        if persist && !use_http {
                            "pinned"
                        } else {
                            "set"
                        }
                    ),
                )
            }
            (
                UnityFeatureBinding::SetField {
                    class_name,
                    field_name,
                    persist,
                    scale,
                    reciprocal,
                    extra_targets,
                    ..
                },
                ref raw_val @ (FeatureValue::Integer(_) | FeatureValue::Float(_)),
            ) => {
                let display_val = match raw_val {
                    FeatureValue::Integer(v) => v.to_string(),
                    FeatureValue::Float(v) => v.to_string(),
                    _ => unreachable!(),
                };
                let mut targets = Vec::with_capacity(1 + extra_targets.len());
                targets.push((class_name.clone(), field_name.clone(), scale, reciprocal));
                for extra in extra_targets {
                    targets.push((
                        extra.class_name,
                        extra.field_name,
                        extra.scale,
                        extra.reciprocal,
                    ));
                }

                let target_count = targets.len();
                let mut calls = Vec::with_capacity(target_count);
                let mut rollback_entries = Vec::new();
                for (idx, (cls, fld, target_scale, target_reciprocal)) in
                    targets.into_iter().enumerate()
                {
                    let val = transform_numeric_value(raw_val, target_scale, target_reciprocal)?;
                    if persist && !use_http {
                        let pin_id = format!("{}:{idx}", command.feature_id);
                        calls.push((
                            "pin_field",
                            json!({
                                "id": pin_id,
                                "class_name": cls,
                                "field_name": fld,
                                "value": val
                            }),
                        ));
                        rollback_entries.push(BridgeRpcRequest {
                            token: auth_token.clone(),
                            op: "unpin_field".to_owned(),
                            params: json!({ "id": pin_id }),
                        });
                    } else if use_http {
                        calls.push((
                            "set_static",
                            json!({ "type": cls, "member": fld, "value": val }),
                        ));
                        rollback_entries.push(BridgeRpcRequest {
                            token: String::new(),
                            op: "undo_write".to_owned(),
                            params: json!({ "steps": 1 }),
                        });
                    } else {
                        calls.push((
                            "set_field",
                            json!({ "class_name": cls, "field_name": fld, "value": val }),
                        ));
                    }
                }

                let rollback_update = if persist && !use_http {
                    RollbackUpdate::Replace(rollback_entries)
                } else if use_http {
                    RollbackUpdate::Append(rollback_entries)
                } else {
                    RollbackUpdate::None
                };
                (
                    calls,
                    rollback_update,
                    format!(
                        "Unity field '{class_name}.{field_name}' = {display_val} (targets={target_count})"
                    ),
                )
            }
            (
                UnityFeatureBinding::InvokeMethod {
                    class_name,
                    method_name,
                    args,
                    pre_steps,
                    ..
                },
                FeatureValue::Action,
            ) => {
                if use_http {
                    if !pre_steps.is_empty() {
                        return Err(RuntimeError::InvalidPayload(format!(
                            "invoke_method feature '{}' with pre_steps requires standalone NekodUnityBridge (nekod_json_line_v1)",
                            command.feature_id
                        )));
                    }
                    (
                        vec![(
                            "invoke_static",
                            json!({ "type": class_name, "method": method_name, "args": args }),
                        )],
                        RollbackUpdate::None,
                        format!("Invoked Unity method '{class_name}.{method_name}'"),
                    )
                } else {
                    let step_count = pre_steps.len();
                    let mut calls = Vec::with_capacity(step_count + 1);
                    for step in pre_steps {
                        match step {
                            UnityActionStep::SetField {
                                class_name: step_class,
                                field_name: step_field,
                                value,
                            } => {
                                calls.push((
                                    "set_field",
                                    json!({
                                        "class_name": step_class,
                                        "field_name": step_field,
                                        "value": value,
                                    }),
                                ));
                            }
                            UnityActionStep::CopyField {
                                from_class,
                                from_field,
                                to_class,
                                to_field,
                            } => {
                                calls.push((
                                    "copy_field",
                                    json!({
                                        "from_class": from_class,
                                        "from_field": from_field,
                                        "to_class": to_class,
                                        "to_field": to_field,
                                    }),
                                ));
                            }
                            UnityActionStep::InvokeMethod {
                                class_name: step_class,
                                method_name: step_method,
                                args: step_args,
                            } => {
                                calls.push(build_invoke_method_call(
                                    &step_class,
                                    &step_method,
                                    &step_args,
                                ));
                            }
                        }
                    }
                    calls.push(build_invoke_method_call(&class_name, &method_name, &args));
                    let summary = if step_count == 0 {
                        format!("Invoked Unity method '{class_name}.{method_name}'")
                    } else {
                        format!(
                            "Invoked Unity method '{class_name}.{method_name}' after {step_count} pre_steps"
                        )
                    };
                    (calls, RollbackUpdate::None, summary)
                }
            }
            (
                UnityFeatureBinding::TargetLock {
                    player_class,
                    camera_class,
                    target_class,
                    lock_method,
                    unlock_method,
                    alive_checks,
                    max_range,
                    height_offset,
                    lock_speed,
                    require_collider,
                    interval_ms,
                    player_tags,
                    ..
                },
                FeatureValue::Toggle(enable),
            ) => {
                if use_http {
                    return Err(RuntimeError::InvalidPayload(format!(
                        "target_lock feature '{}' requires standalone NekodUnityBridge (nekod_json_line_v1)",
                        command.feature_id
                    )));
                }
                let unpin_params = json!({ "id": command.feature_id });
                if enable {
                    let mut pin_map = serde_json::Map::new();
                    pin_map.insert("id".to_owned(), json!(command.feature_id));
                    pin_map.insert("player_class".to_owned(), json!(player_class));
                    pin_map.insert("camera_class".to_owned(), json!(camera_class));
                    pin_map.insert("target_class".to_owned(), json!(target_class));
                    pin_map.insert("lock_method".to_owned(), json!(lock_method));
                    pin_map.insert("unlock_method".to_owned(), json!(unlock_method));
                    pin_map.insert("alive_checks".to_owned(), json!(alive_checks));
                    pin_map.insert("max_range".to_owned(), json!(max_range));
                    pin_map.insert("height_offset".to_owned(), json!(height_offset));
                    pin_map.insert("lock_speed".to_owned(), json!(lock_speed));
                    pin_map.insert("require_collider".to_owned(), json!(require_collider));
                    pin_map.insert("interval_ms".to_owned(), json!(interval_ms));
                    if !player_tags.is_empty() {
                        pin_map.insert("player_tags".to_owned(), json!(player_tags));
                    }
                    (
                        vec![("pin_target_lock", Value::Object(pin_map))],
                        RollbackUpdate::Replace(vec![BridgeRpcRequest {
                            token: auth_token.clone(),
                            op: "unpin_target_lock".to_owned(),
                            params: unpin_params,
                        }]),
                        format!(
                            "Unity target lock '{target_class}' -> '{camera_class}.{lock_method}' armed (range={max_range})"
                        ),
                    )
                } else {
                    (
                        vec![("unpin_target_lock", unpin_params)],
                        RollbackUpdate::RemoveFeature,
                        format!("Unity target lock '{target_class}' disarmed"),
                    )
                }
            }
            (
                UnityFeatureBinding::DamageAura {
                    player_class,
                    target_class,
                    require_active_class,
                    exclude_bool_field,
                    damage_amount,
                    radius,
                    interval_ms,
                    handlers,
                    player_tags,
                    ..
                },
                FeatureValue::Toggle(enable),
            ) => {
                if use_http {
                    return Err(RuntimeError::InvalidPayload(format!(
                        "damage_aura feature '{}' requires standalone NekodUnityBridge (nekod_json_line_v1)",
                        command.feature_id
                    )));
                }
                let unpin_params = json!({ "id": command.feature_id });
                if enable {
                    let mut pin_map = serde_json::Map::new();
                    pin_map.insert("id".to_owned(), json!(command.feature_id));
                    pin_map.insert("player_class".to_owned(), json!(player_class));
                    pin_map.insert("target_class".to_owned(), json!(target_class));
                    if let Some(req_cls) = require_active_class
                        .as_deref()
                        .filter(|s| !s.trim().is_empty())
                    {
                        pin_map.insert("require_active_class".to_owned(), json!(req_cls));
                    }
                    if let Some(ex_fld) = exclude_bool_field
                        .as_deref()
                        .filter(|s| !s.trim().is_empty())
                    {
                        pin_map.insert("exclude_bool_field".to_owned(), json!(ex_fld));
                    }
                    pin_map.insert("damage_amount".to_owned(), json!(damage_amount));
                    pin_map.insert("radius".to_owned(), json!(radius));
                    pin_map.insert("interval_ms".to_owned(), json!(interval_ms));
                    pin_map.insert("handlers".to_owned(), json!(handlers));
                    if !player_tags.is_empty() {
                        pin_map.insert("player_tags".to_owned(), json!(player_tags));
                    }
                    (
                        vec![("pin_damage_aura", Value::Object(pin_map))],
                        RollbackUpdate::Replace(vec![BridgeRpcRequest {
                            token: auth_token.clone(),
                            op: "unpin_damage_aura".to_owned(),
                            params: unpin_params,
                        }]),
                        format!(
                            "Unity damage aura '{target_class}' armed (damage={damage_amount}, radius={radius})"
                        ),
                    )
                } else {
                    (
                        vec![("unpin_damage_aura", unpin_params)],
                        RollbackUpdate::RemoveFeature,
                        format!("Unity damage aura '{target_class}' disarmed"),
                    )
                }
            }
            (
                UnityFeatureBinding::SpinBot {
                    player_class,
                    target_class,
                    require_active_class,
                    spin_node_path,
                    spin_local,
                    late_update,
                    roll_field,
                    roll_speed_deg,
                    orbit_radius,
                    orbit_speed_deg,
                    height_offset,
                    aim_at_target,
                    max_range,
                    require_alive,
                    alive_checks,
                    interval_ms,
                    player_tags,
                    ..
                },
                FeatureValue::Toggle(enable),
            ) => {
                if use_http {
                    return Err(RuntimeError::InvalidPayload(format!(
                        "spin_bot feature '{}' requires standalone NekodUnityBridge (nekod_json_line_v1)",
                        command.feature_id
                    )));
                }
                let unpin_params = json!({ "id": command.feature_id });
                if enable {
                    let mut pin_map = serde_json::Map::new();
                    pin_map.insert("id".to_owned(), json!(command.feature_id));
                    pin_map.insert("player_class".to_owned(), json!(player_class));
                    if let Some(tcls) = target_class.as_deref().filter(|s| !s.trim().is_empty()) {
                        pin_map.insert("target_class".to_owned(), json!(tcls));
                    }
                    if let Some(req_cls) = require_active_class
                        .as_deref()
                        .filter(|s| !s.trim().is_empty())
                    {
                        pin_map.insert("require_active_class".to_owned(), json!(req_cls));
                    }
                    if let Some(node) = spin_node_path.as_deref().filter(|s| !s.trim().is_empty()) {
                        pin_map.insert("spin_node_path".to_owned(), json!(node));
                    }
                    if let Some(fld) = roll_field.as_deref().filter(|s| !s.trim().is_empty()) {
                        pin_map.insert("roll_field".to_owned(), json!(fld));
                    }
                    pin_map.insert("spin_local".to_owned(), json!(spin_local));
                    pin_map.insert("late_update".to_owned(), json!(late_update));
                    pin_map.insert("roll_speed_deg".to_owned(), json!(roll_speed_deg));
                    pin_map.insert("orbit_radius".to_owned(), json!(orbit_radius));
                    pin_map.insert("orbit_speed_deg".to_owned(), json!(orbit_speed_deg));
                    pin_map.insert("height_offset".to_owned(), json!(height_offset));
                    pin_map.insert("aim_at_target".to_owned(), json!(aim_at_target));
                    pin_map.insert("max_range".to_owned(), json!(max_range));
                    pin_map.insert("require_alive".to_owned(), json!(require_alive));
                    pin_map.insert("alive_checks".to_owned(), json!(alive_checks));
                    pin_map.insert("interval_ms".to_owned(), json!(interval_ms));
                    if !player_tags.is_empty() {
                        pin_map.insert("player_tags".to_owned(), json!(player_tags));
                    }
                    (
                        vec![("pin_spinbot", Value::Object(pin_map))],
                        RollbackUpdate::Replace(vec![BridgeRpcRequest {
                            token: auth_token.clone(),
                            op: "unpin_spinbot".to_owned(),
                            params: unpin_params,
                        }]),
                        format!(
                            "Unity SpinBot armed on '{player_class}' (roll={roll_speed_deg} deg/s, late_update={late_update})"
                        ),
                    )
                } else {
                    (
                        vec![("unpin_spinbot", unpin_params)],
                        RollbackUpdate::RemoveFeature,
                        format!("Unity SpinBot '{}' disarmed", command.feature_id),
                    )
                }
            }
            (
                UnityFeatureBinding::SetGameObjectActive {
                    game_object_name,
                    game_object_prefix,
                    game_object_names,
                    game_object_prefixes,
                    enabled_active,
                    disabled_active,
                    include_inactive,
                    ..
                },
                val @ (FeatureValue::Toggle(_) | FeatureValue::Action),
            ) => {
                if use_http {
                    return Err(RuntimeError::InvalidPayload(format!(
                        "set_game_object_active feature '{}' requires standalone NekodUnityBridge (nekod_json_line_v1)",
                        command.feature_id
                    )));
                }
                let is_toggle_enable = matches!(val, FeatureValue::Toggle(true));
                let target_active = match val {
                    FeatureValue::Toggle(true) | FeatureValue::Action => enabled_active,
                    FeatureValue::Toggle(false) => disabled_active.unwrap_or(!enabled_active),
                    _ => unreachable!(),
                };
                let build_go_params = |active_flag: bool| {
                    let mut m = serde_json::Map::new();
                    if let Some(n) = &game_object_name {
                        m.insert("game_object_name".to_owned(), json!(n));
                    }
                    if let Some(pfx) = &game_object_prefix {
                        m.insert("game_object_prefix".to_owned(), json!(pfx));
                    }
                    if !game_object_names.is_empty() {
                        m.insert("game_object_names".to_owned(), json!(game_object_names));
                    }
                    if !game_object_prefixes.is_empty() {
                        m.insert(
                            "game_object_prefixes".to_owned(),
                            json!(game_object_prefixes),
                        );
                    }
                    m.insert("active".to_owned(), json!(active_flag));
                    m.insert("include_inactive".to_owned(), json!(include_inactive));
                    Value::Object(m)
                };
                let params = build_go_params(target_active);
                let rollback_update = if is_toggle_enable {
                    RollbackUpdate::Replace(vec![BridgeRpcRequest {
                        token: auth_token.clone(),
                        op: "set_gameobject_active".to_owned(),
                        params: build_go_params(disabled_active.unwrap_or(!enabled_active)),
                    }])
                } else if matches!(val, FeatureValue::Toggle(false)) {
                    RollbackUpdate::RemoveFeature
                } else {
                    RollbackUpdate::None
                };
                (
                    vec![("set_gameobject_active", params)],
                    rollback_update,
                    format!(
                        "Unity GameObject(s) active={target_active} (feature '{}')",
                        command.feature_id
                    ),
                )
            }
            (
                UnityFeatureBinding::CopyField {
                    from_class,
                    from_field,
                    to_class,
                    to_field,
                    ..
                },
                FeatureValue::Action | FeatureValue::Toggle(true),
            ) => {
                if use_http {
                    return Err(RuntimeError::InvalidPayload(format!(
                        "copy_field feature '{}' requires standalone NekodUnityBridge (nekod_json_line_v1)",
                        command.feature_id
                    )));
                }
                (
                    vec![(
                        "copy_field",
                        json!({
                            "from_class": from_class,
                            "from_field": from_field,
                            "to_class": to_class,
                            "to_field": to_field
                        }),
                    )],
                    RollbackUpdate::None,
                    format!(
                        "Copied Unity field '{from_class}.{from_field}' -> '{to_class}.{to_field}'"
                    ),
                )
            }
            (
                UnityFeatureBinding::ReplaceTextureMaterial {
                    renderer_name_prefix,
                    renderer_name_prefixes,
                    material_name_prefix,
                    material_name_prefixes,
                    asset_path,
                    shader_name,
                    color,
                    ..
                },
                FeatureValue::Toggle(enable),
            ) => {
                if use_http {
                    return Err(RuntimeError::InvalidPayload(format!(
                        "replace_texture_material feature '{}' requires standalone NekodUnityBridge (nekod_json_line_v1)",
                        command.feature_id
                    )));
                }
                let mat_id = format!("mat:{}", command.feature_id);
                if enable {
                    let verified_path = verified_assets.get(&asset_path).ok_or_else(|| {
                        RuntimeError::InvalidPayload(format!(
                            "material asset '{asset_path}' for feature '{}' is not declared or SHA-256 verified in manifest.artifacts",
                            command.feature_id
                        ))
                    })?;
                    let mut m = serde_json::Map::new();
                    m.insert("id".to_owned(), json!(mat_id));
                    m.insert("texture_path".to_owned(), json!(verified_path));
                    if let Some(rp) = renderer_name_prefix {
                        m.insert("renderer_name_prefix".to_owned(), json!(rp));
                    }
                    if !renderer_name_prefixes.is_empty() {
                        m.insert(
                            "renderer_name_prefixes".to_owned(),
                            json!(renderer_name_prefixes),
                        );
                    }
                    if let Some(mp) = material_name_prefix {
                        m.insert("material_name_prefix".to_owned(), json!(mp));
                    }
                    if !material_name_prefixes.is_empty() {
                        m.insert(
                            "material_name_prefixes".to_owned(),
                            json!(material_name_prefixes),
                        );
                    }
                    if let Some(sh) = shader_name {
                        m.insert("shader_name".to_owned(), json!(sh));
                    }
                    if let Some(rgba) = color {
                        m.insert("color".to_owned(), json!(rgba));
                    }
                    (
                        vec![("replace_texture_material", Value::Object(m))],
                        RollbackUpdate::Replace(vec![BridgeRpcRequest {
                            token: auth_token.clone(),
                            op: "restore_texture_material".to_owned(),
                            params: json!({ "id": mat_id }),
                        }]),
                        format!(
                            "Replaced Unity material textures using verified asset '{asset_path}'"
                        ),
                    )
                } else {
                    (
                        vec![("restore_texture_material", json!({ "id": mat_id }))],
                        RollbackUpdate::RemoveFeature,
                        format!(
                            "Restored original Unity material textures for '{}'",
                            command.feature_id
                        ),
                    )
                }
            }
            (
                UnityFeatureBinding::HookSetField {
                    hook_class,
                    hook_method,
                    timing,
                    param_count,
                    apply_immediately,
                    writes,
                    ..
                },
                FeatureValue::Toggle(enable),
            ) => {
                if use_http {
                    return Err(RuntimeError::InvalidPayload(format!(
                        "hook_set_field feature '{}' requires standalone NekodUnityBridge (nekod_json_line_v1)",
                        command.feature_id
                    )));
                }
                let hook_id = format!("hook:{}", command.feature_id);
                if enable {
                    let mut m = serde_json::Map::new();
                    m.insert("id".to_owned(), json!(hook_id));
                    m.insert("hook_class".to_owned(), json!(hook_class));
                    m.insert("hook_method".to_owned(), json!(hook_method));
                    m.insert(
                        "timing".to_owned(),
                        json!(timing.trim().to_ascii_lowercase()),
                    );
                    if let Some(cnt) = param_count {
                        m.insert("param_count".to_owned(), json!(cnt));
                    }
                    m.insert("apply_immediately".to_owned(), json!(apply_immediately));
                    m.insert(
                        "writes".to_owned(),
                        serde_json::to_value(&writes).map_err(|e| {
                            RuntimeError::InvalidPayload(format!(
                                "failed to serialize hook_set_field writes for '{}': {e}",
                                command.feature_id
                            ))
                        })?,
                    );
                    (
                        vec![("pin_method_hook", Value::Object(m))],
                        RollbackUpdate::Replace(vec![BridgeRpcRequest {
                            token: auth_token.clone(),
                            op: "unpin_method_hook".to_owned(),
                            params: json!({ "id": hook_id }),
                        }]),
                        format!(
                            "Installed Unity {timing} method hook on '{hook_class}.{hook_method}' for '{}'",
                            command.feature_id
                        ),
                    )
                } else {
                    (
                        vec![("unpin_method_hook", json!({ "id": hook_id }))],
                        RollbackUpdate::RemoveFeature,
                        format!(
                            "Removed Unity method hook on '{hook_class}.{hook_method}' for '{}'",
                            command.feature_id
                        ),
                    )
                }
            }
            _ => {
                return Err(RuntimeError::InvalidPayload(format!(
                    "control type mismatch for Unity feature '{}'",
                    command.feature_id
                )));
            }
        };

        for (op, params) in calls {
            send_unity_rpc(use_http, endpoint, &auth_token, op, params)
                .map_err(map_bridge_rpc_error)?;
        }

        self.store.with_session_mut(session_id, |session| {
            match rollback_update {
                RollbackUpdate::Replace(reqs) => {
                    session
                        .rollback_stack
                        .retain(|(fid, _)| fid != &command.feature_id);
                    for req in reqs {
                        session
                            .rollback_stack
                            .push((command.feature_id.clone(), req));
                    }
                }
                RollbackUpdate::Append(reqs) => {
                    for req in reqs {
                        session
                            .rollback_stack
                            .push((command.feature_id.clone(), req));
                    }
                }
                RollbackUpdate::RemoveFeature => session
                    .rollback_stack
                    .retain(|(fid, _)| fid != &command.feature_id),
                RollbackUpdate::None => {}
            }
            session.recent_events.push(event_msg);
            Ok(())
        })
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
    use std::collections::{BTreeMap, BTreeSet};
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

    fn unity_request(
        flavor: UnityFlavor,
        loader: UnityLoaderKind,
        port: u16,
    ) -> StartSessionRequest {
        let payload = serde_json::json!({
            "schema_version": "1.0.0",
            "flavor": flavor,
            "loader": loader,
            "host": "127.0.0.1",
            "port": port,
            "auth_token": "session_token_0123456789abcdef",
            "features": [{
                "kind": "toggle_field",
                "feature_id": "infinite-stamina",
                "class_name": "PlayerController",
                "field_name": "drainRate",
                "enabled_value": 0.0,
                "disabled_value": 1.0
            }]
        })
        .to_string();

        let digest = "a".repeat(64);
        let manifest = TrainerManifest {
            schema_version: "1.0.0".parse().unwrap(),
            id: "dev.example.unity-bridge".to_owned(),
            name: "Unity Bridge Mod".to_owned(),
            version: "1.0.0".parse().unwrap(),
            description: "Unity Mono reflection bridge test".to_owned(),
            authors: vec![AuthorInfo {
                name: "Nekod".to_owned(),
                github: None,
                url: None,
            }],
            license: "MIT".to_owned(),
            source_repository: "https://github.com/OmaruKoma/Nekod".to_owned(),
            homepage: None,
            target_game: TargetGameRef {
                game_id: "sample-unity-game".to_owned(),
                game_name: "Sample Unity Game".to_owned(),
                store_ids: None,
            },
            supported_versions: vec![SupportedGameVersion {
                platform: TargetPlatform::WindowsX64,
                version_pattern: "*".to_owned(),
                executable_sha256: None,
                status: CompatibilityStatus::Verified,
            }],
            runtime: nekod_manifest::RuntimeRequirement {
                kind: RuntimeKind::UnityBridgeV1,
                api_version: "1.0.0".parse().unwrap(),
                min_nekod_version: "0.1.0-alpha.0".parse().unwrap(),
                entrypoint: "unity_bridge.json".to_owned(),
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
                    reason: "Discover Unity process".to_owned(),
                    required: true,
                    constraints: None,
                },
                PermissionDeclaration {
                    scope: PermissionScope::ProcessWriteMemory,
                    reason: "Mutate Unity fields via managed reflection bridge".to_owned(),
                    required: true,
                    constraints: None,
                },
            ],
            features: vec![TrainerFeature {
                id: "infinite-stamina".to_owned(),
                name: "Infinite Stamina".to_owned(),
                description: "Set drainRate to 0".to_owned(),
                category: FeatureCategory::Player,
                control_type: FeatureControlType::Toggle,
                default_hotkey: None,
                mutates_memory: true,
                supports_rollback: true,
                risk_level: FeatureRiskLevel::Low,
                value_range: None,
            }],
            artifacts: vec![TrainerArtifact {
                filename: "unity_bridge.json".to_owned(),
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
                pid: 5200,
                process_name: "UnityGame.exe".to_owned(),
                executable_path: "C:\\Games\\UnityGame.exe".to_owned(),
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
    fn unity_bridge_authenticates_enforces_mono_vs_il2cpp_and_rolls_back_field() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let observed_ops = Arc::new(Mutex::new(Vec::<(String, Value)>::new()));
        let ops_clone = observed_ops.clone();

        let server_thread = thread::spawn(move || {
            for stream in listener.incoming().take(4) {
                let mut stream = stream.unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                let req: Value = serde_json::from_str(&line).unwrap();
                if req["token"] != "session_token_0123456789abcdef" {
                    writeln!(stream, "{{\"ok\":false,\"error\":\"unauthorized\"}}").unwrap();
                    continue;
                }
                let op = req["op"].as_str().unwrap().to_owned();
                ops_clone
                    .lock()
                    .unwrap()
                    .push((op.clone(), req["params"].clone()));
                if op == "ping" {
                    writeln!(stream, "{{\"ok\":true,\"data\":{{\"flavor\":\"mono\"}}}}").unwrap();
                } else {
                    writeln!(stream, "{{\"ok\":true,\"data\":{{}}}}").unwrap();
                }
            }
        });

        let router = CompositeRuntimeRouter::with_default_providers();

        // 1. Incompatible Mono/IL2CPP pairing in manifest must fail before connecting.
        assert!(matches!(
            router.start_session(unity_request(
                UnityFlavor::Mono,
                UnityLoaderKind::BepinexIl2cpp,
                port
            )),
            Err(RuntimeError::InvalidPayload(_))
        ));

        // 2. IL2CPP request rejected when running bridge reports Mono flavor.
        assert!(matches!(
            router.start_session(unity_request(
                UnityFlavor::Il2cpp,
                UnityLoaderKind::BepinexIl2cpp,
                port
            )),
            Err(RuntimeError::ProcessIdentityMismatch(_))
        ));

        // 3. Matching Mono session mutates field and restores original value on stop.
        let sid = router
            .start_session(unity_request(
                UnityFlavor::Mono,
                UnityLoaderKind::BepinexMono,
                port,
            ))
            .unwrap();
        router
            .apply_feature(
                &sid,
                FeatureCommand {
                    feature_id: "infinite-stamina".to_owned(),
                    value: FeatureValue::Toggle(true),
                },
            )
            .unwrap();
        let diag = router.stop_session(&sid).unwrap();
        assert_eq!(diag.rollback_failures, 0);
        assert_eq!(diag.runtime_kind, RuntimeKind::UnityBridgeV1);

        server_thread.join().unwrap();
        let log = observed_ops.lock().unwrap();
        assert_eq!(log.len(), 4);
        assert_eq!(log[2].0, "set_field");
        assert_eq!(log[2].1["value"], json!(0.0));
        assert_eq!(log[3].0, "set_field");
        assert_eq!(log[3].1["value"], json!(1.0));
    }

    #[test]
    fn unity_explorer_mcp_http_discovers_pid_port_verifies_il2cpp_and_rolls_back() {
        use std::io::Read;
        use std::time::{SystemTime, UNIX_EPOCH};

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let discovery_dir = std::env::temp_dir().join(format!("nekod_ue_mcp_{nanos}"));
        fs::create_dir_all(&discovery_dir).unwrap();
        fs::write(
            discovery_dir.join("bridge-10456.json"),
            serde_json::json!({
                "pid": 10456,
                "port": port,
                "backend": "IL2CPP",
                "loader": "BepInEx",
                "unity_version": "2022.3.51f1"
            })
            .to_string(),
        )
        .unwrap();

        let observed = Arc::new(Mutex::new(Vec::<(String, Value)>::new()));
        let observed_clone = Arc::clone(&observed);
        let server = thread::spawn(move || {
            for stream in listener.incoming().take(6) {
                let mut stream = stream.unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut content_len = 0usize;
                let mut line = String::new();
                loop {
                    line.clear();
                    reader.read_line(&mut line).unwrap();
                    let trimmed = line.trim_end_matches(['\r', '\n']);
                    if trimmed.is_empty() {
                        break;
                    }
                    if let Some((k, v)) = trimmed.split_once(':')
                        && k.trim().eq_ignore_ascii_case("content-length")
                    {
                        content_len = v.trim().parse().unwrap_or(0);
                    }
                }
                let mut body = vec![0u8; content_len];
                reader.read_exact(&mut body).unwrap();
                let req: Value = serde_json::from_slice(&body).unwrap();
                let method = req["method"].as_str().unwrap().to_owned();
                observed_clone
                    .lock()
                    .unwrap()
                    .push((method.clone(), req["params"].clone()));

                let resp_json = match method.as_str() {
                    "ping" => serde_json::json!({
                        "Success": true,
                        "Data": {
                            "Status": "ok",
                            "Backend": "IL2CPP",
                            "Loader": "BepInEx",
                            "Pid": 10456,
                            "Port": port,
                            "UnityVersion": "2022.3.51f1"
                        }
                    })
                    .to_string(),
                    _ => serde_json::json!({ "Success": true, "Data": {} }).to_string(),
                };
                let http_resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    resp_json.len(),
                    resp_json
                );
                stream.write_all(http_resp.as_bytes()).unwrap();
                stream.flush().unwrap();
            }
        });

        let payload = serde_json::json!({
            "schema_version": "1.0.0",
            "protocol": "unity_explorer_mcp_http_v1",
            "flavor": "il2cpp",
            "loader": "bepinex_il2cpp",
            "host": "127.0.0.1",
            "port": 0,
            "auto_discover_by_pid": true,
            "discovery_dir": discovery_dir.display().to_string(),
            "features": [
                {
                    "kind": "toggle_field",
                    "feature_id": "infinite-stamina",
                    "class_name": "Game.PlayerStats",
                    "field_name": "s_staminaCostMultiplier",
                    "enabled_value": 0.0,
                    "disabled_value": 1.0
                },
                {
                    "kind": "set_field",
                    "feature_id": "time-scale",
                    "class_name": "UnityEngine.Time",
                    "field_name": "timeScale"
                },
                {
                    "kind": "invoke_method",
                    "feature_id": "heal-player",
                    "class_name": "Game.PlayerStats",
                    "method_name": "RestoreAll"
                }
            ]
        })
        .to_string();

        let mut req = unity_request(UnityFlavor::Il2cpp, UnityLoaderKind::BepinexIl2cpp, 0);
        req.target_process.pid = 10456;
        req.trainer.features = vec![
            TrainerFeature {
                id: "infinite-stamina".to_owned(),
                name: "Infinite Stamina".to_owned(),
                description: "No stamina drain".to_owned(),
                category: FeatureCategory::Player,
                control_type: FeatureControlType::Toggle,
                default_hotkey: None,
                mutates_memory: true,
                supports_rollback: true,
                risk_level: FeatureRiskLevel::Low,
                value_range: None,
            },
            TrainerFeature {
                id: "time-scale".to_owned(),
                name: "Time Scale".to_owned(),
                description: "Adjust game speed".to_owned(),
                category: FeatureCategory::Time,
                control_type: FeatureControlType::Float,
                default_hotkey: None,
                mutates_memory: true,
                supports_rollback: true,
                risk_level: FeatureRiskLevel::Low,
                value_range: None,
            },
            TrainerFeature {
                id: "heal-player".to_owned(),
                name: "Heal Player".to_owned(),
                description: "Trigger static restore".to_owned(),
                category: FeatureCategory::Player,
                control_type: FeatureControlType::Action,
                default_hotkey: None,
                mutates_memory: true,
                supports_rollback: false,
                risk_level: FeatureRiskLevel::Low,
                value_range: None,
            },
        ];
        req.declarative_payload_json = Some(payload);

        let router = CompositeRuntimeRouter::with_default_providers();
        let sid = router.start_session(req).unwrap();
        router
            .apply_feature(
                &sid,
                FeatureCommand {
                    feature_id: "infinite-stamina".to_owned(),
                    value: FeatureValue::Toggle(true),
                },
            )
            .unwrap();
        router
            .apply_feature(
                &sid,
                FeatureCommand {
                    feature_id: "heal-player".to_owned(),
                    value: FeatureValue::Action,
                },
            )
            .unwrap();
        router
            .apply_feature(
                &sid,
                FeatureCommand {
                    feature_id: "time-scale".to_owned(),
                    value: FeatureValue::Float(2.0),
                },
            )
            .unwrap();

        let diag = router.stop_session(&sid).unwrap();
        assert_eq!(diag.rollback_failures, 0);
        assert_eq!(diag.active_patches, 0);

        server.join().unwrap();
        let _ = fs::remove_dir_all(discovery_dir);

        let ops = observed.lock().unwrap();
        assert_eq!(ops.len(), 6);
        assert_eq!(ops[0].0, "ping");
        assert_eq!(ops[1].0, "set_static");
        assert_eq!(ops[1].1["value"], json!(0.0));
        assert_eq!(ops[2].0, "invoke_static");
        assert_eq!(ops[2].1["method"], "RestoreAll");
        assert_eq!(ops[3].0, "set_static");
        assert_eq!(ops[3].1["value"], json!(2.0));
        // LIFO rollback: UndoStore 1 step for time-scale, then restore infinite-stamina = 1.0
        assert_eq!(ops[4].0, "undo_write");
        assert_eq!(ops[5].0, "set_static");
        assert_eq!(ops[5].1["value"], json!(1.0));
    }

    #[test]
    fn nekod_standalone_unity_bridge_discovers_pid_port_and_token_without_unity_explorer() {
        use std::time::{SystemTime, UNIX_EPOCH};

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let token = "a1b2c3d4e5f60718293a4b5c6d7e8f90";

        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let discovery_dir = std::env::temp_dir().join(format!("nekod_standalone_unity_{nanos}"));
        fs::create_dir_all(&discovery_dir).unwrap();
        fs::write(
            discovery_dir.join("unity-22040.json"),
            serde_json::json!({
                "pid": 22040,
                "port": port,
                "flavor": "il2cpp",
                "loader": "bepinex_il2cpp",
                "auth_token": token,
                "unity_version": "2022.3.51f1",
                "bridge_version": "0.1.0"
            })
            .to_string(),
        )
        .unwrap();

        let server = thread::spawn(move || {
            for stream in listener.incoming().take(3) {
                let mut stream = stream.unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                let req: Value = serde_json::from_str(&line).unwrap();
                assert_eq!(req["token"], token);
                let op = req["op"].as_str().unwrap();
                if op == "ping" {
                    writeln!(
                        stream,
                        "{{\"ok\":true,\"data\":{{\"flavor\":\"il2cpp\",\"pid\":22040,\"port\":{port}}}}}"
                    )
                    .unwrap();
                } else {
                    writeln!(stream, "{{\"ok\":true,\"data\":{{\"updated\":1}}}}").unwrap();
                }
            }
        });

        let payload = serde_json::json!({
            "schema_version": "1.0.0",
            "protocol": "nekod_json_line_v1",
            "flavor": "il2cpp",
            "loader": "bepinex_il2cpp",
            "host": "127.0.0.1",
            "port": 0,
            "auto_discover_by_pid": true,
            "discovery_dir": discovery_dir.display().to_string(),
            "features": [{
                "kind": "toggle_field",
                "feature_id": "infinite-stamina",
                "class_name": "UnityEngine.Time",
                "field_name": "timeScale",
                "enabled_value": 0.0,
                "disabled_value": 1.0
            }]
        })
        .to_string();

        let mut req = unity_request(UnityFlavor::Il2cpp, UnityLoaderKind::BepinexIl2cpp, 0);
        req.target_process.pid = 22040;
        req.declarative_payload_json = Some(payload);

        let router = CompositeRuntimeRouter::with_default_providers();
        let sid = router.start_session(req).unwrap();
        router
            .apply_feature(
                &sid,
                FeatureCommand {
                    feature_id: "infinite-stamina".to_owned(),
                    value: FeatureValue::Toggle(true),
                },
            )
            .unwrap();
        let diag = router.stop_session(&sid).unwrap();
        assert_eq!(diag.rollback_failures, 0);
        assert_eq!(diag.active_patches, 0);

        server.join().unwrap();
        let _ = fs::remove_dir_all(discovery_dir);
    }

    #[test]
    fn readonly_probe_executes_ping_and_get_field_and_refuses_mutations() {
        use std::time::{SystemTime, UNIX_EPOCH};

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let token = "probe_tok_0123456789abcdef012345";

        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let discovery_dir = std::env::temp_dir().join(format!("nekod_probe_unity_{nanos}"));
        fs::create_dir_all(&discovery_dir).unwrap();
        fs::write(
            discovery_dir.join("unity-44100.json"),
            serde_json::json!({
                "pid": 44100,
                "port": port,
                "flavor": "mono",
                "loader": "bepinex_mono",
                "auth_token": token
            })
            .to_string(),
        )
        .unwrap();

        let dir_str = discovery_dir.display().to_string();
        assert!(matches!(
            probe_nekod_unity_bridge_readonly(44100, Some(&dir_str), "set_field", json!({})),
            Err(RuntimeError::InvalidPayload(_))
        ));
        assert!(matches!(
            probe_nekod_unity_bridge_readonly(44100, Some(&dir_str), "invoke_method", json!({})),
            Err(RuntimeError::InvalidPayload(_))
        ));
        assert!(matches!(
            probe_nekod_unity_bridge_readonly(44100, Some(&dir_str), "pin_spinbot", json!({})),
            Err(RuntimeError::InvalidPayload(_))
        ));
        assert!(matches!(
            probe_nekod_unity_bridge_readonly(
                44100,
                Some(&dir_str),
                "replace_texture_material",
                json!({})
            ),
            Err(RuntimeError::InvalidPayload(_))
        ));

        let server = thread::spawn(move || {
            for stream in listener.incoming().take(5) {
                let mut stream = stream.unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                let req: Value = serde_json::from_str(&line).unwrap();
                assert_eq!(req["token"], token);
                match req["op"].as_str().unwrap() {
                    "ping" => writeln!(
                        stream,
                        "{{\"ok\":true,\"data\":{{\"flavor\":\"mono\",\"pid\":44100,\"port\":{port}}}}}"
                    )
                    .unwrap(),
                    "get_field" => writeln!(
                        stream,
                        "{{\"ok\":true,\"data\":{{\"type\":\"UnityEngine.Time\",\"member\":\"timeScale\",\"value\":1.0}}}}"
                    )
                    .unwrap(),
                    "list_members" => writeln!(
                        stream,
                        "{{\"ok\":true,\"data\":{{\"type\":\"PlayerControlCustom\",\"fields\":[]}}}}"
                    )
                    .unwrap(),
                    "find_instances" => writeln!(
                        stream,
                        "{{\"ok\":true,\"data\":{{\"type\":\"PlayerControlCustom\",\"total\":1,\"instances\":[]}}}}"
                    )
                    .unwrap(),
                    "dump_hierarchy" => writeln!(
                        stream,
                        "{{\"ok\":true,\"data\":{{\"root\":\"AIdealBattle\",\"count\":1,\"nodes\":[]}}}}"
                    )
                    .unwrap(),
                    other => panic!("unexpected op {other}"),
                }
            }
        });

        let ping =
            probe_nekod_unity_bridge_readonly(44100, Some(&dir_str), "ping", json!({})).unwrap();
        assert_eq!(ping["flavor"], "mono");

        let field = probe_nekod_unity_bridge_readonly(
            44100,
            Some(&dir_str),
            "get_field",
            json!({ "class_name": "UnityEngine.Time", "field_name": "timeScale" }),
        )
        .unwrap();
        assert_eq!(field["value"], 1.0);

        let members = probe_nekod_unity_bridge_readonly(
            44100,
            Some(&dir_str),
            "list_members",
            json!({ "class_name": "PlayerControlCustom" }),
        )
        .unwrap();
        assert_eq!(members["type"], "PlayerControlCustom");

        let instances = probe_nekod_unity_bridge_readonly(
            44100,
            Some(&dir_str),
            "find_instances",
            json!({ "class_name": "PlayerControlCustom" }),
        )
        .unwrap();
        assert_eq!(instances["total"], 1);

        let hier = probe_nekod_unity_bridge_readonly(
            44100,
            Some(&dir_str),
            "dump_hierarchy",
            json!({ "class_name": "PlayerControlCustom" }),
        )
        .unwrap();
        assert_eq!(hier["root"], "AIdealBattle");

        server.join().unwrap();
        let _ = fs::remove_dir_all(discovery_dir);
    }

    #[test]
    fn auto_discover_missing_file_reports_bridge_not_ready_instead_of_invalid_token() {
        use std::time::{SystemTime, UNIX_EPOCH};

        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let discovery_dir = std::env::temp_dir().join(format!("nekod_missing_bridge_{nanos}"));
        fs::create_dir_all(&discovery_dir).unwrap();

        let runtime = UnityBridgeRuntime::new();
        let mut req = unity_request(UnityFlavor::Mono, UnityLoaderKind::BepinexMono, 19540);
        req.target_process.pid = 58692;
        req.declarative_payload_json = Some(
            serde_json::json!({
                "schema_version": "1.0.0",
                "protocol": "nekod_json_line_v1",
                "flavor": "mono",
                "loader": "bepinex_mono",
                "host": "127.0.0.1",
                "port": 19540,
                "auto_discover_by_pid": true,
                "discovery_dir": discovery_dir.display().to_string(),
                "features": [{
                    "kind": "toggle_field",
                    "feature_id": "infinite-stamina",
                    "class_name": "PlayerController",
                    "field_name": "drainRate",
                    "enabled_value": 0.0,
                    "disabled_value": 1.0
                }]
            })
            .to_string(),
        );

        let err = runtime.start_session(req).unwrap_err();
        match err {
            RuntimeError::ExecutionFailed(msg) => {
                assert!(msg.contains("not ready"), "unexpected msg: {msg}");
                assert!(msg.contains("unity-58692.json"), "unexpected msg: {msg}");
            }
            other => panic!("expected ExecutionFailed for missing discovery record, got {other:?}"),
        }

        let _ = fs::remove_dir_all(discovery_dir);
    }

    #[test]
    fn diagnostics_does_not_block_while_apply_feature_rpc_is_in_flight() {
        use std::sync::mpsc;
        use std::time::{Duration, Instant};

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let (in_flight_tx, in_flight_rx) = mpsc::channel();

        let server = thread::spawn(move || {
            for (idx, stream) in listener.incoming().take(3).enumerate() {
                let mut stream = stream.unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                let req: Value = serde_json::from_str(&line).unwrap();
                let op = req["op"].as_str().unwrap();
                if op == "set_field" && idx == 1 {
                    let _ = in_flight_tx.send(());
                    thread::sleep(Duration::from_millis(450));
                }
                if op == "ping" {
                    writeln!(
                        stream,
                        "{{\"ok\":true,\"data\":{{\"flavor\":\"mono\",\"pid\":5200,\"port\":{port}}}}}"
                    )
                    .unwrap();
                } else {
                    writeln!(stream, "{{\"ok\":true,\"data\":{{\"updated\":1}}}}").unwrap();
                }
            }
        });

        let runtime = Arc::new(UnityBridgeRuntime::new());
        let req = unity_request(UnityFlavor::Mono, UnityLoaderKind::BepinexMono, port);
        let sid = runtime.start_session(req).unwrap();

        let rt_clone = Arc::clone(&runtime);
        let sid_clone = sid.clone();
        let apply_handle = thread::spawn(move || {
            rt_clone.apply_feature(
                &sid_clone,
                FeatureCommand {
                    feature_id: "infinite-stamina".to_owned(),
                    value: FeatureValue::Toggle(true),
                },
            )
        });

        // Wait until the slow set_field RPC is actively in-flight on the server.
        in_flight_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("set_field request did not arrive");

        let started = Instant::now();
        let diag = runtime.diagnostics(&sid).unwrap();
        let elapsed = started.elapsed();
        assert!(
            elapsed < Duration::from_millis(150),
            "diagnostics blocked for {:?} while apply_feature was in-flight",
            elapsed
        );
        assert_eq!(diag.session_id, sid);

        apply_handle.join().unwrap().unwrap();
        runtime.stop_session(&sid).unwrap();
        server.join().unwrap();
    }

    #[test]
    fn stop_and_rollback_aborts_early_when_bridge_port_is_closed() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();

        let server = thread::spawn(move || {
            for stream in listener.incoming().take(3) {
                let mut stream = stream.unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                let req: Value = serde_json::from_str(&line).unwrap();
                let op = req["op"].as_str().unwrap();
                if op == "ping" {
                    writeln!(
                        stream,
                        "{{\"ok\":true,\"data\":{{\"flavor\":\"mono\",\"pid\":5200,\"port\":{port}}}}}"
                    )
                    .unwrap();
                } else {
                    writeln!(stream, "{{\"ok\":true,\"data\":{{\"updated\":1}}}}").unwrap();
                }
            }
            // Drops listener here -> simulates game closing before rollback.
        });

        let runtime = UnityBridgeRuntime::new();
        let mut req = unity_request(UnityFlavor::Mono, UnityLoaderKind::BepinexMono, port);
        req.declarative_payload_json = Some(
            serde_json::json!({
                "schema_version": "1.0.0",
                "flavor": "mono",
                "loader": "bepinex_mono",
                "host": "127.0.0.1",
                "port": port,
                "auth_token": "session_token_0123456789abcdef",
                "features": [
                    {
                        "kind": "toggle_field",
                        "feature_id": "feat-1",
                        "class_name": "PlayerController",
                        "field_name": "isImmortal",
                        "enabled_value": true,
                        "disabled_value": false
                    },
                    {
                        "kind": "toggle_field",
                        "feature_id": "feat-2",
                        "class_name": "PlayerController",
                        "field_name": "staminaRecovery",
                        "enabled_value": 999.0,
                        "disabled_value": 1.0
                    }
                ]
            })
            .to_string(),
        );

        let sid = runtime.start_session(req).unwrap();
        runtime
            .apply_feature(
                &sid,
                FeatureCommand {
                    feature_id: "feat-1".to_owned(),
                    value: FeatureValue::Toggle(true),
                },
            )
            .unwrap();
        runtime
            .apply_feature(
                &sid,
                FeatureCommand {
                    feature_id: "feat-2".to_owned(),
                    value: FeatureValue::Toggle(true),
                },
            )
            .unwrap();

        server.join().unwrap();

        let diag = runtime.stop_session(&sid).unwrap();
        assert_eq!(diag.rollback_failures, 2);
        assert!(
            diag.recent_events
                .iter()
                .any(|e| e.contains("aborted early") && e.contains("skipped 1")),
            "expected early abort diagnostic, got {:?}",
            diag.recent_events
        );
    }

    #[test]
    fn toggle_field_with_persist_and_extra_targets_pins_and_rolls_back_lifo() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let seen = Arc::new(Mutex::new(Vec::<(String, Value)>::new()));
        let seen_srv = Arc::clone(&seen);

        // ping + 3 pin_field + 3 unpin_field rollback requests = 7
        let server = thread::spawn(move || {
            for stream in listener.incoming().take(7) {
                let mut stream = stream.unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                let req: Value = serde_json::from_str(&line).unwrap();
                let op = req["op"].as_str().unwrap().to_owned();
                if op == "ping" {
                    writeln!(
                        stream,
                        "{{\"ok\":true,\"data\":{{\"flavor\":\"mono\",\"pid\":5200,\"port\":{port}}}}}"
                    )
                    .unwrap();
                } else {
                    seen_srv.lock().unwrap().push((op, req["params"].clone()));
                    writeln!(stream, "{{\"ok\":true,\"data\":{{\"updated\":1}}}}").unwrap();
                }
            }
        });

        let runtime = UnityBridgeRuntime::new();
        let mut req = unity_request(UnityFlavor::Mono, UnityLoaderKind::BepinexMono, port);
        req.declarative_payload_json = Some(
            serde_json::json!({
                "schema_version": "1.0.0",
                "flavor": "mono",
                "loader": "bepinex_mono",
                "host": "127.0.0.1",
                "port": port,
                "auth_token": "session_token_0123456789abcdef",
                "features": [{
                    "kind": "toggle_field",
                    "feature_id": "infinite-ammo",
                    "class_name": "Invector.vShooter.vShooterManager",
                    "field_name": "AllAmmoInfinity",
                    "enabled_value": true,
                    "disabled_value": false,
                    "persist": true,
                    "extra_targets": [
                        {
                            "class_name": "Invector.vShooter.vShooterManager",
                            "field_name": "rWeapon.isInfinityAmmo",
                            "enabled_value": true,
                            "disabled_value": false
                        },
                        {
                            "class_name": "Invector.vShooter.vShooterManager",
                            "field_name": "rWeapon.dontUseReload",
                            "enabled_value": true,
                            "disabled_value": false
                        }
                    ]
                }]
            })
            .to_string(),
        );

        let sid = runtime.start_session(req).unwrap();
        runtime
            .apply_feature(
                &sid,
                FeatureCommand {
                    feature_id: "infinite-ammo".to_owned(),
                    value: FeatureValue::Toggle(true),
                },
            )
            .unwrap();
        let diag = runtime.stop_session(&sid).unwrap();
        assert_eq!(diag.rollback_failures, 0);
        server.join().unwrap();

        let ops = seen.lock().unwrap();
        assert_eq!(ops.len(), 6);
        assert_eq!(ops[0].0, "pin_field");
        assert_eq!(ops[0].1["id"], "infinite-ammo:0");
        assert_eq!(ops[1].0, "pin_field");
        assert_eq!(ops[1].1["id"], "infinite-ammo:1");
        assert_eq!(ops[1].1["field_name"], "rWeapon.isInfinityAmmo");
        assert_eq!(ops[2].0, "pin_field");
        assert_eq!(ops[2].1["id"], "infinite-ammo:2");
        assert_eq!(ops[2].1["field_name"], "rWeapon.dontUseReload");

        // LIFO rollback restores 2 -> 1 -> 0
        assert_eq!(ops[3].0, "unpin_field");
        assert_eq!(ops[3].1["id"], "infinite-ammo:2");
        assert_eq!(ops[3].1["restore_value"], false);
        assert_eq!(ops[4].0, "unpin_field");
        assert_eq!(ops[4].1["id"], "infinite-ammo:1");
        assert_eq!(ops[5].0, "unpin_field");
        assert_eq!(ops[5].1["id"], "infinite-ammo:0");
    }

    #[test]
    fn set_field_with_scale_and_reciprocal_transforms_and_rejects_non_positive() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let seen = Arc::new(Mutex::new(Vec::<(String, Value)>::new()));
        let seen_srv = Arc::clone(&seen);

        // ping + 3 (player-damage) + 2 (attack-speed) + 3 unpin + 2 unpin? wait:
        // player-damage (3 pin) + attack-speed (2 pin) + stop rollback (5 unpin) = 11 total
        let server = thread::spawn(move || {
            for stream in listener.incoming().take(11) {
                let mut stream = stream.unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                let req: Value = serde_json::from_str(&line).unwrap();
                let op = req["op"].as_str().unwrap().to_owned();
                if op == "ping" {
                    writeln!(
                        stream,
                        "{{\"ok\":true,\"data\":{{\"flavor\":\"mono\",\"pid\":5200,\"port\":{port}}}}}"
                    )
                    .unwrap();
                } else {
                    seen_srv.lock().unwrap().push((op, req["params"].clone()));
                    writeln!(stream, "{{\"ok\":true,\"data\":{{\"updated\":1}}}}").unwrap();
                }
            }
        });

        let runtime = UnityBridgeRuntime::new();
        let mut req = unity_request(UnityFlavor::Mono, UnityLoaderKind::BepinexMono, port);
        req.declarative_payload_json = Some(
            serde_json::json!({
                "schema_version": "1.0.0",
                "flavor": "mono",
                "loader": "bepinex_mono",
                "host": "127.0.0.1",
                "port": port,
                "auth_token": "session_token_0123456789abcdef",
                "features": [
                    {
                        "kind": "set_field",
                        "feature_id": "player-damage",
                        "class_name": "Invector.vShooter.vShooterManager",
                        "field_name": "rWeapon.maxDamage",
                        "persist": true,
                        "extra_targets": [
                            {
                                "class_name": "Invector.vShooter.vShooterManager",
                                "field_name": "rWeapon.minDamage",
                                "scale": 0.75
                            },
                            {
                                "class_name": "GunSoulGirl2.GS2_CharacterControllerStat",
                                "field_name": "Damage"
                            }
                        ]
                    },
                    {
                        "kind": "set_field",
                        "feature_id": "attack-speed",
                        "class_name": "Invector.vShooter.vShooterManager",
                        "field_name": "rWeapon.shootFrequency",
                        "persist": true,
                        "reciprocal": true,
                        "extra_targets": [
                            {
                                "class_name": "GunSoulGirl2.GS2_CharacterControllerStat",
                                "field_name": "AtkSpd"
                            }
                        ]
                    }
                ]
            })
            .to_string(),
        );

        let sid = runtime.start_session(req).unwrap();

        let invalid = runtime
            .apply_feature(
                &sid,
                FeatureCommand {
                    feature_id: "attack-speed".to_owned(),
                    value: FeatureValue::Float(0.0),
                },
            )
            .unwrap_err();
        assert!(matches!(invalid, RuntimeError::InvalidPayload(_)));

        runtime
            .apply_feature(
                &sid,
                FeatureCommand {
                    feature_id: "player-damage".to_owned(),
                    value: FeatureValue::Float(4000.0),
                },
            )
            .unwrap();
        runtime
            .apply_feature(
                &sid,
                FeatureCommand {
                    feature_id: "attack-speed".to_owned(),
                    value: FeatureValue::Float(10.0),
                },
            )
            .unwrap();

        let diag = runtime.stop_session(&sid).unwrap();
        assert_eq!(diag.rollback_failures, 0);
        server.join().unwrap();

        let ops = seen.lock().unwrap();
        assert_eq!(ops.len(), 10);
        assert_eq!(ops[0].1["value"], 4000.0);
        assert_eq!(ops[1].1["field_name"], "rWeapon.minDamage");
        assert_eq!(ops[1].1["value"], 3000.0);
        assert_eq!(ops[2].1["field_name"], "Damage");
        assert_eq!(ops[2].1["value"], 4000.0);
        assert_eq!(ops[3].1["field_name"], "rWeapon.shootFrequency");
        assert!((ops[3].1["value"].as_f64().unwrap() - 0.1).abs() < 1e-9);
        assert_eq!(ops[4].1["field_name"], "AtkSpd");
        assert_eq!(ops[4].1["value"], 10.0);
        assert_eq!(ops[5].0, "unpin_field");
    }

    #[test]
    fn persist_feature_reports_upgrade_hint_on_legacy_bridge() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();

        let server = thread::spawn(move || {
            for stream in listener.incoming().take(2) {
                let mut stream = stream.unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                let req: Value = serde_json::from_str(&line).unwrap();
                let op = req["op"].as_str().unwrap();
                if op == "ping" {
                    writeln!(
                        stream,
                        "{{\"ok\":true,\"data\":{{\"flavor\":\"mono\",\"pid\":5200,\"port\":{port}}}}}"
                    )
                    .unwrap();
                } else {
                    writeln!(
                        stream,
                        "{{\"ok\":false,\"error\":\"unsupported op: {op}\"}}"
                    )
                    .unwrap();
                }
            }
        });

        let runtime = UnityBridgeRuntime::new();
        let mut req = unity_request(UnityFlavor::Mono, UnityLoaderKind::BepinexMono, port);
        req.declarative_payload_json = Some(
            serde_json::json!({
                "schema_version": "1.0.0",
                "flavor": "mono",
                "loader": "bepinex_mono",
                "host": "127.0.0.1",
                "port": port,
                "auth_token": "session_token_0123456789abcdef",
                "features": [{
                    "kind": "toggle_field",
                    "feature_id": "god-mode",
                    "class_name": "PlayerController",
                    "field_name": "isImmortal",
                    "enabled_value": true,
                    "disabled_value": false,
                    "persist": true
                }]
            })
            .to_string(),
        );

        let sid = runtime.start_session(req).unwrap();
        let err = runtime
            .apply_feature(
                &sid,
                FeatureCommand {
                    feature_id: "god-mode".to_owned(),
                    value: FeatureValue::Toggle(true),
                },
            )
            .unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("v0.3.0") && msg.contains("outdated"),
            "got: {msg}"
        );

        runtime.stop_session(&sid).unwrap();
        server.join().unwrap();
    }

    #[test]
    fn target_lock_and_damage_aura_pin_and_roll_back_lifo() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let seen = Arc::new(Mutex::new(Vec::<(String, Value)>::new()));
        let seen_srv = Arc::clone(&seen);

        // ping + pin_target_lock + pin_damage_aura + unpin_damage_aura + unpin_target_lock = 5
        let server = thread::spawn(move || {
            for stream in listener.incoming().take(5) {
                let mut stream = stream.unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                let req: Value = serde_json::from_str(&line).unwrap();
                let op = req["op"].as_str().unwrap().to_owned();
                if op == "ping" {
                    writeln!(
                        stream,
                        "{{\"ok\":true,\"data\":{{\"flavor\":\"mono\",\"pid\":5200,\"port\":{port}}}}}"
                    )
                    .unwrap();
                } else {
                    seen_srv.lock().unwrap().push((op, req["params"].clone()));
                    writeln!(stream, "{{\"ok\":true,\"data\":{{\"pinned\":true}}}}").unwrap();
                }
            }
        });

        let runtime = UnityBridgeRuntime::new();
        let mut req = unity_request(UnityFlavor::Mono, UnityLoaderKind::BepinexMono, port);
        req.declarative_payload_json = Some(
            serde_json::json!({
                "schema_version": "1.0.0",
                "flavor": "mono",
                "loader": "bepinex_mono",
                "host": "127.0.0.1",
                "port": port,
                "auth_token": "session_token_0123456789abcdef",
                "features": [
                    {
                        "kind": "target_lock",
                        "feature_id": "auto-lock-enemy",
                        "player_class": "Invector.vCharacterController.vThirdPersonController",
                        "camera_class": "Invector.vCamera.vThirdPersonCamera",
                        "target_class": "GunSoulGirl2.GS2_EnemyStat",
                        "alive_checks": [
                            {
                                "component_class": "Invector.vHealthController",
                                "dead_field": "isDead",
                                "health_field": "currentHealth"
                            }
                        ],
                        "max_range": 55.0,
                        "lock_speed": 12.0,
                        "interval_ms": 60
                    },
                    {
                        "kind": "damage_aura",
                        "feature_id": "kill-aura",
                        "player_class": "Invector.vCharacterController.vThirdPersonController",
                        "target_class": "GunSoulGirl2.GS2_EnemyStat",
                        "damage_amount": 999999,
                        "radius": 30.0,
                        "interval_ms": 250,
                        "handlers": [
                            {
                                "kind": "invector_health",
                                "component_class": "Invector.vHealthController",
                                "damage_type_class": "Invector.vDamage",
                                "method_name": "TakeDamage",
                                "dead_field": "isDead",
                                "health_field": "currentHealth"
                            },
                            {
                                "kind": "emerald_ai",
                                "component_class": "EmeraldAI.EmeraldAISystem",
                                "method_name": "Damage",
                                "dead_field": "IsDead",
                                "health_field": "CurrentHealth"
                            }
                        ]
                    }
                ]
            })
            .to_string(),
        );

        let sid = runtime.start_session(req).unwrap();
        runtime
            .apply_feature(
                &sid,
                FeatureCommand {
                    feature_id: "auto-lock-enemy".to_owned(),
                    value: FeatureValue::Toggle(true),
                },
            )
            .unwrap();
        runtime
            .apply_feature(
                &sid,
                FeatureCommand {
                    feature_id: "kill-aura".to_owned(),
                    value: FeatureValue::Toggle(true),
                },
            )
            .unwrap();

        let diag = runtime.stop_session(&sid).unwrap();
        assert_eq!(diag.rollback_failures, 0);
        server.join().unwrap();

        let ops = seen.lock().unwrap();
        assert_eq!(ops.len(), 4);
        assert_eq!(ops[0].0, "pin_target_lock");
        assert_eq!(ops[0].1["id"], "auto-lock-enemy");
        assert_eq!(ops[0].1["lock_method"], "SetLockTarget");
        assert_eq!(ops[0].1["unlock_method"], "RemoveLockTarget");
        assert_eq!(ops[0].1["max_range"], 55.0);
        assert_eq!(ops[1].0, "pin_damage_aura");
        assert_eq!(ops[1].1["id"], "kill-aura");
        assert_eq!(ops[1].1["damage_amount"], 999999);
        assert_eq!(ops[2].0, "unpin_damage_aura");
        assert_eq!(ops[2].1["id"], "kill-aura");
        assert_eq!(ops[3].0, "unpin_target_lock");
        assert_eq!(ops[3].1["id"], "auto-lock-enemy");
    }

    #[test]
    fn rejects_invalid_damage_aura_handler_kind_and_zero_radius() {
        let runtime = UnityBridgeRuntime::new();

        let bad_kind: UnityBridgePayload = serde_json::from_value(serde_json::json!({
            "schema_version": "1.0.0",
            "flavor": "mono",
            "loader": "bepinex_mono",
            "host": "127.0.0.1",
            "port": 19540,
            "features": [{
                "kind": "damage_aura",
                "feature_id": "kill-aura",
                "player_class": "Player",
                "target_class": "Enemy",
                "damage_amount": 1000,
                "radius": 20.0,
                "handlers": [{
                    "kind": "arbitrary_csharp",
                    "component_class": "Enemy",
                    "method_name": "Kill"
                }]
            }]
        }))
        .unwrap();
        let err = UnityBridgeRuntime::validate_payload(&bad_kind).unwrap_err();
        assert!(matches!(err, RuntimeError::InvalidPayload(_)));

        let bad_radius: UnityBridgePayload = serde_json::from_value(serde_json::json!({
            "schema_version": "1.0.0",
            "flavor": "mono",
            "loader": "bepinex_mono",
            "host": "127.0.0.1",
            "port": 19540,
            "features": [{
                "kind": "damage_aura",
                "feature_id": "kill-aura",
                "player_class": "Player",
                "target_class": "Enemy",
                "damage_amount": 1000,
                "radius": 0.0,
                "handlers": [{
                    "kind": "emerald_ai",
                    "component_class": "EmeraldAI.EmeraldAISystem",
                    "method_name": "Damage"
                }]
            }]
        }))
        .unwrap();
        let err2 = UnityBridgeRuntime::validate_payload(&bad_radius).unwrap_err();
        assert!(matches!(err2, RuntimeError::InvalidPayload(_)));
        let _ = runtime;
    }

    #[test]
    fn spin_bot_game_object_copy_field_invoke_args_and_verified_material_swap_roll_back_lifo() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let seen = Arc::new(Mutex::new(Vec::<(String, Value)>::new()));
        let seen_srv = Arc::clone(&seen);

        // ping + 6 feature applications + 4 rollback calls = 11 RPCs
        let server = thread::spawn(move || {
            for stream in listener.incoming().take(11) {
                let mut stream = stream.unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                let req: Value = serde_json::from_str(&line).unwrap();
                let op = req["op"].as_str().unwrap().to_owned();
                if op == "ping" {
                    writeln!(
                        stream,
                        "{{\"ok\":true,\"data\":{{\"flavor\":\"il2cpp\",\"pid\":5200,\"port\":{port}}}}}"
                    )
                    .unwrap();
                } else {
                    seen_srv.lock().unwrap().push((op, req["params"].clone()));
                    writeln!(stream, "{{\"ok\":true,\"data\":{{\"ok\":true}}}}").unwrap();
                }
            }
        });

        let verified_png = "D:\\safe_pkg\\assets\\uncensor_placeholder.png".to_owned();
        let mut req = unity_request(UnityFlavor::Il2cpp, UnityLoaderKind::BepinexIl2cpp, port);
        req.verified_package_assets.insert(
            "assets/uncensor_placeholder.png".to_owned(),
            verified_png.clone(),
        );
        req.declarative_payload_json = Some(
            serde_json::json!({
                "schema_version": "1.0.0",
                "flavor": "il2cpp",
                "loader": "bepinex_il2cpp",
                "host": "127.0.0.1",
                "port": port,
                "auth_token": "session_token_0123456789abcdef",
                "features": [
                    {
                        "kind": "spin_bot",
                        "feature_id": "spinbot-armature",
                        "player_class": "PlayerControlCustom",
                        "require_active_class": "BattleManager",
                        "spin_node_path": "Armature",
                        "spin_local": true,
                        "late_update": true,
                        "roll_speed_deg": 720.0
                    },
                    {
                        "kind": "set_game_object_active",
                        "feature_id": "undress-male",
                        "game_object_names": ["Bottoms", "Tops"],
                        "enabled_active": false,
                        "disabled_active": true,
                        "include_inactive": true
                    },
                    {
                        "kind": "copy_field",
                        "feature_id": "refill-hp",
                        "from_class": "PlayerControlCustom",
                        "from_field": "HitpointMax",
                        "to_class": "PlayerControlCustom",
                        "to_field": "Hitpoint"
                    },
                    {
                        "kind": "invoke_method",
                        "feature_id": "costume-naked",
                        "class_name": "EventManager",
                        "method_name": "ChangeCostume",
                        "args": [1]
                    },
                    {
                        "kind": "damage_aura",
                        "feature_id": "kill-aura",
                        "player_class": "PlayerControlCustom",
                        "target_class": "CharacterControllerBase",
                        "require_active_class": "BattleManager",
                        "exclude_bool_field": "IsPlayer",
                        "damage_amount": 99999,
                        "radius": 60.0,
                        "interval_ms": 250,
                        "handlers": [{
                            "kind": "call",
                            "component_class": "CharacterControllerBase",
                            "method_name": "Die",
                            "health_field": "Hitpoint",
                            "dead_method": "IsDie",
                            "zero_health_before_call": true,
                            "args": []
                        }]
                    },
                    {
                        "kind": "replace_texture_material",
                        "feature_id": "uncensor-material",
                        "material_name_prefix": "VagWhite",
                        "asset_path": "assets/uncensor_placeholder.png",
                        "shader_name": "Standard",
                        "color": [0.94, 0.8, 0.9, 1.0]
                    }
                ]
            })
            .to_string(),
        );

        let runtime = UnityBridgeRuntime::new();
        let sid = runtime.start_session(req).unwrap();
        runtime
            .apply_feature(
                &sid,
                FeatureCommand {
                    feature_id: "spinbot-armature".to_owned(),
                    value: FeatureValue::Toggle(true),
                },
            )
            .unwrap();
        runtime
            .apply_feature(
                &sid,
                FeatureCommand {
                    feature_id: "undress-male".to_owned(),
                    value: FeatureValue::Toggle(true),
                },
            )
            .unwrap();
        runtime
            .apply_feature(
                &sid,
                FeatureCommand {
                    feature_id: "refill-hp".to_owned(),
                    value: FeatureValue::Action,
                },
            )
            .unwrap();
        runtime
            .apply_feature(
                &sid,
                FeatureCommand {
                    feature_id: "costume-naked".to_owned(),
                    value: FeatureValue::Action,
                },
            )
            .unwrap();
        runtime
            .apply_feature(
                &sid,
                FeatureCommand {
                    feature_id: "kill-aura".to_owned(),
                    value: FeatureValue::Toggle(true),
                },
            )
            .unwrap();
        runtime
            .apply_feature(
                &sid,
                FeatureCommand {
                    feature_id: "uncensor-material".to_owned(),
                    value: FeatureValue::Toggle(true),
                },
            )
            .unwrap();

        let diag = runtime.stop_session(&sid).unwrap();
        assert_eq!(diag.rollback_failures, 0);
        server.join().unwrap();

        let ops = seen.lock().unwrap();
        assert_eq!(ops.len(), 10);
        assert_eq!(ops[0].0, "pin_spinbot");
        assert_eq!(ops[0].1["spin_node_path"], "Armature");
        assert_eq!(ops[0].1["require_active_class"], "BattleManager");
        assert_eq!(ops[0].1["late_update"], true);
        assert_eq!(ops[1].0, "set_gameobject_active");
        assert_eq!(ops[1].1["active"], false);
        assert_eq!(ops[2].0, "copy_field");
        assert_eq!(ops[3].0, "invoke_method_args");
        assert_eq!(ops[3].1["args"], json!([1]));
        assert_eq!(ops[4].0, "pin_damage_aura");
        assert_eq!(ops[4].1["require_active_class"], "BattleManager");
        assert_eq!(ops[4].1["exclude_bool_field"], "IsPlayer");
        assert_eq!(ops[4].1["handlers"][0]["dead_method"], "IsDie");
        assert_eq!(ops[4].1["handlers"][0]["zero_health_before_call"], true);
        assert_eq!(ops[4].1["handlers"][0]["args"], json!([]));
        assert_eq!(ops[5].0, "replace_texture_material");
        assert_eq!(ops[5].1["texture_path"], verified_png);
        assert_eq!(ops[5].1["shader_name"], "Standard");

        // LIFO rollback restores in reverse order: material -> aura -> GameObject -> spinbot
        assert_eq!(ops[6].0, "restore_texture_material");
        assert_eq!(ops[6].1["id"], "mat:uncensor-material");
        assert_eq!(ops[7].0, "unpin_damage_aura");
        assert_eq!(ops[8].0, "set_gameobject_active");
        assert_eq!(ops[8].1["active"], true);
        assert_eq!(ops[9].0, "unpin_spinbot");
    }

    #[test]
    fn replace_texture_material_rejects_unverified_or_unsafe_asset_paths() {
        for bad_asset in [
            "../secret.png",
            "C:\\Windows\\system32\\test.png",
            "assets/payload.dll",
        ] {
            let payload: UnityBridgePayload = serde_json::from_value(serde_json::json!({
                "schema_version": "1.0.0",
                "flavor": "il2cpp",
                "loader": "bepinex_il2cpp",
                "host": "127.0.0.1",
                "port": 19540,
                "features": [{
                    "kind": "replace_texture_material",
                    "feature_id": "bad-mat",
                    "material_name_prefix": "VagWhite",
                    "asset_path": bad_asset
                }]
            }))
            .unwrap();
            let err = UnityBridgeRuntime::validate_payload(&payload).unwrap_err();
            assert!(
                matches!(err, RuntimeError::InvalidPayload(_)),
                "expected validation failure for '{bad_asset}'"
            );
        }

        // Valid relative path NOT present in verified_package_assets must fail closed at runtime.
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = thread::spawn(move || {
            for stream in listener.incoming().take(1) {
                let mut stream = stream.unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                writeln!(
                    stream,
                    "{{\"ok\":true,\"data\":{{\"flavor\":\"il2cpp\",\"pid\":5200,\"port\":{port}}}}}"
                )
                .unwrap();
            }
        });

        let runtime = UnityBridgeRuntime::new();
        let mut req = unity_request(UnityFlavor::Il2cpp, UnityLoaderKind::BepinexIl2cpp, port);
        // Note: verified_package_assets intentionally omitted for "assets/missing.png"!
        req.declarative_payload_json = Some(
            serde_json::json!({
                "schema_version": "1.0.0",
                "flavor": "il2cpp",
                "loader": "bepinex_il2cpp",
                "host": "127.0.0.1",
                "port": port,
                "auth_token": "session_token_0123456789abcdef",
                "features": [{
                    "kind": "replace_texture_material",
                    "feature_id": "missing-asset",
                    "material_name_prefix": "VagWhite",
                    "asset_path": "assets/missing.png"
                }]
            })
            .to_string(),
        );

        let sid = runtime.start_session(req).unwrap();
        let err = runtime
            .apply_feature(
                &sid,
                FeatureCommand {
                    feature_id: "missing-asset".to_owned(),
                    value: FeatureValue::Toggle(true),
                },
            )
            .unwrap_err();
        assert!(matches!(err, RuntimeError::InvalidPayload(_)));
        runtime.stop_session(&sid).unwrap();
        server.join().unwrap();
    }

    #[test]
    fn unity_spinbot_generic_aura_and_texture_slot_payload_validates() {
        let payload: UnityBridgePayload = serde_json::from_value(serde_json::json!({
            "schema_version": "1.0.0",
            "flavor": "il2cpp",
            "loader": "bepinex_il2cpp",
            "host": "127.0.0.1",
            "port": 19540,
            "auto_discover_by_pid": true,
            "features": [
                {
                    "kind": "spin_bot",
                    "feature_id": "spinbot-armature",
                    "player_class": "PlayerRoot",
                    "spin_node_path": "VisualArmature",
                    "require_active_class": "BattleManager",
                    "roll_speed_deg": 720.0
                },
                {
                    "kind": "damage_aura",
                    "feature_id": "kill-aura",
                    "player_class": "PlayerActor",
                    "target_class": "EnemyBase",
                    "exclude_bool_field": "IsPlayerSide",
                    "damage_amount": 9999,
                    "radius": 35.0,
                    "interval_ms": 250,
                    "handlers": [{
                        "kind": "call",
                        "component_class": "EnemyHealth",
                        "method_name": "ApplyDamage",
                        "dead_method": "IsDead",
                        "args": [9999]
                    }]
                },
                {
                    "kind": "replace_texture_material",
                    "feature_id": "uncensor-material-slot",
                    "renderer_name_prefixes": ["SkinnedBody"],
                    "material_name_prefixes": ["SkinMat"],
                    "asset_path": "assets/slot.png"
                }
            ]
        }))
        .expect("unity advanced payload deserializes");
        UnityBridgeRuntime::validate_payload(&payload)
            .expect("unity advanced payload validates cleanly");
        assert_eq!(payload.features.len(), 3);
    }

    #[test]
    fn hook_set_field_sends_pin_and_unpin_method_hook_and_rolls_back_lifo() {
        let bad_timing: UnityBridgePayload = serde_json::from_value(serde_json::json!({
            "schema_version": "1.0.0",
            "flavor": "il2cpp",
            "loader": "bepinex_il2cpp",
            "host": "127.0.0.1",
            "port": 19540,
            "features": [{
                "kind": "hook_set_field",
                "feature_id": "playable-lily",
                "hook_class": "BattleManager",
                "hook_method": "AfterAssetbundleLoad",
                "timing": "transpiler",
                "writes": [{
                    "class_name": "ActionFlowManager",
                    "field_name": "Playable",
                    "value": "LILY"
                }]
            }]
        }))
        .unwrap();
        assert!(matches!(
            UnityBridgeRuntime::validate_payload(&bad_timing),
            Err(RuntimeError::InvalidPayload(_))
        ));

        let missing_mode: UnityBridgePayload = serde_json::from_value(serde_json::json!({
            "schema_version": "1.0.0",
            "flavor": "il2cpp",
            "loader": "bepinex_il2cpp",
            "host": "127.0.0.1",
            "port": 19540,
            "features": [{
                "kind": "hook_set_field",
                "feature_id": "playable-lily",
                "hook_class": "BattleManager",
                "hook_method": "AfterAssetbundleLoad",
                "writes": [{
                    "class_name": "BattleResource",
                    "field_name": "Lily.Selection"
                }]
            }]
        }))
        .unwrap();
        assert!(matches!(
            UnityBridgeRuntime::validate_payload(&missing_mode),
            Err(RuntimeError::InvalidPayload(_))
        ));

        let mixed_mode: UnityBridgePayload = serde_json::from_value(serde_json::json!({
            "schema_version": "1.0.0",
            "flavor": "il2cpp",
            "loader": "bepinex_il2cpp",
            "host": "127.0.0.1",
            "port": 19540,
            "features": [{
                "kind": "hook_set_field",
                "feature_id": "playable-lily",
                "hook_class": "BattleManager",
                "hook_method": "AfterAssetbundleLoad",
                "writes": [{
                    "class_name": "BattleResource",
                    "field_name": "Lily.Selection",
                    "value": "foo",
                    "ensure_component": "SelectionManager"
                }]
            }]
        }))
        .unwrap();
        assert!(matches!(
            UnityBridgeRuntime::validate_payload(&mixed_mode),
            Err(RuntimeError::InvalidPayload(_))
        ));

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let seen = Arc::new(Mutex::new(Vec::<(String, Value)>::new()));
        let seen_srv = Arc::clone(&seen);

        // ping + enable + disable + re-enable + LIFO stop rollback = 5 RPCs
        let server = thread::spawn(move || {
            for stream in listener.incoming().take(5) {
                let mut stream = stream.unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                let req: Value = serde_json::from_str(&line).unwrap();
                let op = req["op"].as_str().unwrap().to_owned();
                if op == "ping" {
                    writeln!(
                        stream,
                        "{{\"ok\":true,\"data\":{{\"flavor\":\"il2cpp\",\"pid\":5200,\"port\":{port}}}}}"
                    )
                    .unwrap();
                } else {
                    seen_srv.lock().unwrap().push((op, req["params"].clone()));
                    writeln!(stream, "{{\"ok\":true,\"data\":{{\"ok\":true}}}}").unwrap();
                }
            }
        });

        let runtime = UnityBridgeRuntime::new();
        let mut req = unity_request(UnityFlavor::Il2cpp, UnityLoaderKind::BepinexIl2cpp, port);
        req.declarative_payload_json = Some(
            serde_json::json!({
                "schema_version": "1.0.0",
                "flavor": "il2cpp",
                "loader": "bepinex_il2cpp",
                "host": "127.0.0.1",
                "port": port,
                "auth_token": "session_token_0123456789abcdef",
                "features": [{
                    "kind": "hook_set_field",
                    "feature_id": "playable-lily",
                    "hook_class": "BattleManager",
                    "hook_method": "AfterAssetbundleLoad",
                    "timing": "prefix",
                    "param_count": 0,
                    "apply_immediately": true,
                    "writes": [
                        {
                            "class_name": "ActionFlowManager",
                            "field_name": "Playable",
                            "value": "LILY",
                            "restore_value": "AI",
                            "include_inactive": true
                        },
                        {
                            "class_name": "BattleResource",
                            "field_name": "Lily.Selection",
                            "ensure_component": "SelectionManager",
                            "init_null_lists": true,
                            "only_if_null": true,
                            "include_inactive": true
                        },
                        {
                            "class_name": "BattleResource",
                            "field_name": "Lily.DashTimeline",
                            "copy_from_class": "BattleResource",
                            "copy_from_field": "AIdeal.DashTimeline",
                            "only_if_null": true,
                            "include_inactive": true,
                            "clone_under_owner": true
                        }
                    ]
                }]
            })
            .to_string(),
        );

        let sid = runtime.start_session(req).unwrap();
        runtime
            .apply_feature(
                &sid,
                FeatureCommand {
                    feature_id: "playable-lily".to_owned(),
                    value: FeatureValue::Toggle(true),
                },
            )
            .unwrap();
        runtime
            .apply_feature(
                &sid,
                FeatureCommand {
                    feature_id: "playable-lily".to_owned(),
                    value: FeatureValue::Toggle(false),
                },
            )
            .unwrap();
        runtime
            .apply_feature(
                &sid,
                FeatureCommand {
                    feature_id: "playable-lily".to_owned(),
                    value: FeatureValue::Toggle(true),
                },
            )
            .unwrap();
        runtime.stop_session(&sid).unwrap();
        server.join().unwrap();

        let ops: Vec<(String, Value)> = seen.lock().unwrap().clone();
        assert_eq!(ops.len(), 4);
        assert_eq!(ops[0].0, "pin_method_hook");
        assert_eq!(ops[0].1["id"], "hook:playable-lily");
        assert_eq!(ops[0].1["hook_class"], "BattleManager");
        assert_eq!(ops[0].1["hook_method"], "AfterAssetbundleLoad");
        assert_eq!(ops[0].1["timing"], "prefix");
        assert_eq!(ops[0].1["param_count"], 0);
        assert_eq!(ops[0].1["apply_immediately"], true);
        assert_eq!(ops[0].1["writes"][0]["class_name"], "ActionFlowManager");
        assert_eq!(ops[0].1["writes"][0]["restore_value"], "AI");
        assert_eq!(
            ops[0].1["writes"][1]["ensure_component"],
            "SelectionManager"
        );
        assert_eq!(ops[0].1["writes"][1]["init_null_lists"], true);
        assert_eq!(ops[0].1["writes"][1]["only_if_null"], true);
        assert_eq!(ops[0].1["writes"][2]["copy_from_class"], "BattleResource");
        assert_eq!(
            ops[0].1["writes"][2]["copy_from_field"],
            "AIdeal.DashTimeline"
        );
        assert_eq!(ops[0].1["writes"][2]["only_if_null"], true);
        assert_eq!(ops[0].1["writes"][2]["clone_under_owner"], true);
        assert_eq!(ops[1].0, "unpin_method_hook");
        assert_eq!(ops[1].1["id"], "hook:playable-lily");
        assert_eq!(ops[2].0, "pin_method_hook");
        assert_eq!(ops[3].0, "unpin_method_hook");
    }

    #[test]
    fn invoke_method_pre_steps_validate_and_execute_in_order() {
        let invalid_step: UnityBridgePayload = serde_json::from_value(serde_json::json!({
            "schema_version": "1.0.0",
            "flavor": "il2cpp",
            "loader": "bepinex_il2cpp",
            "host": "127.0.0.1",
            "port": 19540,
            "features": [{
                "kind": "invoke_method",
                "feature_id": "lily-unique-gun",
                "class_name": "PlayerControlCustom",
                "method_name": "AttackCommon",
                "pre_steps": [{
                    "kind": "copy_field",
                    "from_class": "PlayerControlCustom",
                    "from_field": "",
                    "to_class": "PlayerControlCustom",
                    "to_field": "CurrentAttackParam"
                }]
            }]
        }))
        .unwrap();
        assert!(matches!(
            UnityBridgeRuntime::validate_payload(&invalid_step),
            Err(RuntimeError::InvalidPayload(_))
        ));

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let seen = Arc::new(Mutex::new(Vec::<(String, Value)>::new()));
        let seen_srv = Arc::clone(&seen);

        // ping + 3 pre_steps + primary AttackCommon = 5 RPCs
        let server = thread::spawn(move || {
            for stream in listener.incoming().take(5) {
                let mut stream = stream.unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                let req: Value = serde_json::from_str(&line).unwrap();
                let op = req["op"].as_str().unwrap().to_owned();
                if op == "ping" {
                    writeln!(
                        stream,
                        "{{\"ok\":true,\"data\":{{\"flavor\":\"il2cpp\",\"pid\":5200,\"port\":{port}}}}}"
                    )
                    .unwrap();
                } else {
                    seen_srv.lock().unwrap().push((op, req["params"].clone()));
                    writeln!(stream, "{{\"ok\":true,\"data\":{{\"ok\":true}}}}").unwrap();
                }
            }
        });

        let runtime = UnityBridgeRuntime::new();
        let mut req = unity_request(UnityFlavor::Il2cpp, UnityLoaderKind::BepinexIl2cpp, port);
        req.declarative_payload_json = Some(
            serde_json::json!({
                "schema_version": "1.0.0",
                "flavor": "il2cpp",
                "loader": "bepinex_il2cpp",
                "host": "127.0.0.1",
                "port": port,
                "auth_token": "session_token_0123456789abcdef",
                "features": [{
                    "kind": "invoke_method",
                    "feature_id": "lily-unique-gun",
                    "class_name": "PlayerControlCustom",
                    "method_name": "AttackCommon",
                    "pre_steps": [
                        {
                            "kind": "set_field",
                            "class_name": "PlayerControlCustom",
                            "field_name": "Status",
                            "value": "ATTACK"
                        },
                        {
                            "kind": "invoke_method",
                            "class_name": "PlayerControlCustom",
                            "method_name": "InitActionParams"
                        },
                        {
                            "kind": "copy_field",
                            "from_class": "PlayerControlCustom",
                            "from_field": "Lily.UniqueParamG",
                            "to_class": "PlayerControlCustom",
                            "to_field": "CurrentAttackParam"
                        }
                    ]
                }]
            })
            .to_string(),
        );

        let sid = runtime.start_session(req).unwrap();
        runtime
            .apply_feature(
                &sid,
                FeatureCommand {
                    feature_id: "lily-unique-gun".to_owned(),
                    value: FeatureValue::Action,
                },
            )
            .unwrap();
        let diag = runtime.diagnostics(&sid).unwrap();
        assert!(
            diag.recent_events
                .iter()
                .any(|e| e.contains("after 3 pre_steps"))
        );
        runtime.stop_session(&sid).unwrap();
        server.join().unwrap();

        let ops: Vec<(String, Value)> = seen.lock().unwrap().clone();
        assert_eq!(ops.len(), 4);
        assert_eq!(ops[0].0, "set_field");
        assert_eq!(ops[0].1["class_name"], "PlayerControlCustom");
        assert_eq!(ops[0].1["field_name"], "Status");
        assert_eq!(ops[0].1["value"], "ATTACK");
        assert_eq!(ops[1].0, "invoke_method");
        assert_eq!(ops[1].1["method_name"], "InitActionParams");
        assert_eq!(ops[2].0, "copy_field");
        assert_eq!(ops[2].1["from_field"], "Lily.UniqueParamG");
        assert_eq!(ops[2].1["to_field"], "CurrentAttackParam");
        assert_eq!(ops[3].0, "invoke_method");
        assert_eq!(ops[3].1["class_name"], "PlayerControlCustom");
        assert_eq!(ops[3].1["method_name"], "AttackCommon");
    }

    #[test]
    fn unity_harmony_hook_and_pre_steps_invoke_payload_validates() {
        let payload: UnityBridgePayload = serde_json::from_value(serde_json::json!({
            "schema_version": "1.0.0",
            "flavor": "il2cpp",
            "loader": "bepinex_il2cpp",
            "host": "127.0.0.1",
            "port": 19540,
            "auto_discover_by_pid": true,
            "features": [
                {
                    "kind": "hook_set_field",
                    "feature_id": "character-swap",
                    "hook_class": "BattleManager",
                    "hook_method": "AfterAssetbundleLoad",
                    "timing": "prefix",
                    "writes": [
                        {
                            "class_name": "FlowManager",
                            "field_name": "Playable",
                            "value": "ALT_HERO"
                        },
                        {
                            "class_name": "BattleResource",
                            "field_name": "AltHero.UniqueController",
                            "ensure_component": "AltUniqueController",
                            "only_if_null": true
                        },
                        {
                            "class_name": "BattleResource",
                            "field_name": "AltHero.Camera",
                            "copy_from_class": "BattleResource",
                            "copy_from_field": "MainHero.Camera",
                            "clone_under_owner": true,
                            "only_if_null": true
                        }
                    ]
                },
                {
                    "kind": "invoke_method",
                    "feature_id": "special-skill",
                    "class_name": "PlayerActor",
                    "method_name": "TriggerSpecial",
                    "pre_steps": [
                        {
                            "kind": "copy_field",
                            "from_class": "BattleResource",
                            "from_field": "AltHero.SpecialParam",
                            "to_class": "PlayerActor",
                            "to_field": "ActiveSkillParam"
                        }
                    ]
                }
            ]
        }))
        .expect("harmony hook & pre_steps payload deserializes");
        UnityBridgeRuntime::validate_payload(&payload)
            .expect("harmony hook & pre_steps payload validates cleanly");
        assert_eq!(payload.features.len(), 2);
    }
}
