//! One command policy for planning, download and offline package maintenance.
use lyra_upgrade_core::migration::{MigrationError, pending_migration};
use lyra_upgrade_core::{InstalledPackage, ReleaseManifest};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{self, Read},
    path::Path,
};

#[derive(Clone, Copy)]
pub enum TransactionMode {
    Plan,
    Download,
    Apply,
}

pub fn transaction_arguments(
    manifest: &ReleaseManifest,
    installed: &[InstalledPackage],
    mode: TransactionMode,
) -> Result<Vec<String>, MigrationError> {
    let mut args = Vec::new();
    let pending = if manifest.package_migration.is_some() {
        let pending = pending_migration(manifest, installed)?;
        if pending.is_empty() {
            return Err(MigrationError::AlreadyApplied);
        }
        args.extend(
            [
                "install",
                "--name",
                "--no-recommends",
                "--from",
                &pending[0].repository_alias,
            ]
            .map(str::to_owned),
        );
        pending
    } else {
        args.push("dist-upgrade".into());
        vec![]
    };
    match mode {
        TransactionMode::Plan => args.push("--dry-run".into()),
        TransactionMode::Download => args.push("--download-only".into()),
        TransactionMode::Apply => (),
    }
    args.extend(
        [
            "--details",
            "--no-allow-downgrade",
            "--no-allow-name-change",
            "--no-allow-arch-change",
            "--allow-vendor-change",
        ]
        .map(str::to_owned),
    );
    if manifest.package_migration.is_some() {
        args.push("--".into());
        args.extend(
            pending
                .iter()
                .map(|p| format!("{}.{}={}", p.name, p.architecture, p.to_version)),
        );
    }
    Ok(args)
}

/// Every selected artifact must be present with the digest authorized by the
/// manifest. Reject additional RPMs, links, duplicate payloads and special files.
pub fn verify_payloads(
    manifest: &ReleaseManifest,
    installed: &[InstalledPackage],
    directory: &Path,
) -> io::Result<()> {
    if manifest.package_migration.is_none() {
        return Ok(());
    }
    let pending =
        pending_migration(manifest, installed).map_err(|e| io::Error::other(format!("{e:?}")))?;
    let mut remaining: std::collections::BTreeSet<_> =
        pending.iter().map(|p| p.sha256.as_str()).collect();
    if remaining.len() != pending.len() || remaining.is_empty() {
        return Err(io::Error::other("invalid payload set"));
    }
    let mut files = Vec::new();
    collect(directory, &mut files, 0)?;
    for path in files {
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(&path)?;
        if !file.metadata()?.is_file() {
            return Err(io::Error::other("nonregular RPM payload"));
        }
        let mut hash = Sha256::new();
        let mut buffer = [0u8; 65536];
        loop {
            let size = file.read(&mut buffer)?;
            if size == 0 {
                break;
            }
            hash.update(&buffer[..size]);
        }
        if !remaining.remove(format!("{:x}", hash.finalize()).as_str()) {
            return Err(io::Error::other("unexpected or altered RPM payload"));
        }
    }
    if !remaining.is_empty() {
        return Err(io::Error::other("missing RPM payload"));
    }
    Ok(())
}

fn collect(directory: &Path, files: &mut Vec<std::path::PathBuf>, depth: usize) -> io::Result<()> {
    if depth > 8 || fs::symlink_metadata(directory)?.file_type().is_symlink() {
        return Err(io::Error::other("invalid cache directory"));
    }
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        if kind.is_dir() {
            collect(&entry.path(), files, depth + 1)?;
        } else if !kind.is_file() {
            return Err(io::Error::other("nonregular cache entry"));
        } else if entry.path().extension().is_some_and(|ext| ext == "rpm") {
            if files.len() >= 64 {
                return Err(io::Error::other("too many RPMs"));
            }
            files.push(entry.path());
        }
    }
    Ok(())
}
