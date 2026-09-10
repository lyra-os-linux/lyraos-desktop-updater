use lyra_upgrade_core::{
    HostFacts, OperationKind, PackageAction, PreflightPolicy, ReleaseIdentity, ReleaseManifest,
    RepositoryFact, SolverPolicy, SolverResult, UpgradePlan, VendorTransition, build_plan,
    evaluate_solver_preflight, vendor_policy_blockers,
};
use lyra_upgrade_service::planner::{
    PlannerError, check_confirmed_release_plan, revalidate_release_plan,
};
use lyra_upgrade_service::solver_xml::parse_solver_xml;
use lyra_upgrade_service::vendor_metadata::enrich_from_rpm_xml;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};

fn fixture(case: &str) -> PathBuf {
    std::env::var_os("LYRA_VENDOR_FIXTURES")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/vendor-policy")
        })
        .join(case)
}
fn solver(case: &str) -> SolverResult {
    parse_solver_xml(
        &fs::read_to_string(fixture(case).join("summary.xml")).unwrap(),
        vec!["fixture".into()],
        0,
    )
    .unwrap()
}
fn rpm(case: &str) -> Vec<u8> {
    fs::read(fixture(case).join("installed.xml")).unwrap()
}
fn enriched(case: &str) -> SolverResult {
    let mut result = solver(case);
    enrich_from_rpm_xml(&mut result, &fixture(case).join("raw"), &rpm(case)).unwrap();
    result
}
fn facts() -> HostFacts {
    HostFacts {
        release: ReleaseIdentity {
            version: "1.0".into(),
            edition: "desktop".into(),
            architecture: "x86_64".into(),
            build_id: "fixture".into(),
        },
        root_filesystem: "btrfs".into(),
        snapper_root_configured: true,
        rpm_database_healthy: true,
        package_lock_free: true,
        available_bytes: 64 * 1024 * 1024 * 1024,
        required_download_bytes: 0,
        required_transaction_bytes: 0,
        required_snapshot_bytes: 0,
        on_battery: false,
        battery_percent: None,
        secure_boot_enabled: Some(true),
        repositories: vec![RepositoryFact {
            alias: "fixture".into(),
            enabled: true,
            official: true,
            metadata_valid: true,
            signing_key_trusted: true,
        }],
        held_packages: vec![],
        orphaned_packages: vec![],
    }
}
fn manifest() -> ReleaseManifest {
    serde_json::from_value(serde_json::json!({
        "schema_version":1,"sequence":1,"status":"testing","valid_from":"2026-09-09T00:00:00Z","valid_until":"2026-10-09T00:00:00Z",
        "source":facts().release,"target":{"version":"2.0","edition":"desktop","architecture":"x86_64","build_id":"target"},
        "minimum_updater_version":"0.2.3","minimum_free_space_bytes":1048576,
        "repositories":[{"alias":"fixture","base_url":"https://example.invalid/repo","signing_key_url":"https://example.invalid/key","signing_key_fingerprint":"A".repeat(40),"priority":99}],
        "allowed_removals":[],"allowed_vendor_transitions":[{"from":"Lyra Fixture A","to":"Lyra Fixture B"}],"lockstep_packages":[]
    })).unwrap()
}
fn plan(solver: &SolverResult, manifest: &ReleaseManifest) -> UpgradePlan {
    let facts = facts();
    let report = evaluate_solver_preflight(
        &facts,
        PreflightPolicy {
            minimum_free_space_bytes: manifest.minimum_free_space_bytes,
            ..PreflightPolicy::default()
        },
        solver,
        &manifest.solver_policy(),
    );
    let hash = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(manifest).unwrap())
    );
    build_plan(
        OperationKind::ReleaseUpgrade,
        &facts,
        &report,
        Some(manifest.target.clone()),
        Some(hash),
        solver,
    )
    .unwrap()
}

