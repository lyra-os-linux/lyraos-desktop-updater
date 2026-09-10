use std::collections::{BTreeMap, HashMap};
use std::io::{self, BufRead};
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex};

use lyra_upgrade_core::{
    BootVerification, OperationKind, OperationState, OperationStateRecord, PreflightPolicy,
    ReleaseManifest, SystemBackend, discover_host, evaluate_preflight, load_state, save_state,
};
use lyra_upgrade_protocol::{
    EventLevel, EventSource, OperationEvent, PlannedUpdate, RecoveryAction, Request, Response,
};
use lyra_upgrade_service::event_log::{
    EventLog, append_event, has_write_failure, load_events, record_write_failure, technical_event,
};
use lyra_upgrade_service::executor::{
    ExecutionObserver, OutputStream, execute_update, failure_state, stage_release_upgrade,
};
use lyra_upgrade_service::manifest_fetch::{
    RELEASE_CHANNEL_PATH, fetch_release_manifest, manifest_sequence_path,
    read_last_manifest_sequence, read_release_channel,
};
use lyra_upgrade_service::planner::{plan_release_upgrade, plan_update_with_cached_metadata};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use uuid::Uuid;

struct Service {
    state_root: PathBuf,
    caller_uid: u32,
    plans: HashMap<String, PendingPlan>,
    events: HashMap<String, Arc<Mutex<EventLog>>>,
    workers: Arc<(Mutex<usize>, Condvar)>,
}

#[derive(Clone, serde::Deserialize, serde::Serialize)]
struct PendingPlan {
    planned: PlannedUpdate,
    manifest: Option<ReleaseManifest>,
}

impl Service {
    fn new(caller_uid: u32) -> Self {
        Self {
            state_root: PathBuf::from("/var/lib/lyra-upgrade/operations"),
            caller_uid,
            plans: HashMap::new(),
            events: HashMap::new(),
            workers: Arc::new((Mutex::new(0), Condvar::new())),
        }
    }

    fn handle(&mut self, request: Request) -> Response {
        let request_id = request.request_id().to_string();
        if !request.is_supported() {
            return rejected(request_id, "UNSUPPORTED_PROTOCOL");
        }
        match request {
            Request::Inspect { .. } => self.inspect(request_id),
            Request::PlanUpdate { .. } => self.plan_update(request_id),
            Request::PlanReleaseUpgrade { .. } => self.plan_release_upgrade(request_id),
            Request::Start {
                operation_id,
                plan_sha256,
                confirmed,
                planned,
                ..
            } => self.start(request_id, operation_id, plan_sha256, confirmed, *planned),
            Request::Status {
                operation_id,
                after_sequence,
                ..
            } => self.status(request_id, operation_id, after_sequence.unwrap_or(0)),
            Request::Cancel { operation_id, .. } => self.cancel(request_id, operation_id),
            Request::AcknowledgeRecovery {
                operation_id,
                recovery_action,
                ..
            } => self.acknowledge_recovery(request_id, operation_id, recovery_action),
        }
    }

    fn inspect(&self, request_id: String) -> Response {
        match discover_host(&SystemBackend) {
            Ok(facts) => {
                let preflight = evaluate_preflight(&facts, PreflightPolicy::default());
                Response::Inspection {
                    request_id,
                    facts,
                    preflight,
                }
            }
            Err(_) => rejected(request_id, "DISCOVERY_FAILED"),
        }
    }

