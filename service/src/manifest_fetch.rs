use std::fs;
use std::io::Read;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use lyra_upgrade_core::{
    ManifestChannelPolicy, ManifestError, ReleaseIdentity, ReleaseManifest, RepositoryTransition,
    validate_manifest_route,
};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

pub const RELEASE_MANIFEST_URL: &str =
    "https://downloads.sourceforge.net/project/lyra/releases/1.1/desktop/releases-v1.json";
pub const RELEASE_MANIFEST_SIGNATURE_URL: &str =
    "https://downloads.sourceforge.net/project/lyra/releases/1.1/desktop/releases-v1.json.asc";
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
    InvalidSequence,
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
    Ok(fetch_verified_manifest(installed, last_sequence, channel)?.manifest)
}

pub struct VerifiedManifest {
    pub manifest: ReleaseManifest,
    pub document: Vec<u8>,
    pub signature: Vec<u8>,
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct PreviewCache {
    schema_version: u32,
    document: String,
    signature: String,
}

/// This cache is only a display convenience in the unprivileged process.
/// Planning and Start always retrieve the current signed offer again.
pub fn fetch_release_preview(
    installed: &ReleaseIdentity,
    last_sequence: Option<u64>,
    channel: ManifestChannelPolicy,
    cache: &Path,
) -> Result<(VerifiedManifest, bool), FetchError> {
    match fetch_verified_manifest(installed, last_sequence, channel) {
        Ok(verified) => {
            let _ = save_preview_cache(cache, &verified);
            Ok((verified, false))
        }
        Err(FetchError::Download) => {
            let cached: PreviewCache =
                serde_json::from_slice(&read_regular(cache, 4 * MAX_MANIFEST_BYTES)?)?;
            if cached.schema_version != 1 {
                return Err(FetchError::Signature);
            }
            let directory = tempfile::tempdir()?;
            let document = directory.path().join("manifest.json");
            let signature = directory.path().join("manifest.asc");
            fs::write(&document, cached.document)?;
            fs::write(&signature, cached.signature)?;
            Ok((
                verify_manifest_files(&document, &signature, installed, last_sequence, channel)?,
                true,
            ))
        }
        Err(error) => {
            // Do not redisplay an older cached offer after observing a revoked,
            // expired, unknown or invalid response.
            let _ = fs::remove_file(cache);
            Err(error)
        }
    }
}

fn save_preview_cache(path: &Path, manifest: &VerifiedManifest) -> Result<(), FetchError> {
    use std::io::Write;
    let parent = path.parent().ok_or(FetchError::InvalidChannel)?;
    fs::create_dir_all(parent)?;
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    let cached = PreviewCache {
        schema_version: 1,
        document: String::from_utf8(manifest.document.clone())
            .map_err(|_| FetchError::Signature)?,
        signature: String::from_utf8(manifest.signature.clone())
            .map_err(|_| FetchError::Signature)?,
    };
    serde_json::to_writer(&mut file, &cached)?;
    file.flush()?;
    file.as_file().sync_all()?;
    file.persist(path)
        .map_err(|error| FetchError::Io(error.error))?;
    fs::File::open(parent)?.sync_all()?;
    Ok(())
}

pub fn fetch_verified_manifest(
    installed: &ReleaseIdentity,
    last_sequence: Option<u64>,
    channel: ManifestChannelPolicy,
) -> Result<VerifiedManifest, FetchError> {
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
    let verified = verify_manifest_files(
        &manifest_path,
        &signature_path,
        installed,
        last_sequence,
        channel,
    )?;
    for (index, repository) in verified.manifest.repositories.iter().enumerate() {
        let key_path = directory.path().join(format!("repository-key-{index}.asc"));
        fetch_repository_key(repository, &key_path)?;
    }
    Ok(verified)
}

/// Reused immediately before staging and offline, with the installed trust anchor.
/// The signed bytes are retained; reserializing JSON cannot preserve a signature.
pub fn verify_manifest_files(
    document_path: &Path,
    signature_path: &Path,
    installed: &ReleaseIdentity,
    last_sequence: Option<u64>,
    channel: ManifestChannelPolicy,
) -> Result<VerifiedManifest, FetchError> {
    let document = read_regular(document_path, MAX_MANIFEST_BYTES)?;
    let signature = read_regular(signature_path, MAX_MANIFEST_BYTES)?;
    // Verify private copies of the exact bytes we parse, avoiding path replacement races.
    let directory = tempfile::tempdir()?;
    let document_copy = directory.path().join("manifest.json");
    let signature_copy = directory.path().join("manifest.asc");
    fs::write(&document_copy, &document)?;
    fs::write(&signature_copy, &signature)?;
    verify_signature(&document_copy, &signature_copy, Path::new(RELEASE_KEYRING))?;
    let manifest: ReleaseManifest = serde_json::from_slice(&document)?;
    validate_manifest_route(
        &manifest,
        installed,
        last_sequence,
        env!("CARGO_PKG_VERSION"),
        channel,
    )
    .map_err(FetchError::Route)?;
    validate_time(&manifest)?;
    Ok(VerifiedManifest {
        manifest,
        document,
        signature,
    })
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
    if fingerprints != [repository.signing_key_fingerprint.clone()] {
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
            "--connect-timeout",
            "10",
            "--max-time",
            "60",
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
    let home = tempfile::tempdir()?;
    let output = Command::new("gpg")
        .arg("--homedir")
        .arg(home.path())
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
    let mut primary = false;
    let mut fingerprints = Vec::new();
    for line in output.lines() {
        let fields: Vec<_> = line.split(':').collect();
        match fields.first().copied() {
            Some("pub") => primary = true,
            Some("sub" | "sec" | "ssb") => primary = false,
            Some("fpr") if primary && fields.len() > 9 => {
                fingerprints.push(fields[9].to_owned());
                primary = false;
            }
            _ => {}
        }
    }
    fingerprints
}

fn verify_signature(manifest: &Path, signature: &Path, keyring: &Path) -> Result<(), FetchError> {
    let output = crate::process::output(
        Command::new("gpgv")
            .arg("--keyring")
            .arg(keyring)
            .arg("--")
            .arg(signature)
            .arg(manifest)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("LC_ALL", "C")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null()),
        20,
    )?;
    output
        .status
        .success()
        .then_some(())
        .ok_or(FetchError::Signature)
}

pub fn validate_time(manifest: &ReleaseManifest) -> Result<(), FetchError> {
    validate_time_at(manifest, OffsetDateTime::now_utc())
}

fn validate_time_at(manifest: &ReleaseManifest, now: OffsetDateTime) -> Result<(), FetchError> {
    let from = OffsetDateTime::parse(&manifest.valid_from, &Rfc3339)
        .map_err(|_| FetchError::InvalidTime)?;
    let until = OffsetDateTime::parse(&manifest.valid_until, &Rfc3339)
        .map_err(|_| FetchError::InvalidTime)?;
    if from >= until {
        return Err(FetchError::InvalidTime);
    }
    if now < from {
        return Err(FetchError::NotYetValid);
    }
    if now >= until {
        return Err(FetchError::Expired);
    }
    Ok(())
}

pub fn read_last_manifest_sequence(path: &Path) -> Result<Option<u64>, FetchError> {
    let bytes = match read_regular(path, 32) {
        Ok(bytes) => bytes,
        Err(FetchError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
            // A dangling symlink is not a missing replay record.
            if fs::symlink_metadata(path).is_ok() {
                return Err(FetchError::InvalidSequence);
            }
            return Ok(None);
        }
        Err(error) => return Err(error),
    };
    let text = std::str::from_utf8(&bytes).map_err(|_| FetchError::InvalidSequence)?;
    let text = text.strip_suffix('\n').unwrap_or(text);
    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(FetchError::InvalidSequence);
    }
    text.parse::<u64>()
        .ok()
        .filter(|value| *value > 0)
        .map(Some)
        .ok_or(FetchError::InvalidSequence)
}

