use std::ffi::CString;
use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::process::{Command, Stdio};

use crate::{HostFacts, ReleaseIdentity, RepositoryFact};

/// Minimal read-only adapter used by discovery. Production code may only map
/// these calls to fixed commands/files; tests provide an in-memory fixture.
pub trait DiscoveryBackend {
    fn read(&self, path: &Path) -> Result<String, DiscoverError>;
    fn read_dir(&self, path: &Path) -> Result<Vec<String>, DiscoverError>;
    fn available_bytes(&self, path: &Path) -> Result<u64, DiscoverError>;
    fn run(
        &self,
        program: &'static str,
        arguments: &'static [&'static str],
    ) -> Result<CommandOutput, DiscoverError>;
}

/// Production adapter. Its command surface is deliberately closed: adding a
/// new probe requires a source change and review, never input from a client.
pub struct SystemBackend;

impl DiscoveryBackend for SystemBackend {
    fn read(&self, path: &Path) -> Result<String, DiscoverError> {
        fs::read_to_string(path).map_err(|_| DiscoverError::ReadFailed("system-file"))
    }

    fn read_dir(&self, path: &Path) -> Result<Vec<String>, DiscoverError> {
        let mut entries = fs::read_dir(path)
            .map_err(|_| DiscoverError::ReadFailed("system-directory"))?
            .map(|entry| {
                entry
                    .map_err(|_| DiscoverError::ReadFailed("system-directory-entry"))?
                    .file_name()
                    .into_string()
                    .map_err(|_| DiscoverError::ReadFailed("non-utf8-directory-entry"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        entries.sort();
        Ok(entries)
    }

    fn available_bytes(&self, path: &Path) -> Result<u64, DiscoverError> {
        let path = CString::new(path.as_os_str().as_bytes())
            .map_err(|_| DiscoverError::ReadFailed("filesystem-space"))?;
        let mut stats = std::mem::MaybeUninit::<libc::statvfs>::uninit();
        // SAFETY: `path` is a NUL-terminated CString and `stats` points to
        // writable storage of the exact type required by `statvfs`.
        let result = unsafe { libc::statvfs(path.as_ptr(), stats.as_mut_ptr()) };
        if result != 0 {
            return Err(DiscoverError::ReadFailed("filesystem-space"));
        }
        // SAFETY: `statvfs` returned success and initialized the structure.
        let stats = unsafe { stats.assume_init() };
        Ok(stats.f_bavail.saturating_mul(stats.f_frsize))
    }

    fn run(
        &self,
        program: &'static str,
        arguments: &'static [&'static str],
    ) -> Result<CommandOutput, DiscoverError> {
        if !allowed_probe(program, arguments) {
            return Err(DiscoverError::CommandNotAllowed);
        }
        let output = Command::new(program)
            .args(arguments)
            .env_clear()
            .env("PATH", "/usr/sbin:/usr/bin:/sbin:/bin")
            .env("LC_ALL", "C")
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .map_err(|_| DiscoverError::CommandFailed(program))?;
        Ok(CommandOutput {
            success: output.status.success(),
            stdout: String::from_utf8(output.stdout)
                .map_err(|_| DiscoverError::CommandFailed(program))?,
        })
    }
}

fn allowed_probe(program: &str, arguments: &[&str]) -> bool {
    matches!(
        (program, arguments),
        ("findmnt", ["--noheadings", "--output", "FSTYPE", "/"])
            | ("snapper", ["--no-dbus", "--config", "root", "get-config"])
            | ("rpm", ["--verifydb"])
            | ("mokutil", ["--sb-state"])
            | (
                "zypper",
                ["--non-interactive", "--no-refresh", "lr", "--details"]
            )
            | (
                "zypper",
                [
                    "--non-interactive",
                    "--no-refresh",
                    "locks",
                    "--type",
                    "package"
                ]
            )
            | (
                "zypper",
                [
                    "--non-interactive",
                    "--no-refresh",
                    "packages",
                    "--orphaned"
                ]
            )
    )
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommandOutput {
    pub success: bool,
    pub stdout: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DiscoverError {
    ReadFailed(&'static str),
    CommandFailed(&'static str),
    CommandNotAllowed,
    InvalidRelease,
    InvalidRepository(String),
    InvalidPowerState,
    InvalidPackageManagerState,
}

pub fn discover_host(backend: &impl DiscoveryBackend) -> Result<HostFacts, DiscoverError> {
    let release = parse_release(&backend.read(Path::new("/usr/lib/lyra-os/release"))?)?;
    let build_id = backend
        .read(Path::new("/usr/lib/lyra-os/build-info"))
        .ok()
        .and_then(|content| parse_assignment(&content, "LYRA_BUILD_COMMIT"))
        .unwrap_or_else(|| "unrecorded".into());

    let root_filesystem = successful_stdout(
        backend.run("findmnt", &["--noheadings", "--output", "FSTYPE", "/"])?,
        "findmnt",
    )?
    .trim()
    .to_ascii_lowercase();
    let snapper_root_configured = backend
        .run("snapper", &["--no-dbus", "--config", "root", "get-config"])?
        .success;
    let rpm_database_healthy = backend.run("rpm", &["--verifydb"])?.success;
    let package_lock_free = discover_package_lock(backend)?;
    let available_bytes = backend.available_bytes(Path::new("/"))?;
    let (on_battery, battery_percent) = discover_power(backend)?;
    let secure_boot_enabled = discover_secure_boot(backend);
    let repositories = discover_repositories(backend)?;
    let held_packages = lines_of_successful(backend.run(
        "zypper",
        &[
            "--non-interactive",
            "--no-refresh",
            "locks",
            "--type",
            "package",
        ],
    )?);
    let orphaned_packages = lines_of_successful(backend.run(
        "zypper",
        &[
            "--non-interactive",
            "--no-refresh",
            "packages",
            "--orphaned",
        ],
    )?);

    Ok(HostFacts {
        release: ReleaseIdentity {
            version: release.version,
            edition: "desktop".into(),
            architecture: release.architecture,
            build_id,
        },
        root_filesystem,
        snapper_root_configured,
        rpm_database_healthy,
        package_lock_free,
        available_bytes,
        // The libzypp solver supplies these estimates when PlanUpdate or
        // PlanReleaseUpgrade is requested. Discovery itself never guesses.
        required_download_bytes: 0,
        required_transaction_bytes: 0,
        required_snapshot_bytes: 0,
        on_battery,
        battery_percent,
        secure_boot_enabled,
        repositories,
        held_packages,
        orphaned_packages,
    })
}

fn discover_package_lock(backend: &impl DiscoveryBackend) -> Result<bool, DiscoverError> {
    let marker = match backend.read(Path::new("/run/zypp.pid")) {
        Ok(marker) => marker,
        Err(_) => return Ok(true),
    };
    let marker = marker.trim();
    if marker.is_empty() {
        return Ok(true);
    }
    let pid = marker
        .parse::<u32>()
        .ok()
        .filter(|pid| *pid > 0)
        .ok_or(DiscoverError::InvalidPackageManagerState)?;
    Ok(backend
        .read(Path::new(&format!("/proc/{pid}/comm")))
        .is_err())
}

struct ParsedRelease {
    version: String,
    architecture: String,
}

fn parse_release(content: &str) -> Result<ParsedRelease, DiscoverError> {
    let version =
        parse_assignment(content, "LYRA_VERSION_ID").ok_or(DiscoverError::InvalidRelease)?;
    let architecture =
        parse_assignment(content, "LYRA_ARCHITECTURE").ok_or(DiscoverError::InvalidRelease)?;
    if version.is_empty()
        || architecture.is_empty()
        || !version.bytes().all(is_safe_release_byte)
        || !architecture.bytes().all(is_safe_release_byte)
    {
        return Err(DiscoverError::InvalidRelease);
    }
    Ok(ParsedRelease {
        version,
        architecture,
    })
}

fn parse_assignment(content: &str, name: &str) -> Option<String> {
    content.lines().find_map(|line| {
        let value = line.strip_prefix(name)?.strip_prefix('=')?;
        let value = value.strip_prefix('\'')?.strip_suffix('\'')?;
        Some(value.to_string())
    })
}

fn is_safe_release_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_')
}

fn discover_power(backend: &impl DiscoveryBackend) -> Result<(bool, Option<u8>), DiscoverError> {
    let entries = match backend.read_dir(Path::new("/sys/class/power_supply")) {
        Ok(entries) => entries,
        Err(_) => return Ok((false, None)),
    };
    let mut batteries = Vec::new();
    let mut external_power_online = false;
    for entry in entries {
        if !entry
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b':'))
        {
            return Err(DiscoverError::InvalidPowerState);
        }
        let base = Path::new("/sys/class/power_supply").join(&entry);
        let kind = backend.read(&base.join("type")).unwrap_or_default();
        if kind.trim() == "Battery" {
            if let Ok(capacity) = backend.read(&base.join("capacity")) {
                let percent = capacity
                    .trim()
                    .parse::<u8>()
                    .ok()
                    .filter(|value| *value <= 100)
                    .ok_or(DiscoverError::InvalidPowerState)?;
                batteries.push(percent);
            }
        } else if matches!(kind.trim(), "Mains" | "USB" | "USB_C") {
            external_power_online |= backend
                .read(&base.join("online"))
                .is_ok_and(|value| value.trim() == "1");
        }
    }
    if batteries.is_empty() {
        return Ok((false, None));
    }
    Ok((!external_power_online, batteries.into_iter().min()))
}

fn discover_secure_boot(backend: &impl DiscoveryBackend) -> Option<bool> {
    backend
        .run("mokutil", &["--sb-state"])
        .ok()
        .filter(|output| output.success)
        .and_then(|output| {
            let normalized = output.stdout.to_ascii_lowercase();
            if normalized.contains("secureboot enabled") {
                Some(true)
            } else if normalized.contains("secureboot disabled") {
                Some(false)
            } else {
                None
            }
        })
}

fn discover_repositories(
    backend: &impl DiscoveryBackend,
) -> Result<Vec<RepositoryFact>, DiscoverError> {
    let output = backend.run(
        "zypper",
        &["--non-interactive", "--no-refresh", "lr", "--details"],
    )?;
    let stdout = successful_stdout(output, "zypper-lr")?;
    let mut repositories = Vec::new();
    for line in stdout.lines() {
        let fields: Vec<_> = line.split('|').map(str::trim).collect();
        if fields.len() < 8 || fields[0].parse::<usize>().is_err() {
            continue;
        }
        let alias = fields[1];
        if alias.is_empty()
            || !alias.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_' | b':')
            })
        {
            return Err(DiscoverError::InvalidRepository(alias.into()));
        }
        let enabled = fields[3].eq_ignore_ascii_case("yes");
        let gpg_check = fields[4].to_ascii_lowercase().ends_with("yes");
        let official = matches!(
            alias,
            "repo-oss" | "repo-non-oss" | "repo-lyra" | "repo-vega" | "repo-fina"
        );
        repositories.push(RepositoryFact {
            alias: alias.into(),
            enabled,
            official,
            // `lr` proves configuration only. Cached metadata health is
            // established by the solver phase, so an enabled repository is
            // not declared healthy here merely because it is listed.
            metadata_valid: !enabled,
            signing_key_trusted: !enabled || gpg_check,
        });
    }
    repositories.sort_by(|left, right| left.alias.cmp(&right.alias));
    Ok(repositories)
}

fn successful_stdout(
    output: CommandOutput,
    command: &'static str,
) -> Result<String, DiscoverError> {
    output
        .success
        .then_some(output.stdout)
        .ok_or(DiscoverError::CommandFailed(command))
}

fn lines_of_successful(output: CommandOutput) -> Vec<String> {
    if !output.success {
        return Vec::new();
    }
    let mut values: Vec<_> = output
        .stdout
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('-') && !line.contains('|'))
        .map(str::to_string)
        .collect();
    values.sort();
    values.dedup();
    values
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    #[derive(Default)]
    struct Fixture {
        files: BTreeMap<String, String>,
        directories: BTreeMap<String, Vec<String>>,
        commands: BTreeMap<String, CommandOutput>,
        available: u64,
    }

    impl DiscoveryBackend for Fixture {
        fn read(&self, path: &Path) -> Result<String, DiscoverError> {
            self.files
                .get(path.to_str().unwrap())
                .cloned()
                .ok_or(DiscoverError::ReadFailed("fixture"))
        }

        fn read_dir(&self, path: &Path) -> Result<Vec<String>, DiscoverError> {
            self.directories
                .get(path.to_str().unwrap())
                .cloned()
                .ok_or(DiscoverError::ReadFailed("fixture"))
        }

        fn available_bytes(&self, _path: &Path) -> Result<u64, DiscoverError> {
            Ok(self.available)
        }

        fn run(
            &self,
            program: &'static str,
            arguments: &'static [&'static str],
        ) -> Result<CommandOutput, DiscoverError> {
            let key = format!("{program} {}", arguments.join(" "));
            self.commands
                .get(&key)
                .cloned()
                .ok_or(DiscoverError::CommandFailed("fixture"))
        }
    }

