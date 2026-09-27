use lyra_upgrade_core::migration::{MigrationError, pending_migration, validate_migration_plan};
use lyra_upgrade_core::{
    InstalledPackage, ManifestChannelPolicy, OperationKind, PackageAction, PackageChange,
    ReleaseManifest, validate_manifest_route,
};
use lyra_upgrade_service::migration::{TransactionMode, transaction_arguments, verify_payloads};
use serde_json::json;
use sha2::{Digest, Sha256};

fn manifest() -> ReleaseManifest {
    let identity = json!({"version":"1.1","edition":"desktop","architecture":"x86_64","build_id":"lyra-release-1.1"});
    serde_json::from_value(json!({
        "schema_version":1,"sequence":8,"status":"testing",
        "valid_from":"2026-09-27T00:00:00Z","valid_until":"2026-10-04T00:00:00Z",
        "source":identity,"target":identity,"minimum_updater_version":"0.2.7","minimum_free_space_bytes":1,
        "repositories":[{"alias":"fixes","base_url":"https://fixture.invalid/repo/","signing_key_url":"https://fixture.invalid/key","signing_key_fingerprint":"A".repeat(40),"priority":150}],
        "allowed_removals":[],"lockstep_packages":[],
        "allowed_vendor_transitions":[{"from":"SUSE","to":"OBS","packages":["portal","portal-lang"]}],
        "package_migration":[
            {"name":"portal","architecture":"x86_64","from_version":"1-1","from_vendor":"SUSE","to_version":"2-1","to_vendor":"OBS","repository_alias":"fixes","sha256":"a".repeat(64),"if_installed":false},
            {"name":"portal-lang","architecture":"noarch","from_version":"1-1","from_vendor":"SUSE","to_version":"2-1","to_vendor":"OBS","repository_alias":"fixes","sha256":"b".repeat(64),"if_installed":true}
        ]
    })).unwrap()
}

fn installed(lang: bool) -> Vec<InstalledPackage> {
    let m = manifest();
    m.package_migration
        .unwrap()
        .iter()
        .take(if lang { 2 } else { 1 })
        .map(|p| InstalledPackage {
            name: p.name.clone(),
            architecture: p.architecture.clone(),
            version: p.from_version.clone(),
            vendor: p.from_vendor.clone(),
        })
        .collect()
}
fn changes(lang: bool) -> Vec<PackageChange> {
    manifest()
        .package_migration
        .unwrap()
        .iter()
        .take(if lang { 2 } else { 1 })
        .map(|p| PackageChange {
            name: p.name.clone(),
            architecture: p.architecture.clone(),
            action: PackageAction::Upgrade,
            current_version: Some(p.from_version.clone()),
            proposed_version: Some(p.to_version.clone()),
            current_vendor: Some(p.from_vendor.clone()),
            proposed_vendor: Some(p.to_vendor.clone()),
            repository_alias: Some(p.repository_alias.clone()),
            download_bytes: 1,
            installed_size_before: 1,
            installed_size_after: 1,
        })
        .collect()
}

#[test]
fn same_release_is_explicit_and_requires_the_new_consumer() {
    let mut m = manifest();
    assert_eq!(m.operation(), OperationKind::PackageMigration);
    assert!(
        validate_manifest_route(
            &m,
            &m.source,
            Some(7),
            "0.2.7",
            ManifestChannelPolicy::Testing
        )
        .is_ok()
    );
    for (floor, consumer) in [("0.2.6", "0.2.7"), ("0.2.7", "0.2.6")] {
        m.minimum_updater_version = floor.into();
        assert!(
            validate_manifest_route(
                &m,
                &m.source,
                None,
                consumer,
                ManifestChannelPolicy::Testing
            )
            .is_err()
        );
    }
    m = manifest();
    m.package_migration = None;
    assert!(
        validate_manifest_route(&m, &m.source, None, "0.2.7", ManifestChannelPolicy::Testing)
            .is_err()
    );
    assert!(
        serde_json::to_value(m)
            .unwrap()
            .get("package_migration")
            .is_none()
    );
}

