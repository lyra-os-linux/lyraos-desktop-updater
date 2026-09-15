//! Disposable RPM/zypp root. The solver never receives write access to the live
//! package database. UID 0 required by zypper is mapped to the calling user.
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::repository_context::RepositoryContext;

pub struct Simulation {
    _directory: tempfile::TempDir,
    pub context: RepositoryContext,
    pub root: PathBuf,
}

impl Simulation {
    pub fn new(with_current_repositories: bool) -> io::Result<Self> {
        let directory = tempfile::Builder::new().prefix("lyra-sim-").tempdir()?;
        let root = directory.path().join("root");
        fs::create_dir(&root)?;
        fs::create_dir(root.join("tmp"))?;
        fs::create_dir_all(root.join("var/tmp"))?;
        for path in [
            "usr/lib/sysimage/rpm",
            "usr/lib/rpm/gnupg",
            "etc/zypp/locks",
            "etc/zypp/vars.d",
            "etc/zypp/vendors.d",
            "etc/zypp/systemCheck",
            "var/lib/zypp/AutoInstalled",
            "etc/products.d",
            "etc/os-release",
            "usr/lib/os-release",
        ] {
            copy_tree(
                Path::new("/").join(path).as_path(),
                &root.join(path),
                path != "usr/lib/sysimage/rpm",
            )
            .map_err(|error| io::Error::new(error.kind(), format!("simulation {path}: {error}")))?;
        }
        let mut context =
            RepositoryContext::prepared(&root.join("var/lib/lyra-upgrade-simulation"));
        fs::create_dir_all(&context.repos)?;
        fs::create_dir_all(&context.cache)?;
        if with_current_repositories {
            copy_tree(
                Path::new("/etc/zypp/services.d"),
                &root.join("etc/zypp/services.d"),
                true,
            )?;
            for (source, destination) in [
                ("/etc/zypp/repos.d", &context.repos),
                ("/var/cache/zypp/raw", &context.raw),
                ("/var/cache/zypp/solv", &context.solv),
            ] {
                copy_tree(Path::new(source), destination, true).map_err(|error| {
                    io::Error::new(error.kind(), format!("simulation {source}: {error}"))
                })?;
            }
        }
        context.simulation_root = Some(root.clone());
        Ok(Self {
            _directory: directory,
            context,
            root,
        })
    }
}

fn copy_tree(source: &Path, destination: &Path, optional: bool) -> io::Result<()> {
    let metadata = match fs::symlink_metadata(source) {
        Ok(metadata) => metadata,
        Err(error) if optional && error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    if metadata.is_dir() {
        fs::create_dir_all(destination)?;
        for entry in fs::read_dir(source)? {
            let entry = entry?;
            copy_tree(&entry.path(), &destination.join(entry.file_name()), false).map_err(
                |error| {
                    io::Error::new(error.kind(), format!("{}: {error}", entry.path().display()))
                },
            )?;
        }
    } else if metadata.is_symlink()
        && source
            .file_name()
            .is_some_and(|name| name == ".repo_gpgcheck")
        && matches!(
            fs::read_link(source)?.to_str(),
            Some("true" | "false" | "./true" | "./false")
        )
    {
        std::os::unix::fs::symlink(fs::read_link(source)?, destination)?;
    } else if metadata.is_file() || metadata.is_symlink() {
        // Copy the contents of configuration/cache links, never links back into
        // the real host. cp --reflink keeps large RPM databases inexpensive.
        let resolved = source.canonicalize()?;
        if !fs::metadata(&resolved)?.is_file() {
            return Err(io::Error::other("nonregular simulation input"));
        }
        fs::create_dir_all(
            destination
                .parent()
                .ok_or_else(|| io::Error::other("missing parent"))?,
        )?;
        let result = Command::new("/usr/bin/cp")
            .args(["--reflink=auto", "--"])
            .arg(&resolved)
            .arg(destination)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()?;
        if !result.success() {
            return Err(io::Error::other("simulation copy failed"));
        }
    } else {
        return Err(io::Error::other("special simulation input"));
    }
    Ok(())
}
