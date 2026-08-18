use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::{HostFacts, PreflightIssue, PreflightPolicy, PreflightReport, evaluate_preflight};

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "PascalCase")]
pub enum PackageAction {
    Install,
    Upgrade,
    Downgrade,
    Remove,
    Reinstall,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PackageChange {
    pub name: String,
    pub architecture: String,
    pub action: PackageAction,
    pub current_version: Option<String>,
    pub proposed_version: Option<String>,
    pub current_vendor: Option<String>,
    pub proposed_vendor: Option<String>,
    pub repository_alias: Option<String>,
    pub download_bytes: u64,
    pub installed_size_before: u64,
    pub installed_size_after: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SolverResult {
    pub schema_version: u32,
    pub successful: bool,
    pub problems: Vec<String>,
    pub metadata_valid_repositories: Vec<String>,
    pub changes: Vec<PackageChange>,
    pub download_bytes: u64,
    pub transaction_size_increase: u64,
    pub estimated_snapshot_bytes: u64,
    pub reboot_required: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VendorTransition {
    pub from: String,
    pub to: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SolverPolicy {
    pub allowed_removals: Vec<String>,
    pub allowed_vendor_transitions: Vec<VendorTransition>,
    /// Release policy supplies these groups. Empty does not invent coupling
    /// between otherwise independent Lyra RPMs.
    pub lockstep_packages: Vec<Vec<String>>,
}

pub fn evaluate_solver_preflight(
    facts: &HostFacts,
    preflight_policy: PreflightPolicy,
    solver: &SolverResult,
    solver_policy: &SolverPolicy,
) -> PreflightReport {
    let mut enriched = facts.clone();
    let valid_repositories: BTreeSet<_> = solver.metadata_valid_repositories.iter().collect();
    for repository in &mut enriched.repositories {
        if repository.enabled {
            repository.metadata_valid = valid_repositories.contains(&repository.alias);
        }
    }
    enriched.required_download_bytes = solver.download_bytes;
    enriched.required_transaction_bytes = solver.transaction_size_increase;
    enriched.required_snapshot_bytes = solver.estimated_snapshot_bytes;

    let mut report = evaluate_preflight(&enriched, preflight_policy);
    if solver.schema_version != 1 {
        report
            .blockers
            .push(PreflightIssue::UnsupportedSolverSchema);
    }
    if !solver.successful || !solver.problems.is_empty() {
        report.blockers.push(PreflightIssue::SolverFailed);
    }
    for change in &solver.changes {
        if matches!(change.action, PackageAction::Downgrade) {
            report.blockers.push(PreflightIssue::UnauthorizedDowngrade {
                package: change.name.clone(),
            });
        }
        if matches!(change.action, PackageAction::Remove)
            && !solver_policy.allowed_removals.contains(&change.name)
        {
            report.blockers.push(PreflightIssue::UnauthorizedRemoval {
                package: change.name.clone(),
            });
        }
        if let (Some(from), Some(to)) = (&change.current_vendor, &change.proposed_vendor)
            && from != to
            && !solver_policy
                .allowed_vendor_transitions
                .iter()
                .any(|allowed| allowed.from == *from && allowed.to == *to)
        {
            report
                .blockers
                .push(PreflightIssue::UnauthorizedVendorChange {
                    package: change.name.clone(),
                });
        }
    }
    for group in &solver_policy.lockstep_packages {
        let changed: Vec<_> = group
            .iter()
            .filter(|package| {
                solver
                    .changes
                    .iter()
                    .any(|change| change.name == package.as_str())
            })
            .collect();
        if !changed.is_empty() && changed.len() != group.len() {
            report.blockers.push(PreflightIssue::LockstepViolation);
        }
    }
    report.blockers.sort();
    report.blockers.dedup();
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ReleaseIdentity, RepositoryFact};

    fn facts() -> HostFacts {
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
            required_download_bytes: 0,
            required_transaction_bytes: 0,
            required_snapshot_bytes: 0,
            on_battery: false,
            battery_percent: None,
            secure_boot_enabled: Some(true),
            repositories: vec![RepositoryFact {
                alias: "repo-lyra".into(),
                enabled: true,
                official: true,
                metadata_valid: false,
                signing_key_trusted: true,
            }],
            held_packages: vec![],
            orphaned_packages: vec![],
        }
    }

    fn change(name: &str, action: PackageAction) -> PackageChange {
        PackageChange {
            name: name.into(),
            architecture: "x86_64".into(),
            action,
            current_version: Some("1".into()),
            proposed_version: Some("2".into()),
            current_vendor: Some("openSUSE".into()),
            proposed_vendor: Some("openSUSE".into()),
            repository_alias: Some("repo-lyra".into()),
            download_bytes: 1024,
            installed_size_before: 1024,
            installed_size_after: 2048,
        }
    }

    fn solver(changes: Vec<PackageChange>) -> SolverResult {
        SolverResult {
            schema_version: 1,
            successful: true,
            problems: vec![],
            metadata_valid_repositories: vec!["repo-lyra".into()],
            changes,
            download_bytes: 1024,
            transaction_size_increase: 1024,
            estimated_snapshot_bytes: 4096,
            reboot_required: false,
        }
    }

    #[test]
    fn successful_solver_proves_metadata_and_space_estimate() {
        let report = evaluate_solver_preflight(
            &facts(),
            PreflightPolicy {
                free_space_margin_bytes: 0,
                ..PreflightPolicy::default()
            },
            &solver(vec![change("firefox", PackageAction::Upgrade)]),
            &SolverPolicy::default(),
        );
        assert!(report.passed(), "{:?}", report.blockers);
        assert_eq!(report.required_bytes, 1024 + 1024 + 4096);
    }

    #[test]
    fn downgrade_removal_vendor_change_and_partial_lockstep_are_blocked() {
        let mut downgrade = change("firefox", PackageAction::Downgrade);
        downgrade.proposed_vendor = Some("unknown".into());
        let policy = SolverPolicy {
            lockstep_packages: vec![vec![
                "lyra-release".into(),
                "lyra-installer".into(),
                "lyra-upgrade".into(),
            ]],
            ..SolverPolicy::default()
        };
        let report = evaluate_solver_preflight(
            &facts(),
            PreflightPolicy::default(),
            &solver(vec![
                downgrade,
                change("old-package", PackageAction::Remove),
                change("lyra-release", PackageAction::Upgrade),
            ]),
            &policy,
        );
        assert!(report.blockers.iter().any(|issue| matches!(
            issue,
            PreflightIssue::UnauthorizedDowngrade { package } if package == "firefox"
        )));
        assert!(report.blockers.iter().any(|issue| matches!(
            issue,
            PreflightIssue::UnauthorizedRemoval { package } if package == "old-package"
        )));
        assert!(report.blockers.contains(&PreflightIssue::LockstepViolation));
    }
}