    fn plan_update(&mut self, request_id: String) -> Response {
        let planned = match plan_update_with_cached_metadata() {
            Ok(planned) => planned,
            Err(_) => return rejected(request_id, "PREFLIGHT_BLOCKED"),
        };
        let operation_id = Uuid::new_v4().to_string();
        let now = now();
        let state = OperationStateRecord {
            schema_version: 1,
            operation_id: operation_id.clone(),
            sequence: 1,
            operation: OperationKind::UpdateWithinRelease,
            state: OperationState::AwaitingConfirmation,
            source: planned.facts.release.clone(),
            target: None,
            plan_sha256: planned.plan_sha256.clone(),
            manifest_sha256: None,
            snapshot_number: None,
            recovery: None,
            last_completed_step: Some("planned".to_string()),
            error_code: None,
            boot_verification: Some(BootVerification::Pending),
            created_at: now.clone(),
            updated_at: now,
        };
        if save_state(&self.state_root, &state).is_err()
            || save_operation_owner(&self.state_root, &operation_id, self.caller_uid).is_err()
            || save_pending_plan(
                &self.state_root,
                &operation_id,
                &PendingPlan {
                    planned: planned.clone(),
                    manifest: None,
                },
            )
            .is_err()
        {
            return rejected(request_id, "STATE_WRITE_FAILED");
        }
        let response = Response::Plan {
            request_id,
            operation_id: operation_id.clone(),
            plan_sha256: planned.plan_sha256.clone(),
            plan: Box::new(planned.plan.clone()),
            preflight: planned.preflight.clone(),
            planned: Box::new(planned.clone()),
        };
        self.plans.insert(
            operation_id.clone(),
            PendingPlan {
                planned,
                manifest: None,
            },
        );
        self.events
            .insert(operation_id, Arc::new(Mutex::new(EventLog::default())));
        response
    }

    fn plan_release_upgrade(&mut self, request_id: String) -> Response {
        let facts = match discover_host(&SystemBackend) {
            Ok(facts) => facts,
            Err(_) => return rejected(request_id, "DISCOVERY_FAILED"),
        };
        let channel = match read_release_channel(std::path::Path::new(RELEASE_CHANNEL_PATH)) {
            Ok(channel) => channel,
            Err(_) => return rejected(request_id, "RELEASE_CHANNEL_INVALID"),
        };
        let manifest = match fetch_release_manifest(
            &facts.release,
            read_last_manifest_sequence(&manifest_sequence_path()),
            channel,
        ) {
            Ok(manifest) => manifest,
            Err(_) => return rejected(request_id, "MANIFEST_INVALID"),
        };
        let planned = match plan_release_upgrade(&manifest) {
            Ok(planned) => planned,
            Err(_) => return rejected(request_id, "PREFLIGHT_BLOCKED"),
        };
        let operation_id = Uuid::new_v4().to_string();
        let now = now();
        let state = OperationStateRecord {
            schema_version: 1,
            operation_id: operation_id.clone(),
            sequence: 1,
            operation: OperationKind::ReleaseUpgrade,
            state: OperationState::AwaitingConfirmation,
            source: planned.facts.release.clone(),
            target: Some(manifest.target.clone()),
            plan_sha256: planned.plan_sha256.clone(),
            manifest_sha256: planned.plan.manifest_sha256.clone(),
            snapshot_number: None,
            recovery: None,
            last_completed_step: Some("planned".to_string()),
            error_code: None,
            boot_verification: Some(BootVerification::Pending),
            created_at: now.clone(),
            updated_at: now,
        };
        if save_state(&self.state_root, &state).is_err()
            || save_operation_owner(&self.state_root, &operation_id, self.caller_uid).is_err()
            || save_pending_plan(
                &self.state_root,
                &operation_id,
                &PendingPlan {
                    planned: planned.clone(),
                    manifest: Some(manifest.clone()),
                },
            )
            .is_err()
        {
            return rejected(request_id, "STATE_WRITE_FAILED");
        }
        let response = Response::Plan {
            request_id,
            operation_id: operation_id.clone(),
            plan_sha256: planned.plan_sha256.clone(),
            plan: Box::new(planned.plan.clone()),
            preflight: planned.preflight.clone(),
            planned: Box::new(planned.clone()),
        };
        self.plans.insert(
            operation_id.clone(),
            PendingPlan {
                planned,
                manifest: Some(manifest),
            },
        );
        self.events
            .insert(operation_id, Arc::new(Mutex::new(EventLog::default())));
        response
    }

