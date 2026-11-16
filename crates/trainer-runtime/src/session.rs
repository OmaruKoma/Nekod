//! Session lifecycle, permission enforcement, freeze scheduler, and declarative execution.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use nekod_manifest::{AntiCheatKind, PermissionScope, RuntimeKind, TrustTier, is_valid_sha256_hex};
use nekod_runtime_api::{
    FeatureCommand, FeatureValue, GrantedPermissions, ProcessIdentity, RuntimeDiagnostics,
    RuntimeError, RuntimeSessionId, SessionLifecycleState, StartSessionRequest,
};
use nekod_trainer_sdk::{
    DeclarativeOperation, DeclarativePatchPayload, MAX_FREEZE_INTERVAL_MS, MIN_FREEZE_INTERVAL_MS,
    TrainerPackageDefinition, parse_hex_bytes,
};
use nekod_validator::validate_declarative_patch_str;

use crate::aob::resolve_locator;
use crate::backend::{ProcessAccessMode, ProcessHandle, ProcessMemoryBackend};
use crate::ledger::{PatchLedger, RollbackReport};

const MAX_RECENT_EVENTS: usize = 64;

#[derive(Debug, Clone)]
pub(crate) struct FreezeEntry {
    pub feature_id: String,
    pub address: u64,
    pub bytes: Vec<u8>,
    pub interval: Duration,
    pub last_tick: Instant,
}

pub(crate) struct SessionInner {
    pub session_id: RuntimeSessionId,
    pub target_process: ProcessIdentity,
    pub granted_permissions: GrantedPermissions,
    pub payload: DeclarativePatchPayload,
    pub handle: Box<dyn ProcessHandle>,
    pub ledger: PatchLedger,
    pub state: SessionLifecycleState,
    pub freeze_entries: Vec<FreezeEntry>,
    pub recent_events: Vec<String>,
}

impl SessionInner {
    pub fn push_event(&mut self, msg: impl Into<String>) {
        if self.recent_events.len() >= MAX_RECENT_EVENTS {
            self.recent_events.remove(0);
        }
        self.recent_events.push(msg.into());
    }

    pub fn diagnostics(&self) -> RuntimeDiagnostics {
        let mut active_freeze_features: Vec<&str> = self
            .freeze_entries
            .iter()
            .map(|e| e.feature_id.as_str())
            .collect();
        active_freeze_features.sort_unstable();
        active_freeze_features.dedup();

        RuntimeDiagnostics {
            session_id: self.session_id.clone(),
            runtime_kind: RuntimeKind::DeclarativePatchV1,
            state: self.state,
            active_patches: self.ledger.len(),
            rollback_failures: self.ledger.rollback_failures(),
            rollback_conflicts: self.ledger.rollback_conflicts(),
            active_freeze_features: active_freeze_features.len(),
            target_pid: Some(self.target_process.pid),
            target_process_name: Some(self.target_process.process_name.clone()),
            recent_events: self.recent_events.clone(),
        }
    }
}

pub(crate) struct ManagedSession {
    pub inner: Arc<Mutex<SessionInner>>,
    pub stop_flag: Arc<AtomicBool>,
    pub freeze_thread: Option<JoinHandle<()>>,
}

/// Validates mandatory L3 preflight security invariants for an explicit `expected_kind`
/// before opening any process handle or attaching an adapter.
pub fn validate_session_preflight_for_kind(
    req: &StartSessionRequest,
    expected_kind: RuntimeKind,
) -> Result<(), RuntimeError> {
    req.trainer
        .validate_semantic()
        .map_err(|e| RuntimeError::GrantMismatch(e.to_string()))?;

    if req.granted_permissions.trainer_id != req.trainer.id
        || req.granted_permissions.trainer_version != req.trainer.version.to_string()
        || !is_valid_sha256_hex(&req.granted_permissions.manifest_sha256)
    {
        return Err(RuntimeError::GrantMismatch(
            "granted permissions do not match trainer identity, version, or digest".to_owned(),
        ));
    }

    if req.trainer.runtime.kind != expected_kind {
        return Err(RuntimeError::UntrustedRuntimeBlocked(
            req.trainer.runtime.kind,
            req.granted_permissions.trust_tier,
        ));
    }

    if req.granted_permissions.trust_tier == TrustTier::Unverified
        && req
            .granted_permissions
            .allows(PermissionScope::RuntimeNativeModule)
    {
        return Err(RuntimeError::UntrustedRuntimeBlocked(
            RuntimeKind::NativeProviderV1,
            TrustTier::Unverified,
        ));
    }

    if !req
        .trainer
        .runtime
        .supported_platforms
        .contains(&req.target_process.platform)
    {
        return Err(RuntimeError::UnsupportedPlatform(
            req.target_process.platform,
        ));
    }

    if !req
        .granted_permissions
        .allows(PermissionScope::ProcessDiscover)
    {
        return Err(RuntimeError::PermissionDenied(
            PermissionScope::ProcessDiscover,
        ));
    }

    if !req.trainer.compatibility.singleplayer_only {
        return Err(RuntimeError::AntiCheatDetected(
            "non-singleplayer trainers are prohibited".to_owned(),
        ));
    }

    Ok(())
}