    fn fixture() -> Fixture {
        let mut fixture = Fixture {
            available: 10_000_000_000,
            ..Fixture::default()
        };
        fixture.files.insert(
            "/usr/lib/lyra-os/release".into(),
            "LYRA_ARCHITECTURE='x86_64'\nLYRA_VERSION_ID='2026.08-alpha6'\n".into(),
        );
        fixture
            .directories
            .insert("/sys/class/power_supply".into(), vec![]);
        for (command, stdout) in [
            ("findmnt --noheadings --output FSTYPE /", "btrfs\n"),
            ("snapper --no-dbus --config root get-config", "ok\n"),
            ("rpm --verifydb", ""),
            ("mokutil --sb-state", "SecureBoot enabled\n"),
            (
                "zypper --non-interactive --no-refresh lr --details",
                "1 | repo-lyra | Lyra | Yes | Yes | Yes | 1 | rpm-md | https://example.invalid\n",
            ),
            (
                "zypper --non-interactive --no-refresh locks --type package",
                "",
            ),
            (
                "zypper --non-interactive --no-refresh packages --orphaned",
                "",
            ),
        ] {
            fixture.commands.insert(
                command.into(),
                CommandOutput {
                    success: true,
                    stdout: stdout.into(),
                },
            );
        }
        fixture
    }

