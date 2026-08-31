use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use lyra_upgrade_core::{
    ManifestChannelPolicy, ManifestError, ReleaseIdentity, ReleaseManifest, RepositoryTransition,
    validate_manifest_route,
};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

pub const RELEASE_MANIFEST_URL: &str =
    "https://downloads.sourceforge.net/project/lyra/releases/1.0/desktop/releases-v1.json";
pub const RELEASE_MANIFEST_SIGNATURE_URL: &str =
    "https://downloads.sourceforge.net/project/lyra/releases/1.0/desktop/releases-v1.json.asc";
pub const RELEASE_KEYRING: &str = "/usr/share/lyra-upgrade/release-signing-key.gpg";
pub const RELEASE_CHANNEL_PATH: &str = "/etc/lyra-upgrade/channel";
pub const TESTING_MANIFEST_BASE_URL_PATH: &str = "/etc/lyra-upgrade/testing-manifest-base-url";
const MAX_MANIFEST_BYTES: u64 = 1024 * 1024;

#[derive(Debug)]
pub enum FetchError {
    Io(std::io::Error),
    Download,
    Signature,
    RepositoryKey,
    RepositoryKeyMismatch,
    TooLarge,
    Json(serde_json::Error),
    Route(ManifestError),
    InvalidTime,
    NotYetValid,
    Expired,
    InvalidChannel,
}

impl From<std::io::Error> for FetchError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<serde_json::Error> for FetchError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

pub fn fetch_release_manifest(
    installed: &ReleaseIdentity,
    last_sequence: Option<u64>,
    channel: ManifestChannelPolicy,
) -> Result<ReleaseManifest, FetchError> {
    let directory = tempfile::Builder::new()
        .prefix("lyra-upgrade-manifest-")
        .tempdir()?;
    let manifest_path = directory.path().join("manifest.json");
    let signature_path = directory.path().join("manifest.json.asc");
    let (manifest_url, signature_url) = manifest_urls(channel)?;
    download(&manifest_url, &manifest_path)?;
    download(&signature_url, &signature_path)?;
    if fs::metadata(&manifest_path)?.len() > MAX_MANIFEST_BYTES
        || fs::metadata(&signature_path)?.len() > MAX_MANIFEST_BYTES
    {
        return Err(FetchError::TooLarge);
    }
    verify_signature(&manifest_path, &signature_path)?;
    let manifest: ReleaseManifest = serde_json::from_slice(&fs::read(manifest_path)?)?;
    validate_manifest_route(
        &manifest,
        installed,
        last_sequence,
        env!("CARGO_PKG_VERSION"),
        channel,
    )
    .map_err(FetchError::Route)?;
    validate_time(&manifest)?;
    for (index, repository) in manifest.repositories.iter().enumerate() {
        let key_path = directory.path().join(format!("repository-key-{index}.asc"));
        fetch_repository_key(repository, &key_path)?;
    }
    Ok(manifest)
}

fn manifest_urls(channel: ManifestChannelPolicy) -> Result<(String, String), FetchError> {
    if channel == ManifestChannelPolicy::Stable {
        return Ok((
            RELEASE_MANIFEST_URL.into(),
            RELEASE_MANIFEST_SIGNATURE_URL.into(),
        ));
    }
    testing_manifest_urls(Path::new(TESTING_MANIFEST_BASE_URL_PATH))
}

fn testing_manifest_urls(path: &Path) -> Result<(String, String), FetchError> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.file_type().is_file() || metadata.len() > 4096 {
        return Err(FetchError::InvalidChannel);
    }
    let value = fs::read_to_string(path)?;
    if value.lines().count() != 1 || value.trim() != value {
        return Err(FetchError::InvalidChannel);
    }
    let base = url::Url::parse(&value).map_err(|_| FetchError::InvalidChannel)?;
    if base.scheme() != "https"
        || !base.username().is_empty()
        || base.password().is_some()
        || base.query().is_some()
        || base.fragment().is_some()
        || !base.path().ends_with('/')
    {
        return Err(FetchError::InvalidChannel);
    }
    let manifest = base
        .join("releases-v1.json")
        .map_err(|_| FetchError::InvalidChannel)?;
    let signature = base
        .join("releases-v1.json.asc")
        .map_err(|_| FetchError::InvalidChannel)?;
    Ok((manifest.into(), signature.into()))
}

pub fn fetch_repository_key(
    repository: &RepositoryTransition,
    destination: &Path,
) -> Result<(), FetchError> {
    download(&repository.signing_key_url, destination).map_err(|_| FetchError::RepositoryKey)?;
    if fs::metadata(destination)?.len() > MAX_MANIFEST_BYTES {
        return Err(FetchError::TooLarge);
    }
    let fingerprints = key_fingerprints(destination)?;
    if !fingerprints.contains(&repository.signing_key_fingerprint) {
        return Err(FetchError::RepositoryKeyMismatch);
    }
    Ok(())
}