/// Validates mandatory L3 preflight security invariants before opening any process handle.
pub fn validate_session_preflight(req: &StartSessionRequest) -> Result<(), RuntimeError> {
    validate_session_preflight_for_kind(req, RuntimeKind::DeclarativePatchV1)
}

/// Checks active processes against known kernel/user-mode anti-cheat services.
pub fn check_running_anti_cheat(
    backend: &dyn ProcessMemoryBackend,
    declared_incompatible: &[AntiCheatKind],
) -> Result<(), RuntimeError> {
    if declared_incompatible.is_empty() {
        return Ok(());
    }

    let procs = backend.enumerate_processes()?;
    for proc_info in procs {
        let lower = proc_info.process_name.to_ascii_lowercase();
        for kind in declared_incompatible {
            if matches_anti_cheat_process(&lower, *kind) {
                return Err(RuntimeError::AntiCheatDetected(format!(
                    "detected incompatible anti-cheat process '{}' ({kind:?}); refusing to attach",
                    proc_info.process_name
                )));
            }
        }
    }
    Ok(())
}

fn matches_anti_cheat_process(process_lower: &str, kind: AntiCheatKind) -> bool {
    match kind {
        AntiCheatKind::EasyAntiCheat => {
            process_lower.contains("easyanticheat") || process_lower == "eac.exe"
        }
        AntiCheatKind::Battleye => {
            process_lower.starts_with("beservice")
                || process_lower.starts_with("bedaisy")
                || process_lower.contains("battleye")
        }
        AntiCheatKind::Vanguard => {
            process_lower == "vgc.exe"
                || process_lower == "vgtray.exe"
                || process_lower.contains("vanguard")
        }
        AntiCheatKind::Vac => process_lower.contains("vac"),
        AntiCheatKind::DenuvoAntiCheat => process_lower.contains("denuvo"),
        AntiCheatKind::Custom => false,
    }
}

pub(crate) fn parse_and_verify_payload(
    req: &StartSessionRequest,
) -> Result<DeclarativePatchPayload, RuntimeError> {
    let raw = req.declarative_payload_json.as_deref().ok_or_else(|| {
        RuntimeError::InvalidPayload(
            "StartSessionRequest missing declarative_payload_json".to_owned(),
        )
    })?;

    let payload = validate_declarative_patch_str(raw)
        .map_err(|e| RuntimeError::InvalidPayload(e.to_string()))?;

    let pkg = TrainerPackageDefinition {
        manifest: req.trainer.clone(),
        payload: payload.clone(),
    };
    pkg.validate()
        .map_err(|e| RuntimeError::InvalidPayload(e.to_string()))?;

    Ok(payload)
}

pub(crate) fn verify_operation_permissions(
    perms: &GrantedPermissions,
    op: &DeclarativeOperation,
    planned_write_len: usize,
) -> Result<(), RuntimeError> {
    if !perms.allows(PermissionScope::ProcessReadMemory) {
        return Err(RuntimeError::PermissionDenied(
            PermissionScope::ProcessReadMemory,
        ));
    }
    if !perms.allows(PermissionScope::ProcessWriteMemory) {
        return Err(RuntimeError::PermissionDenied(
            PermissionScope::ProcessWriteMemory,
        ));
    }

    if let Some(constraints) = perms.constraints_for(PermissionScope::ProcessWriteMemory) {
        if let Some(allowed) = &constraints.allowed_modules {
            let target_mod = op.target_module();
            if !allowed.iter().any(|m| m.eq_ignore_ascii_case(target_mod)) {
                return Err(RuntimeError::PermissionConstraintViolation(format!(
                    "module '{target_mod}' is outside granted allowed_modules constraint"
                )));
            }
        }
        if let Some(max_bytes) = constraints.max_write_bytes
            && planned_write_len > max_bytes as usize
        {
            return Err(RuntimeError::PermissionConstraintViolation(format!(
                "write length ({planned_write_len} bytes) exceeds granted max_write_bytes ({max_bytes})"
            )));
        }
    }

    Ok(())
}

