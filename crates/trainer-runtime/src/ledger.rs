//! Safe LIFO `PatchLedger` implementing verified 5-step memory rollback.

use std::time::{SystemTime, UNIX_EPOCH};

use nekod_runtime_api::{PatchActivationState, PatchRecord, ProcessIdentity, RuntimeError};

use crate::backend::ProcessHandle;

/// Outcome of rolling back one feature or an entire session.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct RollbackReport {
    pub restored: usize,
    pub conflicts: Vec<String>,
    pub failures: Vec<String>,
}

/// Ordered ledger of applied memory patches supporting deterministic LIFO rollback.
#[derive(Debug, Default)]
pub struct PatchLedger {
    next_patch_id: u64,
    records: Vec<PatchRecord>,
    history: Vec<PatchRecord>,
    rollback_conflicts: usize,
    rollback_failures: usize,
}

impl PatchLedger {
    /// Appends a raw `PatchRecord` to the active stack (preserving insertion order for LIFO).
    pub fn push(&mut self, mut record: PatchRecord) {
        if record.patch_id == 0 {
            self.next_patch_id = self.next_patch_id.saturating_add(1);
            record.patch_id = self.next_patch_id;
        } else if record.patch_id >= self.next_patch_id {
            self.next_patch_id = record.patch_id.saturating_add(1);
        }
        self.records.push(record);
    }

    /// Records a newly applied patch or updates the expected `patched_bytes` of an
    /// active patch at the exact same `(feature_id, address)` while preserving its
    /// initial `original_bytes` so rollback restores the true pre-trainer state.
    pub fn record_patch(
        &mut self,
        feature_id: &str,
        target: &ProcessIdentity,
        address: u64,
        original_bytes: Vec<u8>,
        patched_bytes: Vec<u8>,
    ) -> u64 {
        let now_ms = epoch_millis_now();
        if let Some(existing) = self
            .records
            .iter_mut()
            .rev()
            .find(|r| r.feature_id == feature_id && r.address == address)
        {
            existing.patched_bytes = patched_bytes;
            existing.applied_at_epoch_ms = now_ms;
            return existing.patch_id;
        }

        self.next_patch_id = self.next_patch_id.saturating_add(1);
        let id = self.next_patch_id;
        self.records.push(PatchRecord {
            patch_id: id,
            feature_id: feature_id.to_owned(),
            target_pid: target.pid,
            process_creation_time_100ns: target.process_creation_time_100ns,
            address,
            original_bytes,
            patched_bytes,
            activation_state: PatchActivationState::Active,
            applied_at_epoch_ms: now_ms,
        });
        id
    }

    pub fn len(&self) -> usize {
        self.records.len()
    }

    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    pub fn rollback_conflicts(&self) -> usize {
        self.rollback_conflicts
    }

    pub fn rollback_failures(&self) -> usize {
        self.rollback_failures
    }

    pub fn active_records(&self) -> &[PatchRecord] {
        &self.records
    }

    /// Drains patches for a specific feature in reverse chronological (LIFO) order.
    pub fn take_feature_rollback_order(&mut self, feature_id: &str) -> Vec<PatchRecord> {
        let mut rollback = Vec::new();
        let mut retained = Vec::with_capacity(self.records.len());
        for rec in self.records.drain(..) {
            if rec.feature_id == feature_id {
                rollback.push(rec);
            } else {
                retained.push(rec);
            }
        }
        self.records = retained;
        rollback.reverse();
        rollback
    }

    /// Executes 5-step verified LIFO rollback for every active patch belonging to `feature_id`.
    pub fn rollback_feature(
        &mut self,
        feature_id: &str,
        handle: &dyn ProcessHandle,
        expected_process: &ProcessIdentity,
    ) -> RollbackReport {
        let to_rollback = self.take_feature_rollback_order(feature_id);
        self.execute_rollback_batch(to_rollback, handle, expected_process)
    }

    /// Executes 5-step verified LIFO rollback across ALL active patches in the session.
    pub fn rollback_all(
        &mut self,
        handle: &dyn ProcessHandle,
        expected_process: &ProcessIdentity,
    ) -> RollbackReport {
        let mut to_rollback = std::mem::take(&mut self.records);
        to_rollback.reverse();
        self.execute_rollback_batch(to_rollback, handle, expected_process)
    }

    /// Marks remaining active records failed if the process disappeared before rollback.
    pub fn abort_on_process_loss(&mut self, reason: &str) -> RollbackReport {
        let mut report = RollbackReport::default();
        let mut remaining = std::mem::take(&mut self.records);
        remaining.reverse();
        for mut rec in remaining {
            self.rollback_failures = self.rollback_failures.saturating_add(1);
            rec.activation_state = PatchActivationState::RollbackConflict;
            report.failures.push(format!(
                "patch {} ({}) could not roll back: {reason}",
                rec.patch_id, rec.feature_id
            ));
            self.history.push(rec);
        }
        report
    }

