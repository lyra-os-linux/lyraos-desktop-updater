use std::io::{BufRead, BufReader, Read};
use std::process::{Command, Output, Stdio};
use std::{fs, io::Write};

use lyra_upgrade_core::{
    OperationState, OperationStateRecord, PersistenceError, ReleaseManifest, save_state,
};

use crate::planner::{PlannerError, plan_update_with_cached_metadata};
use lyra_upgrade_protocol::PlannedUpdate;

const ZYPPER_UPDATE_POLICY: &[&str] = &[
    "--no-allow-downgrade",
    "--no-allow-name-change",
    "--no-allow-arch-change",
    "--no-allow-vendor-change",
];

#[derive(Debug)]
pub enum ExecutionError {
    PlanChanged,
    Refresh(CommandFailure),
    Replan(PlannerError),
    Download(CommandFailure),
    Snapshot(CommandFailure),
    InvalidSnapshot,
    Persist(PersistenceError),
    Apply(CommandFailure),
    PackageManagerRestart(CommandFailure),
    Initramfs(CommandFailure),
    Grub(CommandFailure),
    Transition(lyra_upgrade_core::TransitionError),
    Stage(std::io::Error),
    SystemUpdateExists,
    Busy,
    Cancelled,
}

impl ExecutionError {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::PlanChanged => "PLAN_CHANGED",
            Self::Refresh(_) => "METADATA_REFRESH_FAILED",
            Self::Replan(_) => "PLAN_REVALIDATION_FAILED",
            Self::Download(_) => "DOWNLOAD_FAILED",
            Self::Snapshot(_) | Self::InvalidSnapshot => "SNAPSHOT_FAILED",
            Self::Persist(_) => "STATE_WRITE_FAILED",
            Self::Apply(_) => "ZYPPER_APPLY_FAILED",
            Self::PackageManagerRestart(_) => "ZYPPER_RESTART_REQUIRED",
            Self::Initramfs(_) => "INITRAMFS_FAILED",
            Self::Grub(_) => "BOOTLOADER_FAILED",
            Self::Transition(_) => "INVALID_STATE_TRANSITION",
            Self::Stage(_) => "OFFLINE_STAGE_FAILED",
            Self::SystemUpdateExists => "SYSTEM_UPDATE_BUSY",
            Self::Busy => "TRANSACTION_BUSY",
            Self::Cancelled => "CANCELLED",
        }
    }

    pub const fn is_blocking(&self) -> bool {
        matches!(
            self,
            Self::PlanChanged | Self::Refresh(_) | Self::Replan(_) | Self::Download(_) | Self::Busy
        )
    }
}

pub const fn failure_state(error: &ExecutionError, has_snapshot: bool) -> OperationState {
    if error.is_blocking() && !has_snapshot {
        OperationState::Blocked
    } else if has_snapshot {
        OperationState::NeedsRecovery
    } else {
        OperationState::Failed
    }
}

struct TransactionLock(std::fs::File);

impl TransactionLock {
    fn acquire() -> Result<Self, ExecutionError> {
        Self::acquire_at(std::path::Path::new("/run/lock/lyra-upgrade.lock"))
    }

    fn acquire_at(path: &std::path::Path) -> Result<Self, ExecutionError> {
        use std::os::fd::AsRawFd;
        use std::os::unix::fs::OpenOptionsExt;
        let file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(path)
            .map_err(ExecutionError::Stage)?;
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Err(ExecutionError::Busy);
        }
        Ok(Self(file))
    }
}

impl Drop for TransactionLock {
    fn drop(&mut self) {
        use std::os::fd::AsRawFd;
        unsafe { libc::flock(self.0.as_raw_fd(), libc::LOCK_UN) };
    }
}