pub(crate) fn start_managed_session(
    backend: &dyn ProcessMemoryBackend,
    session_id: RuntimeSessionId,
    req: StartSessionRequest,
) -> Result<ManagedSession, RuntimeError> {
    validate_session_preflight(&req)?;
    check_running_anti_cheat(backend, &req.trainer.compatibility.anti_cheat_incompatible)?;
    let payload = parse_and_verify_payload(&req)?;

    let mode = if req
        .granted_permissions
        .allows(PermissionScope::ProcessWriteMemory)
    {
        ProcessAccessMode::ReadWrite
    } else {
        ProcessAccessMode::ReadOnly
    };

    let handle = backend.open_process(req.target_process.pid, mode)?;
    handle.verify_identity(&req.target_process)?;

    let mut inner = SessionInner {
        session_id: session_id.clone(),
        target_process: req.target_process.clone(),
        granted_permissions: req.granted_permissions,
        payload,
        handle,
        ledger: PatchLedger::default(),
        state: SessionLifecycleState::Attached,
        freeze_entries: Vec::new(),
        recent_events: Vec::new(),
    };
    inner.push_event(format!(
        "Attached to '{}' (PID {}) with {:?} access",
        req.target_process.process_name, req.target_process.pid, mode
    ));

    Ok(ManagedSession {
        inner: Arc::new(Mutex::new(inner)),
        stop_flag: Arc::new(AtomicBool::new(false)),
        freeze_thread: None,
    })
}

pub(crate) fn apply_feature_command(
    session: &mut ManagedSession,
    command: FeatureCommand,
) -> Result<(), RuntimeError> {
    let mut should_spawn_freezer = false;

    {
        let mut inner = session.inner.lock().unwrap();
        if inner.state != SessionLifecycleState::Attached {
            return Err(RuntimeError::ProcessUnavailable(inner.target_process.pid));
        }

        if let Err(err) = inner.handle.verify_identity(&inner.target_process) {
            inner.state = SessionLifecycleState::Faulted;
            inner.freeze_entries.clear();
            inner.push_event(format!("Process identity check failed: {err}"));
            return Err(err);
        }

        let entry = inner
            .payload
            .feature(&command.feature_id)
            .cloned()
            .ok_or_else(|| RuntimeError::UnknownFeature(command.feature_id.clone()))?;

        match command.value {
            FeatureValue::Toggle(false) => {
                inner
                    .freeze_entries
                    .retain(|f| f.feature_id != command.feature_id);
                let target_proc = inner.target_process.clone();
                let report = rollback_feature_inner(&mut inner, &command.feature_id, &target_proc);
                if !report.conflicts.is_empty() || !report.failures.is_empty() {
                    let msg = report
                        .conflicts
                        .iter()
                        .chain(report.failures.iter())
                        .cloned()
                        .collect::<Vec<_>>()
                        .join("; ");
                    return Err(RuntimeError::RollbackFailed {
                        feature_id: command.feature_id,
                        reason: msg,
                    });
                }
                return Ok(());
            }

            FeatureValue::Toggle(true)
            | FeatureValue::Action
            | FeatureValue::Integer(_)
            | FeatureValue::Float(_) => {
                let is_action = matches!(command.value, FeatureValue::Action);
                let is_toggle_on = matches!(command.value, FeatureValue::Toggle(true));

                // Preflight permissions & constraints for every operation before mutating memory.
                let mut prepared = Vec::with_capacity(entry.operations.len());
                for op in &entry.operations {
                    let bytes = resolve_operation_write_bytes(op, &command.value)?;
                    verify_operation_permissions(&inner.granted_permissions, op, bytes.len())?;
                    prepared.push((op, bytes));
                }

                // Clear previous freezes for this feature before inserting updated freeze targets.
                inner
                    .freeze_entries
                    .retain(|f| f.feature_id != command.feature_id);

                let mut new_freezes = Vec::new();
                for (op, patch_bytes) in prepared {
                    let addr = resolve_locator(
                        &*inner.handle,
                        inner.target_process.architecture,
                        op.locator(),
                    )?;

                    if let DeclarativeOperation::BytePatch {
                        expected_original_bytes: Some(exp_hex),
                        ..
                    } = op
                    {
                        let already_active = inner
                            .ledger
                            .active_records()
                            .iter()
                            .any(|r| r.feature_id == command.feature_id && r.address == addr);
                        if !already_active {
                            let expected = parse_hex_bytes(exp_hex)
                                .map_err(|e| RuntimeError::InvalidPayload(e.to_string()))?;
                            let observed = inner.handle.read_memory(addr, expected.len())?;
                            if observed != expected {
                                return Err(RuntimeError::SignatureMismatch(format!(
                                    "feature '{}' at 0x{addr:X}: expected {:02X?}, observed {:02X?}",
                                    command.feature_id, expected, observed
                                )));
                            }
                        }
                    }

                    // Skip double-patching an already active `byte_patch` at same address.
                    if is_toggle_on
                        && matches!(op, DeclarativeOperation::BytePatch { .. })
                        && inner.ledger.active_records().iter().any(|r| {
                            r.feature_id == command.feature_id
                                && r.address == addr
                                && r.patched_bytes == patch_bytes
                        })
                    {
                        continue;
                    }

                    let pre_write = inner.handle.read_memory(addr, patch_bytes.len())?;
                    inner.handle.write_memory(addr, &patch_bytes)?;
                    let target_proc = inner.target_process.clone();
                    inner.ledger.record_patch(
                        &command.feature_id,
                        &target_proc,
                        addr,
                        pre_write,
                        patch_bytes.clone(),
                    );

                    if !is_action
                        && let DeclarativeOperation::WriteValue {
                            freeze_interval_ms: Some(interval_ms),
                            ..
                        } = op
                    {
                        let clamped =
                            (*interval_ms).clamp(MIN_FREEZE_INTERVAL_MS, MAX_FREEZE_INTERVAL_MS);
                        new_freezes.push(FreezeEntry {
                            feature_id: command.feature_id.clone(),
                            address: addr,
                            bytes: patch_bytes,
                            interval: Duration::from_millis(clamped as u64),
                            last_tick: Instant::now(),
                        });
                    }
                }

                if !new_freezes.is_empty() {
                    inner.freeze_entries.extend(new_freezes);
                    should_spawn_freezer = true;
                }

                let ledger_len = inner.ledger.len();
                inner.push_event(format!(
                    "Applied feature '{}' ({ledger_len} active ledger records)",
                    command.feature_id
                ));
            }
        }
    }

    if should_spawn_freezer && session.freeze_thread.is_none() {
        spawn_freeze_worker(session);
    }

    Ok(())
}