    fn execute_rollback_batch(
        &mut self,
        batch: Vec<PatchRecord>,
        handle: &dyn ProcessHandle,
        expected_process: &ProcessIdentity,
    ) -> RollbackReport {
        let mut report = RollbackReport::default();

        for mut rec in batch {
            // Step 1: Verify target process still matches original identity & creation time.
            if let Err(err) = verify_record_process_identity(&rec, handle, expected_process) {
                self.rollback_failures = self.rollback_failures.saturating_add(1);
                rec.activation_state = PatchActivationState::RollbackConflict;
                report.failures.push(format!(
                    "patch {} ({}) aborted rollback at step 1 (process identity): {err}",
                    rec.patch_id, rec.feature_id
                ));
                self.history.push(rec);
                continue;
            }

            // Step 2: Verify target address lies within a valid committed readable region.
            let region = match handle.query_region(rec.address) {
                Ok(r)
                    if r.is_committed
                        && r.is_readable
                        && r.contains_range(rec.address, rec.original_bytes.len()) =>
                {
                    r
                }
                Ok(_) => {
                    self.rollback_conflicts = self.rollback_conflicts.saturating_add(1);
                    rec.activation_state = PatchActivationState::RollbackConflict;
                    report.conflicts.push(format!(
                        "patch {} ({}) at 0x{:X} is no longer inside a valid committed region",
                        rec.patch_id, rec.feature_id, rec.address
                    ));
                    self.history.push(rec);
                    continue;
                }
                Err(err) => {
                    self.rollback_failures = self.rollback_failures.saturating_add(1);
                    rec.activation_state = PatchActivationState::RollbackConflict;
                    report.failures.push(format!(
                        "patch {} ({}) failed VirtualQuery at 0x{:X}: {err}",
                        rec.patch_id, rec.feature_id, rec.address
                    ));
                    self.history.push(rec);
                    continue;
                }
            };
            let _ = region;

            // Step 3: Compare live memory against expected `patched_bytes` before overwriting.
            let current = match handle.read_memory(rec.address, rec.patched_bytes.len()) {
                Ok(bytes) => bytes,
                Err(err) => {
                    self.rollback_failures = self.rollback_failures.saturating_add(1);
                    rec.activation_state = PatchActivationState::RollbackConflict;
                    report.failures.push(format!(
                        "patch {} ({}) failed pre-restore read at 0x{:X}: {err}",
                        rec.patch_id, rec.feature_id, rec.address
                    ));
                    self.history.push(rec);
                    continue;
                }
            };

            if current != rec.patched_bytes {
                // Step 5: Report conflict without silently overwriting modified memory!
                self.rollback_conflicts = self.rollback_conflicts.saturating_add(1);
                rec.activation_state = PatchActivationState::RollbackConflict;
                report.conflicts.push(format!(
                    "patch {} ({}) at 0x{:X} conflict: expected {:02X?}, found {:02X?}; skipping restore",
                    rec.patch_id,
                    rec.feature_id,
                    rec.address,
                    rec.patched_bytes,
                    current
                ));
                self.history.push(rec);
                continue;
            }

            // Step 4: Safe state confirmed -> restore original bytes.
            match handle.write_memory(rec.address, &rec.original_bytes) {
                Ok(()) => {
                    rec.activation_state = PatchActivationState::RolledBack;
                    report.restored = report.restored.saturating_add(1);
                    self.history.push(rec);
                }
                Err(err) => {
                    self.rollback_failures = self.rollback_failures.saturating_add(1);
                    rec.activation_state = PatchActivationState::RollbackConflict;
                    report.failures.push(format!(
                        "patch {} ({}) write_memory restore failed at 0x{:X}: {err}",
                        rec.patch_id, rec.feature_id, rec.address
                    ));
                    self.history.push(rec);
                }
            }
        }

        report
    }
}

fn verify_record_process_identity(
    rec: &PatchRecord,
    handle: &dyn ProcessHandle,
    expected_process: &ProcessIdentity,
) -> Result<(), RuntimeError> {
    if rec.target_pid != expected_process.pid || handle.pid() != rec.target_pid {
        return Err(RuntimeError::ProcessIdentityMismatch(format!(
            "record pid {} does not match session pid {}",
            rec.target_pid, expected_process.pid
        )));
    }
    if rec.process_creation_time_100ns != expected_process.process_creation_time_100ns {
        return Err(RuntimeError::ProcessIdentityMismatch(
            "patch record creation timestamp differs from session identity".to_owned(),
        ));
    }
    handle.verify_identity(expected_process)
}

pub fn epoch_millis_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis().min(u64::MAX as u128) as u64)
        .unwrap_or(0)
}
