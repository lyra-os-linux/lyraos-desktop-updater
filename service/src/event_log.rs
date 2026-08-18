use std::collections::VecDeque;
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

use lyra_upgrade_core::sanitize_technical_line;
use lyra_upgrade_protocol::{EventLevel, EventSource, OperationEvent, TechnicalLine};

pub const MAX_TECHNICAL_LINES: usize = 10_000;
pub const MAX_TECHNICAL_BYTES: usize = 4 * 1024 * 1024;
const MAX_PERSISTED_EVENT_BYTES: u64 = 8 * 1024 * 1024;

#[derive(Debug)]
pub enum EventLogError {
    UnsafePath,
    TooLarge,
    Io(std::io::Error),
    Json(serde_json::Error),
}

impl From<std::io::Error> for EventLogError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<serde_json::Error> for EventLogError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

#[derive(Default)]
pub struct EventLog {
    events: VecDeque<OperationEvent>,
    technical_lines: usize,
    technical_bytes: usize,
}

impl EventLog {
    pub fn push_normative(&mut self, event: OperationEvent) {
        debug_assert!(event.technical.is_none());
        self.events.push_back(event);
    }

    pub fn push_technical(
        &mut self,
        mut event: OperationEvent,
        source: EventSource,
        raw: &str,
    ) -> OperationEvent {
        let sanitized = sanitize_technical_line(raw);
        self.technical_lines = self.technical_lines.saturating_add(1);
        self.technical_bytes = self.technical_bytes.saturating_add(sanitized.text.len());
        event.technical = Some(TechnicalLine {
            source,
            text: sanitized.text,
            truncated: sanitized.truncated,
        });
        self.events.push_back(event.clone());
        self.trim_technical();
        event
    }

    pub fn after(&self, sequence: u64) -> Vec<OperationEvent> {
        self.events
            .iter()
            .filter(|event| event.sequence > sequence)
            .cloned()
            .collect()
    }

    fn trim_technical(&mut self) {
        while self.technical_lines > MAX_TECHNICAL_LINES
            || self.technical_bytes > MAX_TECHNICAL_BYTES
        {
            let Some(index) = self
                .events
                .iter()
                .position(|event| event.technical.is_some())
            else {
                self.technical_lines = 0;
                self.technical_bytes = 0;
                break;
            };
            if let Some(event) = self.events.remove(index)
                && let Some(line) = event.technical
            {
                self.technical_lines = self.technical_lines.saturating_sub(1);
                self.technical_bytes = self.technical_bytes.saturating_sub(line.text.len());
            }
        }
    }
}

pub fn append_event(
    state_root: &Path,
    operation_id: &str,
    event: &OperationEvent,
) -> Result<(), EventLogError> {
    if !valid_operation_id(operation_id) {
        return Err(EventLogError::UnsafePath);
    }
    let path = state_root.join(operation_id).join("events.jsonl");
    if fs::symlink_metadata(&path).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
        return Err(EventLogError::UnsafePath);
    }
    if fs::metadata(&path).is_ok_and(|metadata| metadata.len() >= MAX_PERSISTED_EVENT_BYTES) {
        return Err(EventLogError::TooLarge);
    }
    let mut file = OpenOptions::new()
        .append(true)
        .create(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)?;
    serde_json::to_writer(&mut file, event)?;
    file.write_all(b"\n")?;
    file.sync_data()?;
    Ok(())
}

pub fn load_events(
    state_root: &Path,
    operation_id: &str,
    after: u64,
) -> Result<Vec<OperationEvent>, EventLogError> {
    if !valid_operation_id(operation_id) {
        return Err(EventLogError::UnsafePath);
    }
    let path = state_root.join(operation_id).join("events.jsonl");
    let metadata = fs::symlink_metadata(&path)?;
    if metadata.file_type().is_symlink() {
        return Err(EventLogError::UnsafePath);
    }
    if metadata.len() > MAX_PERSISTED_EVENT_BYTES {
        return Err(EventLogError::TooLarge);
    }
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)?;
    let mut events = Vec::new();
    for line in BufReader::new(file).lines() {
        let line = line?;
        if line.len() > 16 * 1024 {
            return Err(EventLogError::TooLarge);
        }
        let event: OperationEvent = serde_json::from_str(&line)?;
        if event.operation_id != operation_id {
            return Err(EventLogError::UnsafePath);
        }
        if event.sequence > after {
            events.push(event);
        }
    }
    events.sort_by_key(|event| event.sequence);
    events.dedup_by_key(|event| event.sequence);
    Ok(events)
}

fn valid_operation_id(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(index, byte)| {
            if matches!(index, 8 | 13 | 18 | 23) {
                byte == b'-'
            } else {
                byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase()
            }
        })
}

pub fn technical_event(
    operation_id: &str,
    sequence: u64,
    occurred_at: String,
    state: lyra_upgrade_core::OperationState,
) -> OperationEvent {
    OperationEvent {
        operation_id: operation_id.to_string(),
        sequence,
        occurred_at,
        state,
        level: EventLevel::Info,
        message_id: "technical-line".to_string(),
        fields: Default::default(),
        technical: None,
    }
}