    #[test]
    fn discovers_supported_host_without_network_or_mutation() {
        let facts = discover_host(&fixture()).unwrap();
        assert_eq!(facts.release.version, "2026.08-alpha6");
        assert_eq!(facts.root_filesystem, "btrfs");
        assert!(facts.snapper_root_configured);
        assert_eq!(facts.secure_boot_enabled, Some(true));
        assert_eq!(facts.repositories.len(), 1);
        assert!(!facts.repositories[0].metadata_valid);
    }

    #[test]
    fn malformed_release_is_rejected() {
        let mut fixture = fixture();
        fixture.files.insert(
            "/usr/lib/lyra-os/release".into(),
            "LYRA_ARCHITECTURE='x86_64'\nLYRA_VERSION_ID='$(touch /tmp/no)'\n".into(),
        );
        assert_eq!(discover_host(&fixture), Err(DiscoverError::InvalidRelease));
    }

    #[test]
    fn reports_lowest_battery_when_unplugged() {
        let mut fixture = fixture();
        fixture.directories.insert(
            "/sys/class/power_supply".into(),
            vec!["BAT0".into(), "BAT1".into(), "AC".into()],
        );
        fixture.files.extend(BTreeMap::from([
            (
                "/sys/class/power_supply/BAT0/type".into(),
                "Battery\n".into(),
            ),
            (
                "/sys/class/power_supply/BAT0/capacity".into(),
                "70\n".into(),
            ),
            (
                "/sys/class/power_supply/BAT1/type".into(),
                "Battery\n".into(),
            ),
            (
                "/sys/class/power_supply/BAT1/capacity".into(),
                "35\n".into(),
            ),
            ("/sys/class/power_supply/AC/type".into(), "Mains\n".into()),
            ("/sys/class/power_supply/AC/online".into(), "0\n".into()),
        ]));
        let facts = discover_host(&fixture).unwrap();
        assert!(facts.on_battery);
        assert_eq!(facts.battery_percent, Some(35));
    }

