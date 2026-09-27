//! Snapper configuration requires privilege even when read-only. Ask the
//! existing authenticated-peer query broker for one boolean, never grant the
//! caller Snapper access or accept paths/commands from the request.
use lyra_upgrade_core::{CommandOutput, DiscoverError, DiscoveryBackend, SystemBackend};
use lyra_upgrade_protocol::{PROTOCOL_VERSION, Request, Response};
use std::path::Path;

pub struct ReadOnlyDiscovery;

impl DiscoveryBackend for ReadOnlyDiscovery {
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
        if program == "snapper"
            && arguments == ["--no-dbus", "--config", "root", "get-config"]
            && unsafe { libc::geteuid() } != 0
        {
            let id = "snapper-readiness";
            let response = crate::query_client::request(&Request::ReadRecoveryReadiness {
                protocol_version: PROTOCOL_VERSION,
                request_id: id.into(),
            });
            return Ok(CommandOutput {
                success: readiness_response(response, id),
                stdout: String::new(),
            });
        }
        SystemBackend.run(program, arguments)
    }
}

fn readiness_response(response: Result<Response, String>, id: &str) -> bool {
    matches!(response, Ok(Response::RecoveryReadiness { request_id, snapper_root_configured: true }) if request_id == id)
}

/// Called only by the root read broker. Native snapperd performs the Btrfs
/// inspection; the broker keeps its existing capabilities and sandbox.
pub fn recovery_readiness(request_id: String) -> Response {
    let configured = crate::process::output(
        std::process::Command::new("/usr/bin/snapper")
            .args(["--config", "root", "get-config"])
            .env_clear()
            .env("PATH", "/usr/sbin:/usr/bin:/sbin:/bin")
            .env("LC_ALL", "C")
            .stdin(std::process::Stdio::null()),
        5,
    )
    .is_ok_and(|output| output.status.success());
    Response::RecoveryReadiness {
        request_id,
        snapper_root_configured: configured,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recovery_probe_fails_closed_on_wrong_reply_missing_broker_and_failed_snapper() {
        let reply = |id: &str, configured| {
            Ok(Response::RecoveryReadiness {
                request_id: id.into(),
                snapper_root_configured: configured,
            })
        };
        assert!(readiness_response(reply("probe", true), "probe"));
        assert!(!readiness_response(reply("other", true), "probe"));
        assert!(!readiness_response(reply("probe", false), "probe"));
        assert!(!readiness_response(
            Err("QUERY_UNAVAILABLE".into()),
            "probe"
        ));
        assert!(!readiness_response(
            Ok(Response::TrustState {
                request_id: "probe".into(),
                last_manifest_sequence: None
            }),
            "probe"
        ));
    }
}