pub fn stage_release_upgrade(
    state_root: &std::path::Path,
    state: &mut OperationStateRecord,
    confirmed: &PlannedUpdate,
    manifest: &ReleaseManifest,
    observer: &impl ExecutionObserver,
) -> Result<ExecutionOutcome, ExecutionError> {
    let _transaction_lock = TransactionLock::acquire()?;
    if state.operation != lyra_upgrade_core::OperationKind::ReleaseUpgrade
        || confirmed.plan.target.as_ref() != Some(&manifest.target)
        || confirmed.plan.manifest_sha256.as_ref() != state.manifest_sha256.as_ref()
    {
        return Err(ExecutionError::PlanChanged);
    }
    let operation_dir = state_root.join(&state.operation_id);
    let repos_dir = operation_dir.join("repos.d");
    let cache_dir = operation_dir.join("cache");
    let raw_dir = cache_dir.join("raw");
    let solv_dir = cache_dir.join("solv");
    let packages_dir = cache_dir.join("packages");
    fs::create_dir_all(&repos_dir).map_err(ExecutionError::Stage)?;
    fs::create_dir_all(&packages_dir).map_err(ExecutionError::Stage)?;
    for repository in &manifest.repositories {
        let content = format!(
            "[{alias}]\nname={alias}\nenabled=1\nautorefresh=0\nkeeppackages=0\nbaseurl={url}\ntype=rpm-md\ngpgcheck=1\npriority={priority}\n",
            alias = repository.alias,
            url = repository.base_url,
            priority = repository.priority,
        );
        write_private(
            &repos_dir.join(format!("{}.repo", repository.alias)),
            content.as_bytes(),
        )?;
    }
    write_private(
        &operation_dir.join("manifest.json"),
        &serde_json::to_vec_pretty(manifest)
            .map_err(|error| ExecutionError::Stage(std::io::Error::other(error)))?,
    )?;
    write_private(
        &operation_dir.join("plan.json"),
        &serde_json::to_vec_pretty(&confirmed.plan)
            .map_err(|error| ExecutionError::Stage(std::io::Error::other(error)))?,
    )?;

    state
        .transition_to(OperationState::Downloading)
        .map_err(ExecutionError::Transition)?;
    persist(state_root, state, observer)?;
    let common = [
        "--non-interactive",
        "--reposd-dir",
        repos_dir.to_str().ok_or_else(|| {
            ExecutionError::Stage(std::io::Error::other("non-UTF8 repository path"))
        })?,
        "--cache-dir",
        cache_dir
            .to_str()
            .ok_or_else(|| ExecutionError::Stage(std::io::Error::other("non-UTF8 cache path")))?,
        "--raw-cache-dir",
        raw_dir.to_str().ok_or_else(|| {
            ExecutionError::Stage(std::io::Error::other("non-UTF8 raw cache path"))
        })?,
        "--solv-cache-dir",
        solv_dir.to_str().ok_or_else(|| {
            ExecutionError::Stage(std::io::Error::other("non-UTF8 solv cache path"))
        })?,
        "--pkg-cache-dir",
        packages_dir.to_str().ok_or_else(|| {
            ExecutionError::Stage(std::io::Error::other("non-UTF8 package cache path"))
        })?,
    ];
    let mut refresh_args = common.to_vec();
    refresh_args.push("refresh");
    let refresh = run_observed("zypper", &refresh_args, observer);
    require_success("zypper", refresh).map_err(ExecutionError::Refresh)?;
    if observer.cancel_requested() {
        return Err(ExecutionError::Cancelled);
    }
    let mut download_args = common.to_vec();
    download_args.extend([
        "--xmlout",
        "dist-upgrade",
        "--download-only",
        "--details",
        "--no-allow-downgrade",
        "--no-allow-name-change",
        "--no-allow-arch-change",
        "--allow-vendor-change",
    ]);
    let download = run_observed("zypper", &download_args, observer);
    require_success("zypper", download).map_err(ExecutionError::Download)?;
    if observer.cancel_requested() {
        return Err(ExecutionError::Cancelled);
    }

    state
        .transition_to(OperationState::Snapshotting)
        .map_err(ExecutionError::Transition)?;
    persist(state_root, state, observer)?;
    let snapshot = run_observed(
        "snapper",
        &[
            "--no-dbus",
            "--config",
            "root",
            "create",
            "--read-only",
            "--description",
            "Lyra release upgrade",
            "--print-number",
        ],
        observer,
    );
    let snapshot = require_success("snapper", snapshot).map_err(ExecutionError::Snapshot)?;
    let snapshot_stdout = snapshot.stdout();
    state.snapshot_number = Some(parse_snapshot_number(&snapshot_stdout)?);
    save_state(state_root, state).map_err(ExecutionError::Persist)?;
    stage_system_update(&operation_dir)?;
    state
        .transition_to(OperationState::ReadyToReboot)
        .map_err(ExecutionError::Transition)?;
    persist(state_root, state, observer)?;
    Ok(ExecutionOutcome {
        reboot_required: true,
        package_manager_restart: false,
    })
}