#[test]
fn migration_cannot_change_identity_or_disable_replay_channel_checks() {
    let m = manifest();
    assert_eq!(
        validate_manifest_route(
            &m,
            &m.source,
            Some(8),
            "0.2.7",
            ManifestChannelPolicy::Testing
        ),
        Err(lyra_upgrade_core::ManifestError::MigrationAlreadyApplied)
    );
    assert_eq!(
        validate_manifest_route(
            &m,
            &m.source,
            Some(9),
            "0.2.7",
            ManifestChannelPolicy::Testing
        ),
        Err(lyra_upgrade_core::ManifestError::Replay)
    );
    assert!(
        validate_manifest_route(
            &m,
            &m.source,
            Some(8),
            "0.2.7",
            ManifestChannelPolicy::Testing
        )
        .is_err()
    );
    assert!(
        validate_manifest_route(&m, &m.source, None, "0.2.7", ManifestChannelPolicy::Stable)
            .is_err()
    );
    for field in ["version", "edition", "architecture", "build_id"] {
        let mut value = serde_json::to_value(&m).unwrap();
        value["target"][field] = json!("changed");
        let changed: ReleaseManifest = serde_json::from_value(value).unwrap();
        assert!(
            validate_manifest_route(
                &changed,
                &m.source,
                None,
                "0.2.7",
                ManifestChannelPolicy::Testing
            )
            .is_err()
        );
    }
}

#[test]
fn policy_rejects_null_empty_duplicate_and_broad_rules() {
    let mut value = serde_json::to_value(manifest()).unwrap();
    value["package_migration"] = json!(null);
    assert!(serde_json::from_value::<ReleaseManifest>(value).is_err());
    for variant in 0..8 {
        let mut m = manifest();
        match variant {
            0 => m.package_migration = Some(vec![]),
            1 => {
                let p = m.package_migration.as_ref().unwrap()[0].clone();
                m.package_migration.as_mut().unwrap().push(p);
            }
            2 => m.package_migration.as_mut().unwrap()[0].if_installed = true,
            3 => m.package_migration.as_mut().unwrap()[0].sha256 = "bad".into(),
            4 => m.allowed_vendor_transitions[0].packages = None,
            5 => m.allowed_removals.push("other".into()),
            6 => m.package_migration.as_mut().unwrap()[0].to_version = "2*".into(),
            _ => m.package_migration.as_mut().unwrap()[0].to_version = "1-1".into(),
        }
        assert!(
            pending_migration(&m, &installed(true)).is_err(),
            "variant {variant}"
        );
    }
}

#[test]
fn optional_absence_is_preserved_and_unknown_installations_stop() {
    let m = manifest();
    assert_eq!(pending_migration(&m, &installed(false)).unwrap().len(), 1);
    assert_eq!(pending_migration(&m, &installed(true)).unwrap().len(), 2);
    assert!(pending_migration(&m, &[]).is_err());
    for variant in 0..4 {
        let mut packages = installed(true);
        match variant {
            0 => packages[1].version = "unexpected".into(),
            1 => packages[1].vendor = "other".into(),
            2 => packages[1].architecture = "i586".into(),
            _ => packages.push(packages[1].clone()),
        }
        assert!(pending_migration(&m, &packages).is_err());
    }
}

#[test]
fn exact_plan_rejects_additions_removals_downgrades_wrong_repository_and_omissions() {
    let m = manifest();
    for lang in [false, true] {
        assert!(validate_migration_plan(&m, &installed(lang), &changes(lang)).is_ok());
    }
    for variant in 0..9 {
        let mut plan = changes(true);
        match variant {
            0 => {
                let mut extra = plan[0].clone();
                extra.name = "unrelated".into();
                extra.proposed_vendor = extra.current_vendor.clone();
                plan.push(extra);
            }
            1 => plan[0].action = PackageAction::Remove,
            2 => plan[0].action = PackageAction::Downgrade,
            3 => plan[0].action = PackageAction::Install,
            4 => plan[0].repository_alias = Some("other".into()),
            5 => plan[0].proposed_version = Some("3-1".into()),
            6 => plan[0].current_version = Some("0-1".into()),
            7 => {
                plan.pop();
            }
            _ => plan[1] = plan[0].clone(),
        }
        assert!(
            validate_migration_plan(&m, &installed(true), &plan).is_err(),
            "variant {variant}"
        );
    }
    assert!(validate_migration_plan(&m, &installed(false), &changes(true)).is_err());
}

