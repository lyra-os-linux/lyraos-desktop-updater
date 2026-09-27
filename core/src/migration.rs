//! Exact, signed package maintenance without changing the product identity.
use crate::{InstalledPackage, PackageAction, PackageChange, ReleaseManifest};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PackageMigration {
    pub name: String,
    pub architecture: String,
    pub from_version: String,
    pub from_vendor: String,
    pub to_version: String,
    pub to_vendor: String,
    pub repository_alias: String,
    pub sha256: String,
    /// Absent translations stay absent. This never authorizes a new install.
    pub if_installed: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MigrationError {
    InvalidPolicy,
    InstalledIdentity(String),
    UnexpectedTransaction,
    AlreadyApplied,
}

pub fn validate_migration(manifest: &ReleaseManifest) -> Result<(), MigrationError> {
    let Some(packages) = &manifest.package_migration else {
        return Ok(());
    };
    let mut names = BTreeSet::new();
    let edition = |value: &str| {
        !value.is_empty()
            && value.len() <= 256
            && value.as_bytes()[0].is_ascii_alphanumeric()
            && value
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._:+~^-".contains(&b))
    };
    if manifest.source != manifest.target
        || packages.is_empty()
        || packages.len() > 64
        || packages.iter().all(|p| p.if_installed)
        || !manifest.allowed_removals.is_empty()
        || !manifest.lockstep_packages.is_empty()
        || packages.iter().any(|p| {
            !crate::manifest::valid_package(&p.name)
                || !names.insert(&p.name)
                || !matches!(p.architecture.as_str(), "x86_64" | "noarch")
                || !edition(&p.from_version)
                || !edition(&p.to_version)
                || !crate::valid_vendor(&p.from_vendor)
                || !crate::valid_vendor(&p.to_vendor)
                // A same-edition vendor replacement would require a forced
                // reinstall. This maintenance path deliberately refuses it.
                || p.from_version == p.to_version
                || p.sha256.len() != 64
                || !p
                    .sha256
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                || !manifest
                    .repositories
                    .iter()
                    .any(|r| r.alias == p.repository_alias)
                || p.repository_alias != packages[0].repository_alias
        })
        || manifest.allowed_vendor_transitions.iter().any(|rule| {
            rule.packages.as_ref().is_none_or(|scope| {
                scope.iter().any(|name| {
                    !packages.iter().any(|p| {
                        &p.name == name && p.from_vendor == rule.from && p.to_vendor == rule.to
                    })
                })
            })
        })
    {
        return Err(MigrationError::InvalidPolicy);
    }
    Ok(())
}

pub fn pending_migration<'a>(
    manifest: &'a ReleaseManifest,
    installed: &[InstalledPackage],
) -> Result<Vec<&'a PackageMigration>, MigrationError> {
    validate_migration(manifest)?;
    let mut pending = Vec::new();
    for package in manifest.package_migration.as_deref().unwrap_or_default() {
        let matching: Vec<_> = installed
            .iter()
            .filter(|p| p.name == package.name)
            .collect();
        if matching.is_empty() && package.if_installed {
            continue;
        }
        if matching.len() != 1 || matching[0].architecture != package.architecture {
            return Err(MigrationError::InstalledIdentity(package.name.clone()));
        }
        let current = matching[0];
        if current.version == package.to_version && current.vendor == package.to_vendor {
            continue;
        }
        if current.version != package.from_version || current.vendor != package.from_vendor {
            return Err(MigrationError::InstalledIdentity(package.name.clone()));
        }
        pending.push(package);
    }
    Ok(pending)
}

pub fn validate_migration_plan(
    manifest: &ReleaseManifest,
    installed: &[InstalledPackage],
    changes: &[PackageChange],
) -> Result<(), MigrationError> {
    if manifest.package_migration.is_none() {
        return Ok(());
    }
    let pending = pending_migration(manifest, installed)?;
    let mut seen = BTreeSet::new();
    if changes.len() != pending.len()
        || changes.iter().any(|change| {
            !seen.insert(&change.name)
                || !pending.iter().any(|p| {
                    change.name == p.name
                        && change.architecture == p.architecture
                        && matches!(
                            change.action,
                            PackageAction::Upgrade | PackageAction::Reinstall
                        )
                        && change.current_version.as_ref() == Some(&p.from_version)
                        && change.current_vendor.as_ref() == Some(&p.from_vendor)
                        && change.proposed_version.as_ref() == Some(&p.to_version)
                        && change.proposed_vendor.as_ref() == Some(&p.to_vendor)
                        && change.repository_alias.as_ref() == Some(&p.repository_alias)
                })
        })
    {
        return Err(MigrationError::UnexpectedTransaction);
    }
    Ok(())
}
