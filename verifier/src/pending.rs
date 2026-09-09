use std::ffi::OsStr;
use std::fs;
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use lyra_upgrade_core::{OperationState, OperationStateRecord, load_state};

#[derive(Default)]
pub struct PendingScan {
    pub operation: Option<(PathBuf, OperationStateRecord)>,
    pub invalid_entries: usize,
}

// A filename is diagnostic metadata, not a trusted journal line. Keep its
// identifier recognizable without emitting control bytes or unbounded names.
fn diagnostic_name(name: &OsStr) -> String {
    let bytes = name.as_bytes();
    let mut result: String = bytes
        .iter()
        .take(64)
        .flat_map(|byte| byte.escape_ascii())
        .map(char::from)
        .collect();
    if bytes.len() > 64 {
        result.push_str("...");
    }
    result
}

/// Read candidates without altering any operation, including malformed ones.
/// An incomplete scan is distinct from a healthy store with no pending work.
pub fn pending_operation(
    root: &Path,
    mut diagnostic: impl FnMut(&str, &str),
) -> io::Result<PendingScan> {
    match fs::symlink_metadata(root) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(PendingScan::default()),
        Err(error) => return Err(error),
        Ok(metadata) if !metadata.file_type().is_dir() => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "unsafe state root",
            ));
        }
        Ok(_) => {}
    }
    let mut scan = PendingScan::default();
    let mut candidates = Vec::new();
    let mut invalid = |name: &OsStr, reason| {
        scan.invalid_entries += 1;
        diagnostic(&diagnostic_name(name), reason);
    };
    for entry in fs::read_dir(root)? {
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => {
                invalid(
                    OsStr::new("<unreadable-entry>"),
                    "directory-entry-unreadable",
                );
                continue;
            }
        };
        let name = entry.file_name();
        match entry.file_type() {
            Ok(kind) if kind.is_dir() => {}
            Ok(_) => {
                invalid(&name, "not-an-operation-directory");
                continue;
            }
            Err(_) => {
                invalid(&name, "directory-type-unreadable");
                continue;
            }
        }
        let Some(operation_id) = name.to_str() else {
            invalid(&name, "non-utf8-operation-id");
            continue;
        };
        // Do not enter load_state's reader for a FIFO, device or symlink.
        match fs::symlink_metadata(entry.path().join("state.json")) {
            Ok(metadata) if metadata.file_type().is_file() => {}
            Ok(_) => {
                invalid(&name, "not-a-regular-state-file");
                continue;
            }
            Err(_) => {
                invalid(&name, "state-file-unavailable");
                continue;
            }
        }
        let state = match load_state(root, operation_id) {
            Ok(state) => state,
            Err(_) => {
                // Never print raw JSON/errors; they may contain state contents.
                invalid(&name, "state-validation-failed");
                continue;
            }
        };
        if matches!(
            state.state,
            OperationState::AwaitingReboot | OperationState::VerifyingBoot
        ) {
            candidates.push((entry.path(), state));
        }
    }
    candidates.sort_by(|left, right| {
        right
            .1
            .updated_at
            .cmp(&left.1.updated_at)
            .then_with(|| left.1.operation_id.cmp(&right.1.operation_id))
    });
    scan.operation = candidates.into_iter().next();
    Ok(scan)
}

