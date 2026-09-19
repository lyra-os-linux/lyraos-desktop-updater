//! Decide marker ownership before opening Lyra state or acquiring a lock.
use std::fs;
use std::io::ErrorKind;
use std::path::{Component, Path, PathBuf};

pub fn resolve(marker: &Path, root: &Path) -> Result<Option<PathBuf>, String> {
    let target = match fs::read_link(marker) {
        Ok(target) => target,
        // PackageKit may remove its marker between unit condition and startup.
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("cannot read offline update marker: {error}")),
    };
    let absolute = if target.is_absolute() {
        target
    } else {
        marker.parent().ok_or("marker has no parent")?.join(target)
    };
    let mut normalized = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::ParentDir => {
                normalized.pop();
            }
            Component::CurDir => {}
            other => normalized.push(other.as_os_str()),
        }
    }
    // Do not canonicalize foreign destinations: neither they nor the Lyra
    // state directory need exist for another updater to own this boot.
    if !absolute.starts_with(root) && !normalized.starts_with(root) {
        return Ok(None);
    }
    if normalized.parent() != Some(root) {
        return Err(
            "Lyra offline marker must name one operation directly under the state root".into(),
        );
    }
    let canonical_root = fs::canonicalize(root)
        .map_err(|error| format!("cannot resolve Lyra offline state root: {error}"))?;
    let directory = fs::canonicalize(&normalized)
        .map_err(|error| format!("cannot resolve Lyra offline operation: {error}"))?;
    if directory.parent() != Some(canonical_root.as_path())
        || directory != canonical_root.join(normalized.file_name().ok_or("missing operation id")?)
    {
        return Err("Lyra offline operation resolves outside its expected directory".into());
    }
    if !directory.is_dir() {
        return Err("Lyra offline operation is not a directory".into());
    }
    Ok(Some(directory))
}

pub fn require_current(marker: &Path, root: &Path, operation: &Path) -> Result<(), String> {
    if resolve(marker, root)?.as_deref() != Some(operation) {
        return Err("offline update marker no longer belongs to this Lyra operation".into());
    }
    Ok(())
}

pub fn remove_current(marker: &Path, root: &Path, operation: &Path) -> Result<(), String> {
    require_current(marker, root, operation)?;
    fs::remove_file(marker).map_err(|error| format!("cannot remove Lyra offline marker: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    #[test]
    fn foreign_and_absent_markers_do_not_require_lyra_state() {
        let temp = super::super::tests::temporary("foreign-marker");
        let marker = temp.join("system-update");
        let root = temp.join("operations");
        assert_eq!(resolve(&marker, &root).unwrap(), None);
        for target in [
            temp.join("PackageKit"),
            PathBuf::from("PackageKit"),
            temp.join("operations-other/job"),
        ] {
            symlink(&target, &marker).unwrap();
            assert_eq!(resolve(&marker, &root).unwrap(), None);
            assert_eq!(fs::read_link(&marker).unwrap(), target);
            assert!(!root.exists());
            fs::remove_file(&marker).unwrap();
            assert_eq!(resolve(&marker, &root).unwrap(), None);
        }
        fs::remove_dir_all(temp).unwrap();
    }

    #[test]
    fn broken_lyra_requests_remain_errors() {
        let temp = super::super::tests::temporary("broken-marker");
        let marker = temp.join("system-update");
        let root = temp.join("operations");
        for target in [
            root.join("missing"),
            root.clone(),
            root.join("nested/job"),
            root.join("../PackageKit"),
        ] {
            symlink(target, &marker).unwrap();
            assert!(resolve(&marker, &root).is_err());
            assert!(fs::read_link(&marker).is_ok());
            fs::remove_file(&marker).unwrap();
        }
        fs::write(&marker, "invalid marker").unwrap();
        assert!(resolve(&marker, &root).is_err());
        fs::remove_dir_all(temp).unwrap();
    }

    #[test]
    fn owned_requests_resolve_but_cleanup_preserves_replacements() {
        let temp = super::super::tests::temporary("owned-marker");
        let marker = temp.join("system-update");
        let root = temp.join("operations");
        let operation = root.join("job");
        fs::create_dir_all(&operation).unwrap();
        for target in [operation.clone(), PathBuf::from("operations/job")] {
            symlink(target, &marker).unwrap();
            assert_eq!(resolve(&marker, &root).unwrap(), Some(operation.clone()));
            remove_current(&marker, &root, &operation).unwrap();
        }
        symlink(temp.join("PackageKit"), &marker).unwrap();
        assert!(remove_current(&marker, &root, &operation).is_err());
        assert_eq!(fs::read_link(&marker).unwrap(), temp.join("PackageKit"));
        fs::remove_file(&marker).unwrap();
        assert!(require_current(&marker, &root, &operation).is_err());
        symlink(&operation, root.join("alias")).unwrap();
        symlink(root.join("alias"), &marker).unwrap();
        assert!(resolve(&marker, &root).is_err());
        fs::remove_dir_all(temp).unwrap();
    }
}
