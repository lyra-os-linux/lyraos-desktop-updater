use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::os::unix::fs::FileTypeExt;
use std::path::Path;
use std::process::{Command, Stdio};

use lyra_upgrade_core::{OperationState, load_state};
use serde::Serialize;

const STATE_ROOT: &str = "/var/lib/lyra-upgrade/operations";
const VIRTIO_EVIDENCE_PORT: &str = "/dev/virtio-ports/org.lyraos.UpgradeEvidence";

#[derive(Debug, Serialize)]
struct GuestEvidence {
    schema: u32,
    status: &'static str,
    mode: &'static str,
    installation_uuid: String,
    boot_id: String,
    session: &'static str,
    release: ReleaseEvidence,
    upgrade: UpgradeEvidence,
}

#[derive(Debug, Serialize)]
struct ReleaseEvidence {
    id: String,
    version_id: String,
    edition: String,
    architecture: String,
    build_id: String,
}

#[derive(Debug, Serialize)]
struct UpgradeEvidence {
    package_version: Option<String>,
    operation_id: Option<String>,
    operation_state: Option<OperationState>,
    operation_sequence: Option<u64>,
    source_version: Option<String>,
    target_version: Option<String>,
    snapshot_recorded: Option<bool>,
}

fn main() {
    let arguments = std::env::args().collect::<Vec<_>>();
    let emit_virtio = match arguments.as_slice() {
        [_] => false,
        [_, argument] if argument == "--emit-virtio" => true,
        _ => {
            eprintln!("usage: lyra-upgrade-probe [--emit-virtio]");
            std::process::exit(2);
        }
    };
    match collect(Path::new("/"), Path::new(STATE_ROOT), package_version()) {
        Ok(evidence) => {
            if emit_virtio {
                if let Err(error) = emit_to_virtio(&evidence, Path::new(VIRTIO_EVIDENCE_PORT)) {
                    eprintln!("lyra-upgrade-probe: {error}");
                    std::process::exit(1);
                }
            } else {
                println!("{}", serde_json::to_string_pretty(&evidence).unwrap());
            }
        }
        Err(error) => {
            eprintln!("lyra-upgrade-probe: {error}");
            std::process::exit(1);
        }
    }
}

fn emit_to_virtio(evidence: &GuestEvidence, path: &Path) -> Result<(), String> {
    if !path.exists() {
        return Ok(());
    }
    let metadata = fs::symlink_metadata(path).map_err(|_| "cannot inspect virtio evidence port")?;
    if !metadata.file_type().is_char_device() {
        return Err("virtio evidence port is not a character device".into());
    }
    let mut port = fs::OpenOptions::new()
        .write(true)
        .open(path)
        .map_err(|_| "cannot open virtio evidence port")?;
    serde_json::to_writer(&mut port, evidence).map_err(|_| "cannot serialize guest evidence")?;
    port.write_all(b"\n")
        .map_err(|_| "cannot write guest evidence")?;
    port.flush()
        .map_err(|_| "cannot flush guest evidence".to_string())
}

fn collect(
    root: &Path,
    state_root: &Path,
    package_version: Option<String>,
) -> Result<GuestEvidence, String> {
    let os_release = parse_os_release(&read_small(&root.join("etc/os-release"))?);
    let product_release = read_small(&root.join("usr/lib/lyra-os/product-release"))?;
    let installation_uuid = read_small(&root.join("sys/class/dmi/id/product_uuid"))?.to_lowercase();
    let boot_id = read_small(&root.join("proc/sys/kernel/random/boot_id"))?.to_lowercase();
    validate_uuid(&installation_uuid)?;
    validate_uuid(&boot_id)?;
    let latest = latest_operation(state_root);
    Ok(GuestEvidence {
        schema: 1,
        status: "observed",
        mode: "guest-upgrade-state",
        installation_uuid,
        boot_id,
        session: if root.join("run/overlay/live").exists() {
            "live"
        } else {
            "installed"
        },
        release: ReleaseEvidence {
            id: required(&os_release, "ID")?,
            version_id: product_fact(&product_release, "LYRA_VERSION_ID")?,
            edition: product_fact(&product_release, "LYRA_EDITION")?,
            architecture: product_fact(&product_release, "LYRA_ARCHITECTURE")?,
            build_id: product_fact(&product_release, "LYRA_BUILD_ID")?,
        },
        upgrade: UpgradeEvidence {
            package_version,
            operation_id: latest.as_ref().map(|state| state.operation_id.clone()),
            operation_state: latest.as_ref().map(|state| state.state),
            operation_sequence: latest.as_ref().map(|state| state.sequence),
            source_version: latest.as_ref().map(|state| state.source.version.clone()),
            target_version: latest
                .as_ref()
                .and_then(|state| state.target.as_ref().map(|target| target.version.clone())),
            snapshot_recorded: latest.as_ref().map(|state| state.snapshot_number.is_some()),
        },
    })
}

fn read_small(path: &Path) -> Result<String, String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|_| format!("missing required fact: {}", path.display()))?;
    if !metadata.file_type().is_file() || metadata.len() > 64 * 1024 {
        return Err(format!("unsafe required fact: {}", path.display()));
    }
    fs::read_to_string(path)
        .map(|value| value.trim().to_string())
        .map_err(|_| format!("unreadable required fact: {}", path.display()))
}

