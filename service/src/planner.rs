use std::process::{Command, Stdio};

use lyra_upgrade_core::{
    OperationKind, PlanError, PreflightPolicy, ReleaseManifest, SolverPolicy, SystemBackend,
    build_plan, discover_host, evaluate_solver_preflight,
};
use lyra_upgrade_protocol::PlannedUpdate;
use sha2::{Digest, Sha256};

use crate::manifest_fetch::{FetchError, fetch_repository_key};
use crate::repository_context::{PreparedDiscovery, RepositoryContext};
use crate::solver_xml::{SolverXmlError, parse_solver_xml};
use crate::vendor_metadata::{VendorMetadataError, enrich_solver_vendors};

#[derive(Debug)]
pub enum PlannerError {
    PlanChanged,
    Discovery(lyra_upgrade_core::DiscoverError),
    Spawn(std::io::Error),
    SolverExit { code: Option<i32>, stderr: String },
    SolverXml(SolverXmlError),
    VendorMetadata(VendorMetadataError),
    Blocked(Vec<lyra_upgrade_core::PreflightIssue>),
    Plan(PlanError),
    Serialize(serde_json::Error),
    RepositoryKey(FetchError),
}

pub fn plan_update_with_cached_metadata() -> Result<PlannedUpdate, PlannerError> {
    let facts = discover_host(&SystemBackend).map_err(PlannerError::Discovery)?;
    let output = Command::new("zypper")
        .args([
            "--xmlout",
            "--non-interactive",
            "--no-refresh",
            "update",
            "--dry-run",
            "--details",
            "--no-allow-downgrade",
            "--no-allow-name-change",
            "--no-allow-arch-change",
            "--no-allow-vendor-change",
        ])
        .env_clear()
        .env("PATH", "/usr/sbin:/usr/bin:/sbin:/bin")
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .output()
        .map_err(PlannerError::Spawn)?;
    if !output.status.success() {
        return Err(PlannerError::SolverExit {
            code: output.status.code(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        });
    }
    let xml = String::from_utf8_lossy(&output.stdout);
    let metadata_valid_repositories = facts
        .repositories
        .iter()
        .filter(|repository| repository.enabled && repository.signing_key_trusted)
        .map(|repository| repository.alias.clone())
        .collect();
    let mut solver =
        parse_solver_xml(&xml, metadata_valid_repositories, 0).map_err(PlannerError::SolverXml)?;
    enrich_solver_vendors(&mut solver, std::path::Path::new("/var/cache/zypp/raw"))
        .map_err(PlannerError::VendorMetadata)?;
    let preflight = evaluate_solver_preflight(
        &facts,
        PreflightPolicy::default(),
        &solver,
        &SolverPolicy::default(),
    );
    if !preflight.passed() {
        return Err(PlannerError::Blocked(preflight.blockers.clone()));
    }
    let plan = build_plan(
        OperationKind::UpdateWithinRelease,
        &facts,
        &preflight,
        None,
        None,
        &solver,
    )
    .map_err(PlannerError::Plan)?;
    let plan_sha256 = plan.sha256().map_err(PlannerError::Serialize)?;
    Ok(PlannedUpdate {
        facts,
        solver,
        preflight,
        plan,
        plan_sha256,
        manifest: None,
    })
}

pub fn plan_release_upgrade(manifest: &ReleaseManifest) -> Result<PlannedUpdate, PlannerError> {
    let simulation = tempfile::Builder::new()
        .prefix("lyra-upgrade-solver-")
        .tempdir()
        .map_err(PlannerError::Spawn)?;
    let context = RepositoryContext::prepared(simulation.path());
    let repos_dir = &context.repos;
    let raw_dir = &context.raw;
    let keys_dir = simulation.path().join("keys");
    std::fs::create_dir_all(repos_dir).map_err(PlannerError::Spawn)?;
    std::fs::create_dir_all(&keys_dir).map_err(PlannerError::Spawn)?;
    for repository in &manifest.repositories {
        let key_path = keys_dir.join(format!("{}.asc", repository.alias));
        fetch_repository_key(repository, &key_path).map_err(PlannerError::RepositoryKey)?;
        let content = format!(
            "[{alias}]\nname={alias}\nenabled=1\nautorefresh=0\nkeeppackages=0\nbaseurl={url}\ngpgkey={key_url}\ntype=rpm-md\ngpgcheck=1\npriority={priority}\n",
            alias = repository.alias,
            url = repository.base_url,
            key_url = format_args!("file://{}", key_path.display()),
            priority = repository.priority,
        );
        std::fs::write(
            repos_dir.join(format!("{}.repo", repository.alias)),
            content,
        )
        .map_err(PlannerError::Spawn)?;
    }
    let refresh = run_with_simulation(&context, &["refresh"])?;
    if !refresh.status.success() {
        return Err(PlannerError::SolverExit {
            code: refresh.status.code(),
            stderr: String::from_utf8_lossy(&refresh.stderr).into_owned(),
        });
    }
    let dry_run = run_with_simulation(
        &context,
        &[
            "--xmlout",
            "--no-refresh",
            "dist-upgrade",
            "--dry-run",
            "--details",
            "--no-allow-downgrade",
            "--no-allow-name-change",
            "--no-allow-arch-change",
            "--allow-vendor-change",
        ],
    )?;
    if !dry_run.status.success() {
        return Err(PlannerError::SolverExit {
            code: dry_run.status.code(),
            stderr: String::from_utf8_lossy(&dry_run.stderr).into_owned(),
        });
    }
    let facts =
        discover_host(&PreparedDiscovery { context: &context }).map_err(PlannerError::Discovery)?;
    let metadata = manifest
        .repositories
        .iter()
        .map(|repository| repository.alias.clone())
        .collect();
    let mut solver = parse_solver_xml(&String::from_utf8_lossy(&dry_run.stdout), metadata, 0)
        .map_err(PlannerError::SolverXml)?;
    enrich_solver_vendors(&mut solver, raw_dir).map_err(PlannerError::VendorMetadata)?;
    let preflight = evaluate_solver_preflight(
        &facts,
        PreflightPolicy {
            minimum_free_space_bytes: manifest.minimum_free_space_bytes,
            ..PreflightPolicy::default()
        },
        &solver,
        &manifest.solver_policy(),
    );
    if !preflight.passed() {
        return Err(PlannerError::Blocked(preflight.blockers.clone()));
    }
    let canonical_manifest = serde_json::to_vec(manifest).map_err(PlannerError::Serialize)?;
    let manifest_sha256 = format!("{:x}", Sha256::digest(canonical_manifest));
    let plan = build_plan(
        OperationKind::ReleaseUpgrade,
        &facts,
        &preflight,
        Some(manifest.target.clone()),
        Some(manifest_sha256),
        &solver,
    )
    .map_err(PlannerError::Plan)?;
    let plan_sha256 = plan.sha256().map_err(PlannerError::Serialize)?;
    Ok(PlannedUpdate {
        facts,
        solver,
        preflight,
        plan,
        plan_sha256,
        manifest: Some(manifest.clone()),
    })
}

fn run_with_simulation(
    context: &RepositoryContext,
    arguments: &[&str],
) -> Result<std::process::Output, PlannerError> {
    context
        .command()
        .arg("--non-interactive")
        .args(arguments)
        .output()
        .map_err(PlannerError::Spawn)
}

/// Shared authorization boundary for staging and the offline executor.
/// The allowlist itself must be the one bound into the approved plan.
pub fn check_confirmed_release_plan(
    manifest: &ReleaseManifest,
    confirmed: &lyra_upgrade_core::UpgradePlan,
    expected_hash: &str,
) -> Result<(), PlannerError> {
    let manifest_hash = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(manifest).map_err(PlannerError::Serialize)?)
    );
    if confirmed.operation != OperationKind::ReleaseUpgrade
        || confirmed.target.as_ref() != Some(&manifest.target)
        || confirmed.manifest_sha256.as_deref() != Some(manifest_hash.as_str())
        || confirmed.sha256().map_err(PlannerError::Serialize)? != expected_hash
    {
        return Err(PlannerError::PlanChanged);
    }
    let blockers = lyra_upgrade_core::vendor_policy_blockers(
        &confirmed.package_changes,
        &manifest.solver_policy(),
    );
    if !blockers.is_empty() {
        return Err(PlannerError::Blocked(blockers));
    }
    Ok(())
}

