use serde::{Deserialize, Serialize};

use crate::{ReleaseIdentity, SolverPolicy, VendorTransition};

pub const MANIFEST_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReleaseManifest {
    pub schema_version: u32,
    pub sequence: u64,
    pub status: ManifestStatus,
    pub valid_from: String,
    pub valid_until: String,
    pub source: ReleaseIdentity,
    pub target: ReleaseIdentity,
    pub minimum_updater_version: String,
    pub repositories: Vec<RepositoryTransition>,
    pub allowed_removals: Vec<String>,
    pub allowed_vendor_transitions: Vec<VendorTransitionWire>,
    pub lockstep_packages: Vec<Vec<String>>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ManifestStatus {
    Testing,
    Available,
    Paused,
    Withdrawn,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RepositoryTransition {
    pub alias: String,
    pub base_url: String,
    pub signing_key_fingerprint: String,
    pub priority: u16,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct VendorTransitionWire {
    pub from: String,
    pub to: String,
}

impl ReleaseManifest {
    pub fn solver_policy(&self) -> SolverPolicy {
        SolverPolicy {
            allowed_removals: self.allowed_removals.clone(),
            allowed_vendor_transitions: self
                .allowed_vendor_transitions
                .iter()
                .map(|transition| VendorTransition {
                    from: transition.from.clone(),
                    to: transition.to.clone(),
                })
                .collect(),
            lockstep_packages: self.lockstep_packages.clone(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ManifestError {
    UnsupportedSchema,
    NotAvailable,
    SourceMismatch,
    TargetNotNewer,
    UnsupportedTarget,
    Replay,
    InvalidRepository,
    DuplicateRepository,
    InvalidFingerprint,
    InvalidPolicy,
}

pub fn validate_manifest_route(
    manifest: &ReleaseManifest,
    installed: &ReleaseIdentity,
    last_sequence: Option<u64>,
) -> Result<(), ManifestError> {
    if manifest.schema_version != MANIFEST_SCHEMA_VERSION {
        return Err(ManifestError::UnsupportedSchema);
    }
    if manifest.status != ManifestStatus::Available {
        return Err(ManifestError::NotAvailable);
    }
    if manifest.source.version != installed.version
        || manifest.source.edition != installed.edition
        || manifest.source.architecture != installed.architecture
    {
        return Err(ManifestError::SourceMismatch);
    }
    if manifest.target.edition != "desktop" || manifest.target.architecture != "x86_64" {
        return Err(ManifestError::UnsupportedTarget);
    }
    if !valid_version_transition(&installed.version, &manifest.target.version) {
        return Err(ManifestError::TargetNotNewer);
    }
    if last_sequence.is_some_and(|sequence| manifest.sequence < sequence) {
        return Err(ManifestError::Replay);
    }
    let mut aliases = std::collections::BTreeSet::new();
    for repository in &manifest.repositories {
        if !valid_alias(&repository.alias) || !valid_https_url(&repository.base_url) {
            return Err(ManifestError::InvalidRepository);
        }
        if !aliases.insert(&repository.alias) {
            return Err(ManifestError::DuplicateRepository);
        }
        let fingerprint: String = repository
            .signing_key_fingerprint
            .chars()
            .filter(|character| !character.is_ascii_whitespace())
            .collect();
        if fingerprint.len() != 40
            || !fingerprint
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_lowercase())
        {
            return Err(ManifestError::InvalidFingerprint);
        }
    }
    if manifest.lockstep_packages.iter().any(|group| {
        group.len() < 2 || group.iter().any(|package| !valid_package(package)) || {
            let unique: std::collections::BTreeSet<_> = group.iter().collect();
            unique.len() != group.len()
        }
    }) {
        return Err(ManifestError::InvalidPolicy);
    }
    Ok(())
}

fn valid_version_transition(source: &str, target: &str) -> bool {
    if source == target || is_legacy_calendar_version(target) {
        return false;
    }
    let Some(target) = semantic_version(target) else {
        return false;
    };
    if is_legacy_calendar_version(source) {
        return true;
    }
    semantic_version(source).is_some_and(|source| target > source)
}

fn semantic_version(value: &str) -> Option<(u64, u64, u64)> {
    let mut components = value.split('.');
    let major = components.next()?.parse().ok()?;
    let minor = components.next()?.parse().ok()?;
    let patch = components.next().map(str::parse).transpose().ok()?.unwrap_or(0);
    (components.next().is_none()).then_some((major, minor, patch))
}

fn is_legacy_calendar_version(value: &str) -> bool {
    matches!(value, "27.02" | "27.06" | "28.02")
        || value.starts_with("2026.08")
        || value.starts_with("27.02-")
}

fn valid_https_url(value: &str) -> bool {
    value.starts_with("https://")
        && !value.contains('@')
        && !value.contains(['?', '#'])
        && value.bytes().all(|byte| !byte.is_ascii_control())
}

fn valid_alias(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_' | b':'))
}

fn valid_package(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'.' | b'-' | b'_'))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity(version: &str) -> ReleaseIdentity {
        ReleaseIdentity {
            version: version.into(),
            edition: "desktop".into(),
            architecture: "x86_64".into(),
            build_id: "fixture".into(),
        }
    }

    fn manifest() -> ReleaseManifest {
        ReleaseManifest {
            schema_version: 1,
            sequence: 7,
            status: ManifestStatus::Available,
            valid_from: "2027-01-01T00:00:00Z".into(),
            valid_until: "2027-12-31T00:00:00Z".into(),
            source: identity("1.0"),
            target: identity("1.1"),
            minimum_updater_version: "0.1.0".into(),
            repositories: vec![RepositoryTransition {
                alias: "repo-oss".into(),
                base_url: "https://download.opensuse.org/distribution/leap/16.1/repo/oss/".into(),
                signing_key_fingerprint: "01B63EEDBE6B079126A0116EFA7353A131ECEFEB".into(),
                priority: 20,
            }],
            allowed_removals: vec![],
            allowed_vendor_transitions: vec![],
            lockstep_packages: vec![vec!["lyra-release".into(), "lyra-upgrade".into()]],
        }
    }

    #[test]
    fn rejects_replay_and_unsigned_style_urls() {
        assert_eq!(
            validate_manifest_route(&manifest(), &identity("1.0"), Some(8)),
            Err(ManifestError::Replay)
        );
        let mut invalid = manifest();
        invalid.repositories[0].base_url = "https://user:pass@example.test/repo?x=1".into();
        assert_eq!(
            validate_manifest_route(&invalid, &identity("1.0"), None),
            Err(ManifestError::InvalidRepository)
        );
    }

    #[test]
    fn accepts_semantic_upgrades_and_only_legacy_sources() {
        assert!(valid_version_transition("1.0", "1.0.1"));
        assert!(valid_version_transition("1.0.1", "1.1"));
        assert!(valid_version_transition("27.02", "1.0"));
        assert!(valid_version_transition("2026.08-alpha6", "1.0"));
        assert!(!valid_version_transition("1.1", "1.0.1"));
        assert!(!valid_version_transition("1.0", "27.06"));
        assert!(!valid_version_transition("1.0-alpha.7", "1.0"));
    }
}
