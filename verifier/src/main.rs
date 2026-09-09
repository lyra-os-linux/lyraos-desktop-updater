use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use lyra_upgrade_core::{
    BootVerification, OperationState, SystemBackend, discover_host, load_state, save_state,
};

const STATE_ROOT: &str = "/var/lib/lyra-upgrade/operations";

const CHECK_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(180);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(i32)]
enum CheckFailure {
    Discovery = 10,
    Identity,
    Filesystem,
    RpmDatabase,
    Dependencies,
    BootTarget,
    FailedUnits,
    Grub,
    Timeout,
    Worker,
}

impl CheckFailure {
    fn code(self) -> &'static str {
        match self {
            Self::Discovery => "POST_BOOT_DISCOVERY_FAILED",
            Self::Identity => "POST_BOOT_IDENTITY_FAILED",
            Self::Filesystem => "POST_BOOT_FILESYSTEM_FAILED",
            Self::RpmDatabase => "POST_BOOT_RPM_DATABASE_FAILED",
            Self::Dependencies => "POST_BOOT_DEPENDENCIES_FAILED",
            Self::BootTarget => "POST_BOOT_TARGET_NOT_ACTIVE",
            Self::FailedUnits => "POST_BOOT_FAILED_UNITS",
            Self::Grub => "POST_BOOT_GRUB_FAILED",
            Self::Timeout => "POST_BOOT_VERIFICATION_TIMEOUT",
            Self::Worker => "POST_BOOT_WORKER_FAILED",
        }
    }

    fn from_status(status: std::process::ExitStatus) -> Option<Self> {
        use std::os::unix::process::ExitStatusExt;
        // GNU timeout may kill its own process group when TERM is ignored.
        // Rust then reports a signal, while a shell reports status 137.
        let code = status
            .code()
            .or_else(|| status.signal().map(|signal| 128 + signal));
        match code {
            Some(0) => None,
            Some(10) => Some(Self::Discovery),
            Some(11) => Some(Self::Identity),
            Some(12) => Some(Self::Filesystem),
            Some(13) => Some(Self::RpmDatabase),
            Some(14) => Some(Self::Dependencies),
            Some(15) => Some(Self::BootTarget),
            Some(16) => Some(Self::FailedUnits),
            Some(17) => Some(Self::Grub),
            Some(124 | 137) => Some(Self::Timeout),
            _ => Some(Self::Worker),
        }
    }
}

fn main() {
    let arguments: Vec<_> = std::env::args().skip(1).collect();
    if let [mode, id] = arguments.as_slice()
        && mode == "--check-boot"
    {
        let result = load_state(Path::new(STATE_ROOT), id)
            .map_err(|_| CheckFailure::Worker)
            .and_then(|state| verify(&state));
        if let Err(error) = result {
            eprintln!("lyra-upgrade-verify: {}", error.code());
            std::process::exit(error as i32);
        }
        return;
    }
    if !arguments.is_empty() {
        eprintln!("lyra-upgrade-verify: invalid arguments");
        std::process::exit(2);
    }
    let Some((operation_dir, mut state)) = pending_operation() else {
        return;
    };
    if state.state == OperationState::AwaitingReboot {
        state
            .transition_to(OperationState::VerifyingBoot)
            .expect("post-boot transition");
    }
    persist(&state);
    // Supervise all probes, including discovery subprocesses. GNU timeout
    // signals the whole process group, then kills it after a grace period.
    // The parent remains alive to persist a recovery result on timeout.
    let failure = std::env::current_exe()
        .ok()
        .and_then(|exe| {
            bounded_status(&exe, &["--check-boot", &state.operation_id], CHECK_TIMEOUT).ok()
        })
        .map_or(Some(CheckFailure::Worker), |status| {
            CheckFailure::from_status(status)
        });
    finalize_verification(
        &mut state,
        &operation_dir,
        Path::new("/var/lib/lyra-upgrade/last-manifest-sequence"),
        failure.is_none(),
    );
    if let Some(error) = failure {
        state.error_code = Some(error.code().into());
        eprintln!("lyra-upgrade-verify: {}", error.code());
    }
    persist(&state);
    if state.state != OperationState::Completed {
        std::process::exit(1);
    }
}

