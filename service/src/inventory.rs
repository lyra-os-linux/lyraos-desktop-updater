//! Audit package headers and RPM configuration artifacts, never personal files.
use lyra_upgrade_core::InventoryReport;
use std::fs;
use std::io::{self, Read, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::Path;

/// Require every retained package and every proposed RPM identity. Old
/// versions of upgraded packages may remain for the kernel multiversion policy.
pub fn verify_result(
    plan: &lyra_upgrade_core::UpgradePlan,
    report: &InventoryReport,
) -> io::Result<()> {
    use lyra_upgrade_core::{InstalledPackage, PackageAction};
    use std::collections::BTreeSet;
    if plan.schema_version != 3 || plan.installed_packages.is_empty() {
        return Err(io::Error::other("missing confirmed inventory"));
    }
    let mut required: BTreeSet<_> = plan.installed_packages.iter().cloned().collect();
    let mut allowed = required.clone();
    for change in &plan.package_changes {
        if let Some(version) = &change.current_version {
            let previous = InstalledPackage {
                name: change.name.clone(),
                version: version.clone(),
                architecture: change.architecture.clone(),
                vendor: change
                    .current_vendor
                    .clone()
                    .ok_or_else(|| io::Error::other("missing vendor"))?,
            };
            if !required.remove(&previous) {
                return Err(io::Error::other("unknown previous package"));
            }
            if change.action == PackageAction::Remove {
                allowed.remove(&previous);
            }
        }
        if let Some(version) = &change.proposed_version {
            let proposed = InstalledPackage {
                name: change.name.clone(),
                version: version.clone(),
                architecture: change.architecture.clone(),
                vendor: change
                    .proposed_vendor
                    .clone()
                    .ok_or_else(|| io::Error::other("missing vendor"))?,
            };
            required.insert(proposed.clone());
            allowed.insert(proposed);
        }
    }
    let actual: BTreeSet<_> = report.installed_packages.iter().cloned().collect();
    if !required.is_subset(&actual) || !actual.is_subset(&allowed) {
        return Err(io::Error::other(
            "installed packages differ from confirmed plan",
        ));
    }
    Ok(())
}

pub fn capture() -> io::Result<InventoryReport> {
    let installed_packages = crate::vendor_metadata::installed_packages(None)
        .map_err(|error| io::Error::other(format!("inventory: {error:?}")))?;
    let mut configuration_artifacts = Vec::new();
    collect_artifacts(Path::new("/etc"), &mut configuration_artifacts, &mut 0)?;
    configuration_artifacts.sort();
    Ok(InventoryReport {
        schema_version: 1,
        installed_packages,
        configuration_artifacts,
    })
}

fn collect_artifacts(
    directory: &Path,
    artifacts: &mut Vec<String>,
    count: &mut usize,
) -> io::Result<()> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        *count += 1;
        if *count > 100_000 {
            return Err(io::Error::other("configuration inventory exceeds limit"));
        }
        let kind = entry.file_type()?;
        if kind.is_dir() {
            collect_artifacts(&entry.path(), artifacts, count)?;
        } else if kind.is_file()
            && entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.ends_with(".rpmnew") || name.ends_with(".rpmsave"))
        {
            artifacts.push(entry.path().to_string_lossy().into_owned());
        }
        // No symlinks are followed and no configuration content is read.
    }
    Ok(())
}

pub fn save(path: &Path, report: &InventoryReport) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("missing inventory parent"))?;
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    file.as_file()
        .set_permissions(fs::Permissions::from_mode(0o600))?;
    serde_json::to_writer(&mut file, report)?;
    file.write_all(b"\n")?;
    file.as_file().sync_all()?;
    file.persist(path).map_err(|error| error.error)?;
    fs::File::open(parent)?.sync_all()
}

