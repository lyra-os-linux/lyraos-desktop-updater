//! Fixed repository/cache context shared by release planning and offline execution.
use lyra_upgrade_core::{CommandOutput, DiscoverError, DiscoveryBackend, SystemBackend};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub struct RepositoryContext {
    pub repos: PathBuf,
    pub cache: PathBuf,
    pub raw: PathBuf,
    pub solv: PathBuf,
    pub packages: PathBuf,
}

impl RepositoryContext {
    pub fn prepared(operation_dir: &Path) -> Self {
        let cache = operation_dir.join("cache");
        Self {
            repos: operation_dir.join("repos.d"),
            raw: cache.join("raw"),
            solv: cache.join("solv"),
            packages: cache.join("packages"),
            cache,
        }
    }
    pub fn arguments(&self) -> Vec<OsString> {
        [
            ("--reposd-dir", &self.repos),
            ("--cache-dir", &self.cache),
            ("--raw-cache-dir", &self.raw),
            ("--solv-cache-dir", &self.solv),
            ("--pkg-cache-dir", &self.packages),
        ]
        .into_iter()
        .flat_map(|(flag, path)| [OsString::from(flag), path.as_os_str().to_owned()])
        .collect()
    }
    pub fn command(&self) -> Command {
        let mut command = Command::new("zypper");
        command
            .args(self.arguments())
            .env_clear()
            .env("PATH", "/usr/sbin:/usr/bin:/sbin:/bin")
            .env("LC_ALL", "C")
            .stdin(Stdio::null());
        command
    }
}

/// Hardware/RPM probes keep their closed core implementation. Zypper inventory
/// probes must use the same prepared cache as the solver; --no-refresh alone
/// still reads global metadata for packages --orphaned.
pub struct PreparedDiscovery<'a> {
    pub context: &'a RepositoryContext,
}
impl DiscoveryBackend for PreparedDiscovery<'_> {
    fn read(&self, path: &Path) -> Result<String, DiscoverError> {
        SystemBackend.read(path)
    }
    fn read_dir(&self, path: &Path) -> Result<Vec<String>, DiscoverError> {
        SystemBackend.read_dir(path)
    }
    fn available_bytes(&self, path: &Path) -> Result<u64, DiscoverError> {
        SystemBackend.available_bytes(path)
    }
    fn run(
        &self,
        program: &'static str,
        arguments: &'static [&'static str],
    ) -> Result<CommandOutput, DiscoverError> {
        if program != "zypper" {
            return SystemBackend.run(program, arguments);
        }
        if !matches!(
            arguments,
            ["--non-interactive", "--no-refresh", "lr", "--details"]
                | ["--non-interactive", "--no-refresh", "locks"]
                | [
                    "--non-interactive",
                    "--no-refresh",
                    "packages",
                    "--orphaned"
                ]
        ) {
            return Err(DiscoverError::CommandNotAllowed);
        }
        let output = self
            .context
            .command()
            .args(arguments)
            .output()
            .map_err(|_| DiscoverError::CommandFailed("zypper"))?;
        Ok(CommandOutput {
            success: output.status.success(),
            stdout: String::from_utf8(output.stdout)
                .map_err(|_| DiscoverError::CommandFailed("zypper"))?,
        })
    }
}