fn download(url: &str, destination: &Path) -> Result<(), FetchError> {
    let status = Command::new("curl")
        .args([
            "--fail",
            "--silent",
            "--show-error",
            "--location",
            "--max-redirs",
            "3",
            "--proto",
            "=https",
            "--proto-redir",
            "=https",
            "--tlsv1.2",
            "--max-filesize",
            "1048576",
            "--output",
        ])
        .arg(destination)
        .arg(url)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;
    status.success().then_some(()).ok_or(FetchError::Download)
}

fn key_fingerprints(key: &Path) -> Result<Vec<String>, FetchError> {
    let output = Command::new("gpg")
        .args([
            "--batch",
            "--quiet",
            "--no-options",
            "--no-default-keyring",
            "--keyring",
            "/dev/null",
            "--with-colons",
            "--import-options",
            "show-only",
            "--import",
        ])
        .arg(key)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .output()?;
    if !output.status.success() {
        return Err(FetchError::RepositoryKey);
    }
    Ok(parse_fingerprints(&String::from_utf8_lossy(&output.stdout)))
}

fn parse_fingerprints(output: &str) -> Vec<String> {
    output
        .lines()
        .filter_map(|line| {
            let fields: Vec<_> = line.split(':').collect();
            (fields.first() == Some(&"fpr") && fields.len() > 9).then(|| fields[9].to_string())
        })
        .collect()
}

fn verify_signature(manifest: &Path, signature: &Path) -> Result<(), FetchError> {
    let status = Command::new("gpgv")
        .args(["--keyring", RELEASE_KEYRING, "--"])
        .arg(signature)
        .arg(manifest)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;
    status.success().then_some(()).ok_or(FetchError::Signature)
}

fn validate_time(manifest: &ReleaseManifest) -> Result<(), FetchError> {
    let from = OffsetDateTime::parse(&manifest.valid_from, &Rfc3339)
        .map_err(|_| FetchError::InvalidTime)?;
    let until = OffsetDateTime::parse(&manifest.valid_until, &Rfc3339)
        .map_err(|_| FetchError::InvalidTime)?;
    let now = OffsetDateTime::now_utc();
    if now < from {
        return Err(FetchError::NotYetValid);
    }
    if now > until {
        return Err(FetchError::Expired);
    }
    Ok(())
}

pub fn read_last_manifest_sequence(path: &Path) -> Option<u64> {
    fs::read_to_string(path).ok()?.trim().parse().ok()
}

pub fn read_release_channel(path: &Path) -> Result<ManifestChannelPolicy, FetchError> {
    match fs::read_to_string(path) {
        Ok(value) => match value.trim() {
            "stable" => Ok(ManifestChannelPolicy::Stable),
            "testing" => Ok(ManifestChannelPolicy::Testing),
            _ => Err(FetchError::InvalidChannel),
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Ok(ManifestChannelPolicy::Stable)
        }
        Err(error) => Err(FetchError::Io(error)),
    }
}

pub fn manifest_sequence_path() -> PathBuf {
    PathBuf::from("/var/lib/lyra-upgrade/last-manifest-sequence")
}

#[cfg(test)]
mod tests {
    use super::{parse_fingerprints, read_release_channel, testing_manifest_urls};
    use lyra_upgrade_core::ManifestChannelPolicy;

    #[test]
    fn parses_only_machine_readable_fingerprints() {
        let output = "pub:-:2048:1:1234::::::\nfpr:::::::::AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA:\nuid:::::::::Lyra:\nfpr:::::::::BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB:\n";
        assert_eq!(
            parse_fingerprints(output),
            vec![
                "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
                "BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB"
            ]
        );
    }

    #[test]
    fn release_channel_defaults_stable_and_rejects_ambiguous_values() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("channel");
        assert_eq!(
            read_release_channel(&path).unwrap(),
            ManifestChannelPolicy::Stable
        );
        std::fs::write(&path, "testing\n").unwrap();
        assert_eq!(
            read_release_channel(&path).unwrap(),
            ManifestChannelPolicy::Testing
        );
        std::fs::write(&path, "beta\n").unwrap();
        assert!(read_release_channel(&path).is_err());
    }

    #[test]
    fn testing_manifest_source_is_explicit_https_and_fixed_filenames() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("testing-source");
        std::fs::write(&path, "https://example.test/controlled/1/desktop/").unwrap();
        assert_eq!(
            testing_manifest_urls(&path).unwrap(),
            (
                "https://example.test/controlled/1/desktop/releases-v1.json".into(),
                "https://example.test/controlled/1/desktop/releases-v1.json.asc".into(),
            )
        );
        for invalid in [
            "http://example.test/path/",
            "https://user@example.test/path/",
            "https://example.test/path/?query=1",
            "https://example.test/path",
            "https://example.test/path/\n",
        ] {
            std::fs::write(&path, invalid).unwrap();
            assert!(
                testing_manifest_urls(&path).is_err(),
                "accepted {invalid:?}"
            );
        }
    }
}