#[test]
fn actual_zypper_vendor_only_and_upgrade_lists_are_not_lost_or_double_counted() {
    for (case, action, edition) in [
        ("vendor-only", PackageAction::Reinstall, "1-1"),
        ("upgrade-vendor", PackageAction::Upgrade, "2-1"),
    ] {
        let parsed = enriched(case);
        assert_eq!(parsed.changes.len(), 1);
        let change = &parsed.changes[0];
        assert_eq!(change.action, action);
        assert_eq!(change.current_version.as_deref(), Some("1-1"));
        assert_eq!(change.proposed_version.as_deref(), Some(edition));
        assert_eq!(change.current_vendor.as_deref(), Some("Lyra Fixture A"));
        assert_eq!(change.proposed_vendor.as_deref(), Some("Lyra Fixture B"));
        assert_eq!(change.repository_alias.as_deref(), Some("fixture"));
        assert_eq!(
            vendor_policy_blockers(&parsed.changes, &SolverPolicy::default()).len(),
            1
        );
        assert!(vendor_policy_blockers(&parsed.changes, &manifest().solver_policy()).is_empty());
    }
}

#[test]
fn only_the_exact_directed_vendor_pair_is_permitted() {
    let result = enriched("vendor-only");
    for (from, to) in [
        ("Lyra Fixture B", "Lyra Fixture A"),
        ("Lyra", "Lyra Fixture B"),
        ("lyra fixture a", "Lyra Fixture B"),
        ("*", "*"),
        ("", ""),
    ] {
        let policy = SolverPolicy {
            allowed_vendor_transitions: vec![VendorTransition {
                from: from.into(),
                to: to.into(),
            }],
            ..SolverPolicy::default()
        };
        assert!(
            !vendor_policy_blockers(&result.changes, &policy).is_empty(),
            "{from:?} -> {to:?}"
        );
    }
}

#[test]
fn required_vendor_identities_cannot_be_absent_empty_or_control_characters() {
    let base = enriched("vendor-only").changes.remove(0);
    for action in [
        PackageAction::Install,
        PackageAction::Remove,
        PackageAction::Upgrade,
        PackageAction::Downgrade,
        PackageAction::Reinstall,
    ] {
        let mut change = base.clone();
        change.action = action;
        if action == PackageAction::Install {
            change.current_vendor = None;
        }
        if action == PackageAction::Remove {
            change.proposed_vendor = None;
        }
        assert!(vendor_policy_blockers(&[change.clone()], &manifest().solver_policy()).is_empty());
        for unknown in [
            None,
            Some("".into()),
            Some(" \t".into()),
            Some("vendor\nspoof".into()),
        ] {
            if action != PackageAction::Install {
                let mut mutated = change.clone();
                mutated.current_vendor = unknown.clone();
                assert!(
                    !vendor_policy_blockers(&[mutated], &manifest().solver_policy()).is_empty()
                );
            }
            if action != PackageAction::Remove {
                let mut mutated = change.clone();
                mutated.proposed_vendor = unknown.clone();
                assert!(
                    !vendor_policy_blockers(&[mutated], &manifest().solver_policy()).is_empty()
                );
            }
        }
    }
}

#[test]
fn vendor_only_change_is_reviewed_and_bound_to_the_plan_hash() {
    let result = enriched("vendor-only");
    let plan = plan(&result, &manifest());
    let hash = plan.sha256().unwrap();
    assert_eq!(plan.package_changes, result.changes);
    for side in [true, false] {
        let mut mutated = plan.clone();
        if side {
            mutated.package_changes[0].current_vendor = Some("Different".into());
        } else {
            mutated.package_changes[0].proposed_vendor = Some("Different".into());
        }
        assert_ne!(hash, mutated.sha256().unwrap());
    }
    let mut omitted = plan.clone();
    omitted.package_changes.clear();
    assert_ne!(hash, omitted.sha256().unwrap());
}

