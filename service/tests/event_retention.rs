use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;

use lyra_upgrade_core::OperationState;
use lyra_upgrade_protocol::{EventLevel, EventSource, OperationEvent, TechnicalLine};
use lyra_upgrade_service::event_log::{
    EventLog, EventLogError, MAX_PERSISTED_EVENT_BYTES, TRUNCATED_MESSAGE, append_event,
    load_events,
};

const ID: &str = "00000000-0000-4000-8000-000000000011";

fn event(sequence: u64, technical: bool) -> OperationEvent {
    OperationEvent {
        operation_id: ID.into(),
        sequence,
        occurred_at: "2026-09-09T12:00:00Z".into(),
        state: OperationState::Applying,
        level: EventLevel::Info,
        message_id: if technical {
            "technical-line"
        } else {
            "state.applying"
        }
        .into(),
        fields: Default::default(),
        technical: technical.then(|| TechnicalLine {
            source: EventSource::ZypperStdout,
            text: String::new(),
            truncated: false,
        }),
    }
}

fn sized_event(sequence: u64, technical: bool, size: usize) -> OperationEvent {
    let mut e = event(sequence, technical);
    if !technical {
        e.fields.insert("fixture".into(), String::new());
    }
    let base = serde_json::to_vec(&e).unwrap().len() + 1;
    let padding = "x".repeat(size - base);
    if let Some(line) = &mut e.technical {
        line.text = padding;
    } else {
        e.fields.insert("fixture".into(), padding);
    }
    assert_eq!(serde_json::to_vec(&e).unwrap().len() + 1, size);
    e
}

fn fixture() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join(ID)).unwrap();
    root
}

// Seed a real byte boundary using valid records, including an early state event.
fn seed(root: &Path, bytes: usize, technical: bool) -> u64 {
    let mut file = fs::File::create(root.join(ID).join("events.jsonl")).unwrap();
    let mut remaining = bytes;
    let mut sequence = 0;
    while remaining > 0 {
        sequence += 1;
        let mut size = remaining.min(8192);
        if remaining > size && remaining - size < 512 {
            size -= 512;
        }
        let e = sized_event(sequence, technical && sequence != 1, size);
        serde_json::to_writer(&mut file, &e).unwrap();
        file.write_all(b"\n").unwrap();
        remaining -= size;
    }
    file.sync_all().unwrap();
    sequence
}

#[test]
fn exact_limit_compacts_only_when_next_serialized_record_would_overflow() {
    let root = fixture();
    let next = event(1025, true);
    let line_size = serde_json::to_vec(&next).unwrap().len() + 1;
    let high = seed(
        root.path(),
        MAX_PERSISTED_EVENT_BYTES as usize - line_size,
        true,
    );
    assert_eq!(high + 1, next.sequence);
    append_event(root.path(), ID, &next).unwrap();
    let path = root.path().join(ID).join("events.jsonl");
    assert_eq!(
        fs::metadata(&path).unwrap().len(),
        MAX_PERSISTED_EVENT_BYTES
    );
    let full = load_events(root.path(), ID, 0).unwrap();
    assert!(!full.incomplete);
    assert!(
        !full
            .events
            .iter()
            .any(|e| e.message_id == TRUNCATED_MESSAGE)
    );
    append_event(root.path(), ID, &event(next.sequence + 1, false)).unwrap();
    assert!(fs::metadata(path).unwrap().len() <= MAX_PERSISTED_EVENT_BYTES);
    let compacted = load_events(root.path(), ID, 0).unwrap();
    assert!(!compacted.incomplete);
    assert_eq!(compacted.events[0].message_id, TRUNCATED_MESSAGE);
    assert!(
        compacted
            .events
            .iter()
            .any(|e| e.sequence == 1 && e.technical.is_none())
    );
    assert_eq!(compacted.events.last().unwrap().sequence, next.sequence + 1);
    assert_eq!(
        load_events(root.path(), ID, next.sequence + 1)
            .unwrap()
            .events
            .len(),
        1
    );
}

