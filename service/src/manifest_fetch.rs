use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use lyra_upgrade_core::{ManifestError, ReleaseIdentity, ReleaseManifest, validate_manifest_route};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

pub const RELEASE_MANIFEST_URL: &str =
    "https://downloads.sourceforge.net/project/lyra/releases/1.0/desktop/releases-v1.json";
pub const RELEASE_MANIFEST_SIGNATURE_URL: &str =
    "https://downloads.sourceforge.net/project/lyra/releases/1.0/desktop/releases-v1.json.asc";
pub const RELEASE_KEYRING: &str = "/usr/share/lyra-upgrade/release-signing-key.gpg";
const MAX_MANIFEST_BYTES: u64 = 1024 * 1024;

#[derive(Debug)]
pub enum FetchError {
    Io(std::io::Error),
    Download,
    Signature,
    TooLarge,
    Json(serde_json::Error),
    Route(ManifestError),
    InvalidTime,
    NotYetValid,
    Expired,
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
) -> Result<ReleaseManifest, FetchError> {
    let directory = tempfile::Builder::new()
        .prefix("lyra-upgrade-manifest-")
        .tempdir()?;
    let manifest_path = directory.path().join("manifest.json");
    let signature_path = directory.path().join("manifest.json.asc");
    download(RELEASE_MANIFEST_URL, &manifest_path)?;
    download(RELEASE_MANIFEST_SIGNATURE_URL, &signature_path)?;
    if fs::metadata(&manifest_path)?.len() > MAX_MANIFEST_BYTES
        || fs::metadata(&signature_path)?.len() > MAX_MANIFEST_BYTES
    {
        return Err(FetchError::TooLarge);
    }
    verify_signature(&manifest_path, &signature_path)?;
    let manifest: ReleaseManifest = serde_json::from_slice(&fs::read(manifest_path)?)?;
    validate_manifest_route(&manifest, installed, last_sequence).map_err(FetchError::Route)?;
    validate_time(&manifest)?;
    Ok(manifest)
}

fn download(url: &'static str, destination: &Path) -> Result<(), FetchError> {
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

pub fn manifest_sequence_path() -> PathBuf {
    PathBuf::from("/var/lib/lyra-upgrade/last-manifest-sequence")
}