    fn start(
        &mut self,
        request_id: String,
        operation_id: String,
        plan_sha256: String,
        confirmed: bool,
        submitted: PlannedUpdate,
    ) -> Response {
        if !confirmed {
            return rejected(request_id, "CONFIRMATION_REQUIRED");
        }
        let operation_shape_valid = valid_operation_shape(
            submitted.plan.operation,
            submitted.manifest.is_some(),
            submitted
                .manifest
                .as_ref()
                .is_some_and(|manifest| submitted.plan.target.as_ref() == Some(&manifest.target)),
            submitted.plan.target.is_some(),
            submitted.plan.manifest_sha256.is_some(),
        );
        if submitted.plan_sha256 != plan_sha256
            || submitted.plan.sha256().ok().as_deref() != Some(&plan_sha256)
            || !operation_shape_valid
            || submitted.facts.release != submitted.plan.source
            || submitted.solver.changes != submitted.plan.package_changes
            || submitted.solver.reboot_required != submitted.plan.reboot_required
        {
            return rejected(request_id, "PLAN_HASH_MISMATCH");
        }
        if !operation_owned_by(&self.state_root, &operation_id, self.caller_uid) {
            // A first PlanUpdate may be produced by the unprivileged client and
            // submitted to a fresh privileged process. Release upgrades never
            // use this fallback: their signed manifest and pending plan must
            // already have been persisted by PlanReleaseUpgrade.
            if submitted.plan.operation != OperationKind::UpdateWithinRelease {
                return rejected(request_id, "OPERATION_NOT_FOUND");
            }
            if std::fs::symlink_metadata(self.state_root.join(&operation_id)).is_ok() {
                return rejected(request_id, "OPERATION_NOT_FOUND");
            }
            let timestamp = now();
            let initial = OperationStateRecord {
                schema_version: 1,
                operation_id: operation_id.clone(),
                sequence: 1,
                operation: OperationKind::UpdateWithinRelease,
                state: OperationState::AwaitingConfirmation,
                source: submitted.facts.release.clone(),
                target: None,
                plan_sha256: plan_sha256.clone(),
                manifest_sha256: None,
                snapshot_number: None,
                recovery: None,
                last_completed_step: Some("planned".to_string()),
                error_code: None,
                boot_verification: Some(BootVerification::Pending),
                created_at: timestamp.clone(),
                updated_at: timestamp,
            };
            let pending = PendingPlan {
                planned: submitted.clone(),
                manifest: None,
            };
            if save_state(&self.state_root, &initial).is_err()
                || save_pending_plan(&self.state_root, &operation_id, &pending).is_err()
                || save_operation_owner(&self.state_root, &operation_id, self.caller_uid).is_err()
            {
                return rejected(request_id, "STATE_WRITE_FAILED");
            }
            self.plans.insert(operation_id.clone(), pending);
        }
        let planned = self
            .plans
            .get(&operation_id)
            .cloned()
            .or_else(|| load_pending_plan(&self.state_root, &operation_id).ok());
        let Some(planned) = planned else {
            return rejected(request_id, "PLAN_NOT_AVAILABLE");
        };
        if planned.planned.plan_sha256 != plan_sha256
            || planned.planned != submitted
            || planned.manifest != submitted.manifest
        {
            return rejected(request_id, "PLAN_HASH_MISMATCH");
        }
        let mut state = match load_state(&self.state_root, &operation_id) {
            Ok(state) => state,
            Err(_) => return rejected(request_id, "STATE_READ_FAILED"),
        };
        if state.state != OperationState::AwaitingConfirmation {
            return rejected(request_id, "INVALID_STATE");
        }
        if has_write_failure(&self.state_root, &operation_id) {
            return rejected(request_id, "EVENT_LOG_WRITE_FAILED");
        }
        let history = match load_events(&self.state_root, &operation_id, 0) {
            Ok(history) if !history.incomplete => history,
            _ => return rejected(request_id, "EVENT_LOG_READ_FAILED"),
        };
        let log = Arc::new(Mutex::new(EventLog::restore(history.events)));
        self.events.insert(operation_id.clone(), log.clone());
        let observer = Observer {
            operation_id: operation_id.clone(),
            state_root: self.state_root.clone(),
            log,
            current_state: Mutex::new(state.state),
        };
        let state_root = self.state_root.clone();
        let workers = self.workers.clone();
        if let Ok(mut active) = workers.0.lock() {
            *active = active.saturating_add(1);
        }
        std::thread::spawn(move || {
            let execution = match state.operation {
                OperationKind::UpdateWithinRelease => {
                    execute_update(&state_root, &mut state, &planned.planned, &observer).map(|_| ())
                }
                OperationKind::ReleaseUpgrade => match planned.manifest.as_ref() {
                    Some(manifest) => stage_release_upgrade(
                        &state_root,
                        &mut state,
                        &planned.planned,
                        manifest,
                        &observer,
                    )
                    .map(|_| ()),
                    None => Err(lyra_upgrade_service::executor::ExecutionError::PlanChanged),
                },
            };
            if let Err(error) = execution {
                if load_state(&state_root, &state.operation_id)
                    .is_ok_and(|saved| saved.error_code.as_deref() == Some("CANCELLED"))
                {
                    if let Ok(mut active) = workers.0.lock() {
                        *active = active.saturating_sub(1);
                        workers.1.notify_all();
                    }
                    return;
                }
                state.state = failure_state(&error, state.snapshot_number.is_some());
                state.error_code = Some(error.code().to_string());
                state.sequence = state.sequence.saturating_add(1);
                state.updated_at = now();
                let _ = save_state(&state_root, &state);
            }
            if let Ok(mut active) = workers.0.lock() {
                *active = active.saturating_sub(1);
                workers.1.notify_all();
            }
        });
        Response::Accepted {
            request_id,
            operation_id: Some(operation_id),
        }
    }