fn persist(state: &lyra_upgrade_core::OperationStateRecord) {
    if let Err(error) = save_state(Path::new(STATE_ROOT), state) {
        eprintln!("lyra-upgrade-verify: cannot persist verification result: {error:?}");
        std::process::exit(1);
    }
}

fn bounded_status(
    program: &Path,
    arguments: &[&str],
    timeout: std::time::Duration,
) -> std::io::Result<std::process::ExitStatus> {
    Command::new("timeout")
        .args([
            "--signal=TERM",
            "--kill-after=5s",
            &format!("{}s", timeout.as_secs_f64()),
        ])
        .arg(program)
        .args(arguments)
        .env_clear()
        .env("PATH", "/usr/sbin:/usr/bin:/sbin:/bin")
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .status()
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

fn verify(state: &lyra_upgrade_core::OperationStateRecord) -> Result<(), CheckFailure> {
    let facts = discover_host(&SystemBackend).map_err(|_| CheckFailure::Discovery)?;
    if let Some(target) = &state.target
        && (facts.release.version != target.version
            || facts.release.edition != target.edition
            || facts.release.architecture != target.architecture)
    {
        return Err(CheckFailure::Identity);
    }
    if facts.root_filesystem != "btrfs" || !facts.snapper_root_configured {
        return Err(CheckFailure::Filesystem);
    }
    if !run("rpm", &["--verifydb"]) {
        return Err(CheckFailure::RpmDatabase);
    }
    // Package verification semantics are qualified separately in audit #12.
    if !run("zypper", &["--non-interactive", "--no-refresh", "verify"]) {
        return Err(CheckFailure::Dependencies);
    }
    if !run("systemctl", &["is-active", "--quiet", "multi-user.target"]) {
        return Err(CheckFailure::BootTarget);
    }
    let failed = Command::new("systemctl")
        .args([
            "--failed",
            "--no-legend",
            "--plain",
            "--no-pager",
            "list-units",
        ])
        .env_clear()
        .env("PATH", "/usr/sbin:/usr/bin:/sbin:/bin")
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map_err(|_| CheckFailure::FailedUnits)?;
    if !failed.status.success() || !failed.stdout.iter().all(u8::is_ascii_whitespace) {
        return Err(CheckFailure::FailedUnits);
    }
    if !run("test", &["-s", "/boot/grub2/grub.cfg"]) {
        return Err(CheckFailure::Grub);
    }
    Ok(())
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
    use super::{CheckFailure, bounded_status, finalize_verification, read_manifest_sequence};
    use lyra_upgrade_core::{
        BootVerification, OperationKind, OperationState, OperationStateRecord, ReleaseIdentity,
        STATE_SCHEMA_VERSION,
    };
    use std::fs;

    #[test]
    fn supervisor_preserves_probe_diagnosis_and_bounds_a_hung_process() {
        use std::os::unix::process::ExitStatusExt;
        use std::path::Path;
        use std::time::{Duration, Instant};
        let status = bounded_status(
            Path::new("/bin/sh"),
            &["-c", "exit 16"],
            Duration::from_secs(2),
        )
        .unwrap();
        assert_eq!(
            CheckFailure::from_status(status),
            Some(CheckFailure::FailedUnits)
        );
        let start = Instant::now();
        let status = bounded_status(
            Path::new("/bin/sh"),
            &["-c", "sleep 20"],
            Duration::from_millis(100),
        )
        .unwrap();
        assert_eq!(
            CheckFailure::from_status(status),
            Some(CheckFailure::Timeout)
        );
        assert!(start.elapsed() < Duration::from_secs(7));
        assert_eq!(
            CheckFailure::from_status(std::process::ExitStatus::from_raw(127 << 8)),
            Some(CheckFailure::Worker)
        );
        assert_eq!(
            CheckFailure::from_status(std::process::ExitStatus::from_raw(9)),
            Some(CheckFailure::Timeout)
        );
    }

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
