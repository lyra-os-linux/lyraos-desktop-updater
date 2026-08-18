use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Component, Path};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::{OperationKind, OperationState, OperationStateRecord, STATE_SCHEMA_VERSION};

#[derive(Debug)]
pub enum PersistenceError {
    InvalidOperationId,
    UnsafePath,
    Io(io::Error),
    Json(serde_json::Error),
    UnsupportedSchema,
    InvalidState,
}

impl From<io::Error> for PersistenceError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<serde_json::Error> for PersistenceError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

pub fn save_state(root: &Path, state: &OperationStateRecord) -> Result<(), PersistenceError> {
    validate_operation_id(&state.operation_id)?;
    validate_state(state)?;
    prepare_root(root)?;
    let operation_dir = root.join(&state.operation_id);
    ensure_directory(&operation_dir)?;
    let destination = operation_dir.join("state.json");
    reject_symlink(&destination)?;

    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let temporary = operation_dir.join(format!(".state.json.{}.{}", std::process::id(), nonce));
    let result = write_and_replace(&temporary, &destination, state);
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

pub fn load_state(
    root: &Path,
    operation_id: &str,
) -> Result<OperationStateRecord, PersistenceError> {
    validate_operation_id(operation_id)?;
    reject_symlink(root)?;
    let operation_dir = root.join(operation_id);
    reject_symlink(&operation_dir)?;
    let path = operation_dir.join("state.json");
    reject_symlink(&path)?;
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)?;
    let state: OperationStateRecord = serde_json::from_reader(file)?;
    if state.schema_version != STATE_SCHEMA_VERSION {
        return Err(PersistenceError::UnsupportedSchema);
    }
    if state.operation_id != operation_id {
        return Err(PersistenceError::UnsafePath);
    }
    validate_state(&state)?;
    Ok(state)
}

fn validate_state(state: &OperationStateRecord) -> Result<(), PersistenceError> {
    if state.schema_version != STATE_SCHEMA_VERSION {
        return Err(PersistenceError::UnsupportedSchema);
    }
    if !is_sha256(&state.plan_sha256)
        || state
            .manifest_sha256
            .as_deref()
            .is_some_and(|hash| !is_sha256(hash))
        || state.snapshot_number == Some(0)
        || (matches!(
            state.state,
            OperationState::Applying
                | OperationState::ReadyToReboot
                | OperationState::ApplyingOffline
                | OperationState::AwaitingReboot
                | OperationState::VerifyingBoot
                | OperationState::NeedsRecovery
                | OperationState::Completed
        ) && state.snapshot_number.is_none())
    {
        return Err(PersistenceError::InvalidState);
    }
    let operation_shape_is_valid = match state.operation {
        OperationKind::UpdateWithinRelease => {
            state.target.is_none() && state.manifest_sha256.is_none()
        }
        OperationKind::ReleaseUpgrade => state.target.is_some() && state.manifest_sha256.is_some(),
    };
    operation_shape_is_valid
        .then_some(())
        .ok_or(PersistenceError::InvalidState)
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn prepare_root(root: &Path) -> Result<(), PersistenceError> {
    if root
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(PersistenceError::UnsafePath);
    }
    reject_symlink(root)?;
    ensure_directory(root)
}

fn ensure_directory(path: &Path) -> Result<(), PersistenceError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            Err(PersistenceError::UnsafePath)
        }
        // SAFETY: `geteuid` has no arguments, does not dereference memory and
        // only returns the kernel's effective UID for this process.
        Ok(metadata) if metadata.uid() != unsafe { libc::geteuid() } => {
            Err(PersistenceError::UnsafePath)
        }
        Ok(_) => {
            fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
            Ok(())
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            fs::create_dir(path)?;
            fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
            Ok(())
        }
        Err(error) => Err(PersistenceError::Io(error)),
    }
}

fn reject_symlink(path: &Path) -> Result<(), PersistenceError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(PersistenceError::UnsafePath),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(PersistenceError::Io(error)),
    }
}

fn write_and_replace(
    temporary: &Path,
    destination: &Path,
    state: &OperationStateRecord,
) -> Result<(), PersistenceError> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(temporary)?;
    serde_json::to_writer_pretty(&mut file, state)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    fs::rename(temporary, destination)?;
    sync_directory(destination.parent().ok_or(PersistenceError::UnsafePath)?)?;
    Ok(())
}

