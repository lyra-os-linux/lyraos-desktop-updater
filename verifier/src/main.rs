use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use lyra_upgrade_core::{
    BootVerification, OperationState, SystemBackend, discover_host, load_state, save_state,
};

const STATE_ROOT: &str = "/var/lib/lyra-upgrade/operations";

fn main() {
    let Some((operation_dir, mut state)) = pending_operation() else {
        return;
    };
    let passed = verify(&state);
    finalize_verification(
        &mut state,
        &operation_dir,
        Path::new("/var/lib/lyra-upgrade/last-manifest-sequence"),
        passed,
    );
    if let Err(error) = save_state(Path::new(STATE_ROOT), &state) {
        eprintln!("lyra-upgrade-verify: cannot persist verification result: {error:?}");
        std::process::exit(1);
    }
}

fn finalize_verification(
    state: &mut lyra_upgrade_core::OperationStateRecord,
    operation_dir: &Path,
    sequence_path: &Path,
    passed: bool,
) {
    state.sequence = state.sequence.saturating_add(1);
    if passed {
        if state.operation == lyra_upgrade_core::OperationKind::ReleaseUpgrade {
            let persisted = read_manifest_sequence(operation_dir)
                .ok_or(())
                .and_then(|sequence| write_sequence(sequence_path, sequence).map_err(|_| ()));
            if persisted.is_err() {
                state.state = OperationState::NeedsRecovery;
                state.boot_verification = Some(BootVerification::Failed);
                state.error_code = Some("MANIFEST_SEQUENCE_PERSIST_FAILED".into());
                return;
            }
        }
        state.state = OperationState::Completed;
        state.boot_verification = Some(BootVerification::Passed);
        state.last_completed_step = Some("post-boot-verification".into());
        state.error_code = None;
    } else {
        state.state = OperationState::NeedsRecovery;
        state.boot_verification = Some(BootVerification::Failed);
        state.error_code = Some("POST_BOOT_VERIFICATION_FAILED".into());
    }
}

fn pending_operation() -> Option<(PathBuf, lyra_upgrade_core::OperationStateRecord)> {
    let mut candidates = Vec::new();
    for entry in fs::read_dir(STATE_ROOT).ok()? {
        let entry = entry.ok()?;
        if !entry.file_type().ok()?.is_dir() {
            continue;
        }
        let operation_id = entry.file_name().into_string().ok()?;
        let state = load_state(Path::new(STATE_ROOT), &operation_id).ok()?;
        if matches!(
            state.state,
            OperationState::AwaitingReboot | OperationState::VerifyingBoot
        ) {
            candidates.push((entry.path(), state));
        }
    }
    candidates.sort_by(|left, right| right.1.updated_at.cmp(&left.1.updated_at));
    candidates.into_iter().next()
}

fn verify(state: &lyra_upgrade_core::OperationStateRecord) -> bool {
    let Ok(facts) = discover_host(&SystemBackend) else {
        return false;
    };
    if let Some(target) = &state.target
        && (facts.release.version != target.version
            || facts.release.edition != target.edition
            || facts.release.architecture != target.architecture)
    {
        return false;
    }
    if facts.root_filesystem != "btrfs" || !facts.snapper_root_configured {
        return false;
    }
    if !run("rpm", &["--verifydb"]) {
        return false;
    }
    if !run("zypper", &["--non-interactive", "--no-refresh", "verify"]) {
        return false;
    }
    if !run("systemctl", &["is-system-running", "--wait"]) {
        return false;
    }
    run("test", &["-s", "/boot/grub2/grub.cfg"])
}

fn run(program: &str, arguments: &[&str]) -> bool {
    Command::new(program)
        .args(arguments)
        .env_clear()
        .env("PATH", "/usr/sbin:/usr/bin:/sbin:/bin")
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

fn read_manifest_sequence(operation_dir: &Path) -> Option<u64> {
    let path = operation_dir.join("manifest.json");
    let metadata = fs::symlink_metadata(&path).ok()?;
    if !metadata.file_type().is_file() || metadata.len() > 1024 * 1024 {
        return None;
    }
    let document: serde_json::Value = serde_json::from_slice(&fs::read(path).ok()?).ok()?;
    document.get("sequence")?.as_u64()
}

fn write_sequence(path: &Path, sequence: u64) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let temporary = path.with_extension("tmp");
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temporary)?;
    writeln!(file, "{sequence}")?;
    file.sync_all()?;
    fs::rename(temporary, path)?;
    fs::File::open(path.parent().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "sequence path has no parent",
        )
    })?)?
    .sync_all()
}

