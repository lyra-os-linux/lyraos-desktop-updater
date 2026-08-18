use std::fs;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use lyra_upgrade_core::{
    OperationState, PreflightPolicy, ReleaseManifest, SystemBackend, UpgradePlan, build_plan,
    discover_host, evaluate_solver_preflight, load_state, save_state,
};
use lyra_upgrade_service::solver_xml::parse_solver_xml;
use sha2::{Digest, Sha256};

const STATE_ROOT: &str = "/var/lib/lyra-upgrade/operations";

fn main() {
    if let Err(error) = run() {
        eprintln!("lyra-upgrade-offline: {error}");
        mark_recovery();
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let operation_dir = resolve_operation_dir()?;
    let operation_id = operation_dir
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("invalid operation id")?;
    let mut state = load_state(Path::new(STATE_ROOT), operation_id)
        .map_err(|_| "cannot load operation state")?;
    if state.state != OperationState::ReadyToReboot || state.snapshot_number.is_none() {
        return Err("operation is not ready for offline application".into());
    }
    let manifest: ReleaseManifest = read_json(&operation_dir.join("manifest.json"))?;
    let confirmed_plan: UpgradePlan = read_json(&operation_dir.join("plan.json"))?;
    if confirmed_plan.sha256().map_err(|_| "cannot hash plan")? != state.plan_sha256 {
        return Err("persisted plan hash mismatch".into());
    }
    let manifest_hash = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&manifest).map_err(|_| "cannot hash manifest")?)
    );
    if state.manifest_sha256.as_deref() != Some(&manifest_hash) {
        return Err("persisted manifest hash mismatch".into());
    }

    state
        .transition_to(OperationState::ApplyingOffline)
        .map_err(|_| "invalid offline transition")?;
    save_state(Path::new(STATE_ROOT), &state).map_err(|_| "cannot persist offline state")?;
    install_repository_set(&manifest, operation_id)?;
    revalidate_plan(
        &operation_dir,
        &manifest,
        &confirmed_plan,
        &state.plan_sha256,
    )?;

    let packages = operation_dir.join("cache/packages");
    let output = command(
        "zypper",
        &[
            "--xmlout",
            "--non-interactive",
            "--no-refresh",
            "--pkg-cache-dir",
            packages.to_str().ok_or("non-UTF8 package cache")?,
            "dist-upgrade",
            "--details",
            "--no-allow-downgrade",
            "--no-allow-name-change",
            "--no-allow-arch-change",
            "--allow-vendor-change",
        ],
    );
    if !matches!(output.status.code(), Some(0 | 102 | 103)) {
        return Err(format!(
            "zypper dup failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    require_success(
        command("dracut", &["--regenerate-all", "--force"]),
        "dracut",
    )?;
    require_success(
        command("grub2-mkconfig", &["-o", "/boot/grub2/grub.cfg"]),
        "grub2-mkconfig",
    )?;

    state
        .transition_to(OperationState::AwaitingReboot)
        .map_err(|_| "invalid post-apply transition")?;
    state.last_completed_step = Some("offline-apply".into());
    save_state(Path::new(STATE_ROOT), &state).map_err(|_| "cannot persist result")?;
    remove_system_update_marker()?;
    Ok(())
}

fn revalidate_plan(
    operation_dir: &Path,
    manifest: &ReleaseManifest,
    confirmed: &UpgradePlan,
    expected_hash: &str,
) -> Result<(), String> {
    let dry_run = command(
        "zypper",
        &[
            "--xmlout",
            "--non-interactive",
            "--no-refresh",
            "--pkg-cache-dir",
            operation_dir
                .join("cache/packages")
                .to_str()
                .ok_or("non-UTF8 package cache")?,
            "dist-upgrade",
            "--dry-run",
            "--details",
            "--no-allow-downgrade",
            "--no-allow-name-change",
            "--no-allow-arch-change",
            "--allow-vendor-change",
        ],
    );
    if !dry_run.status.success() {
        return Err("offline dry-run failed".into());
    }
    let facts = discover_host(&SystemBackend).map_err(|_| "offline discovery failed")?;
    let metadata = manifest
        .repositories
        .iter()
        .map(|repository| repository.alias.clone())
        .collect();
    let solver = parse_solver_xml(&String::from_utf8_lossy(&dry_run.stdout), metadata, 0)
        .map_err(|_| "cannot parse offline solver result")?;
    let report = evaluate_solver_preflight(
        &facts,
        PreflightPolicy::default(),
        &solver,
        &manifest.solver_policy(),
    );
    if !report.passed() {
        return Err("offline preflight blocked".into());
    }
    let rebuilt = build_plan(
        lyra_upgrade_core::OperationKind::ReleaseUpgrade,
        &facts,
        &report,
        Some(manifest.target.clone()),
        confirmed.manifest_sha256.clone(),
        &solver,
    )
    .map_err(|_| "cannot rebuild offline plan")?;
    if rebuilt.sha256().map_err(|_| "cannot hash offline plan")? != expected_hash {
        return Err("offline plan differs from confirmed plan".into());
    }
    Ok(())
}

fn install_repository_set(manifest: &ReleaseManifest, operation_id: &str) -> Result<(), String> {
    let live = Path::new("/etc/zypp/repos.d");
    let parent = live.parent().ok_or("repository directory has no parent")?;
    let staging = parent.join(format!("repos.d.lyra-new-{operation_id}"));
    let backup = parent.join(format!("repos.d.lyra-backup-{operation_id}"));
    if staging.exists() || backup.exists() {
        return Err("repository staging or backup already exists".into());
    }
    fs::create_dir(&staging).map_err(|error| error.to_string())?;
    fs::set_permissions(&staging, fs::Permissions::from_mode(0o755))
        .map_err(|error| error.to_string())?;
    for repository in &manifest.repositories {
        let content = format!(
            "[{alias}]\nname={alias}\nenabled=1\nautorefresh=1\nbaseurl={url}\ntype=rpm-md\ngpgcheck=1\npriority={priority}\n",
            alias = repository.alias,
            url = repository.base_url,
            priority = repository.priority,
        );
        let path = staging.join(format!("{}.repo", repository.alias));
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true).mode(0o644);
        std::io::Write::write_all(
            &mut options.open(path).map_err(|error| error.to_string())?,
            content.as_bytes(),
        )
        .map_err(|error| error.to_string())?;
    }
    fs::rename(live, &backup).map_err(|error| error.to_string())?;
    if let Err(error) = fs::rename(&staging, live) {
        let _ = fs::rename(&backup, live);
        return Err(error.to_string());
    }
    Ok(())
}