    #[test]
    fn accepts_kernel_usb_c_power_supply_names() {
        let mut fixture = fixture();
        fixture.directories.insert(
            "/sys/class/power_supply".into(),
            vec!["BAT1".into(), "ucsi-source-psy-USBC000:001".into()],
        );
        fixture.files.extend(BTreeMap::from([
            (
                "/sys/class/power_supply/BAT1/type".into(),
                "Battery\n".into(),
            ),
            (
                "/sys/class/power_supply/BAT1/capacity".into(),
                "100\n".into(),
            ),
            (
                "/sys/class/power_supply/ucsi-source-psy-USBC000:001/type".into(),
                "USB_C\n".into(),
            ),
            (
                "/sys/class/power_supply/ucsi-source-psy-USBC000:001/online".into(),
                "1\n".into(),
            ),
        ]));

        let facts = discover_host(&fixture).unwrap();
        assert!(!facts.on_battery);
        assert_eq!(facts.battery_percent, Some(100));
    }

    #[test]
    fn ignores_empty_or_stale_zypp_pid_markers() {
        let mut empty = fixture();
        empty.files.insert("/run/zypp.pid".into(), "\n".into());
        assert!(discover_host(&empty).unwrap().package_lock_free);

        let mut stale = fixture();
        stale.files.insert("/run/zypp.pid".into(), "4242\n".into());
        assert!(discover_host(&stale).unwrap().package_lock_free);
    }

    #[test]
    fn detects_a_live_zypp_pid() {
        let mut fixture = fixture();
        fixture
            .files
            .insert("/run/zypp.pid".into(), "4242\n".into());
        fixture
            .files
            .insert("/proc/4242/comm".into(), "zypper\n".into());
        assert!(!discover_host(&fixture).unwrap().package_lock_free);
    }

    #[test]
    fn rejects_a_malformed_zypp_pid_marker() {
        let mut fixture = fixture();
        fixture
            .files
            .insert("/run/zypp.pid".into(), "not-a-pid\n".into());
        assert_eq!(
            discover_host(&fixture),
            Err(DiscoverError::InvalidPackageManagerState)
        );
    }

    #[test]
    fn production_adapter_has_a_closed_command_allowlist() {
        assert!(allowed_probe("rpm", &["--verifydb"]));
        assert!(!allowed_probe("sh", &["-c", "id"]));
        assert!(!allowed_probe("zypper", &["update"]));
    }
}
