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
    state.sequence = state.sequence.saturating_add(1);
    if passed {
        state.state = OperationState::Completed;
        state.boot_verification = Some(BootVerification::Passed);
        state.last_completed_step = Some("post-boot-verification".into());
        state.error_code = None;
        if state.operation == lyra_upgrade_core::OperationKind::ReleaseUpgrade
            && let Some(sequence) = read_manifest_sequence(&operation_dir)
        {
            let _ = write_sequence(sequence);
        }
    } else {
        state.state = OperationState::NeedsRecovery;
        state.boot_verification = Some(BootVerification::Failed);
        state.error_code = Some("POST_BOOT_VERIFICATION_FAILED".into());
    }
    let _ = save_state(Path::new(STATE_ROOT), &state);
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
    let contents = fs::read(operation_dir.join("manifest.json")).ok()?;
    let marker = b"\"sequence\"";
    let start = contents
        .windows(marker.len())
        .position(|window| window == marker)?
        + marker.len();
    let tail = std::str::from_utf8(&contents[start..]).ok()?;
    tail.trim_start_matches(|character: char| character.is_whitespace() || character == ':')
        .split(|character: char| !character.is_ascii_digit())
        .next()?
        .parse()
        .ok()
}

fn write_sequence(sequence: u64) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let path = Path::new("/var/lib/lyra-upgrade/last-manifest-sequence");
    let temporary = path.with_extension("tmp");
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temporary)?;
    writeln!(file, "{sequence}")?;
    file.sync_all()?;
    fs::rename(temporary, path)
}
