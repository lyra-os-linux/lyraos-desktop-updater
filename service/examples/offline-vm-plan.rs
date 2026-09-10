//! Test-only plan generator. Refuses to run outside the disposable offline VM.
use lyra_upgrade_core::{
    OperationKind, OperationState, OperationStateRecord, PreflightPolicy, ReleaseManifest,
    build_plan, discover_host, evaluate_solver_preflight, save_state,
};
use lyra_upgrade_service::{
    repository_context::{PreparedDiscovery, RepositoryContext},
    solver_xml::parse_solver_xml,
    vendor_metadata::enrich_solver_vendors,
};
use sha2::{Digest, Sha256};
use std::{fs, path::Path};

fn main() {
    let cmdline = fs::read_to_string("/proc/cmdline").unwrap();
    assert!(
        cmdline
            .split_whitespace()
            .any(|arg| arg == "lyra.updater-offline-test=1"),
        "disposable VM required"
    );
    let id = "00000000-0000-4000-8000-000000000006";
    let root = Path::new("/var/lib/lyra-upgrade/operations");
    let dir = root.join(id);
    let manifest: ReleaseManifest =
        serde_json::from_slice(&fs::read(dir.join("manifest.json")).unwrap()).unwrap();
    let context = RepositoryContext::prepared(&dir);
    let facts = discover_host(&PreparedDiscovery { context: &context }).unwrap();
    let output = context
        .command()
        .args([
            "--xmlout",
            "--non-interactive",
            "--no-refresh",
            "dist-upgrade",
            "--dry-run",
            "--details",
            "--no-allow-downgrade",
            "--no-allow-name-change",
            "--no-allow-arch-change",
            "--allow-vendor-change",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    let aliases = manifest
        .repositories
        .iter()
        .map(|repo| repo.alias.clone())
        .collect();
    let mut solver =
        parse_solver_xml(&String::from_utf8_lossy(&output.stdout), aliases, 0).unwrap();
    enrich_solver_vendors(&mut solver, &context.raw).unwrap();
    let report = evaluate_solver_preflight(
        &facts,
        PreflightPolicy {
            minimum_free_space_bytes: manifest.minimum_free_space_bytes,
            ..PreflightPolicy::default()
        },
        &solver,
        &manifest.solver_policy(),
    );
    assert!(report.passed(), "{:?}", report.blockers);
    let manifest_hash = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&manifest).unwrap())
    );
    let plan = build_plan(
        OperationKind::ReleaseUpgrade,
        &facts,
        &report,
        Some(manifest.target.clone()),
        Some(manifest_hash.clone()),
        &solver,
    )
    .unwrap();
    let after = discover_host(&PreparedDiscovery { context: &context }).unwrap();
    assert_eq!(facts.held_packages, after.held_packages);
    assert_eq!(facts.orphaned_packages, after.orphaned_packages);
    fs::write(
        dir.join("plan.json"),
        serde_json::to_vec_pretty(&plan).unwrap(),
    )
    .unwrap();
    let snapshot_number = fs::read_to_string("/test/snapshot-number")
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    let state = OperationStateRecord {
        schema_version: 1,
        operation_id: id.into(),
        sequence: 1,
        operation: OperationKind::ReleaseUpgrade,
        state: OperationState::ReadyToReboot,
        source: facts.release,
        target: Some(manifest.target),
        plan_sha256: plan.sha256().unwrap(),
        manifest_sha256: Some(manifest_hash),
        snapshot_number: Some(snapshot_number),
        last_completed_step: None,
        error_code: None,
        boot_verification: None,
        created_at: "2026-09-09T00:00:00Z".into(),
        updated_at: "2026-09-09T00:00:00Z".into(),
    };
    save_state(root, &state).unwrap();
    std::os::unix::fs::symlink(&dir, "/system-update").unwrap();
    println!("PLAN_READY {}", state.plan_sha256);
}