#[test]
fn repeated_or_partial_migration_is_idempotent() {
    let m = manifest();
    let mut packages = installed(true);
    packages[0].version = "2-1".into();
    packages[0].vendor = "OBS".into();
    assert!(validate_migration_plan(&m, &packages, &changes(true)[1..]).is_ok());
    packages[1].version = "2-1".into();
    packages[1].vendor = "OBS".into();
    assert!(validate_migration_plan(&m, &packages, &[]).is_ok());
    assert!(matches!(
        transaction_arguments(&m, &packages, TransactionMode::Apply),
        Err(MigrationError::AlreadyApplied)
    ));
}

#[test]
fn all_phases_select_exact_versions_with_the_same_restricted_command() {
    let m = manifest();
    for mode in [
        TransactionMode::Plan,
        TransactionMode::Download,
        TransactionMode::Apply,
    ] {
        let args = transaction_arguments(&m, &installed(false), mode).unwrap();
        assert_eq!(args[0], "install");
        assert!(args.windows(2).any(|pair| pair == ["--from", "fixes"]));
        assert_eq!(args.last().unwrap(), "portal.x86_64=2-1");
        for required in [
            "--no-recommends",
            "--no-allow-downgrade",
            "--no-allow-name-change",
            "--no-allow-arch-change",
            "--allow-vendor-change",
            "--",
        ] {
            assert!(args.iter().any(|a| a == required));
        }
        assert!(
            !args
                .iter()
                .any(|a| a.contains("portal-lang") || a == "dist-upgrade" || a == "--force")
        );
    }
}

#[test]
fn cached_payloads_are_exact_and_fail_closed() {
    let mut m = manifest();
    let root = tempfile::tempdir().unwrap();
    m.package_migration.as_mut().unwrap()[0].sha256 =
        format!("{:x}", Sha256::digest(b"authorized payload"));
    let payload = root.path().join("package.rpm");
    assert!(verify_payloads(&m, &installed(false), root.path()).is_err());
    std::fs::write(&payload, b"authorized payload").unwrap();
    assert!(verify_payloads(&m, &installed(false), root.path()).is_ok());
    std::fs::write(root.path().join("extra.rpm"), b"authorized payload").unwrap();
    assert!(verify_payloads(&m, &installed(false), root.path()).is_err());
    std::fs::remove_file(root.path().join("extra.rpm")).unwrap();
    std::fs::write(&payload, b"altered payload").unwrap();
    assert!(verify_payloads(&m, &installed(false), root.path()).is_err());
    std::fs::remove_file(&payload).unwrap();
    std::os::unix::fs::symlink("/dev/zero", &payload).unwrap();
    assert!(verify_payloads(&m, &installed(false), root.path()).is_err());
}

#[test]
fn confirmed_manifest_binds_operation_package_identities_and_payload_digests() {
    use lyra_upgrade_service::planner::check_confirmed_release_plan;
    let m = manifest();
    let mut plan: lyra_upgrade_core::UpgradePlan = serde_json::from_value(json!({
        "schema_version":3,"operation":"PackageMigration","source":m.source,"target":m.target,
        "manifest_sha256":format!("{:x}",Sha256::digest(serde_json::to_vec(&m).unwrap())),
        "required_bytes":1,"facts":{},"repositories":[],"metadata_valid_repositories":[],
        "third_party_repositories":[],"held_packages":[],"orphaned_packages":[],
        "installed_packages":installed(true),"package_changes":changes(true),"reboot_required":true
    }))
    .unwrap();
    let hash = plan.sha256().unwrap();
    assert!(check_confirmed_release_plan(&m, &plan, &hash).is_ok());
    let mut altered = m.clone();
    altered.package_migration.as_mut().unwrap()[0].sha256 = "c".repeat(64);
    assert!(check_confirmed_release_plan(&altered, &plan, &hash).is_err());
    plan.operation = OperationKind::ReleaseUpgrade;
    assert!(check_confirmed_release_plan(&m, &plan, &plan.sha256().unwrap()).is_err());
    plan.operation = OperationKind::PackageMigration;
    plan.package_changes[0].name = "other".into();
    assert!(check_confirmed_release_plan(&m, &plan, &plan.sha256().unwrap()).is_err());
}