#[test]
fn repeated_retention_preserves_every_state_event_and_cumulative_notice() {
    let root = fixture();
    let mut sequence = seed(root.path(), MAX_PERSISTED_EVENT_BYTES as usize, true);
    let mut expected_states = vec![1];
    let mut last_count = 0;
    let mut rotations = 0;
    for index in 0..1200 {
        sequence += 1;
        let technical = index % 100 != 0;
        if !technical {
            expected_states.push(sequence);
        }
        append_event(root.path(), ID, &sized_event(sequence, technical, 8192)).unwrap();
        let size = fs::metadata(root.path().join(ID).join("events.jsonl"))
            .unwrap()
            .len();
        assert!(size <= MAX_PERSISTED_EVENT_BYTES);
        if index % 100 == 0 || index == 1199 {
            let history = load_events(root.path(), ID, 0).unwrap();
            assert!(!history.incomplete);
            let count: u64 = history.events[0].fields["lines"].parse().unwrap();
            if count > last_count {
                rotations += 1;
            }
            assert!(count >= last_count);
            last_count = count;
            let states: Vec<_> = history
                .events
                .iter()
                .filter(|e| e.sequence != 0 && e.technical.is_none())
                .map(|e| e.sequence)
                .collect();
            assert_eq!(states, expected_states);
        }
    }
    assert!(rotations >= 3);
}

#[test]
fn legacy_overflow_remains_readable_and_is_repaired_on_append() {
    let root = fixture();
    let high = seed(root.path(), MAX_PERSISTED_EVENT_BYTES as usize + 1000, true);
    let history = load_events(root.path(), ID, 0).unwrap();
    assert!(!history.incomplete);
    assert_eq!(history.events.last().unwrap().sequence, high);
    append_event(root.path(), ID, &event(high + 1, false)).unwrap();
    assert!(
        fs::metadata(root.path().join(ID).join("events.jsonl"))
            .unwrap()
            .len()
            <= MAX_PERSISTED_EVENT_BYTES
    );
    assert!(!load_events(root.path(), ID, 0).unwrap().incomplete);
}

#[test]
fn partial_or_corrupt_tail_returns_valid_prefix_and_is_not_overwritten() {
    for tail in [b"{\"sequence\":".as_slice(), b"invalid\n", b"{}\n"] {
        let root = fixture();
        append_event(root.path(), ID, &event(1, false)).unwrap();
        let path = root.path().join(ID).join("events.jsonl");
        OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(tail)
            .unwrap();
        let before = fs::read(&path).unwrap();
        let history = load_events(root.path(), ID, 0).unwrap();
        assert!(history.incomplete);
        assert_eq!(history.events, vec![event(1, false)]);
        assert!(append_event(root.path(), ID, &event(2, false)).is_err());
        assert_eq!(fs::read(path).unwrap(), before);
    }
}

#[test]
fn oversized_record_is_rejected_before_creating_or_changing_history() {
    let root = fixture();
    // Escaped JSON is larger than the in-memory text.
    let mut e = event(1, true);
    e.technical.as_mut().unwrap().text = "\u{0001}".repeat(4096);
    assert!(matches!(
        append_event(root.path(), ID, &e),
        Err(EventLogError::TooLarge)
    ));
    assert!(!root.path().join(ID).join("events.jsonl").exists());
    append_event(root.path(), ID, &event(1, false)).unwrap();
    let before = fs::read(root.path().join(ID).join("events.jsonl")).unwrap();
    e.sequence = 2;
    assert!(append_event(root.path(), ID, &e).is_err());
    assert_eq!(
        fs::read(root.path().join(ID).join("events.jsonl")).unwrap(),
        before
    );
}

#[test]
fn normative_only_capacity_failure_preserves_existing_history() {
    let root = fixture();
    let high = seed(root.path(), MAX_PERSISTED_EVENT_BYTES as usize, false);
    let path = root.path().join(ID).join("events.jsonl");
    let before = fs::read(&path).unwrap();
    assert!(matches!(
        append_event(root.path(), ID, &event(high + 1, false)),
        Err(EventLogError::TooLarge)
    ));
    assert_eq!(fs::read(path).unwrap(), before);
    assert!(!load_events(root.path(), ID, 0).unwrap().incomplete);
}

#[test]
fn discarded_latest_technical_sequence_cannot_be_reused_after_restart() {
    let root = fixture();
    // Leave room for the retention marker alongside a large normative history.
    let high = seed(
        root.path(),
        MAX_PERSISTED_EVENT_BYTES as usize - 1024,
        false,
    );
    append_event(root.path(), ID, &sized_event(high + 1, true, 8192)).unwrap();
    let history = load_events(root.path(), ID, 0).unwrap();
    assert!(!history.incomplete);
    assert!(!history.events.iter().any(|e| e.technical.is_some()));
    assert_eq!(EventLog::restore(history.events).next_sequence(), high + 2);
    assert!(append_event(root.path(), ID, &event(high + 1, false)).is_err());
    append_event(root.path(), ID, &event(high + 2, false)).unwrap();
    assert!(!load_events(root.path(), ID, 0).unwrap().incomplete);
}

