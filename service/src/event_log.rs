use std::collections::VecDeque;
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::Path;

use lyra_upgrade_core::sanitize_technical_line;
use lyra_upgrade_protocol::{EventLevel, EventSource, OperationEvent, TechnicalLine};

pub const MAX_TECHNICAL_LINES: usize = 10_000;
pub const MAX_TECHNICAL_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_PERSISTED_EVENT_BYTES: u64 = 8 * 1024 * 1024;

#[derive(Debug)]
pub enum EventLogError {
    UnsafePath,
    TooLarge,
    Io(std::io::Error),
    Json(serde_json::Error),
    InvalidHistory,
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
    last_sequence: u64,
    pub persistence_failed: bool,
}

impl EventLog {
    pub fn next_sequence(&self) -> u64 {
        self.last_sequence.saturating_add(1)
    }

    pub fn restore(events: Vec<OperationEvent>) -> Self {
        let mut log = Self::default();
        for event in events {
            log.last_sequence = log.last_sequence.max(event.sequence);
            if event.sequence == 0 {
                log.last_sequence = log.last_sequence.max(
                    event
                        .fields
                        .get("high_sequence")
                        .and_then(|n| n.parse().ok())
                        .unwrap_or(0),
                );
            }
            if let Some(line) = &event.technical {
                log.technical_lines += 1;
                log.technical_bytes += line.text.len();
            }
            log.events.push_back(event);
        }
        log.trim_technical();
        log
    }