fn read_regular(path: &Path, maximum: u64) -> Result<Vec<u8>, FetchError> {
    let file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.nlink() != 1 {
        return Err(FetchError::InvalidSequence);
    }
    if metadata.len() > maximum {
        return Err(FetchError::TooLarge);
    }
    let mut bytes = Vec::new();
    file.take(maximum + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > maximum {
        return Err(FetchError::TooLarge);
    }
    Ok(bytes)
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

    fn time_fixture() -> super::ReleaseManifest {
        serde_json::from_value(serde_json::json!({
            "schema_version":1,"sequence":1,"status":"available",
            "valid_from":"2026-09-01T00:00:00Z","valid_until":"2026-10-01T00:00:00Z",
            "source":{"version":"1.0","edition":"desktop","architecture":"x86_64","build_id":"source"},
            "target":{"version":"1.1","edition":"desktop","architecture":"x86_64","build_id":"target"},
            "minimum_updater_version":"0.2.3","minimum_free_space_bytes":1,
            "repositories":[],"allowed_removals":[],"allowed_vendor_transitions":[],"lockstep_packages":[]
        })).unwrap()
    }

    #[test]
    fn expiry_is_rechecked_with_a_closed_time_window() {
        let mut manifest = time_fixture();
        let parse = |s: &str| super::OffsetDateTime::parse(s, &super::Rfc3339).unwrap();
        assert!(super::validate_time_at(&manifest, parse("2026-09-01T00:00:00Z")).is_ok());
        assert!(matches!(
            super::validate_time_at(&manifest, parse("2026-10-01T00:00:00Z")),
            Err(super::FetchError::Expired)
        ));
        assert!(matches!(
            super::validate_time_at(&manifest, parse("2026-08-31T23:59:59Z")),
            Err(super::FetchError::NotYetValid)
        ));
        manifest.valid_until = manifest.valid_from.clone();
        assert!(matches!(
            super::validate_time_at(&manifest, parse("2026-09-15T00:00:00Z")),
            Err(super::FetchError::InvalidTime)
        ));
    }

    #[test]
    fn native_signature_rejects_changed_bytes_and_untrusted_keyring() {
        let home = tempfile::tempdir().unwrap();
        use std::os::unix::fs::PermissionsExt;
        super::fs::set_permissions(home.path(), super::fs::Permissions::from_mode(0o700)).unwrap();
        let run = |args: &[&str]| {
            let output = crate::process::output(
                super::Command::new("gpg")
                    .arg("--homedir")
                    .arg(home.path())
                    .args(["--batch", "--pinentry-mode", "loopback", "--passphrase", ""])
                    .args(args),
                20,
            )
            .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            output.stdout
        };
        run(&[
            "--quick-generate-key",
            "Lyra isolated fixture <fixture@invalid.test>",
            "ed25519",
            "sign",
            "0",
        ]);
        let keyring = home.path().join("trusted.gpg");
        super::fs::write(&keyring, run(&["--export"])).unwrap();
        let document = home.path().join("manifest.json");
        super::fs::write(&document, b"{\"sequence\":1}\n").unwrap();
        run(&["--detach-sign", document.to_str().unwrap()]);
        let signature = home.path().join("manifest.json.sig");
        super::verify_signature(&document, &signature, &keyring).unwrap();
        super::fs::write(&document, b"{\"sequence\":2}\n").unwrap();
        assert!(super::verify_signature(&document, &signature, &keyring).is_err());
        super::fs::write(&document, b"{\"sequence\":1}\n").unwrap();
        super::fs::write(&keyring, b"").unwrap();
        assert!(super::verify_signature(&document, &signature, &keyring).is_err());
        let _ = super::Command::new("gpgconf")
            .arg("--homedir")
            .arg(home.path())
            .args(["--kill", "gpg-agent"])
            .status();
    }

    #[test]
    fn parses_only_machine_readable_fingerprints() {
        let output = "pub:-:2048:1:1234::::::\nfpr:::::::::AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA:\nuid:::::::::Lyra:\npub:-:2048:1:5678::::::\nfpr:::::::::BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB:\n";
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
