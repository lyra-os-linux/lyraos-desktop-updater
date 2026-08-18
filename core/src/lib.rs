//! Unprivileged domain types for Lyra Upgrade.
//!
//! This crate must remain usable without root and must not modify repositories,
//! packages, snapshots or boot state while inspecting or planning.

mod discovery;
mod manifest;
mod persistence;
mod preflight;
mod sanitize;
mod solver;

pub use discovery::{CommandOutput, DiscoverError, DiscoveryBackend, SystemBackend, discover_host};
pub use manifest::{ManifestError, ReleaseManifest, RepositoryTransition, validate_manifest_route};
pub use persistence::{PersistenceError, load_state, save_state};
pub use preflight::{
    HostFacts, PlanError, PreflightIssue, PreflightPolicy, PreflightReport, RepositoryFact,
    UpgradePlan, build_plan, evaluate_preflight,
};
pub use sanitize::{SanitizedLine, sanitize_technical_line};
pub use solver::{
    PackageAction, PackageChange, SolverPolicy, SolverResult, VendorTransition,
    evaluate_solver_preflight,
};

use serde::{Deserialize, Serialize};

pub const STATE_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub enum OperationKind {
    UpdateWithinRelease,
    ReleaseUpgrade,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub enum OperationState {
    Idle,
    Checking,
    Available,
    Preflight,
    Planned,
    AwaitingConfirmation,
    Downloading,
    Snapshotting,
    Applying,
    ReadyToReboot,
    ApplyingOffline,
    AwaitingReboot,
    VerifyingBoot,
    Blocked,
    Failed,
    NeedsRecovery,
    Completed,
}

impl OperationState {
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Failed | Self::NeedsRecovery | Self::Completed)
    }

    pub const fn is_cancelable(self) -> bool {
        matches!(
            self,
            Self::Idle
                | Self::Checking
                | Self::Available
                | Self::Preflight
                | Self::Planned
                | Self::AwaitingConfirmation
                | Self::Downloading
                | Self::Blocked
        )
    }

    pub const fn permits_system_write(self) -> bool {
        matches!(
            self,
            Self::Snapshotting | Self::Applying | Self::ReadyToReboot | Self::ApplyingOffline
        )
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReleaseIdentity {
    pub version: String,
    pub edition: String,
    pub architecture: String,
    pub build_id: String,
}

impl ReleaseIdentity {
    pub fn is_supported_desktop(&self) -> bool {
        self.edition == "desktop" && self.architecture == "x86_64"
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OperationStateRecord {
    pub schema_version: u32,
    pub operation_id: String,
    pub sequence: u64,
    pub operation: OperationKind,
    pub state: OperationState,
    pub source: ReleaseIdentity,
    pub target: Option<ReleaseIdentity>,
    pub plan_sha256: String,
    pub manifest_sha256: Option<String>,
    pub snapshot_number: Option<u64>,
    pub last_completed_step: Option<String>,
    pub error_code: Option<String>,
    pub boot_verification: Option<BootVerification>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub enum BootVerification {
    Pending,
    Passed,
    Failed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransitionError {
    UnsupportedSchema,
    InvalidTransition,
    SnapshotNotRecorded,
}

impl OperationStateRecord {
    pub fn transition_to(&mut self, next: OperationState) -> Result<(), TransitionError> {
        if self.schema_version != STATE_SCHEMA_VERSION {
            return Err(TransitionError::UnsupportedSchema);
        }
        if next == OperationState::Applying && self.snapshot_number.is_none() {
            return Err(TransitionError::SnapshotNotRecorded);
        }
        if !valid_transition(self.operation, self.state, next) {
            return Err(TransitionError::InvalidTransition);
        }
        self.state = next;
        self.sequence = self.sequence.saturating_add(1);
        Ok(())
    }
}

pub const fn valid_transition(
    operation: OperationKind,
    current: OperationState,
    next: OperationState,
) -> bool {
    use OperationState::*;
    if matches!(next, Blocked) {
        return matches!(current, Checking | Preflight | Downloading);
    }
    if matches!(next, Failed) {
        return !matches!(current, Completed | NeedsRecovery);
    }
    if matches!(next, NeedsRecovery) {
        return matches!(
            current,
            Snapshotting | Applying | ApplyingOffline | VerifyingBoot
        );
    }
    match (current, next) {
        (Idle, Checking)
        | (Checking, Available)
        | (Available, Preflight)
        | (Preflight, Planned)
        | (Planned, AwaitingConfirmation)
        | (AwaitingConfirmation, Downloading)
        | (Downloading, Snapshotting)
        | (Applying, AwaitingReboot)
        | (Applying, Completed)
        | (AwaitingReboot, VerifyingBoot)
        | (VerifyingBoot, Completed)
        | (Blocked, Preflight)
        | (Blocked, Idle) => true,
        (Snapshotting, Applying) => matches!(operation, OperationKind::UpdateWithinRelease),
        (Snapshotting, ReadyToReboot)
        | (ReadyToReboot, ApplyingOffline)
        | (ApplyingOffline, AwaitingReboot) => matches!(operation, OperationKind::ReleaseUpgrade),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(kind: OperationKind, state: OperationState) -> OperationStateRecord {
        OperationStateRecord {
            schema_version: STATE_SCHEMA_VERSION,
            operation_id: "00000000-0000-4000-8000-000000000000".into(),
            sequence: 0,
            operation: kind,
            state,
            source: ReleaseIdentity {
                version: "2026.08-alpha6".into(),
                edition: "desktop".into(),
                architecture: "x86_64".into(),
                build_id: "test".into(),
            },
            target: None,
            plan_sha256: "0".repeat(64),
            manifest_sha256: None,
            snapshot_number: None,
            last_completed_step: None,
            error_code: None,
            boot_verification: None,
            created_at: "2026-08-18T00:00:00Z".into(),
            updated_at: "2026-08-18T00:00:00Z".into(),
        }
    }

    #[test]
    fn applying_requires_persisted_snapshot() {
        let mut state = record(
            OperationKind::UpdateWithinRelease,
            OperationState::Snapshotting,
        );
        assert_eq!(
            state.transition_to(OperationState::Applying),
            Err(TransitionError::SnapshotNotRecorded)
        );
        state.snapshot_number = Some(42);
        assert_eq!(state.transition_to(OperationState::Applying), Ok(()));
    }

    #[test]
    fn release_upgrade_cannot_use_online_apply_path() {
        assert!(!valid_transition(
            OperationKind::ReleaseUpgrade,
            OperationState::Snapshotting,
            OperationState::Applying
        ));
        assert!(valid_transition(
            OperationKind::ReleaseUpgrade,
            OperationState::Snapshotting,
            OperationState::ReadyToReboot
        ));
    }

    #[test]
    fn update_cannot_enter_offline_release_upgrade_path() {
        assert!(!valid_transition(
            OperationKind::UpdateWithinRelease,
            OperationState::Snapshotting,
            OperationState::ReadyToReboot
        ));
    }
}