#[test]
fn staging_and_offline_shared_boundary_rejects_vendor_drift_and_changed_allowlist() {
    let result = enriched("vendor-only");
    let manifest = manifest();
    let plan = plan(&result, &manifest);
    let hash = plan.sha256().unwrap();
    check_confirmed_release_plan(&manifest, &plan, &hash).unwrap();
    revalidate_release_plan(&facts(), &result, &manifest, &plan, &hash).unwrap();
    for vendor in [None, Some("Unexpected vendor".into())] {
        let mut drift = result.clone();
        drift.changes[0].proposed_vendor = vendor;
        assert!(revalidate_release_plan(&facts(), &drift, &manifest, &plan, &hash).is_err());
    }
    // Even a currently allowed identity (no transition) changes the approved hash.
    let mut same_vendor = result.clone();
    same_vendor.changes[0].proposed_vendor = same_vendor.changes[0].current_vendor.clone();
    assert!(matches!(
        revalidate_release_plan(&facts(), &same_vendor, &manifest, &plan, &hash),
        Err(PlannerError::PlanChanged)
    ));
    let mut relaxed = manifest.clone();
    relaxed.allowed_vendor_transitions.clear();
    assert!(matches!(
        check_confirmed_release_plan(&relaxed, &plan, &hash),
        Err(PlannerError::PlanChanged)
    ));
    // Downloading cached payloads legitimately reduces remaining space needs.
    let mut cached = result.clone();
    cached.download_bytes = 0;
    revalidate_release_plan(&facts(), &cached, &manifest, &plan, &hash).unwrap();
    let mut too_small = facts();
    too_small.available_bytes = 0;
    assert!(revalidate_release_plan(&too_small, &cached, &manifest, &plan, &hash).is_err());
}

