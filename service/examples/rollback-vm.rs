//! Exercise the authorized scheduling implementation only in the disposable VM.
use std::{fs, path::Path};
fn main() {
    assert!(
        fs::read_to_string("/proc/cmdline")
            .unwrap()
            .split_whitespace()
            .any(|arg| arg == "lyra.updater-offline-test=1"),
        "disposable VM required"
    );
    let root = Path::new("/var/lib/lyra-upgrade/operations");
    let mut state =
        lyra_upgrade_core::load_state(root, "00000000-0000-4000-8000-000000000006").unwrap();
    match lyra_upgrade_service::recovery::schedule_rollback(
        root,
        &mut state,
        "2026-09-09T12:00:00Z",
    ) {
        Ok(()) => println!(
            "ROLLBACK_SCHEDULED {}",
            serde_json::to_string(&state.recovery).unwrap()
        ),
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    }
}