    fn status(&self, request_id: String, operation_id: String, after: u64) -> Response {
        if !operation_owned_by(&self.state_root, &operation_id, self.caller_uid) {
            return rejected(request_id, "OPERATION_NOT_FOUND");
        }
        let state = match load_state(&self.state_root, &operation_id) {
            Ok(state) => state,
            Err(_) => return rejected(request_id, "OPERATION_NOT_FOUND"),
        };
        // Disk is authoritative even while the worker is active. A cached
        // nonempty tail must not conceal a damaged or failed durable history.
        let (mut events, mut history_error) =
            match load_events(&self.state_root, &operation_id, after) {
                Ok(history) => (
                    history.events,
                    history.incomplete.then_some("EVENT_LOG_READ_FAILED"),
                ),
                Err(_) => (Vec::new(), Some("EVENT_LOG_READ_FAILED")),
            };
        if has_write_failure(&self.state_root, &operation_id) {
            history_error = Some("EVENT_LOG_WRITE_FAILED");
        }
        if let Some(log) = self.events.get(&operation_id) {
            match log.lock() {
                Ok(log) => {
                    if log.persistence_failed {
                        history_error = Some("EVENT_LOG_WRITE_FAILED");
                    }
                    if history_error.is_some() {
                        events.extend(log.after(after));
                        events.sort_by_key(|event| event.sequence);
                        events.dedup_by_key(|event| event.sequence);
                    }
                }
                Err(_) => history_error = Some("EVENT_LOG_READ_FAILED"),
            }
        }
        if let Some(code) = history_error {
            // An out-of-band diagnostic keeps both errors visible when the
            // operation itself failed. It never consumes a persisted sequence.
            events.push(OperationEvent {
                operation_id: operation_id.clone(),
                sequence: 0,
                occurred_at: state.updated_at.clone(),
                state: state.state,
                level: EventLevel::Warning,
                message_id: format!("error_{code}"),
                fields: Default::default(),
                technical: None,
            });
        }
        Response::Status {
            request_id,
            operation_id,
            sequence: state.sequence,
            state: state.state,
            snapshot_number: state.snapshot_number,
            recovered: state.state == OperationState::Completed
                && state.recovery.is_some()
                && state.boot_verification == Some(lyra_upgrade_core::BootVerification::Passed),
            error_code: state
                .error_code
                .or_else(|| history_error.map(str::to_owned)),
            events,
        }
    }

