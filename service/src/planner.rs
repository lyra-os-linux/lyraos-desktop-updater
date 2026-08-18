use std::process::{Command, Stdio};

use lyra_upgrade_core::{
    OperationKind, PlanError, PreflightPolicy, ReleaseManifest, SolverPolicy, SystemBackend,
    build_plan, discover_host, evaluate_solver_preflight,
};
use lyra_upgrade_protocol::PlannedUpdate;
use sha2::{Digest, Sha256};

use crate::solver_xml::{SolverXmlError, parse_solver_xml};

#[derive(Debug)]
pub enum PlannerError {
    Discovery(lyra_upgrade_core::DiscoverError),
    Spawn(std::io::Error),
    SolverExit { code: Option<i32>, stderr: String },
    SolverXml(SolverXmlError),
    Blocked(Vec<lyra_upgrade_core::PreflightIssue>),
    Plan(PlanError),
    Serialize(serde_json::Error),
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
    let solver =
        parse_solver_xml(&xml, metadata_valid_repositories, 0).map_err(PlannerError::SolverXml)?;
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
    let facts = discover_host(&SystemBackend).map_err(PlannerError::Discovery)?;
    let simulation = tempfile::Builder::new()
        .prefix("lyra-upgrade-solver-")
        .tempdir()
        .map_err(PlannerError::Spawn)?;
    let repos_dir = simulation.path().join("repos.d");
    let cache_dir = simulation.path().join("cache");
    let raw_dir = simulation.path().join("raw");
    let solv_dir = simulation.path().join("solv");
    let packages_dir = simulation.path().join("packages");
    std::fs::create_dir_all(&repos_dir).map_err(PlannerError::Spawn)?;
    for repository in &manifest.repositories {
        let content = format!(
            "[{alias}]\nname={alias}\nenabled=1\nautorefresh=0\nbaseurl={url}\ntype=rpm-md\ngpgcheck=1\npriority={priority}\n",
            alias = repository.alias,
            url = repository.base_url,
            priority = repository.priority,
        );
        std::fs::write(
            repos_dir.join(format!("{}.repo", repository.alias)),
            content,
        )
        .map_err(PlannerError::Spawn)?;
    }
    let paths = SimulationPaths {
        repos: &repos_dir,
        cache: &cache_dir,
        raw: &raw_dir,
        solv: &solv_dir,
        packages: &packages_dir,
    };
    let refresh = run_with_simulation(&paths, &["refresh"])?;
    if !refresh.status.success() {
        return Err(PlannerError::SolverExit {
            code: refresh.status.code(),
            stderr: String::from_utf8_lossy(&refresh.stderr).into_owned(),
        });
    }
    let dry_run = run_with_simulation(
        &paths,
        &[
            "--xmlout",
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
    let metadata = manifest
        .repositories
        .iter()
        .map(|repository| repository.alias.clone())
        .collect();
    let solver = parse_solver_xml(&String::from_utf8_lossy(&dry_run.stdout), metadata, 0)
        .map_err(PlannerError::SolverXml)?;
    let preflight = evaluate_solver_preflight(
        &facts,
        PreflightPolicy::default(),
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

struct SimulationPaths<'a> {
    repos: &'a std::path::Path,
    cache: &'a std::path::Path,
    raw: &'a std::path::Path,
    solv: &'a std::path::Path,
    packages: &'a std::path::Path,
}

fn run_with_simulation(
    paths: &SimulationPaths<'_>,
    arguments: &[&str],
) -> Result<std::process::Output, PlannerError> {
    Command::new("zypper")
        .arg("--non-interactive")
        .arg("--reposd-dir")
        .arg(paths.repos)
        .arg("--cache-dir")
        .arg(paths.cache)
        .arg("--raw-cache-dir")
        .arg(paths.raw)
        .arg("--solv-cache-dir")
        .arg(paths.solv)
        .arg("--pkg-cache-dir")
        .arg(paths.packages)
        .args(arguments)
        .env_clear()
        .env("PATH", "/usr/sbin:/usr/bin:/sbin:/bin")
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .output()
        .map_err(PlannerError::Spawn)
}