pub fn load(path: &Path) -> io::Result<InventoryReport> {
    let file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)?;
    if !file.metadata()?.is_file() || file.metadata()?.len() > 4 * 1024 * 1024 {
        return Err(io::Error::other("invalid inventory"));
    }
    let report: InventoryReport = serde_json::from_reader(file.take(4 * 1024 * 1024 + 1))?;
    if report.schema_version != 1 {
        return Err(io::Error::other("unsupported inventory"));
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_retained_or_proposed_packages_cannot_report_success() {
        use lyra_upgrade_core::{InstalledPackage, PackageAction, PackageChange};
        let package = |name: &str, version: &str| InstalledPackage {
            name: name.into(),
            version: version.into(),
            architecture: "x86_64".into(),
            vendor: "Lyra".into(),
        };
        let old = package("kernel-default", "1-1");
        let new = package("kernel-default", "2-1");
        let retained = package("personal-app", "1-1");
        let mut plan: lyra_upgrade_core::UpgradePlan = serde_json::from_value(serde_json::json!({
            "schema_version":3,"operation":"UpdateWithinRelease",
            "source":{"version":"1","edition":"desktop","architecture":"x86_64","build_id":"fixture"},
            "target":null,"manifest_sha256":null,"required_bytes":0,"facts":{},"repositories":[],
            "metadata_valid_repositories":[],"third_party_repositories":[],"held_packages":[],
            "installed_packages":[old,retained],"orphaned_packages":[],"package_changes":[],"reboot_required":true
        })).unwrap();
        plan.package_changes.push(PackageChange {
            name: old.name.clone(),
            architecture: old.architecture.clone(),
            action: PackageAction::Upgrade,
            current_version: Some(old.version.clone()),
            proposed_version: Some(new.version.clone()),
            current_vendor: Some(old.vendor.clone()),
            proposed_vendor: Some(new.vendor.clone()),
            repository_alias: Some("lyra".into()),
            download_bytes: 0,
            installed_size_before: 0,
            installed_size_after: 0,
        });
        let report = |packages| InventoryReport {
            schema_version: 1,
            installed_packages: packages,
            configuration_artifacts: vec![],
        };
        assert!(verify_result(&plan, &report(vec![retained.clone(), new.clone()])).is_ok());
        assert!(
            verify_result(
                &plan,
                &report(vec![retained.clone(), old.clone(), new.clone()])
            )
            .is_ok()
        );
        assert!(verify_result(&plan, &report(vec![new.clone()])).is_err());
        assert!(verify_result(&plan, &report(vec![retained.clone(), old.clone()])).is_err());
        assert!(
            verify_result(
                &plan,
                &report(vec![
                    retained.clone(),
                    new.clone(),
                    package("unexpected", "1")
                ])
            )
            .is_err()
        );
        plan.package_changes[0].action = PackageAction::Remove;
        plan.package_changes[0].proposed_version = None;
        plan.package_changes[0].proposed_vendor = None;
        assert!(verify_result(&plan, &report(vec![retained.clone()])).is_ok());
        assert!(verify_result(&plan, &report(vec![retained, old])).is_err());
    }
    #[test]
    fn report_lists_only_artifacts_without_following_links_or_reading_contents() {
        let directory = tempfile::tempdir().unwrap();
        fs::write(directory.path().join("service.rpmnew"), "secret").unwrap();
        fs::write(directory.path().join("service.conf"), "local settings").unwrap();
        std::os::unix::fs::symlink("/home", directory.path().join("personal")).unwrap();
        let mut artifacts = Vec::new();
        collect_artifacts(directory.path(), &mut artifacts, &mut 0).unwrap();
        assert_eq!(
            artifacts,
            [directory.path().join("service.rpmnew").to_str().unwrap()]
        );
        let report = InventoryReport {
            schema_version: 1,
            installed_packages: vec![],
            configuration_artifacts: artifacts,
        };
        let output = directory.path().join("report.json");
        save(&output, &report).unwrap();
        assert_eq!(load(&output).unwrap(), report);
        assert!(!fs::read_to_string(&output).unwrap().contains("secret"));
        assert_eq!(
            fs::metadata(output).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}