pub fn revalidate_release_plan(
    facts: &lyra_upgrade_core::HostFacts,
    solver: &lyra_upgrade_core::SolverResult,
    manifest: &ReleaseManifest,
    confirmed: &lyra_upgrade_core::UpgradePlan,
    expected_hash: &str,
) -> Result<(), PlannerError> {
    check_confirmed_release_plan(manifest, confirmed, expected_hash)?;
    let report = evaluate_solver_preflight(
        facts,
        PreflightPolicy {
            minimum_free_space_bytes: manifest.minimum_free_space_bytes,
            ..PreflightPolicy::default()
        },
        solver,
        &manifest.solver_policy(),
    );
    if !report.passed() {
        return Err(PlannerError::Blocked(report.blockers));
    }
    let mut rebuilt = build_plan(
        OperationKind::ReleaseUpgrade,
        facts,
        &report,
        Some(manifest.target.clone()),
        confirmed.manifest_sha256.clone(),
        solver,
    )
    .map_err(PlannerError::Plan)?;
    // Cached downloads change the remaining space estimate, not the reviewed
    // transaction. The fresh estimate was still checked above before proceeding.
    rebuilt.required_bytes = confirmed.required_bytes;
    if rebuilt.sha256().map_err(PlannerError::Serialize)? != expected_hash {
        return Err(PlannerError::PlanChanged);
    }
    Ok(())
}
