use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{OperationKind, ReleaseIdentity};

pub const PLAN_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RepositoryFact {
    pub alias: String,
    pub enabled: bool,
    pub official: bool,
    pub metadata_valid: bool,
    pub signing_key_trusted: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HostFacts {
    pub release: ReleaseIdentity,
    pub root_filesystem: String,
    pub snapper_root_configured: bool,
    pub rpm_database_healthy: bool,
    pub package_lock_free: bool,
    pub available_bytes: u64,
    pub required_download_bytes: u64,
    pub required_transaction_bytes: u64,
    pub required_snapshot_bytes: u64,
    pub on_battery: bool,
    pub battery_percent: Option<u8>,
    pub secure_boot_enabled: Option<bool>,
    pub repositories: Vec<RepositoryFact>,
    pub held_packages: Vec<String>,
    pub orphaned_packages: Vec<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PreflightPolicy {
    pub minimum_battery_percent: u8,
    pub free_space_margin_bytes: u64,
    pub require_secure_boot_fact: bool,
}

impl Default for PreflightPolicy {
    fn default() -> Self {
        Self {
            minimum_battery_percent: 40,
            free_space_margin_bytes: 1024 * 1024 * 1024,
            require_secure_boot_fact: true,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PreflightIssue {
    UnsupportedEdition,
    UnsupportedArchitecture,
    RootNotBtrfs,
    SnapperUnavailable,
    RpmDatabaseUnhealthy,
    PackageManagerBusy,
    InsufficientSpace,
    BatteryTooLow,
    BatteryStateUnknown,
    SecureBootStateUnknown,
    RepositoryMetadataInvalid { alias: String },
    RepositoryKeyUntrusted { alias: String },
    UnsupportedSolverSchema,
    SolverFailed,
    UnauthorizedDowngrade { package: String },
    UnauthorizedRemoval { package: String },
    UnauthorizedVendorChange { package: String },
    LockstepViolation,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PreflightReport {
    pub required_bytes: u64,
    pub available_bytes: u64,
    pub blockers: Vec<PreflightIssue>,
    pub third_party_repositories: Vec<String>,
    pub held_packages: Vec<String>,
    pub orphaned_packages: Vec<String>,
}

impl PreflightReport {
    pub fn passed(&self) -> bool {
        self.blockers.is_empty()
    }
}

pub fn evaluate_preflight(facts: &HostFacts, policy: PreflightPolicy) -> PreflightReport {
    let mut blockers = Vec::new();
    if facts.release.edition != "desktop" {
        blockers.push(PreflightIssue::UnsupportedEdition);
    }
    if facts.release.architecture != "x86_64" {
        blockers.push(PreflightIssue::UnsupportedArchitecture);
    }
    if facts.root_filesystem != "btrfs" {
        blockers.push(PreflightIssue::RootNotBtrfs);
    }
    if !facts.snapper_root_configured {
        blockers.push(PreflightIssue::SnapperUnavailable);
    }
    if !facts.rpm_database_healthy {
        blockers.push(PreflightIssue::RpmDatabaseUnhealthy);
    }
    if !facts.package_lock_free {
        blockers.push(PreflightIssue::PackageManagerBusy);
    }

    let required_bytes = facts
        .required_download_bytes
        .saturating_add(facts.required_transaction_bytes)
        .saturating_add(facts.required_snapshot_bytes)
        .saturating_add(policy.free_space_margin_bytes);
    if facts.available_bytes < required_bytes {
        blockers.push(PreflightIssue::InsufficientSpace);
    }

    if facts.on_battery {
        match facts.battery_percent {
            Some(percent) if percent < policy.minimum_battery_percent => {
                blockers.push(PreflightIssue::BatteryTooLow);
            }
            None => blockers.push(PreflightIssue::BatteryStateUnknown),
            Some(_) => {}
        }
    }
    if policy.require_secure_boot_fact && facts.secure_boot_enabled.is_none() {
        blockers.push(PreflightIssue::SecureBootStateUnknown);
    }

    let mut third_party_repositories = Vec::new();
    for repository in facts
        .repositories
        .iter()
        .filter(|repository| repository.enabled)
    {
        if !repository.metadata_valid {
            blockers.push(PreflightIssue::RepositoryMetadataInvalid {
                alias: repository.alias.clone(),
            });
        }
        if !repository.signing_key_trusted {
            blockers.push(PreflightIssue::RepositoryKeyUntrusted {
                alias: repository.alias.clone(),
            });
        }
        if !repository.official {
            third_party_repositories.push(repository.alias.clone());
        }
    }

    blockers.sort();
    blockers.dedup();
    third_party_repositories.sort();
    third_party_repositories.dedup();
    let mut held_packages = facts.held_packages.clone();
    held_packages.sort();
    held_packages.dedup();
    let mut orphaned_packages = facts.orphaned_packages.clone();
    orphaned_packages.sort();
    orphaned_packages.dedup();

    PreflightReport {
        required_bytes,
        available_bytes: facts.available_bytes,
        blockers,
        third_party_repositories,
        held_packages,
        orphaned_packages,
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct UpgradePlan {
    pub schema_version: u32,
    pub operation: OperationKind,
    pub source: ReleaseIdentity,
    pub target: Option<ReleaseIdentity>,
    pub manifest_sha256: Option<String>,
    pub required_bytes: u64,
    pub facts: BTreeMap<String, String>,
    pub third_party_repositories: Vec<String>,
    pub held_packages: Vec<String>,
    pub orphaned_packages: Vec<String>,
    pub package_changes: Vec<crate::PackageChange>,
    pub reboot_required: bool,
}

impl UpgradePlan {
    pub fn canonical_json(&self) -> Result<Vec<u8>, serde_json::Error> {
        serde_json::to_vec(self)
    }

    pub fn sha256(&self) -> Result<String, serde_json::Error> {
        let digest = Sha256::digest(self.canonical_json()?);
        Ok(format!("{digest:x}"))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PlanError {
    Blocked(Vec<PreflightIssue>),
    TargetRequired,
    ManifestRequired,
    TargetNotAllowed,
}

pub fn build_plan(
    operation: OperationKind,
    facts: &HostFacts,
    report: &PreflightReport,
    target: Option<ReleaseIdentity>,
    manifest_sha256: Option<String>,
    solver: &crate::SolverResult,
) -> Result<UpgradePlan, PlanError> {
    if !report.passed() {
        return Err(PlanError::Blocked(report.blockers.clone()));
    }
    if matches!(operation, OperationKind::ReleaseUpgrade) {
        let target_release = target.as_ref().ok_or(PlanError::TargetRequired)?;
        if target_release.edition != facts.release.edition
            || target_release.architecture != facts.release.architecture
            || target_release.version == facts.release.version
        {
            return Err(PlanError::TargetNotAllowed);
        }
        if manifest_sha256
            .as_deref()
            .is_none_or(|hash| !is_sha256(hash))
        {
            return Err(PlanError::ManifestRequired);
        }
    } else if target.is_some() || manifest_sha256.is_some() {
        return Err(PlanError::TargetNotAllowed);
    }

    let facts_map = BTreeMap::from([
        ("root_filesystem".into(), facts.root_filesystem.clone()),
        (
            "secure_boot".into(),
            facts
                .secure_boot_enabled
                .map(|enabled| enabled.to_string())
                .unwrap_or_else(|| "unknown".into()),
        ),
    ]);
    Ok(UpgradePlan {
        schema_version: PLAN_SCHEMA_VERSION,
        operation,
        source: facts.release.clone(),
        target,
        manifest_sha256,
        required_bytes: report.required_bytes,
        facts: facts_map,
        third_party_repositories: report.third_party_repositories.clone(),
        held_packages: report.held_packages.clone(),
        orphaned_packages: report.orphaned_packages.clone(),
        package_changes: {
            let mut changes = solver.changes.clone();
            changes.sort();
            changes
        },
        reboot_required: solver.reboot_required,
    })
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn healthy_facts() -> HostFacts {
        HostFacts {
            release: ReleaseIdentity {
                version: "2026.08-alpha6".into(),
                edition: "desktop".into(),
                architecture: "x86_64".into(),
                build_id: "fixture".into(),
            },
            root_filesystem: "btrfs".into(),
            snapper_root_configured: true,
            rpm_database_healthy: true,
            package_lock_free: true,
            available_bytes: 8 * 1024 * 1024 * 1024,
            required_download_bytes: 1024,
            required_transaction_bytes: 2048,
            required_snapshot_bytes: 4096,
            on_battery: false,
            battery_percent: None,
            secure_boot_enabled: Some(true),
            repositories: vec![RepositoryFact {
                alias: "lyra".into(),
                enabled: true,
                official: true,
                metadata_valid: true,
                signing_key_trusted: true,
            }],
            held_packages: vec![],
            orphaned_packages: vec![],
        }
    }

    fn successful_solver() -> crate::SolverResult {
        crate::SolverResult {
            schema_version: 1,
            successful: true,
            problems: vec![],
            metadata_valid_repositories: vec!["lyra".into()],
            changes: vec![],
            download_bytes: 0,
            transaction_size_increase: 0,
            estimated_snapshot_bytes: 0,
            reboot_required: false,
        }
    }

    #[test]
    fn reports_all_independent_blockers_without_touching_the_host() {
        let mut facts = healthy_facts();
        facts.root_filesystem = "ext4".into();
        facts.snapper_root_configured = false;
        facts.rpm_database_healthy = false;
        facts.package_lock_free = false;
        facts.available_bytes = 0;
        facts.on_battery = true;
        facts.battery_percent = Some(5);
        facts.secure_boot_enabled = None;
        let report = evaluate_preflight(&facts, PreflightPolicy::default());
        assert_eq!(report.blockers.len(), 7);
        assert!(report.blockers.contains(&PreflightIssue::RootNotBtrfs));
        assert!(
            report
                .blockers
                .contains(&PreflightIssue::SnapperUnavailable)
        );
        assert!(report.blockers.contains(&PreflightIssue::InsufficientSpace));
    }

    #[test]
    fn plan_hash_is_stable_when_input_lists_arrive_in_another_order() {
        let mut first = healthy_facts();
        first.held_packages = vec!["zeta".into(), "alpha".into()];
        let mut second = first.clone();
        second.held_packages.reverse();
        let first_report = evaluate_preflight(&first, PreflightPolicy::default());
        let second_report = evaluate_preflight(&second, PreflightPolicy::default());
        let first_plan = build_plan(
            OperationKind::UpdateWithinRelease,
            &first,
            &first_report,
            None,
            None,
            &successful_solver(),
        )
        .unwrap();
        let second_plan = build_plan(
            OperationKind::UpdateWithinRelease,
            &second,
            &second_report,
            None,
            None,
            &successful_solver(),
        )
        .unwrap();
        assert_eq!(first_plan.sha256().unwrap(), second_plan.sha256().unwrap());
    }

    #[test]
    fn release_upgrade_requires_target_and_manifest() {
        let facts = healthy_facts();
        let report = evaluate_preflight(&facts, PreflightPolicy::default());
        assert_eq!(
            build_plan(
                OperationKind::ReleaseUpgrade,
                &facts,
                &report,
                None,
                None,
                &successful_solver()
            ),
            Err(PlanError::TargetRequired)
        );
    }
}