pub(crate) fn stop_managed_session(session: &mut ManagedSession) -> RuntimeDiagnostics {
    session.stop_flag.store(true, Ordering::SeqCst);
    if let Some(handle) = session.freeze_thread.take() {
        let _ = handle.join();
    }

    let mut inner = session.inner.lock().unwrap();
    inner.freeze_entries.clear();
    let target_proc = inner.target_process.clone();
    let identity_ok = inner.handle.verify_identity(&target_proc);

    let report = match identity_ok {
        Ok(()) => rollback_all_inner(&mut inner, &target_proc),
        Err(err) => {
            let reason = err.to_string();
            let rep = inner.ledger.abort_on_process_loss(&reason);
            inner.push_event(format!(
                "Session stopped after target exit/change: {reason}"
            ));
            rep
        }
    };

    if report.conflicts.is_empty() && report.failures.is_empty() {
        inner.state = SessionLifecycleState::Stopped;
        inner.push_event(format!(
            "Session stopped cleanly ({} patches rolled back in LIFO order)",
            report.restored
        ));
    } else {
        inner.state = SessionLifecycleState::Stopped;
        inner.push_event(format!(
            "Session stopped with {} restored, {} conflicts, {} failures",
            report.restored,
            report.conflicts.len(),
            report.failures.len()
        ));
    }

    inner.diagnostics()
}

fn rollback_feature_inner(
    inner: &mut SessionInner,
    feature_id: &str,
    target_proc: &ProcessIdentity,
) -> RollbackReport {
    // Temporarily move `ledger` out so `handle` & `ledger` don't double-borrow `inner`.
    let mut ledger = std::mem::take(&mut inner.ledger);
    let report = ledger.rollback_feature(feature_id, &*inner.handle, target_proc);
    inner.ledger = ledger;
    log_rollback_report(inner, &format!("feature '{feature_id}'"), &report);
    report
}

