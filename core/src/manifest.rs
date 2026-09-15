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
    pub minimum_free_space_bytes: u64,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ManifestChannelPolicy {
    Stable,
    Testing,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RepositoryTransition {
    pub alias: String,
    pub base_url: String,
    pub signing_key_url: String,
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
    PrereleaseNotAllowed,
    SourceMismatch,
    TargetNotNewer,
    UnsupportedTarget,
    Replay,
    InvalidMinimumUpdaterVersion,
    UpdaterTooOld,
    InvalidRepository,
    DuplicateRepository,
    InvalidFingerprint,
    InvalidPolicy,
}

pub fn validate_manifest_route(
    manifest: &ReleaseManifest,
    installed: &ReleaseIdentity,
    last_sequence: Option<u64>,
    updater_version: &str,
    channel: ManifestChannelPolicy,
) -> Result<(), ManifestError> {
    if manifest.schema_version != MANIFEST_SCHEMA_VERSION {
        return Err(ManifestError::UnsupportedSchema);
    }
    if manifest.status != ManifestStatus::Available
        && !(manifest.status == ManifestStatus::Testing
            && channel == ManifestChannelPolicy::Testing)
    {
        return Err(ManifestError::NotAvailable);
    }
    if &manifest.source != installed {
        return Err(ManifestError::SourceMismatch);
    }
    if manifest.target.edition != "desktop"
        || manifest.target.architecture != "x86_64"
        || manifest.target.build_id.is_empty()
        || !manifest
            .target
            .build_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
    {
        return Err(ManifestError::UnsupportedTarget);
    }
    if !valid_version_transition(&installed.version, &manifest.target.version) {
        return Err(ManifestError::TargetNotNewer);
    }
    if is_prerelease(&manifest.target.version) && channel != ManifestChannelPolicy::Testing {
        return Err(ManifestError::PrereleaseNotAllowed);
    }
    if manifest.sequence == 0 || last_sequence.is_some_and(|sequence| manifest.sequence <= sequence)
    {
        return Err(ManifestError::Replay);
    }
    let minimum_updater = semantic_version(&manifest.minimum_updater_version)
        .ok_or(ManifestError::InvalidMinimumUpdaterVersion)?;
    let current_updater =
        semantic_version(updater_version).ok_or(ManifestError::InvalidMinimumUpdaterVersion)?;
    if current_updater < minimum_updater {
        return Err(ManifestError::UpdaterTooOld);
    }
    if manifest.repositories.is_empty() {
        return Err(ManifestError::InvalidRepository);
    }
    let mut aliases = std::collections::BTreeSet::new();
    for repository in &manifest.repositories {
        if !valid_alias(&repository.alias)
            || !valid_https_url(&repository.base_url)
            || !valid_https_url(&repository.signing_key_url)
            || !(1..=200).contains(&repository.priority)
        {
            return Err(ManifestError::InvalidRepository);
        }
        if !aliases.insert(&repository.alias) {
            return Err(ManifestError::DuplicateRepository);
        }
        let fingerprint = &repository.signing_key_fingerprint;
        if fingerprint.len() != 40
            || !fingerprint
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_lowercase())
        {
            return Err(ManifestError::InvalidFingerprint);
        }
    }
    if manifest
        .allowed_vendor_transitions
        .iter()
        .any(|transition| {
            !crate::valid_vendor(&transition.from) || !crate::valid_vendor(&transition.to)
        })
        || manifest.minimum_free_space_bytes == 0
        || manifest
            .allowed_removals
            .iter()
            .any(|name| !valid_package(name))
        || manifest.lockstep_packages.iter().any(|group| {
            group.len() < 2 || group.iter().any(|package| !valid_package(package)) || {
                let unique: std::collections::BTreeSet<_> = group.iter().collect();
                unique.len() != group.len()
            }
        })
    {
        return Err(ManifestError::InvalidPolicy);
    }
    Ok(())
}

fn valid_version_transition(source: &str, target: &str) -> bool {
    if source == target || is_legacy_calendar_version(target) {
        return false;
    }
    let Some((target, _)) = release_version(target) else {
        return false;
    };
    if is_legacy_calendar_version(source) {
        return true;
    }
    release_version(source).is_some_and(|(source, _)| target > source)
}

fn semantic_version(value: &str) -> Option<(u64, u64, u64)> {
    let mut components = value.split('.');
    let major = components.next()?.parse().ok()?;
    let minor = components.next()?.parse().ok()?;
    let patch = components
        .next()
        .map(str::parse)
        .transpose()
        .ok()?
        .unwrap_or(0);
    (components.next().is_none()).then_some((major, minor, patch))
}

fn is_prerelease(value: &str) -> bool {
    release_version(value).is_some_and(|(_, prerelease)| prerelease)
}

fn release_version(value: &str) -> Option<((u64, u64, u64), bool)> {
    match value.split_once('-') {
        None => semantic_version(value).map(|version| (version, false)),
        Some((version, suffix))
            if !suffix.is_empty()
                && suffix
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-')) =>
        {
            semantic_version(version).map(|version| (version, true))
        }
        Some(_) => None,
    }
}

