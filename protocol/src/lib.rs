//! Typed protocol shared by the unprivileged clients and privileged service.

use std::collections::BTreeMap;

use lyra_upgrade_core::{
    HostFacts, OperationState, PreflightReport, ReleaseManifest, SolverResult, UpgradePlan,
};
use serde::{Deserialize, Serialize};

pub const PROTOCOL_VERSION: u32 = 3;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum Request {
    CheckRelease {
        protocol_version: u32,
        request_id: String,
    },
    ReadTrustState {
        protocol_version: u32,
        request_id: String,
    },
    Inspect {
        protocol_version: u32,
        request_id: String,
    },
    PlanUpdate {
        protocol_version: u32,
        request_id: String,
    },
    PlanReleaseUpgrade {
        protocol_version: u32,
        request_id: String,
        manifest_sha256: String,
    },
    Start {
        protocol_version: u32,
        request_id: String,
        operation_id: String,
        plan_sha256: String,
        confirmed: bool,
        planned: Box<PlannedUpdate>,
    },
    Status {
        protocol_version: u32,
        request_id: String,
        operation_id: String,
        after_sequence: Option<u64>,
    },
    Cancel {
        protocol_version: u32,
        request_id: String,
        operation_id: String,
    },
    AcknowledgeRecovery {
        protocol_version: u32,
        request_id: String,
        operation_id: String,
        recovery_action: RecoveryAction,
    },
}

impl Request {
    pub const fn protocol_version(&self) -> u32 {
        match self {
            Self::CheckRelease {
                protocol_version, ..
            }
            | Self::ReadTrustState {
                protocol_version, ..
            } => *protocol_version,
            Self::Inspect {
                protocol_version, ..
            }
            | Self::PlanUpdate {
                protocol_version, ..
            }
            | Self::PlanReleaseUpgrade {
                protocol_version, ..
            }
            | Self::Start {
                protocol_version, ..
            }
            | Self::Status {
                protocol_version, ..
            }
            | Self::Cancel {
                protocol_version, ..
            }
            | Self::AcknowledgeRecovery {
                protocol_version, ..
            } => *protocol_version,
        }
    }

    pub const fn is_supported(&self) -> bool {
        self.protocol_version() == PROTOCOL_VERSION
    }

    pub fn request_id(&self) -> &str {
        match self {
            Self::CheckRelease { request_id, .. } | Self::ReadTrustState { request_id, .. } => {
                request_id
            }
            Self::Inspect { request_id, .. }
            | Self::PlanUpdate { request_id, .. }
            | Self::PlanReleaseUpgrade { request_id, .. }
            | Self::Start { request_id, .. }
            | Self::Status { request_id, .. }
            | Self::Cancel { request_id, .. }
            | Self::AcknowledgeRecovery { request_id, .. } => request_id,
        }
    }

    pub fn needs_authorization(&self) -> bool {
        matches!(
            self,
            Self::Start { .. }
                | Self::Cancel { .. }
                | Self::AcknowledgeRecovery {
                    recovery_action: RecoveryAction::Rollback | RecoveryAction::KeepCurrent,
                    ..
                }
        )
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum RecoveryAction {
    ShowDiagnostics,
    Rollback,
    KeepCurrent,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum Response {
    PreflightBlocked {
        request_id: String,
        blockers: Vec<lyra_upgrade_core::PreflightIssue>,
    },
    ReleaseOffer {
        request_id: String,
        manifest_sha256: String,
        manifest: Box<ReleaseManifest>,
        cached: bool,
    },
    TrustState {
        request_id: String,
        last_manifest_sequence: Option<u64>,
    },
    Rejected {
        request_id: String,
        error_code: String,
    },
    Accepted {
        request_id: String,
        operation_id: Option<String>,
    },
    Inspection {
        request_id: String,
        facts: Box<HostFacts>,
        preflight: PreflightReport,
    },
    Plan {
        request_id: String,
        operation_id: String,
        plan_sha256: String,
        plan: Box<UpgradePlan>,
        preflight: PreflightReport,
        planned: Box<PlannedUpdate>,
    },
    Status {
        request_id: String,
        operation_id: String,
        sequence: u64,
        state: OperationState,
        snapshot_number: Option<u64>,
        #[serde(default)]
        recovered: bool,
        error_code: Option<String>,
        events: Vec<OperationEvent>,
        #[serde(default)]
        report: Option<lyra_upgrade_core::InventoryReport>,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PlannedUpdate {
    pub facts: HostFacts,
    pub solver: SolverResult,
    pub preflight: PreflightReport,
    pub plan: UpgradePlan,
    pub plan_sha256: String,
    pub manifest: Option<ReleaseManifest>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub enum EventLevel {
    Info,
    Warning,
    Error,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub enum EventSource {
    Service,
    ZypperStdout,
    ZypperStderr,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TechnicalLine {
    pub source: EventSource,
    pub text: String,
    pub truncated: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OperationEvent {
    pub operation_id: String,
    pub sequence: u64,
    pub occurred_at: String,
    pub state: OperationState,
    pub level: EventLevel,
    pub message_id: String,
    pub fields: BTreeMap<String, String>,
    pub technical: Option<TechnicalLine>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_admin_operations_request_authorization() {
        for request in [
            Request::Inspect {
                protocol_version: 3,
                request_id: "a".into(),
            },
            Request::PlanUpdate {
                protocol_version: 3,
                request_id: "a".into(),
            },
            Request::PlanReleaseUpgrade {
                protocol_version: 3,
                request_id: "a".into(),
                manifest_sha256: "a".repeat(64),
            },
            Request::CheckRelease {
                protocol_version: 3,
                request_id: "a".into(),
            },
            Request::Status {
                protocol_version: 3,
                request_id: "a".into(),
                operation_id: "a".into(),
                after_sequence: None,
            },
            Request::AcknowledgeRecovery {
                protocol_version: 3,
                request_id: "a".into(),
                operation_id: "a".into(),
                recovery_action: RecoveryAction::ShowDiagnostics,
            },
        ] {
            assert!(!request.needs_authorization());
        }
        assert!(
            Request::Cancel {
                protocol_version: 3,
                request_id: "a".into(),
                operation_id: "a".into()
            }
            .needs_authorization()
        );
        for recovery_action in [RecoveryAction::Rollback, RecoveryAction::KeepCurrent] {
            assert!(
                Request::AcknowledgeRecovery {
                    protocol_version: 3,
                    request_id: "a".into(),
                    operation_id: "a".into(),
                    recovery_action
                }
                .needs_authorization()
            );
        }
    }

    #[test]
    fn rejects_unknown_fields_during_deserialization() {
        let json = r#"{"kind":"Inspect","protocol_version":2,"request_id":"x","command":"sh"}"#;
        assert!(serde_json::from_str::<Request>(json).is_err());
    }

    #[test]
    fn unknown_protocol_is_not_supported() {
        let request = Request::Inspect {
            protocol_version: 99,
            request_id: "x".into(),
        };
        assert!(!request.is_supported());
    }
}