fn parse_os_release(input: &str) -> BTreeMap<String, String> {
    input
        .lines()
        .filter_map(|line| {
            let (key, value) = line.split_once('=')?;
            Some((key.to_string(), value.trim_matches('"').to_string()))
        })
        .collect()
}

fn required(values: &BTreeMap<String, String>, key: &str) -> Result<String, String> {
    values
        .get(key)
        .filter(|value| !value.is_empty())
        .cloned()
        .ok_or_else(|| format!("missing {key} in os-release"))
}

fn product_fact(input: &str, key: &str) -> Result<String, String> {
    let prefix = format!("{key}='");
    let value = input
        .lines()
        .find_map(|line| line.strip_prefix(&prefix)?.strip_suffix('\''))
        .filter(|value| {
            !value.is_empty()
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
        })
        .ok_or_else(|| format!("missing or invalid {key} in product-release"))?;
    Ok(value.to_string())
}

fn validate_uuid(value: &str) -> Result<(), String> {
    let parts: Vec<_> = value.split('-').collect();
    if parts.iter().map(|part| part.len()).eq([8, 4, 4, 4, 12])
        && parts
            .iter()
            .all(|part| part.chars().all(|character| character.is_ascii_hexdigit()))
    {
        Ok(())
    } else {
        Err("malformed UUID fact".into())
    }
}

fn latest_operation(root: &Path) -> Option<lyra_upgrade_core::OperationStateRecord> {
    let mut states = fs::read_dir(root)
        .ok()?
        .filter_map(|entry| {
            let entry = entry.ok()?;
            if !entry.file_type().ok()?.is_dir() {
                return None;
            }
            load_state(root, entry.file_name().to_str()?).ok()
        })
        .collect::<Vec<_>>();
    states.sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
    states.into_iter().next()
}

fn package_version() -> Option<String> {
    let output = Command::new("rpm")
        .args(["-q", "--qf", "%{VERSION}-%{RELEASE}", "lyra-upgrade"])
        .env_clear()
        .env("PATH", "/usr/bin:/usr/sbin")
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    #[test]
    fn probe_emits_only_fixed_sanitized_guest_facts() {
        let root = std::env::temp_dir().join(format!("lyra-upgrade-probe-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("etc")).unwrap();
        fs::create_dir_all(root.join("usr/lib/lyra-os")).unwrap();
        fs::create_dir_all(root.join("sys/class/dmi/id")).unwrap();
        fs::create_dir_all(root.join("proc/sys/kernel/random")).unwrap();
        fs::write(
            root.join("etc/os-release"),
            "ID=lyra-os\nVERSION_ID=1.0-alpha.7\nPRETTY_NAME=secret\n",
        )
        .unwrap();
        fs::write(
            root.join("usr/lib/lyra-os/product-release"),
            "LYRA_VERSION_ID='1.0'\nLYRA_EDITION='desktop'\nLYRA_ARCHITECTURE='x86_64'\nLYRA_BUILD_ID='lyra-release-1.0'\n",
        )
        .unwrap();
        fs::write(
            root.join("sys/class/dmi/id/product_uuid"),
            "12345678-1234-4234-8234-123456789ABC\n",
        )
        .unwrap();
        fs::write(
            root.join("proc/sys/kernel/random/boot_id"),
            "87654321-4321-4321-8321-CBA987654321\n",
        )
        .unwrap();
        let evidence = collect(&root, &root.join("state"), Some("0.2.0-1".into())).unwrap();
        let json = serde_json::to_value(evidence).unwrap();
        assert_eq!(json["status"], "observed");
        assert_eq!(json["session"], "installed");
        assert_eq!(json["release"]["version_id"], "1.0");
        assert_eq!(json["release"]["build_id"], "lyra-release-1.0");
        assert!(json.to_string().find("secret").is_none());

        let target = root.join("uuid-target");
        fs::write(&target, "12345678-1234-4234-8234-123456789abc").unwrap();
        fs::remove_file(root.join("sys/class/dmi/id/product_uuid")).unwrap();
        symlink(target, root.join("sys/class/dmi/id/product_uuid")).unwrap();
        assert!(collect(&root, &root.join("state"), None).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn virtio_emission_is_optional_and_rejects_regular_files() {
        let root =
            std::env::temp_dir().join(format!("lyra-upgrade-probe-port-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir(&root).unwrap();
        let evidence = GuestEvidence {
            schema: 1,
            status: "observed",
            mode: "guest-upgrade-state",
            installation_uuid: "12345678-1234-4234-8234-123456789abc".into(),
            boot_id: "87654321-4321-4321-8321-cba987654321".into(),
            session: "installed",
            release: ReleaseEvidence {
                id: "lyra-os".into(),
                version_id: "1.0".into(),
                edition: "desktop".into(),
                architecture: "x86_64".into(),
                build_id: "lyra-release-1.0".into(),
            },
            upgrade: UpgradeEvidence {
                package_version: None,
                operation_id: None,
                operation_state: None,
                operation_sequence: None,
                source_version: None,
                target_version: None,
                snapshot_recorded: None,
            },
        };
        assert!(emit_to_virtio(&evidence, &root.join("absent")).is_ok());
        fs::write(root.join("regular"), b"").unwrap();
        assert!(emit_to_virtio(&evidence, &root.join("regular")).is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