#[cfg(test)]
mod tests {
    use super::{finalize_verification, read_manifest_sequence};
    use lyra_upgrade_core::{
        BootVerification, OperationKind, OperationState, OperationStateRecord, ReleaseIdentity,
        STATE_SCHEMA_VERSION,
    };
    use std::fs;

    fn state(kind: OperationKind) -> OperationStateRecord {
        OperationStateRecord {
            schema_version: STATE_SCHEMA_VERSION,
            operation_id: "00000000-0000-4000-8000-000000000000".into(),
            sequence: 1,
            operation: kind,
            state: OperationState::VerifyingBoot,
            source: ReleaseIdentity {
                version: "1.0-alpha.6".into(),
                edition: "desktop".into(),
                architecture: "x86_64".into(),
                build_id: "source".into(),
            },
            target: None,
            plan_sha256: "0".repeat(64),
            manifest_sha256: None,
            snapshot_number: Some(42),
            last_completed_step: None,
            error_code: None,
            boot_verification: Some(BootVerification::Pending),
            created_at: "2026-08-31T00:00:00Z".into(),
            updated_at: "2026-08-31T00:00:00Z".into(),
        }
    }

    #[test]
    fn manifest_sequence_rejects_missing_incomplete_and_invalid_values() {
        let root = std::env::temp_dir().join(format!(
            "lyra-upgrade-verifier-sequence-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir(&root).unwrap();

        fs::write(root.join("manifest.json"), br#"{"sequence":42}"#).unwrap();
        assert_eq!(read_manifest_sequence(&root), Some(42));
        fs::write(root.join("manifest.json"), br#"{"target":"x"}"#).unwrap();
        assert_eq!(read_manifest_sequence(&root), None);
        fs::write(root.join("manifest.json"), br#"{"sequence":}"#).unwrap();
        assert_eq!(read_manifest_sequence(&root), None);
        fs::write(root.join("manifest.json"), br#"{"sequence":"tampered"}"#).unwrap();
        assert_eq!(read_manifest_sequence(&root), None);
        fs::write(
            root.join("manifest.json"),
            br#"{"note":"embedded \\"sequence\\":99 must not count"}"#,
        )
        .unwrap();
        assert_eq!(read_manifest_sequence(&root), None);

        let target = root.join("target.json");
        fs::write(&target, br#"{"sequence":43}"#).unwrap();
        fs::remove_file(root.join("manifest.json")).unwrap();
        std::os::unix::fs::symlink(&target, root.join("manifest.json")).unwrap();
        assert_eq!(read_manifest_sequence(&root), None);

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn release_upgrade_fails_closed_when_replay_sequence_cannot_be_persisted() {
        let root = std::env::temp_dir().join(format!(
            "lyra-upgrade-verifier-finalize-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir(&root).unwrap();
        fs::write(root.join("manifest.json"), br#"{"sequence":42}"#).unwrap();

        let mut completed = state(OperationKind::ReleaseUpgrade);
        let sequence_path = root.join("last-sequence");
        finalize_verification(&mut completed, &root, &sequence_path, true);
        assert_eq!(completed.state, OperationState::Completed);
        assert_eq!(fs::read_to_string(&sequence_path).unwrap(), "42\n");

        let mut blocked = state(OperationKind::ReleaseUpgrade);
        fs::write(root.join("last-sequence.tmp"), b"stale").unwrap();
        finalize_verification(&mut blocked, &root, &sequence_path, true);
        assert_eq!(blocked.state, OperationState::NeedsRecovery);
        assert_eq!(
            blocked.error_code.as_deref(),
            Some("MANIFEST_SEQUENCE_PERSIST_FAILED")
        );

        fs::remove_dir_all(root).unwrap();
    }
}
