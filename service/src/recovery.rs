//! Authorized rollback scheduling; state survives in a separate Btrfs subvolume.
use lyra_upgrade_core::recovery::{
    RollbackGoal, containing_subvolume_id, default_subvolume_id, inspect_subvolume, snapshot_path,
};
use lyra_upgrade_core::{
    BootVerification, OperationState, OperationStateRecord, release_identity_at, save_state,
};
use std::{
    fs,
    path::Path,
    process::{Command, Stdio},
};

pub fn schedule_rollback(
    state_root: &Path,
    state: &mut OperationStateRecord,
    now: &str,
) -> Result<(), &'static str> {
    let _lock = crate::executor::TransactionLock::acquire().map_err(|_| "TRANSACTION_BUSY")?;
    *state = lyra_upgrade_core::load_state(state_root, &state.operation_id)
        .map_err(|_| "STATE_READ_FAILED")?;
    if state.state != OperationState::NeedsRecovery {
        return Err("RECOVERY_NOT_REQUIRED");
    }
    if state
        .recovery
        .as_ref()
        .is_some_and(|goal| goal.boot_snapshot.is_none())
    {
        return Err("ROLLBACK_INTENT_INCOMPLETE");
    }
    let number = state.snapshot_number.ok_or("SNAPSHOT_NOT_AVAILABLE")?;
    let source = snapshot_path(number);
    if release_identity_at(&source).map_err(|_| "SNAPSHOT_IDENTITY_INVALID")? != state.source {
        return Err("SNAPSHOT_IDENTITY_MISMATCH");
    }
    // A source snapshot predating this recovery contract cannot verify the new
    // intent after reboot. Decline before changing the selected boot root.
    if fs::read_to_string(source.join("usr/lib/lyra-upgrade/recovery-format"))
        .ok()
        .as_deref()
        != Some("1\n")
    {
        return Err("SNAPSHOT_RECOVERY_UNSUPPORTED");
    }
    if containing_subvolume_id(state_root)? == containing_subvolume_id(Path::new("/"))? {
        return Err("RECOVERY_STATE_NOT_PERSISTENT");
    }
    let source_snapshot = inspect_subvolume(&source)?;
    state.recovery = Some(RollbackGoal {
        source_snapshot_number: number,
        source_snapshot,
        boot_snapshot: None,
    });
    state.sequence = state.sequence.saturating_add(1);
    state.updated_at = now.into();
    state.last_completed_step = Some("rollback-preparing".into());
    state.boot_verification = Some(BootVerification::Pending);
    save_state(state_root, state).map_err(|_| "STATE_WRITE_FAILED")?;
    let result = finish_rollback(state_root, state, number);
    if let Err(error) = result {
        state.state = OperationState::NeedsRecovery;
        state.error_code = Some(error.into());
        state.sequence = state.sequence.saturating_add(1);
        save_state(state_root, state).map_err(|_| "STATE_WRITE_FAILED")?;
    }
    result
}

fn finish_rollback(
    state_root: &Path,
    state: &mut OperationStateRecord,
    number: u64,
) -> Result<(), &'static str> {
    let output = Command::new("snapper")
        .args([
            "--no-dbus",
            "--ambit",
            "classic",
            "--quiet",
            "--config",
            "root",
            "rollback",
            "--print-number",
            &number.to_string(),
        ])
        .env_clear()
        .env("PATH", "/usr/sbin:/usr/bin:/sbin:/bin")
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .output()
        .map_err(|_| "ROLLBACK_FAILED")?;
    if !output.status.success() {
        return Err("ROLLBACK_FAILED");
    }
    let boot_number: u64 = String::from_utf8(output.stdout)
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .filter(|n| *n > 0)
        .ok_or("ROLLBACK_RESULT_INVALID")?;
    let boot = inspect_subvolume(&snapshot_path(boot_number))?;
    let goal = state
        .recovery
        .as_mut()
        .ok_or("ROLLBACK_INTENT_INCOMPLETE")?;
    if boot.parent_uuid.as_deref() != Some(&goal.source_snapshot.uuid)
        || default_subvolume_id()? != boot.id
        || release_identity_at(&snapshot_path(boot_number))
            .map_err(|_| "SNAPSHOT_IDENTITY_INVALID")?
            != state.source
    {
        return Err("ROLLBACK_RESULT_INVALID");
    }
    goal.boot_snapshot = Some(boot);
    state.state = OperationState::AwaitingReboot;
    state.sequence = state.sequence.saturating_add(1);
    state.error_code = None;
    state.last_completed_step = Some("rollback-scheduled".into());
    save_state(state_root, state).map_err(|_| "STATE_WRITE_FAILED")
}