    fn cancel(&mut self, request_id: String, operation_id: String) -> Response {
        if !operation_owned_by(&self.state_root, &operation_id, self.caller_uid) {
            return rejected(request_id, "OPERATION_NOT_FOUND");
        }
        let mut state = match load_state(&self.state_root, &operation_id) {
            Ok(state) => state,
            Err(_) => return rejected(request_id, "OPERATION_NOT_FOUND"),
        };
        if !state.state.is_cancelable() {
            return rejected(request_id, "NOT_CANCELABLE");
        }
        state.state = OperationState::Failed;
        state.error_code = Some("CANCELLED".to_string());
        state.sequence = state.sequence.saturating_add(1);
        state.updated_at = now();
        if save_state(&self.state_root, &state).is_err() {
            return rejected(request_id, "STATE_WRITE_FAILED");
        }
        self.status(request_id, operation_id, 0)
    }

    fn acknowledge_recovery(
        &mut self,
        request_id: String,
        operation_id: String,
        action: RecoveryAction,
    ) -> Response {
        if !operation_owned_by(&self.state_root, &operation_id, self.caller_uid) {
            return rejected(request_id, "OPERATION_NOT_FOUND");
        }
        let mut state = match load_state(&self.state_root, &operation_id) {
            Ok(state) => state,
            Err(_) => return rejected(request_id, "OPERATION_NOT_FOUND"),
        };
        if state.state != OperationState::NeedsRecovery {
            return rejected(request_id, "RECOVERY_NOT_REQUIRED");
        }
        match action {
            RecoveryAction::ShowDiagnostics => return self.status(request_id, operation_id, 0),
            RecoveryAction::KeepCurrent => {
                state.state = OperationState::Failed;
                state.error_code = Some("RECOVERY_DECLINED".to_string());
            }
            RecoveryAction::Rollback => {
                if let Err(error) = lyra_upgrade_service::recovery::schedule_rollback(
                    &self.state_root,
                    &mut state,
                    &now(),
                ) {
                    return rejected(request_id, error);
                }
                return self.status(request_id, operation_id, 0);
            }
        }
        state.sequence = state.sequence.saturating_add(1);
        state.updated_at = now();
        if save_state(&self.state_root, &state).is_err() {
            return rejected(request_id, "STATE_WRITE_FAILED");
        }
        self.status(request_id, operation_id, 0)
    }
}

fn valid_operation_shape(
    operation: OperationKind,
    has_manifest: bool,
    target_matches_manifest: bool,
    has_target: bool,
    has_manifest_sha256: bool,
) -> bool {
    match operation {
        OperationKind::UpdateWithinRelease => !has_manifest && !has_target && !has_manifest_sha256,
        OperationKind::ReleaseUpgrade => {
            has_manifest && target_matches_manifest && has_target && has_manifest_sha256
        }
    }
}

fn save_operation_owner(
    root: &std::path::Path,
    operation_id: &str,
    uid: u32,
) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    if !valid_operation_id(operation_id) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "invalid operation id",
        ));
    }
    let directory = root.join(operation_id);
    let path = directory.join("owner");
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)?;
    writeln!(file, "{uid}")?;
    file.sync_all()
}

fn operation_owned_by(root: &std::path::Path, operation_id: &str, uid: u32) -> bool {
    if !valid_operation_id(operation_id) {
        return false;
    }
    let path = root.join(operation_id).join("owner");
    let Ok(metadata) = std::fs::symlink_metadata(&path) else {
        return false;
    };
    if !metadata.file_type().is_file() || metadata.len() > 32 {
        return false;
    }
    std::fs::read_to_string(path)
        .ok()
        .and_then(|value| value.trim().parse::<u32>().ok())
        == Some(uid)
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

fn caller_uid() -> Option<u32> {
    std::env::var("PKEXEC_UID").ok()?.parse().ok()
}

fn save_pending_plan(
    root: &std::path::Path,
    operation_id: &str,
    plan: &PendingPlan,
) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let directory = root.join(operation_id);
    std::fs::create_dir_all(&directory)?;
    let path = directory.join("pending-plan.json");
    let temporary = directory.join("pending-plan.json.tmp");
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(&temporary)?;
    serde_json::to_writer_pretty(&mut file, plan).map_err(std::io::Error::other)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    std::fs::rename(temporary, path)?;
    Ok(())
}