#[test]
fn unsafe_files_and_operation_directories_are_rejected() {
    use std::os::unix::fs::symlink;
    for name in ["events.jsonl", ".events.lock"] {
        let root = fixture();
        let outside = root.path().join("outside");
        fs::write(&outside, "sentinel").unwrap();
        symlink(&outside, root.path().join(ID).join(name)).unwrap();
        assert!(append_event(root.path(), ID, &event(1, false)).is_err());
        assert!(load_events(root.path(), ID, 0).is_err());
        assert_eq!(fs::read_to_string(outside).unwrap(), "sentinel");
    }
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    symlink(outside.path(), root.path().join(ID)).unwrap();
    assert!(load_events(root.path(), ID, 0).is_err());
    assert!(append_event(root.path(), ID, &event(1, false)).is_err());
    assert!(load_events(root.path(), "../../etc", 0).is_err());
}

#[test]
fn malformed_large_input_is_bounded_and_reports_incomplete_history() {
    let root = fixture();
    append_event(root.path(), ID, &event(1, false)).unwrap();
    let path = root.path().join(ID).join("events.jsonl");
    OpenOptions::new()
        .append(true)
        .open(path)
        .unwrap()
        .write_all(&vec![b'x'; 1024 * 1024])
        .unwrap();
    let history = load_events(root.path(), ID, 0).unwrap();
    assert!(history.incomplete);
    assert_eq!(history.events, vec![event(1, false)]);
}

#[test]
fn reopen_in_fresh_process() {
    if let Some(root) = std::env::var_os("LYRA_EVENT_TEST_ROOT") {
        let root = Path::new(&root);
        let history = load_events(root, ID, 0).unwrap();
        assert!(!history.incomplete);
        assert_eq!(history.events[0].message_id, TRUNCATED_MESSAGE);
        assert!(history.events.iter().any(|e| e.sequence == 1));
        let next = EventLog::restore(history.events).next_sequence();
        append_event(root, ID, &event(next, false)).unwrap();
        assert_eq!(
            load_events(root, ID, 0)
                .unwrap()
                .events
                .last()
                .unwrap()
                .sequence,
            next
        );
        return;
    }
    let root = fixture();
    let high = seed(root.path(), MAX_PERSISTED_EVENT_BYTES as usize, true);
    append_event(root.path(), ID, &event(high + 1, false)).unwrap();
    let result = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "reopen_in_fresh_process", "--nocapture"])
        .env("LYRA_EVENT_TEST_ROOT", root.path())
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    let history = load_events(root.path(), ID, 0).unwrap();
    assert!(!history.incomplete);
    assert_eq!(history.events.last().unwrap().sequence, high + 2);
}

#[test]
fn serialized_record_limit_includes_the_newline() {
    let root = fixture();
    let e = sized_event(1, true, 16 * 1024);
    append_event(root.path(), ID, &e).unwrap();
    assert_eq!(load_events(root.path(), ID, 0).unwrap().events, vec![e]);
    assert!(matches!(
        append_event(root.path(), ID, &sized_event(2, true, 16 * 1024 + 1)),
        Err(EventLogError::TooLarge)
    ));
}

#[test]
fn fifo_and_hard_links_cannot_be_used_as_history_or_lock() {
    use std::os::unix::ffi::OsStrExt;
    for name in ["events.jsonl", ".events.lock"] {
        for hardlink in [true, false] {
            let root = fixture();
            let path = root.path().join(ID).join(name);
            if hardlink {
                let target = root.path().join("outside");
                fs::write(&target, "sentinel").unwrap();
                fs::hard_link(target, &path).unwrap();
            } else {
                let name = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
                assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
            }
            assert!(append_event(root.path(), ID, &event(1, false)).is_err());
            assert!(load_events(root.path(), ID, 0).is_err());
        }
    }
}

#[test]
fn readers_and_writer_share_a_stable_lock_across_atomic_compaction() {
    let root = fixture();
    let high = seed(root.path(), MAX_PERSISTED_EVENT_BYTES as usize, true);
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let reader_root = root.path().to_path_buf();
    let reader_barrier = barrier.clone();
    let reader = std::thread::spawn(move || {
        reader_barrier.wait();
        for _ in 0..50 {
            let history = load_events(&reader_root, ID, 0).unwrap();
            assert!(!history.incomplete);
            assert!(history.events.iter().any(|e| e.sequence == 1));
        }
    });
    barrier.wait();
    for offset in 1..=40 {
        append_event(root.path(), ID, &event(high + offset, false)).unwrap();
    }
    reader.join().unwrap();
    let history = load_events(root.path(), ID, 0).unwrap();
    assert!(!history.incomplete);
    assert_eq!(history.events[0].message_id, TRUNCATED_MESSAGE);
    assert_eq!(history.events.last().unwrap().sequence, high + 40);
}