fn resolve_operation_dir() -> Result<PathBuf, String> {
    let marker = Path::new("/system-update");
    let metadata = fs::symlink_metadata(marker).map_err(|error| error.to_string())?;
    if !metadata.file_type().is_symlink() {
        return Err("/system-update is not a symbolic link".into());
    }
    let target = fs::read_link(marker).map_err(|error| error.to_string())?;
    let root = fs::canonicalize(STATE_ROOT).map_err(|error| error.to_string())?;
    let target = fs::canonicalize(target).map_err(|error| error.to_string())?;
    if target.parent() != Some(root.as_path()) {
        return Err("/system-update target is outside the state root".into());
    }
    Ok(target)
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, String> {
    if fs::symlink_metadata(path)
        .map_err(|error| error.to_string())?
        .file_type()
        .is_symlink()
    {
        return Err("refusing symbolic-link state".into());
    }
    let bytes = fs::read(path).map_err(|error| error.to_string())?;
    if bytes.len() > 4 * 1024 * 1024 {
        return Err("state file is too large".into());
    }
    serde_json::from_slice(&bytes).map_err(|error| error.to_string())
}

fn command(program: &str, arguments: &[&str]) -> Output {
    Command::new(program)
        .args(arguments)
        .env_clear()
        .env("PATH", "/usr/sbin:/usr/bin:/sbin:/bin")
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .output()
        .unwrap_or_else(|error| synthetic_output(error.to_string()))
}

fn require_success(output: Output, program: &str) -> Result<(), String> {
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "{program} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ))
    }
}

fn synthetic_output(message: String) -> Output {
    use std::os::unix::process::ExitStatusExt;
    Output {
        status: std::process::ExitStatus::from_raw(127 << 8),
        stdout: Vec::new(),
        stderr: message.into_bytes(),
    }
}

fn remove_system_update_marker() -> Result<(), String> {
    let marker = Path::new("/system-update");
    if fs::symlink_metadata(marker)
        .map_err(|error| error.to_string())?
        .file_type()
        .is_symlink()
    {
        fs::remove_file(marker).map_err(|error| error.to_string())
    } else {
        Err("refusing to remove non-symlink /system-update".into())
    }
}

fn mark_recovery() {
    let Ok(operation_dir) = resolve_operation_dir() else {
        return;
    };
    let Some(operation_id) = operation_dir.file_name().and_then(|name| name.to_str()) else {
        return;
    };
    if let Ok(mut state) = load_state(Path::new(STATE_ROOT), operation_id) {
        state.state = OperationState::NeedsRecovery;
        state.error_code = Some("OFFLINE_APPLY_FAILED".into());
        state.sequence = state.sequence.saturating_add(1);
        let _ = save_state(Path::new(STATE_ROOT), &state);
    }
    let _ = remove_system_update_marker();
}