fn load_pending_plan(root: &std::path::Path, operation_id: &str) -> std::io::Result<PendingPlan> {
    let path = root.join(operation_id).join("pending-plan.json");
    let metadata = std::fs::symlink_metadata(&path)?;
    if !metadata.file_type().is_file() || metadata.len() > 8 * 1024 * 1024 {
        return Err(std::io::Error::other("invalid pending plan"));
    }
    let file = std::fs::File::open(path)?;
    serde_json::from_reader(file).map_err(std::io::Error::other)
}

struct Observer {
    operation_id: String,
    state_root: PathBuf,
    log: Arc<Mutex<EventLog>>,
    current_state: Mutex<OperationState>,
}

impl ExecutionObserver for Observer {
    fn command_line(&self, program: &'static str, stream: OutputStream, line: &str) {
        let Ok(mut log) = self.log.lock() else { return };
        let source = match stream {
            OutputStream::Stdout => EventSource::ZypperStdout,
            OutputStream::Stderr => EventSource::ZypperStderr,
        };
        let sequence = next_sequence(&log);
        let state = self
            .current_state
            .lock()
            .map(|state| *state)
            .unwrap_or(OperationState::Failed);
        let event = technical_event(&self.operation_id, sequence, now(), state);
        let event = log.push_technical(event, source, &format!("{program}: {line}"));
        if append_event(&self.state_root, &self.operation_id, &event).is_err() {
            if !log.persistence_failed {
                record_write_failure(&self.state_root, &self.operation_id);
            }
            log.persistence_failed = true;
        }
    }

    fn state_changed(&self, state: OperationState) {
        if let Ok(mut current) = self.current_state.lock() {
            *current = state;
        }
        let Ok(mut log) = self.log.lock() else { return };
        let event = OperationEvent {
            operation_id: self.operation_id.clone(),
            sequence: next_sequence(&log),
            occurred_at: now(),
            state,
            level: EventLevel::Info,
            message_id: format!("state.{}", state_name(state)),
            fields: BTreeMap::new(),
            technical: None,
        };
        if append_event(&self.state_root, &self.operation_id, &event).is_err() {
            if !log.persistence_failed {
                record_write_failure(&self.state_root, &self.operation_id);
            }
            log.persistence_failed = true;
        }
        log.push_normative(event);
    }

    fn cancel_requested(&self) -> bool {
        load_state(&self.state_root, &self.operation_id)
            .is_ok_and(|state| state.error_code.as_deref() == Some("CANCELLED"))
    }
}

fn next_sequence(log: &EventLog) -> u64 {
    log.next_sequence()
}

fn state_name(state: OperationState) -> &'static str {
    match state {
        OperationState::Idle => "idle",
        OperationState::Checking => "checking",
        OperationState::Available => "available",
        OperationState::Preflight => "preflight",
        OperationState::Planned => "planned",
        OperationState::AwaitingConfirmation => "awaiting-confirmation",
        OperationState::Downloading => "downloading",
        OperationState::Snapshotting => "snapshotting",
        OperationState::Applying => "applying",
        OperationState::ReadyToReboot => "ready-to-reboot",
        OperationState::ApplyingOffline => "applying-offline",
        OperationState::AwaitingReboot => "awaiting-reboot",
        OperationState::VerifyingBoot => "verifying-boot",
        OperationState::Blocked => "blocked",
        OperationState::Failed => "failed",
        OperationState::NeedsRecovery => "needs-recovery",
        OperationState::Completed => "completed",
    }
}

fn now() -> String {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string())
}

fn rejected(request_id: String, error_code: &str) -> Response {
    Response::Rejected {
        request_id,
        error_code: error_code.to_string(),
    }
}

fn main() {
    let Some(caller_uid) = caller_uid() else {
        eprintln!("lyra-upgrade-service: missing authenticated caller identity");
        std::process::exit(1);
    };
    let mut service = Service::new(caller_uid);
    for line in io::stdin().lock().lines() {
        let response = match line
            .ok()
            .and_then(|line| serde_json::from_str::<Request>(&line).ok())
        {
            Some(request) => service.handle(request),
            None => rejected("unknown".to_string(), "INVALID_REQUEST"),
        };
        println!(
            "{}",
            serde_json::to_string(&response).expect("serialize response")
        );
    }
    let (active, finished) = &*service.workers;
    if let Ok(mut active) = active.lock() {
        while *active > 0 {
            active = finished
                .wait(active)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
        }
    }
}