fn cache(primary: &str) -> tempfile::TempDir {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().join("fixture/repodata");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("primary.xml"), primary).unwrap();
    fs::write(dir.join("repomd.xml"), format!(r#"<repomd xmlns="http://linux.duke.edu/metadata/repo"><data type="primary"><checksum type="sha256">{:x}</checksum><location href="repodata/primary.xml"/></data></repomd>"#, Sha256::digest(primary))).unwrap();
    temp
}
fn primary() -> String {
    fs::read_to_string(fixture("vendor-only").join("primary.xml")).unwrap()
}

#[test]
fn candidate_requires_exact_name_epoch_version_architecture_and_repository() {
    for (from, to) in [
        ("<name>lyra-vendor-fixture</name>", "<name>other</name>"),
        ("epoch=\"0\"", "epoch=\"1\""),
        ("ver=\"1\"", "ver=\"9\""),
        ("rel=\"1\"", "rel=\"9\""),
        ("<arch>noarch</arch>", "<arch>x86_64</arch>"),
    ] {
        let cache = cache(&primary().replace(from, to));
        assert!(
            enrich_from_rpm_xml(
                &mut solver("vendor-only"),
                cache.path(),
                &rpm("vendor-only")
            )
            .is_err(),
            "{from}"
        );
    }
    for alias in ["other", "../fixture", "/fixture", ".", ".."] {
        let mut result = solver("vendor-only");
        result.changes[0].repository_alias = Some(alias.into());
        assert!(
            enrich_from_rpm_xml(
                &mut result,
                &fixture("vendor-only").join("raw"),
                &rpm("vendor-only")
            )
            .is_err()
        );
    }
}

#[test]
fn epoch_and_xml_escaped_vendors_are_preserved_as_exact_identities() {
    let source = primary()
        .replace("epoch=\"0\"", "epoch=\"7\"")
        .replace("Lyra Fixture B", "A &amp; B &#67;<![CDATA[ / D]]>")
        .replace("rpm:", "pkg:");
    let source = source.replace("xmlns:rpm", "xmlns:pkg");
    let cache = cache(&source);
    let mut result = solver("vendor-only");
    result.changes[0].proposed_version = Some("7:1-1".into());
    enrich_from_rpm_xml(&mut result, cache.path(), &rpm("vendor-only")).unwrap();
    assert_eq!(
        result.changes[0].proposed_vendor.as_deref(),
        Some("A & B C / D")
    );
}

#[test]
fn missing_ambiguous_and_wrong_namespace_vendors_fail_without_partial_mutation() {
    let source = primary();
    let start = source.find("<package ").unwrap();
    let end = source.find("</package>").unwrap() + "</package>".len();
    let duplicate = source.replace(
        "</metadata>",
        &format!("{}</metadata>", &source[start..end]),
    );
    for modified in [
        source.replace("<rpm:vendor>Lyra Fixture B</rpm:vendor>", ""),
        source.replace("Lyra Fixture B", ""),
        source.replace(
            "http://linux.duke.edu/metadata/rpm",
            "https://evil.invalid/",
        ),
        source.replace(
            "</rpm:vendor>",
            "</rpm:vendor><rpm:vendor>Forged</rpm:vendor>",
        ),
        duplicate,
    ] {
        let cache = cache(&modified);
        let mut result = solver("vendor-only");
        let before = result.clone();
        assert!(enrich_from_rpm_xml(&mut result, cache.path(), &rpm("vendor-only")).is_err());
        assert_eq!(result, before);
    }
}

#[test]
fn installed_rpm_missing_or_ambiguous_identity_is_never_guessed() {
    let rpm = String::from_utf8(rpm("vendor-only")).unwrap();
    let start = rpm.find("<package>\t<string>lyra-vendor-fixture").unwrap();
    let end = rpm[start..].find("</package>").unwrap() + start + "</package>".len();
    for modified in [
        rpm.replace("Lyra Fixture A", ""),
        rpm.replace("<string>noarch</string>", "<string>x86_64</string>"),
        rpm.replace("</rpmdb>", &format!("{}</rpmdb>", &rpm[start..end])),
    ] {
        assert!(
            enrich_from_rpm_xml(
                &mut solver("vendor-only"),
                &fixture("vendor-only").join("raw"),
                modified.as_bytes()
            )
            .is_err()
        );
    }
}

#[test]
fn checksum_mismatch_missing_metadata_and_path_escape_are_blocked() {
    let source = primary();
    for mode in ["mismatch", "missing", "escape", "symlink"] {
        let cache = cache(&source);
        let dir = cache.path().join("fixture/repodata");
        match mode {
            "mismatch" => fs::write(
                dir.join("primary.xml"),
                source.replace("Lyra Fixture B", "Forged"),
            )
            .unwrap(),
            "missing" => fs::remove_file(dir.join("primary.xml")).unwrap(),
            "escape" => {
                let xml = fs::read_to_string(dir.join("repomd.xml"))
                    .unwrap()
                    .replace("repodata/primary.xml", "../../primary.xml");
                fs::write(dir.join("repomd.xml"), xml).unwrap();
            }
            _ => {
                fs::remove_file(dir.join("primary.xml")).unwrap();
                std::os::unix::fs::symlink(
                    fixture("vendor-only").join("primary.xml"),
                    dir.join("primary.xml"),
                )
                .unwrap();
            }
        }
        assert!(
            enrich_from_rpm_xml(
                &mut solver("vendor-only"),
                cache.path(),
                &rpm("vendor-only")
            )
            .is_err(),
            "{mode}"
        );
    }
}

#[test]
fn malformed_or_incomplete_solver_summaries_are_rejected() {
    let xml = fs::read_to_string(fixture("upgrade-vendor").join("summary.xml")).unwrap();
    for modified in [
        xml.replace("packages-to-change=\"1\"", "packages-to-change=\"2\""),
        xml.replace("edition-old=\"1-1\"", ""),
        xml.replace("arch-old=\"noarch\"", "arch-old=\"x86_64\""),
        xml.replace("type=\"package\"", "type=\"unknown\""),
        xml.replace("</stream>", ""),
        xml.replace("</install-summary>", ""),
        xml.replace("<stream>", "<!DOCTYPE stream><stream>"),
    ] {
        assert!(parse_solver_xml(&modified, vec!["fixture".into()], 0).is_err());
    }
}

#[test]
fn production_compression_formats_and_sha512_are_supported_and_checked() {
    use sha2::Sha512;
    use std::process::{Command, Stdio};
    let source = primary();
    for (program, extension) in [("gzip", "gz"), ("xz", "xz"), ("zstd", "zst")] {
        let cache = cache(&source);
        let dir = cache.path().join("fixture/repodata");
        let compressed = Command::new(program)
            .arg("-c")
            .stdin(fs::File::open(dir.join("primary.xml")).unwrap())
            .stdout(Stdio::piped())
            .output()
            .unwrap();
        assert!(compressed.status.success());
        let name = format!("primary.xml.{extension}");
        fs::write(dir.join(&name), &compressed.stdout).unwrap();
        fs::write(dir.join("repomd.xml"), format!(r#"<repomd xmlns="http://linux.duke.edu/metadata/repo"><data type="primary"><checksum type="sha512">{:x}</checksum><location href="repodata/{name}"/></data></repomd>"#, Sha512::digest(&compressed.stdout))).unwrap();
        let mut result = solver("vendor-only");
        enrich_from_rpm_xml(&mut result, cache.path(), &rpm("vendor-only")).unwrap();
        assert_eq!(
            result.changes[0].proposed_vendor.as_deref(),
            Some("Lyra Fixture B")
        );
        // A correct checksum over a truncated compressed stream is insufficient.
        let truncated = &compressed.stdout[..compressed.stdout.len() / 2];
        fs::write(dir.join(&name), truncated).unwrap();
        fs::write(dir.join("repomd.xml"), format!(r#"<repomd xmlns="http://linux.duke.edu/metadata/repo"><data type="primary"><checksum type="sha512">{:x}</checksum><location href="repodata/{name}"/></data></repomd>"#, Sha512::digest(truncated))).unwrap();
        assert!(
            enrich_from_rpm_xml(
                &mut solver("vendor-only"),
                cache.path(),
                &rpm("vendor-only")
            )
            .is_err()
        );
    }
}

#[test]
fn virtual_patterns_are_excluded_but_unknown_rpm_actions_cannot_disappear() {
    let xml = r#"<stream><install-summary download-size="0" space-usage-diff="0" space-usage-installed="0" space-usage-removed="0" packages-to-change="0"><to-install><solvable type="pattern" name="desktop" edition="1-1" arch="noarch" repository="fixture"/></to-install></install-summary></stream>"#;
    assert!(
        parse_solver_xml(xml, vec!["fixture".into()], 0)
            .unwrap()
            .changes
            .is_empty()
    );
    let changed = xml
        .replace("type=\"pattern\"", "type=\"package\"")
        .replace("to-install", "future-action")
        .replace("packages-to-change=\"0\"", "packages-to-change=\"1\"");
    assert!(parse_solver_xml(&changed, vec!["fixture".into()], 0).is_err());
}

#[test]
fn manifest_rejects_empty_vendor_identity_even_if_its_pair_would_match() {
    let original = manifest();
    for bad in ["", " \t", "Lyra\nInjected"] {
        for side in [true, false] {
            let mut manifest = original.clone();
            if side {
                manifest.allowed_vendor_transitions[0].from = bad.into();
            } else {
                manifest.allowed_vendor_transitions[0].to = bad.into();
            }
            assert_eq!(
                lyra_upgrade_core::validate_manifest_route(
                    &manifest,
                    &facts().release,
                    None,
                    "0.2.3",
                    lyra_upgrade_core::ManifestChannelPolicy::Testing
                ),
                Err(lyra_upgrade_core::ManifestError::InvalidPolicy)
            );
        }
    }
}

#[test]
fn large_upgrade_preserves_all_vendor_changes_without_duplicate_plan_entries() {
    let original = fs::read_to_string(fixture("upgrade-vendor").join("summary.xml")).unwrap();
    let start = original.find("<solvable ").unwrap();
    let end = original[start..].find("</solvable>").unwrap() + start + "</solvable>".len();
    let entries: String = (0..1801)
        .map(|index| {
            original[start..end].replace("lyra-vendor-fixture", &format!("package-{index:04}"))
        })
        .collect();
    let xml = format!(
        r#"<stream><install-summary download-size="0" space-usage-diff="0" space-usage-installed="0" space-usage-removed="0" packages-to-change="1801"><to-upgrade>{entries}</to-upgrade><to-change-vendor>{entries}</to-change-vendor></install-summary></stream>"#
    );
    let parsed = parse_solver_xml(&xml, vec!["fixture".into()], 0).unwrap();
    assert_eq!(parsed.changes.len(), 1801);
    assert!(
        parsed
            .changes
            .iter()
            .all(|change| change.action == PackageAction::Upgrade)
    );
    assert_eq!(
        vendor_policy_blockers(&parsed.changes, &SolverPolicy::default()).len(),
        1801
    );
}