#[cfg(test)]
mod tests {
    use super::*;
    use lyra_upgrade_core::save_state;
    use std::os::unix::ffi::OsStringExt;
    use std::os::unix::fs::symlink;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT: AtomicUsize = AtomicUsize::new(0);
    const ID: &str = "00000000-0000-4000-8000-000000000010";
    const EMPTY: &str = "00000000-0000-4000-8000-000000000011";
    const BROKEN: &str = "00000000-0000-4000-8000-000000000012";

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "lyra-pending-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
        fn scan(&self) -> (PendingScan, Vec<(String, String)>) {
            let mut messages = Vec::new();
            let result = pending_operation(&self.0, |name, reason| {
                messages.push((name.into(), reason.into()))
            })
            .unwrap();
            (result, messages)
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }
    fn state(id: &str) -> OperationStateRecord {
        serde_json::from_value(serde_json::json!({
            "schema_version":1,"operation_id":id,"sequence":1,"operation":"UpdateWithinRelease",
            "state":"AwaitingReboot","source":{"version":"1","edition":"desktop","architecture":"x86_64","build_id":"fixture"},
            "target":null,"plan_sha256":"a".repeat(64),"manifest_sha256":null,
            "snapshot_number":10,"recovery":null,"last_completed_step":"offline-apply","error_code":null,
            "boot_verification":"Pending","created_at":"2026-09-09T00:00:00Z","updated_at":"2026-09-09T00:00:00Z"
        })).unwrap()
    }

    #[test]
    fn mixed_entries_never_hide_pending_work_in_any_creation_order() {
        // All 24 orders exercise a valid operation alongside the reported faults.
        for a in 0..4 {
            for b in 0..4 {
                for c in 0..4 {
                    for d in 0..4 {
                        let order = [a, b, c, d];
                        if (0..4).any(|i| (i + 1..4).any(|j| order[i] == order[j])) {
                            continue;
                        }
                        let fixture = Fixture::new();
                        for entry in order {
                            match entry {
                                0 => save_state(&fixture.0, &state(ID)).unwrap(),
                                1 => fs::create_dir(fixture.0.join(EMPTY)).unwrap(),
                                2 => {
                                    fs::create_dir(fixture.0.join(BROKEN)).unwrap();
                                    fs::write(
                                        fixture.0.join(BROKEN).join("state.json"),
                                        b"{\"state\":",
                                    )
                                    .unwrap();
                                }
                                _ => {
                                    fs::write(fixture.0.join("unexpected-file"), b"private-content")
                                        .unwrap()
                                }
                            }
                        }
                        let before = fs::read(fixture.0.join(ID).join("state.json")).unwrap();
                        let (scan, messages) = fixture.scan();
                        assert_eq!(
                            scan.operation.unwrap().1.operation_id,
                            ID,
                            "order {order:?}"
                        );
                        assert_eq!(scan.invalid_entries, 3);
                        assert!(messages.iter().any(|(name, _)| name == EMPTY));
                        assert!(messages.iter().any(|(name, _)| name == BROKEN));
                        assert_eq!(
                            fs::read(fixture.0.join(ID).join("state.json")).unwrap(),
                            before
                        );
                        assert_eq!(
                            fs::read(fixture.0.join(BROKEN).join("state.json")).unwrap(),
                            b"{\"state\":"
                        );
                        assert_eq!(fs::read_dir(fixture.0.join(EMPTY)).unwrap().count(), 0);
                    }
                }
            }
        }
    }

    #[test]
    fn corrupt_pending_state_remains_untouched_and_cannot_mean_no_work() {
        let fixture = Fixture::new();
        save_state(&fixture.0, &state(ID)).unwrap();
        let path = fixture.0.join(ID).join("state.json");
        let truncated = b"{\"state\":\"AwaitingReboot\",\"secret\":\"do-not-log";
        fs::write(&path, truncated).unwrap();
        let (scan, messages) = fixture.scan();
        assert!(scan.operation.is_none());
        assert_eq!(scan.invalid_entries, 1);
        assert_eq!(
            messages,
            vec![(ID.into(), "state-validation-failed".into())]
        );
        assert_eq!(fs::read(path).unwrap(), truncated);
        assert_eq!(fs::read_dir(fixture.0.join(ID)).unwrap().count(), 1);
    }

