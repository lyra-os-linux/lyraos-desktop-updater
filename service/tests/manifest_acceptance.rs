use lyra_upgrade_service::manifest_fetch::{FetchError, read_last_manifest_sequence};
use std::fs;

#[test]
fn damaged_replay_record_never_becomes_a_first_upgrade() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("sequence");
    assert_eq!(read_last_manifest_sequence(&path).unwrap(), None);
    for valid in ["1", "42\n", "18446744073709551615\n"] {
        fs::write(&path, valid).unwrap();
        assert_eq!(
            read_last_manifest_sequence(&path).unwrap(),
            Some(valid.trim().parse().unwrap())
        );
    }
    for invalid in [
        "",
        "0",
        "-1",
        "+1",
        " 1",
        "1\n2",
        "18446744073709551616",
        "broken",
    ] {
        fs::write(&path, invalid).unwrap();
        assert!(
            read_last_manifest_sequence(&path).is_err(),
            "accepted {invalid:?}"
        );
    }
    fs::write(&path, vec![b'1'; 100]).unwrap();
    assert!(matches!(
        read_last_manifest_sequence(&path),
        Err(FetchError::TooLarge)
    ));
    fs::remove_file(&path).unwrap();
    std::os::unix::fs::symlink(root.path().join("missing"), &path).unwrap();
    assert!(read_last_manifest_sequence(&path).is_err());
    fs::remove_file(&path).unwrap();
    fs::create_dir(&path).unwrap();
    assert!(read_last_manifest_sequence(&path).is_err());
}

#[test]
fn replay_record_refuses_links_and_never_blocks_on_a_fifo() {
    use std::os::unix::ffi::OsStrExt;
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("sequence");
    let target = root.path().join("target");
    fs::write(&target, "42").unwrap();
    fs::hard_link(&target, &path).unwrap();
    assert!(read_last_manifest_sequence(&path).is_err());
    fs::remove_file(&path).unwrap();
    let name = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
    assert!(read_last_manifest_sequence(&path).is_err());
    assert_eq!(fs::read_to_string(target).unwrap(), "42");
}