fn sync_directory(path: &Path) -> Result<(), PersistenceError> {
    File::open(path)?.sync_all()?;
    Ok(())
}

fn validate_operation_id(value: &str) -> Result<(), PersistenceError> {
    let valid = value.len() == 36
        && value.bytes().enumerate().all(|(index, byte)| {
            if matches!(index, 8 | 13 | 18 | 23) {
                byte == b'-'
            } else {
                byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase()
            }
        });
    valid
        .then_some(())
        .ok_or(PersistenceError::InvalidOperationId)
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::symlink;
    use std::path::PathBuf;

    use super::*;
    use crate::{BootVerification, OperationKind, OperationState, ReleaseIdentity};

    fn temporary_root(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "lyra-upgrade-test-{}-{}-{name}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    fn state() -> OperationStateRecord {
        OperationStateRecord {
            schema_version: STATE_SCHEMA_VERSION,
            operation_id: "00000000-0000-4000-8000-000000000000".into(),
            sequence: 1,
            operation: OperationKind::UpdateWithinRelease,
            state: OperationState::Planned,
            source: ReleaseIdentity {
                version: "2026.08-alpha6".into(),
                edition: "desktop".into(),
                architecture: "x86_64".into(),
                build_id: "fixture".into(),
            },
            target: None,
            plan_sha256: "0".repeat(64),
            manifest_sha256: None,
            snapshot_number: None,
            last_completed_step: None,
            error_code: None,
            boot_verification: Some(BootVerification::Pending),
            created_at: "2026-08-18T00:00:00Z".into(),
            updated_at: "2026-08-18T00:00:00Z".into(),
        }
    }

    #[test]
    fn atomically_round_trips_state_with_private_permissions() {
        let root = temporary_root("roundtrip");
        save_state(&root, &state()).unwrap();
        assert_eq!(load_state(&root, &state().operation_id).unwrap(), state());
        let mode = fs::metadata(root.join(&state().operation_id).join("state.json"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn refuses_symbolic_link_state() {
        let root = temporary_root("symlink");
        let operation_dir = root.join(&state().operation_id);
        fs::create_dir_all(&operation_dir).unwrap();
        let target = root.join("target.json");
        fs::write(&target, b"{}\n").unwrap();
        symlink(&target, operation_dir.join("state.json")).unwrap();
        assert!(matches!(
            save_state(&root, &state()),
            Err(PersistenceError::UnsafePath)
        ));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn refuses_truncated_state() {
        let root = temporary_root("truncated");
        save_state(&root, &state()).unwrap();
        fs::write(root.join(&state().operation_id).join("state.json"), b"{").unwrap();
        assert!(matches!(
            load_state(&root, &state().operation_id),
            Err(PersistenceError::Json(_))
        ));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn refuses_unknown_schema_and_fields() {
        let root = temporary_root("unknown");
        save_state(&root, &state()).unwrap();
        let path = root.join(&state().operation_id).join("state.json");
        let mut value = serde_json::to_value(state()).unwrap();
        value["schema_version"] = serde_json::json!(STATE_SCHEMA_VERSION + 1);
        fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(matches!(
            load_state(&root, &state().operation_id),
            Err(PersistenceError::UnsupportedSchema)
        ));
        value["schema_version"] = serde_json::json!(STATE_SCHEMA_VERSION);
        value["unknown"] = serde_json::json!(true);
        fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(matches!(
            load_state(&root, &state().operation_id),
            Err(PersistenceError::Json(_))
        ));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn refuses_semantically_tampered_state() {
        let root = temporary_root("tampered");
        let mut invalid = state();
        invalid.plan_sha256 = "not-a-hash".into();
        assert!(matches!(
            save_state(&root, &invalid),
            Err(PersistenceError::InvalidState)
        ));

        let mut invalid = state();
        invalid.state = OperationState::Applying;
        assert!(matches!(
            save_state(&root, &invalid),
            Err(PersistenceError::InvalidState)
        ));

        let mut invalid = state();
        invalid.snapshot_number = Some(0);
        assert!(matches!(
            save_state(&root, &invalid),
            Err(PersistenceError::InvalidState)
        ));
    }
}