    #[test]
    fn invalid_names_and_special_files_are_bounded_diagnostics_not_readers() {
        let fixture = Fixture::new();
        save_state(&fixture.0, &state(ID)).unwrap();
        let bad = std::ffi::OsString::from_vec(b"invalid\n\x1b\xffname".to_vec());
        fs::create_dir(fixture.0.join(&bad)).unwrap();
        let link = fixture.0.join("linked-operation");
        symlink(fixture.0.join(ID), link).unwrap();
        fs::create_dir(fixture.0.join(EMPTY)).unwrap();
        symlink(
            fixture.0.join(ID).join("state.json"),
            fixture.0.join(EMPTY).join("state.json"),
        )
        .unwrap();
        fs::create_dir(fixture.0.join(BROKEN)).unwrap();
        assert!(
            std::process::Command::new("mkfifo")
                .arg(fixture.0.join(BROKEN).join("state.json"))
                .status()
                .unwrap()
                .success()
        );
        let (scan, messages) = fixture.scan();
        assert_eq!(scan.operation.unwrap().1.operation_id, ID);
        assert_eq!(scan.invalid_entries, 4);
        assert!(
            messages
                .iter()
                .any(|(name, _)| name == r"invalid\n\x1b\xffname")
        );
        assert!(
            messages
                .iter()
                .all(|(name, reason)| !name.contains('\n') && !reason.contains('\n'))
        );
        assert!(diagnostic_name(OsStr::new(&"x".repeat(255))).len() <= 67);
    }

    #[test]
    fn invalid_schema_or_mismatched_identity_cannot_be_selected() {
        for mutation in ["schema_version", "operation_id", "plan_sha256"] {
            let fixture = Fixture::new();
            save_state(&fixture.0, &state(ID)).unwrap();
            let path = fixture.0.join(ID).join("state.json");
            let mut value = serde_json::to_value(state(ID)).unwrap();
            value[mutation] = match mutation {
                "schema_version" => serde_json::json!(99),
                "operation_id" => serde_json::json!(EMPTY),
                _ => serde_json::json!("invalid"),
            };
            let data = serde_json::to_vec(&value).unwrap();
            fs::write(&path, &data).unwrap();
            let (scan, _) = fixture.scan();
            assert!(scan.operation.is_none());
            assert_eq!(scan.invalid_entries, 1);
            assert_eq!(fs::read(path).unwrap(), data);
        }
    }

    #[test]
    fn latest_pending_candidate_is_deterministic_and_completed_states_are_ignored() {
        let fixture = Fixture::new();
        let first = state(ID);
        let mut second = state(EMPTY);
        let mut completed = state(BROKEN);
        completed.state = OperationState::Completed;
        completed.updated_at = "2026-09-10T00:00:00Z".into();
        save_state(&fixture.0, &completed).unwrap();
        save_state(&fixture.0, &second).unwrap();
        save_state(&fixture.0, &first).unwrap();
        assert_eq!(fixture.scan().0.operation.unwrap().1.operation_id, ID);
        second.updated_at = "2026-09-09T12:00:00Z".into();
        second.state = OperationState::VerifyingBoot;
        save_state(&fixture.0, &second).unwrap();
        let (scan, _) = fixture.scan();
        assert_eq!(scan.operation.unwrap().1.operation_id, EMPTY);
        assert_eq!(scan.invalid_entries, 0);
    }

    #[test]
    fn missing_empty_and_invalid_roots_have_distinct_results() {
        let fixture = Fixture::new();
        let (scan, messages) = fixture.scan();
        assert!(scan.operation.is_none());
        assert!(messages.is_empty());
        assert_eq!(
            pending_operation(&fixture.0.join("absent"), |_, _| panic!())
                .unwrap()
                .invalid_entries,
            0
        );
        let file = fixture.0.join("file");
        fs::write(&file, b"not a directory").unwrap();
        assert!(pending_operation(&file, |_, _| panic!()).is_err());
        let link = fixture.0.join("link");
        symlink(&fixture.0, &link).unwrap();
        assert!(pending_operation(&link, |_, _| panic!()).is_err());
    }
}