fn is_legacy_calendar_version(value: &str) -> bool {
    matches!(value, "27.02" | "27.06" | "28.02")
        || value.starts_with("2026.08")
        || value.starts_with("27.02-")
}

fn valid_https_url(value: &str) -> bool {
    !value.chars().any(char::is_whitespace)
        && url::Url::parse(value).is_ok_and(|url| {
            url.scheme() == "https"
                && url.host_str().is_some()
                && url.username().is_empty()
                && url.password().is_none()
                && url.query().is_none()
                && url.fragment().is_none()
        })
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

    #[test]
    fn route_is_bound_to_exact_build_and_repository_policy() {
        let valid = manifest();
        let mut installed = identity("1.0");
        installed.build_id = "different-build".into();
        assert_eq!(
            validate_manifest_route(
                &valid,
                &installed,
                None,
                "0.2.3",
                ManifestChannelPolicy::Stable
            ),
            Err(ManifestError::SourceMismatch)
        );
        for mutate in [
            (|m: &mut ReleaseManifest| m.repositories.clear()) as fn(&mut ReleaseManifest),
            |m| m.repositories[0].priority = 0,
            |m| m.repositories[0].base_url = "https://".into(),
            |m| m.repositories[0].signing_key_fingerprint.push(' '),
            |m| m.allowed_removals.push("--all".to_owned() + "\nother"),
            |m| m.target.build_id.clear(),
        ] {
            let mut candidate = valid.clone();
            mutate(&mut candidate);
            assert!(
                validate_manifest_route(
                    &candidate,
                    &identity("1.0"),
                    None,
                    "0.2.3",
                    ManifestChannelPolicy::Stable
                )
                .is_err()
            );
        }
    }

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
            minimum_free_space_bytes: 8 * 1024 * 1024 * 1024,
            repositories: vec![RepositoryTransition {
                alias: "repo-oss".into(),
                base_url: "https://download.opensuse.org/distribution/leap/16.1/repo/oss/".into(),
                signing_key_url: "https://download.opensuse.org/repositories/keys/repo-oss.asc"
                    .into(),
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
            validate_manifest_route(
                &manifest(),
                &identity("1.0"),
                Some(8),
                "0.2.1",
                ManifestChannelPolicy::Stable,
            ),
            Err(ManifestError::Replay)
        );
        let mut invalid = manifest();
        invalid.repositories[0].base_url = "https://user:pass@example.test/repo?x=1".into();
        assert_eq!(
            validate_manifest_route(
                &invalid,
                &identity("1.0"),
                None,
                "0.2.1",
                ManifestChannelPolicy::Stable,
            ),
            Err(ManifestError::InvalidRepository)
        );
    }

    #[test]
    fn rejects_repeated_sequences_and_incompatible_updaters() {
        assert_eq!(
            validate_manifest_route(
                &manifest(),
                &identity("1.0"),
                Some(7),
                "0.2.1",
                ManifestChannelPolicy::Stable,
            ),
            Err(ManifestError::Replay)
        );
        assert_eq!(
            validate_manifest_route(
                &manifest(),
                &identity("1.0"),
                None,
                "0.0.9",
                ManifestChannelPolicy::Stable,
            ),
            Err(ManifestError::UpdaterTooOld)
        );
        let mut invalid = manifest();
        invalid.minimum_updater_version = "0.1-beta".into();
        assert_eq!(
            validate_manifest_route(
                &invalid,
                &identity("1.0"),
                None,
                "0.2.1",
                ManifestChannelPolicy::Stable,
            ),
            Err(ManifestError::InvalidMinimumUpdaterVersion)
        );
        invalid = manifest();
        invalid.minimum_free_space_bytes = 0;
        assert_eq!(
            validate_manifest_route(
                &invalid,
                &identity("1.0"),
                None,
                "0.2.1",
                ManifestChannelPolicy::Stable,
            ),
            Err(ManifestError::InvalidPolicy)
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

    #[test]
    fn prerelease_routes_require_explicit_testing_policy() {
        let mut candidate = manifest();
        candidate.status = ManifestStatus::Testing;
        candidate.target.version = "1.1-beta.1".into();
        assert_eq!(
            validate_manifest_route(
                &candidate,
                &identity("1.0"),
                None,
                "0.2.1",
                ManifestChannelPolicy::Stable,
            ),
            Err(ManifestError::NotAvailable)
        );
        assert_eq!(
            validate_manifest_route(
                &candidate,
                &identity("1.0"),
                None,
                "0.2.1",
                ManifestChannelPolicy::Testing,
            ),
            Ok(())
        );

        candidate.status = ManifestStatus::Available;
        assert_eq!(
            validate_manifest_route(
                &candidate,
                &identity("1.0"),
                None,
                "0.2.1",
                ManifestChannelPolicy::Stable,
            ),
            Err(ManifestError::PrereleaseNotAllowed)
        );
    }
}