fn write_private(path: &std::path::Path, content: &[u8]) -> Result<(), ExecutionError> {
    use std::os::unix::fs::OpenOptionsExt;
    let temporary = path.with_extension("tmp");
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(&temporary)
        .map_err(ExecutionError::Stage)?;
    file.write_all(content).map_err(ExecutionError::Stage)?;
    file.write_all(b"\n").map_err(ExecutionError::Stage)?;
    file.sync_all().map_err(ExecutionError::Stage)?;
    fs::rename(temporary, path).map_err(ExecutionError::Stage)?;
    Ok(())
}

fn stage_system_update(operation_dir: &std::path::Path) -> Result<(), ExecutionError> {
    use std::os::unix::fs::symlink;
    let marker = std::path::Path::new("/system-update");
    match fs::symlink_metadata(marker) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            let target = fs::read_link(marker).map_err(ExecutionError::Stage)?;
            if target != operation_dir {
                return Err(ExecutionError::SystemUpdateExists);
            }
            Ok(())
        }
        Ok(_) => Err(ExecutionError::SystemUpdateExists),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            symlink(operation_dir, marker).map_err(ExecutionError::Stage)
        }
        Err(error) => Err(ExecutionError::Stage(error)),
    }
}

#[derive(Debug)]
pub struct CommandFailure {
    pub program: &'static str,
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExecutionOutcome {
    pub reboot_required: bool,
    pub package_manager_restart: bool,
}

pub trait ExecutionObserver: Send + Sync {
    fn command_line(&self, program: &'static str, stream: OutputStream, line: &str);
    fn state_changed(&self, state: OperationState);
    fn cancel_requested(&self) -> bool;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OutputStream {
    Stdout,
    Stderr,
}

pub fn execute_update(
    state_root: &std::path::Path,
    state: &mut OperationStateRecord,
    confirmed: &PlannedUpdate,
    observer: &impl ExecutionObserver,
) -> Result<ExecutionOutcome, ExecutionError> {
    let _transaction_lock = TransactionLock::acquire()?;
    state
        .transition_to(OperationState::Downloading)
        .map_err(ExecutionError::Transition)?;
    persist(state_root, state, observer)?;
    let refresh = run_observed("zypper", &["--non-interactive", "refresh"], observer);
    require_success("zypper", refresh).map_err(ExecutionError::Refresh)?;
    if observer.cancel_requested() {
        return Err(ExecutionError::Cancelled);
    }

    let fresh = plan_update_with_cached_metadata().map_err(ExecutionError::Replan)?;
    if fresh.plan_sha256 != confirmed.plan_sha256 {
        return Err(ExecutionError::PlanChanged);
    }

    let mut download_args = vec![
        "--xmlout",
        "--non-interactive",
        "--no-refresh",
        "update",
        "--download-only",
        "--details",
    ];
    download_args.extend_from_slice(ZYPPER_UPDATE_POLICY);
    let download = run_observed("zypper", &download_args, observer);
    require_success("zypper", download).map_err(ExecutionError::Download)?;
    if observer.cancel_requested() {
        return Err(ExecutionError::Cancelled);
    }

    state
        .transition_to(OperationState::Snapshotting)
        .map_err(ExecutionError::Transition)?;
    persist(state_root, state, observer)?;
    let snapshot = run_observed(
        "snapper",
        &[
            "--no-dbus",
            "--config",
            "root",
            "create",
            "--read-only",
            "--description",
            "Lyra Upgrade",
            "--print-number",
        ],
        observer,
    );
    let snapshot = require_success("snapper", snapshot).map_err(ExecutionError::Snapshot)?;
    let snapshot_stdout = snapshot.stdout();
    let snapshot_number = parse_snapshot_number(&snapshot_stdout)?;
    state.snapshot_number = Some(snapshot_number);
    save_state(state_root, state).map_err(ExecutionError::Persist)?;

    state
        .transition_to(OperationState::Applying)
        .map_err(ExecutionError::Transition)?;
    persist(state_root, state, observer)?;
    let mut apply_args = vec![
        "--xmlout",
        "--non-interactive",
        "--no-refresh",
        "update",
        "--details",
    ];
    apply_args.extend_from_slice(ZYPPER_UPDATE_POLICY);
    let apply = run_observed("zypper", &apply_args, observer);
    let code = apply.status.code();
    match classify_apply_exit(code) {
        Ok(_) => {}
        Err(ApplyExitError::RestartPackageManager) => {
            return Err(ExecutionError::PackageManagerRestart(failure(
                "zypper", apply,
            )));
        }
        Err(ApplyExitError::Failed) => {
            return Err(ExecutionError::Apply(failure("zypper", apply)));
        }
    }

    let touches_boot = fresh.solver.changes.iter().any(|change| {
        change.name.starts_with("kernel-")
            || matches!(
                change.name.as_str(),
                "dracut" | "shim" | "grub2" | "grub2-x86_64-efi"
            )
    });
    if touches_boot {
        let dracut = run_observed("dracut", &["--regenerate-all", "--force"], observer);
        require_success("dracut", dracut).map_err(ExecutionError::Initramfs)?;
        let grub = run_observed("grub2-mkconfig", &["-o", "/boot/grub2/grub.cfg"], observer);
        require_success("grub2-mkconfig", grub).map_err(ExecutionError::Grub)?;
    }

    let reboot_required = fresh.solver.reboot_required || code == Some(102) || touches_boot;
    let package_manager_restart = false;
    let next = if reboot_required {
        OperationState::AwaitingReboot
    } else {
        OperationState::Completed
    };
    state
        .transition_to(next)
        .map_err(ExecutionError::Transition)?;
    persist(state_root, state, observer)?;
    Ok(ExecutionOutcome {
        reboot_required,
        package_manager_restart,
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ApplyExit {
    Completed,
    RebootRequired,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ApplyExitError {
    RestartPackageManager,
    Failed,
}

fn classify_apply_exit(code: Option<i32>) -> Result<ApplyExit, ApplyExitError> {
    match code {
        Some(0) => Ok(ApplyExit::Completed),
        Some(102) => Ok(ApplyExit::RebootRequired),
        Some(103) => Err(ApplyExitError::RestartPackageManager),
        _ => Err(ApplyExitError::Failed),
    }
}

fn parse_snapshot_number(stdout: &str) -> Result<u64, ExecutionError> {
    stdout
        .lines()
        .rev()
        .find_map(|line| line.trim().parse::<u64>().ok())
        .filter(|number| *number > 0)
        .ok_or(ExecutionError::InvalidSnapshot)
}

fn persist(
    root: &std::path::Path,
    state: &OperationStateRecord,
    observer: &impl ExecutionObserver,
) -> Result<(), ExecutionError> {
    save_state(root, state).map_err(ExecutionError::Persist)?;
    observer.state_changed(state.state);
    Ok(())
}

fn run_observed(
    program: &'static str,
    arguments: &[&str],
    observer: &impl ExecutionObserver,
) -> Output {
    let child = Command::new(program)
        .args(arguments)
        .env_clear()
        .env("PATH", "/usr/sbin:/usr/bin:/sbin:/bin")
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();
    let Ok(mut child) = child else {
        let error = child.expect_err("checked failed spawn");
        observer.command_line(program, OutputStream::Stderr, &error.to_string());
        return Output {
            status: synthetic_failure_status(),
            stdout: Vec::new(),
            stderr: error.to_string().into_bytes(),
        };
    };
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let (stdout, stderr) = std::thread::scope(|scope| {
        let out = scope.spawn(|| collect_stream(stdout, program, OutputStream::Stdout, observer));
        let err = scope.spawn(|| collect_stream(stderr, program, OutputStream::Stderr, observer));
        (
            out.join().unwrap_or_default(),
            err.join().unwrap_or_default(),
        )
    });
    let status = child.wait().unwrap_or_else(|_| synthetic_failure_status());
    Output {
        status,
        stdout,
        stderr,
    }
}

fn collect_stream(
    stream: Option<impl Read>,
    program: &'static str,
    kind: OutputStream,
    observer: &impl ExecutionObserver,
) -> Vec<u8> {
    let Some(stream) = stream else {
        return Vec::new();
    };
    let mut bytes = Vec::new();
    for line in BufReader::new(stream).split(b'\n') {
        match line {
            Ok(line) => {
                observer.command_line(program, kind, &String::from_utf8_lossy(&line));
                bytes.extend_from_slice(&line);
                bytes.push(b'\n');
            }
            Err(error) => {
                observer.command_line(program, OutputStream::Stderr, &error.to_string());
                break;
            }
        }
    }
    bytes
}

#[cfg(unix)]
fn synthetic_failure_status() -> std::process::ExitStatus {
    use std::os::unix::process::ExitStatusExt;
    std::process::ExitStatus::from_raw(127 << 8)
}

fn require_success(program: &'static str, output: Output) -> Result<Output, CommandFailure> {
    if output.status.success() {
        Ok(output)
    } else {
        Err(failure(program, output))
    }
}

fn failure(program: &'static str, output: Output) -> CommandFailure {
    CommandFailure {
        program,
        code: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

trait OutputText {
    fn stdout(&self) -> String;
}

impl OutputText for Output {
    fn stdout(&self) -> String {
        String::from_utf8_lossy(&self.stdout).into_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_supported_zypper_exit_codes() {
        assert_eq!(classify_apply_exit(Some(0)), Ok(ApplyExit::Completed));
        assert_eq!(
            classify_apply_exit(Some(102)),
            Ok(ApplyExit::RebootRequired)
        );
        assert_eq!(
            classify_apply_exit(Some(103)),
            Err(ApplyExitError::RestartPackageManager)
        );
        assert_eq!(classify_apply_exit(Some(4)), Err(ApplyExitError::Failed));
        assert_eq!(classify_apply_exit(None), Err(ApplyExitError::Failed));
    }

    #[test]
    fn parses_only_positive_snapper_numbers() {
        assert_eq!(
            parse_snapshot_number("Creating snapshot\n42\n").unwrap(),
            42
        );
        assert!(matches!(
            parse_snapshot_number("snapshot failed\n"),
            Err(ExecutionError::InvalidSnapshot)
        ));
        assert!(matches!(
            parse_snapshot_number("0\n"),
            Err(ExecutionError::InvalidSnapshot)
        ));
    }

    #[test]
    fn transaction_lock_rejects_a_concurrent_holder() {
        let path =
            std::env::temp_dir().join(format!("lyra-upgrade-lock-test-{}", std::process::id()));
        let first = TransactionLock::acquire_at(&path).unwrap();
        assert!(matches!(
            TransactionLock::acquire_at(&path),
            Err(ExecutionError::Busy)
        ));
        drop(first);
        let second = TransactionLock::acquire_at(&path).unwrap();
        drop(second);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn preserves_stable_failure_codes_and_blocking_boundary() {
        let busy = ExecutionError::Busy;
        assert_eq!(busy.code(), "TRANSACTION_BUSY");
        assert!(busy.is_blocking());
        let invalid_snapshot = ExecutionError::InvalidSnapshot;
        assert_eq!(invalid_snapshot.code(), "SNAPSHOT_FAILED");
        assert!(!invalid_snapshot.is_blocking());
    }

    #[test]
    fn maps_failures_according_to_the_snapshot_boundary() {
        let network = ExecutionError::Refresh(CommandFailure {
            program: "zypper",
            code: Some(4),
            stdout: String::new(),
            stderr: "network unavailable".to_string(),
        });
        assert_eq!(failure_state(&network, false), OperationState::Blocked);
        assert_eq!(
            failure_state(&ExecutionError::Busy, false),
            OperationState::Blocked
        );
        assert_eq!(
            failure_state(&ExecutionError::InvalidSnapshot, false),
            OperationState::Failed
        );

        let apply = ExecutionError::Apply(CommandFailure {
            program: "zypper",
            code: Some(4),
            stdout: String::new(),
            stderr: "apply failed".to_string(),
        });
        assert_eq!(failure_state(&apply, true), OperationState::NeedsRecovery);
        let dracut = ExecutionError::Initramfs(CommandFailure {
            program: "dracut",
            code: Some(1),
            stdout: String::new(),
            stderr: "initramfs failed".to_string(),
        });
        assert_eq!(failure_state(&dracut, true), OperationState::NeedsRecovery);
    }
}