fn rollback_all_inner(inner: &mut SessionInner, target_proc: &ProcessIdentity) -> RollbackReport {
    let mut ledger = std::mem::take(&mut inner.ledger);
    let report = ledger.rollback_all(&*inner.handle, target_proc);
    inner.ledger = ledger;
    log_rollback_report(inner, "session teardown", &report);
    report
}

fn log_rollback_report(inner: &mut SessionInner, scope: &str, report: &RollbackReport) {
    if report.restored > 0 {
        inner.push_event(format!(
            "Rolled back {} patch(es) for {scope}",
            report.restored
        ));
    }
    for conflict in &report.conflicts {
        inner.push_event(format!("ROLLBACK CONFLICT: {conflict}"));
    }
    for failure in &report.failures {
        inner.push_event(format!("ROLLBACK FAILURE: {failure}"));
    }
}

fn resolve_operation_write_bytes(
    op: &DeclarativeOperation,
    command_value: &FeatureValue,
) -> Result<Vec<u8>, RuntimeError> {
    match op {
        DeclarativeOperation::BytePatch { patch_bytes, .. } => {
            parse_hex_bytes(patch_bytes).map_err(|e| RuntimeError::InvalidPayload(e.to_string()))
        }
        DeclarativeOperation::WriteValue {
            value_type, value, ..
        } => {
            let numeric = match command_value {
                FeatureValue::Integer(v) => *v as f64,
                FeatureValue::Float(v) => *v,
                FeatureValue::Toggle(true) | FeatureValue::Action => value.ok_or_else(|| {
                    RuntimeError::InvalidPayload(
                        "write_value operation requires an explicit numeric value".to_owned(),
                    )
                })?,
                FeatureValue::Toggle(false) => unreachable!("handled above"),
            };
            value_type
                .encode_value(numeric)
                .map_err(|e| RuntimeError::InvalidPayload(e.to_string()))
        }
    }
}

fn spawn_freeze_worker(session: &mut ManagedSession) {
    let inner_arc = Arc::clone(&session.inner);
    let stop_flag = Arc::clone(&session.stop_flag);

    let join = thread::Builder::new()
        .name(format!(
            "nekod-freeze-{}",
            session.inner.lock().unwrap().session_id.0
        ))
        .spawn(move || {
            let mut next_check = Duration::from_millis(25);
            while !stop_flag.load(Ordering::SeqCst) {
                thread::sleep(next_check.min(Duration::from_millis(25)));
                if stop_flag.load(Ordering::SeqCst) {
                    break;
                }

                let mut guard = inner_arc.lock().unwrap();
                if guard.state != SessionLifecycleState::Attached || guard.freeze_entries.is_empty()
                {
                    next_check = Duration::from_millis(25);
                    continue;
                }

                let target_proc = guard.target_process.clone();
                if let Err(err) = guard.handle.verify_identity(&target_proc) {
                    guard.state = SessionLifecycleState::Faulted;
                    guard.freeze_entries.clear();
                    guard.push_event(format!("Freeze worker stopped (target lost): {err}"));
                    break;
                }

                let now = Instant::now();
                let mut due_indices = Vec::new();
                for (idx, item) in guard.freeze_entries.iter().enumerate() {
                    if now.duration_since(item.last_tick) >= item.interval {
                        due_indices.push(idx);
                    }
                }

                for idx in due_indices {
                    let (addr, bytes, feat_id) = {
                        let entry = &mut guard.freeze_entries[idx];
                        entry.last_tick = now;
                        (entry.address, entry.bytes.clone(), entry.feature_id.clone())
                    };
                    if let Err(err) = guard.handle.write_memory(addr, &bytes) {
                        guard.state = SessionLifecycleState::Faulted;
                        guard.freeze_entries.clear();
                        guard.push_event(format!(
                            "Freeze write failed for feature '{feat_id}' at 0x{addr:X}: {err}"
                        ));
                        break;
                    }
                }

                next_check = guard
                    .freeze_entries
                    .iter()
                    .map(|e| e.interval)
                    .min()
                    .unwrap_or(Duration::from_millis(50));
            }
        })
        .ok();

    session.freeze_thread = join;
}

pub(crate) fn insert_session_id() -> RuntimeSessionId {
    use std::sync::atomic::AtomicU64;
    static COUNTER: AtomicU64 = AtomicU64::new(1);
    let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
    RuntimeSessionId(format!("sess-{seq:06}"))
}

pub(crate) type SessionMap = BTreeMap<String, ManagedSession>;