    pub fn push_normative(&mut self, event: OperationEvent) {
        debug_assert!(event.technical.is_none());
        self.last_sequence = self.last_sequence.max(event.sequence);
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
        self.last_sequence = self.last_sequence.max(event.sequence);
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

const MAX_RECORD_BYTES: usize = 16 * 1024;
// Read the small overflow produced by older writers, without unbounded input.
const MAX_LEGACY_BYTES: u64 = MAX_PERSISTED_EVENT_BYTES + MAX_RECORD_BYTES as u64;
pub const TRUNCATED_MESSAGE: &str = "history-truncated";

pub struct EventHistory {
    pub events: Vec<OperationEvent>,
    pub incomplete: bool,
}

fn operation_directory(root: &Path, id: &str) -> Result<std::path::PathBuf, EventLogError> {
    if !valid_operation_id(id) {
        return Err(EventLogError::UnsafePath);
    }
    for path in [root.to_path_buf(), root.join(id)] {
        if !fs::symlink_metadata(path)?.file_type().is_dir() {
            return Err(EventLogError::UnsafePath);
        }
    }
    Ok(root.join(id))
}

fn locked(directory: &Path, exclusive: bool) -> Result<std::fs::File, EventLogError> {
    // A stable inode serializes readers with append/atomic replacement.
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
        .open(directory.join(".events.lock"))?;
    if !file.metadata()?.is_file() || file.metadata()?.nlink() != 1 {
        return Err(EventLogError::UnsafePath);
    }
    let mode = if exclusive {
        libc::LOCK_EX
    } else {
        libc::LOCK_SH
    };
    use std::os::fd::AsRawFd;
    if unsafe { libc::flock(file.as_raw_fd(), mode) } != 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(file)
}

fn open_events(path: &Path, append: bool) -> Result<std::fs::File, EventLogError> {
    let file = OpenOptions::new()
        .read(true)
        .append(append)
        .create(append)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
        .open(path)?;
    if !file.metadata()?.is_file() || file.metadata()?.nlink() != 1 {
        return Err(EventLogError::UnsafePath);
    }
    Ok(file)
}

fn encode(event: &OperationEvent) -> Result<Vec<u8>, EventLogError> {
    let mut bytes = serde_json::to_vec(event)?;
    bytes.push(b'\n');
    if bytes.len() > MAX_RECORD_BYTES {
        return Err(EventLogError::TooLarge);
    }
    Ok(bytes)
}

fn read_history(file: std::fs::File, id: &str) -> Result<EventHistory, EventLogError> {
    let mut reader = BufReader::new(file.take(MAX_LEGACY_BYTES + 1));
    let mut total = 0;
    let mut events = Vec::new();
    let mut incomplete = false;
    let mut previous = 0;
    loop {
        let mut bytes = Vec::new();
        let size = (&mut reader)
            .take((MAX_RECORD_BYTES + 1) as u64)
            .read_until(b'\n', &mut bytes);
        let size = match size {
            Ok(size) => size,
            Err(_) => {
                incomplete = true;
                break;
            }
        };
        if size == 0 {
            break;
        }
        total += size as u64;
        if size > MAX_RECORD_BYTES || total > MAX_LEGACY_BYTES || bytes.last() != Some(&b'\n') {
            incomplete = true;
            break;
        }
        let event = match serde_json::from_slice::<OperationEvent>(&bytes) {
            Ok(event) => event,
            Err(_) => {
                incomplete = true;
                break;
            }
        };
        let marker = event.sequence == 0
            && events.is_empty()
            && event.message_id == TRUNCATED_MESSAGE
            && event.technical.is_none()
            && event
                .fields
                .get("lines")
                .is_some_and(|n| n.parse::<u64>().is_ok())
            && event
                .fields
                .get("high_sequence")
                .is_some_and(|n| n.parse::<u64>().is_ok());
        if event.operation_id != id
            || (!marker && (event.sequence == 0 || event.sequence <= previous))
        {
            incomplete = true;
            break;
        }
        previous = event.sequence;
        events.push(event);
    }
    Ok(EventHistory { events, incomplete })
}

fn compact(
    mut events: Vec<OperationEvent>,
    incoming: &OperationEvent,
) -> Result<Vec<u8>, EventLogError> {
    let mut dropped = 0u64;
    if events.first().is_some_and(|e| e.sequence == 0) {
        dropped = events.remove(0).fields["lines"]
            .parse()
            .map_err(|_| EventLogError::InvalidHistory)?;
    }
    events.push(incoming.clone());
    let mut records: Vec<_> = events
        .into_iter()
        .map(|event| {
            let bytes = encode(&event)?;
            Ok((event, bytes))
        })
        .collect::<Result<_, EventLogError>>()?;
    let mut size: usize = records.iter().map(|(_, bytes)| bytes.len()).sum();
    // Amortize compaction: keep roughly half a segment of recent technical
    // output. Normative records are never evicted, even above this target.
    let target = MAX_PERSISTED_EVENT_BYTES as usize / 2;
    records.retain(|(event, bytes)| {
        if size > target && event.technical.is_some() {
            size -= bytes.len();
            dropped = dropped.saturating_add(1);
            false
        } else {
            true
        }
    });
    let mut output = Vec::new();
    if dropped != 0 {
        let mut marker = incoming.clone();
        marker.sequence = 0;
        marker.level = EventLevel::Warning;
        marker.message_id = TRUNCATED_MESSAGE.into();
        marker.technical = None;
        marker.fields = [
            ("lines".into(), dropped.to_string()),
            ("high_sequence".into(), incoming.sequence.to_string()),
        ]
        .into();
        output.extend(encode(&marker)?);
    }
    for (_, bytes) in records {
        output.extend(bytes);
    }
    if output.len() as u64 > MAX_PERSISTED_EVENT_BYTES {
        return Err(EventLogError::TooLarge);
    }
    Ok(output)
}

pub fn append_event(root: &Path, id: &str, event: &OperationEvent) -> Result<(), EventLogError> {
    if event.operation_id != id || event.sequence == 0 {
        return Err(EventLogError::UnsafePath);
    }
    // Check the complete serialized line before opening/writing the log.
    let bytes = encode(event)?;
    let directory = operation_directory(root, id)?;
    let _lock = locked(&directory, true)?;
    let path = directory.join("events.jsonl");
    let mut file = open_events(&path, true)?;
    let size = file.metadata()?.len();
    if size != 0 {
        // Compaction can discard the most recent technical record when the
        // normative history alone exceeds the target. Honor its high-water mark.
        let mut head = Vec::new();
        BufReader::new((&mut file).take(MAX_RECORD_BYTES as u64)).read_until(b'\n', &mut head)?;
        let first: OperationEvent = serde_json::from_slice(&head)?;
        if first.sequence == 0
            && (first.message_id != TRUNCATED_MESSAGE
                || first
                    .fields
                    .get("high_sequence")
                    .and_then(|n| n.parse::<u64>().ok())
                    .is_none_or(|high| high >= event.sequence))
        {
            return Err(EventLogError::InvalidHistory);
        }
        // Never append onto a partial record, or reset the sequence after restart.
        file.seek(SeekFrom::Start(
            size.saturating_sub(MAX_RECORD_BYTES as u64),
        ))?;
        let mut tail = Vec::new();
        file.read_to_end(&mut tail)?;
        if tail.last() != Some(&b'\n') {
            return Err(EventLogError::InvalidHistory);
        }
        let last = tail[..tail.len() - 1]
            .rsplit(|b| *b == b'\n')
            .next()
            .ok_or(EventLogError::InvalidHistory)?;
        let previous: OperationEvent = serde_json::from_slice(last)?;
        if previous.operation_id != id || previous.sequence >= event.sequence {
            return Err(EventLogError::InvalidHistory);
        }
    }
    if size.saturating_add(bytes.len() as u64) <= MAX_PERSISTED_EVENT_BYTES {
        file.write_all(&bytes)?;
        file.sync_data()?;
        if size == 0 {
            std::fs::File::open(&directory)?.sync_all()?;
        }
        return Ok(());
    }
    file.seek(SeekFrom::Start(0))?;
    let history = read_history(file, id)?;
    if history.incomplete {
        return Err(EventLogError::InvalidHistory);
    }
    let output = compact(history.events, event)?;
    let mut temporary = tempfile::NamedTempFile::new_in(&directory)?;
    temporary.write_all(&output)?;
    temporary.as_file().sync_all()?;
    temporary.persist(&path).map_err(|error| error.error)?;
    std::fs::File::open(&directory)?.sync_all()?;
    Ok(())
}

pub fn load_events(root: &Path, id: &str, after: u64) -> Result<EventHistory, EventLogError> {
    let directory = operation_directory(root, id)?;
    let _lock = locked(&directory, false)?;
    let file = match open_events(&directory.join("events.jsonl"), false) {
        Ok(file) => file,
        Err(EventLogError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(EventHistory {
                events: Vec::new(),
                incomplete: false,
            });
        }
        Err(error) => return Err(error),
    };
    let mut history = read_history(file, id)?;
    history
        .events
        .retain(|event| event.sequence == 0 || event.sequence > after);
    Ok(history)
}

pub fn record_write_failure(root: &Path, id: &str) {
    // Report even if a full/unavailable filesystem also rejects the marker.
    eprintln!("lyra-upgrade-service: EVENT_LOG_WRITE_FAILED operation={id}");
    if let Ok(directory) = operation_directory(root, id) {
        let result = (|| -> std::io::Result<()> {
            let mut marker = tempfile::NamedTempFile::new_in(&directory)?;
            marker.write_all(b"EVENT_LOG_WRITE_FAILED\n")?;
            marker.as_file().sync_all()?;
            marker
                .persist(directory.join("events.error"))
                .map_err(|e| e.error)?;
            std::fs::File::open(directory)?.sync_all()
        })();
        if result.is_err() {
            eprintln!("lyra-upgrade-service: EVENT_LOG_ERROR_MARKER_FAILED");
        }
    }
}

pub fn has_write_failure(root: &Path, id: &str) -> bool {
    fs::symlink_metadata(root.join(id).join("events.error")).is_ok()
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