#[cfg(test)]
mod ownership_tests {
    use super::{operation_owned_by, save_operation_owner, valid_operation_shape};
    use lyra_upgrade_core::OperationKind;

    #[test]
    fn start_accepts_only_complete_shapes_for_update_and_release_upgrade() {
        assert!(valid_operation_shape(
            OperationKind::UpdateWithinRelease,
            false,
            false,
            false,
            false,
        ));
        assert!(valid_operation_shape(
            OperationKind::ReleaseUpgrade,
            true,
            true,
            true,
            true,
        ));
        assert!(!valid_operation_shape(
            OperationKind::ReleaseUpgrade,
            false,
            false,
            true,
            true,
        ));
        assert!(!valid_operation_shape(
            OperationKind::ReleaseUpgrade,
            true,
            false,
            true,
            true,
        ));
        assert!(!valid_operation_shape(
            OperationKind::UpdateWithinRelease,
            true,
            true,
            true,
            true,
        ));
    }

    #[test]
    fn operation_owner_is_required_and_cannot_be_replaced() {
        let root =
            std::env::temp_dir().join(format!("lyra-upgrade-owner-test-{}", std::process::id()));
        let operation = "00000000-0000-4000-8000-000000000000";
        let directory = root.join(operation);
        std::fs::create_dir_all(&directory).unwrap();

        assert!(!operation_owned_by(&root, operation, 1000));
        save_operation_owner(&root, operation, 1000).unwrap();
        assert!(operation_owned_by(&root, operation, 1000));
        assert!(!operation_owned_by(&root, operation, 1001));
        assert!(save_operation_owner(&root, operation, 1001).is_err());

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn symbolic_link_owner_is_rejected() {
        use std::os::unix::fs::symlink;
        let root = std::env::temp_dir().join(format!(
            "lyra-upgrade-owner-symlink-test-{}",
            std::process::id()
        ));
        let operation = "00000000-0000-4000-8000-000000000001";
        let directory = root.join(operation);
        std::fs::create_dir_all(&directory).unwrap();
        let target = root.join("owner-target");
        std::fs::write(&target, b"1000\n").unwrap();
        symlink(target, directory.join("owner")).unwrap();

        assert!(!operation_owned_by(&root, operation, 1000));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn path_traversal_operation_id_is_rejected() {
        let root = std::env::temp_dir();
        assert!(!operation_owned_by(&root, "../../etc", 1000));
        assert!(save_operation_owner(&root, "../../etc", 1000).is_err());
    }
}

#[cfg(test)]
mod event_status_tests {
    use super::*;
    use lyra_upgrade_core::ReleaseIdentity;
    use std::fs;

    const ID: &str = "00000000-0000-4000-8000-000000000011";

    fn fixture() -> (tempfile::TempDir, Service, OperationStateRecord) {
        let root = tempfile::tempdir().unwrap();
        let mut service = Service::new(1000);
        service.state_root = root.path().to_path_buf();
        let state = OperationStateRecord {
            schema_version: 1,
            operation_id: ID.into(),
            sequence: 7,
            operation: OperationKind::UpdateWithinRelease,
            state: OperationState::AwaitingConfirmation,
            source: ReleaseIdentity {
                version: "1.0".into(),
                edition: "desktop".into(),
                architecture: "x86_64".into(),
                build_id: "test".into(),
            },
            target: None,
            plan_sha256: "a".repeat(64),
            manifest_sha256: None,
            snapshot_number: None,
            recovery: None,
            last_completed_step: Some("planned".into()),
            error_code: None,
            boot_verification: Some(BootVerification::Pending),
            created_at: now(),
            updated_at: now(),
        };
        save_state(root.path(), &state).unwrap();
        save_operation_owner(root.path(), ID, 1000).unwrap();
        (root, service, state)
    }

    fn observer(service: &mut Service) -> Observer {
        let log = Arc::new(Mutex::new(EventLog::default()));
        service.events.insert(ID.into(), log.clone());
        Observer {
            operation_id: ID.into(),
            state_root: service.state_root.clone(),
            log,
            current_state: Mutex::new(OperationState::Applying),
        }
    }

    #[test]
    fn never_written_history_is_empty_without_false_error() {
        let (_root, service, _) = fixture();
        let Response::Status {
            events, error_code, ..
        } = service.status("s".into(), ID.into(), 0)
        else {
            panic!()
        };
        assert!(events.is_empty());
        assert_eq!(error_code, None);
    }

    #[test]
    fn cached_events_cannot_hide_read_failure_or_replace_operation_error() {
        let (root, mut service, mut state) = fixture();
        let observer = observer(&mut service);
        observer.state_changed(OperationState::Applying);
        observer.command_line("zypper", OutputStream::Stdout, "available details");
        let path = root.path().join(ID).join("events.jsonl");
        use std::io::Write;
        fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(b"{partial")
            .unwrap();
        state.state = OperationState::Failed;
        state.error_code = Some("ZYPPER_APPLY_FAILED".into());
        save_state(root.path(), &state).unwrap();
        let saved = fs::read(root.path().join(ID).join("state.json")).unwrap();
        let Response::Status {
            events,
            error_code,
            state,
            sequence,
            ..
        } = service.status("s".into(), ID.into(), 0)
        else {
            panic!()
        };
        assert_eq!(state, OperationState::Failed);
        assert_eq!(sequence, 7);
        assert_eq!(error_code.as_deref(), Some("ZYPPER_APPLY_FAILED"));
        assert!(
            events
                .iter()
                .any(|e| e.message_id == "error_EVENT_LOG_READ_FAILED")
        );
        assert_eq!(events.iter().filter(|e| e.sequence > 0).count(), 2);
        assert_eq!(
            fs::read(root.path().join(ID).join("state.json")).unwrap(),
            saved
        );
        // A new service with no RAM cache still exposes the readable prefix.
        let mut restarted = Service::new(1000);
        restarted.state_root = root.path().to_path_buf();
        let response = restarted.status("reopened".into(), ID.into(), 0);
        let json = serde_json::to_string(&response).unwrap();
        let Response::Status {
            events, error_code, ..
        } = serde_json::from_str::<Response>(&json).unwrap()
        else {
            panic!()
        };
        assert_eq!(error_code.as_deref(), Some("ZYPPER_APPLY_FAILED"));
        assert_eq!(events.iter().filter(|e| e.sequence > 0).count(), 2);
        assert!(
            events
                .iter()
                .any(|e| e.message_id == "error_EVENT_LOG_READ_FAILED")
        );
    }

    #[test]
    fn both_observer_paths_surface_write_failure_live_and_after_reopen() {
        for technical in [true, false] {
            let (root, mut service, _) = fixture();
            let observer = observer(&mut service);
            // A directory at the log path forces a real open/write failure.
            fs::create_dir(root.path().join(ID).join("events.jsonl")).unwrap();
            if technical {
                observer.command_line("zypper", OutputStream::Stderr, "saved only in RAM");
            } else {
                observer.state_changed(OperationState::Applying);
            }
            assert!(has_write_failure(root.path(), ID));
            let Response::Status {
                events, error_code, ..
            } = service.status("s".into(), ID.into(), 0)
            else {
                panic!()
            };
            assert_eq!(error_code.as_deref(), Some("EVENT_LOG_WRITE_FAILED"));
            assert_eq!(events.iter().filter(|e| e.sequence > 0).count(), 1);
            assert!(
                events
                    .iter()
                    .any(|e| e.message_id == "error_EVENT_LOG_WRITE_FAILED")
            );
            let mut restarted = Service::new(1000);
            restarted.state_root = root.path().to_path_buf();
            let Response::Status {
                events, error_code, ..
            } = restarted.status("s".into(), ID.into(), 0)
            else {
                panic!()
            };
            assert_eq!(error_code.as_deref(), Some("EVENT_LOG_WRITE_FAILED"));
            assert_eq!(events.len(), 1);
            assert_eq!(events[0].message_id, "error_EVENT_LOG_WRITE_FAILED");
        }
    }
}
