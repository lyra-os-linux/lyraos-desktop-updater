use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};

use quick_xml::Reader;
use quick_xml::events::Event;

const MAX_OUTPUT: usize = 1024 * 1024;
const ARGUMENTS: &[&str] = &[
    "--xmlout",
    "--non-interactive",
    "--no-refresh",
    "verify",
    "--dry-run",
    "--details",
];

pub fn verify() -> Result<(), ()> {
    verify_with(Path::new("/usr/bin/zypper"))
}

fn verify_with(program: &Path) -> Result<(), ()> {
    // The existing parent supervises the complete worker with a 180s deadline.
    // Dry-run is mandatory: any needed repair belongs to a separately planned
    // and authorized operation, never to the VerifyingBoot phase.
    let mut child = Command::new(program)
        .args(ARGUMENTS)
        .env_clear()
        .env("PATH", "/usr/sbin:/usr/bin:/sbin:/bin")
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| ())?;
    let mut bytes = Vec::new();
    let result = child
        .stdout
        .take()
        .ok_or(())?
        .take((MAX_OUTPUT + 1) as u64)
        .read_to_end(&mut bytes);
    if result.is_err() || bytes.len() > MAX_OUTPUT {
        let _ = child.kill();
        let _ = child.wait();
        return Err(());
    }
    let status = child.wait().map_err(|_| ())?;
    healthy_output(status.success(), &bytes)
}

fn healthy_output(success: bool, bytes: &[u8]) -> Result<(), ()> {
    if !success || bytes.len() > MAX_OUTPUT {
        return Err(());
    }
    let xml = std::str::from_utf8(bytes).map_err(|_| ())?;
    let mut reader = Reader::from_str(xml);
    let mut path: Vec<Vec<u8>> = Vec::new();
    let mut root_seen = false;
    let mut declaration_seen = false;
    let mut summary_seen = false;
    loop {
        let event = reader.read_event().map_err(|_| ())?;
        match &event {
            Event::Start(element) | Event::Empty(element) => {
                let name = element.name();
                let name = name.as_ref();
                if path.is_empty() {
                    if root_seen || name != b"stream" {
                        return Err(());
                    }
                    root_seen = true;
                }
                if path.iter().any(|part| part == b"install-summary") {
                    // A healthy dependency summary has no proposed actions,
                    // even if a malformed response claims packages-to-change=0.
                    return Err(());
                }
                let mut attributes = std::collections::BTreeMap::new();
                for attr in element.attributes() {
                    let attr = attr.map_err(|_| ())?;
                    let value = attr
                        .decode_and_unescape_value(reader.decoder())
                        .map_err(|_| ())?;
                    attributes.insert(attr.key.as_ref().to_vec(), value.into_owned());
                }
                if name == b"install-summary" {
                    if summary_seen || path.as_slice() != [b"stream".to_vec()] {
                        return Err(());
                    }
                    summary_seen = true;
                    for field in [
                        b"packages-to-change".as_slice(),
                        b"download-size",
                        b"space-usage-diff",
                        b"space-usage-installed",
                        b"space-usage-removed",
                    ] {
                        if attributes
                            .get(field)
                            .ok_or(())?
                            .parse::<i64>()
                            .map_err(|_| ())?
                            != 0
                        {
                            return Err(());
                        }
                    }
                }
                if name == b"prompt"
                    || name == b"solvable"
                    || name.starts_with(b"to-")
                    || (name == b"message"
                        && attributes
                            .get(b"type".as_slice())
                            .is_some_and(|value| value == "error"))
                {
                    return Err(());
                }
                if matches!(event, Event::Start(_)) {
                    if path.len() >= 32 {
                        return Err(());
                    }
                    path.push(name.to_vec());
                }
            }
            Event::End(element) => {
                if path.pop().as_deref() != Some(element.name().as_ref()) {
                    return Err(());
                }
            }
            Event::Text(text) => {
                if (path.is_empty() || path.last().is_some_and(|part| part == b"install-summary"))
                    && !text.as_ref().iter().all(u8::is_ascii_whitespace)
                {
                    return Err(());
                }
            }
            Event::GeneralRef(reference) => {
                if path.is_empty() || path.last().is_some_and(|part| part == b"install-summary") {
                    return Err(());
                }
                if reference.resolve_char_ref().map_err(|_| ())?.is_none()
                    && !matches!(
                        reference.as_ref(),
                        b"amp" | b"lt" | b"gt" | b"apos" | b"quot"
                    )
                {
                    return Err(());
                }
            }
            Event::Decl(_) => {
                if declaration_seen || root_seen {
                    return Err(());
                }
                declaration_seen = true;
            }
            Event::DocType(_) | Event::PI(_) | Event::CData(_) => return Err(()),
            Event::Eof => break,
            _ => {}
        }
    }
    if !root_seen || !summary_seen || !path.is_empty() {
        return Err(());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    const HEALTHY: &[u8] = include_bytes!("../tests/fixtures/verify-healthy.xml");
    const INSTALL: &[u8] = include_bytes!("../tests/fixtures/verify-install.xml");
    const REMOVE: &[u8] = include_bytes!("../tests/fixtures/verify-remove.xml");

    #[test]
    fn actual_zypper_success_is_not_enough_when_it_proposes_a_repair() {
        assert_eq!(healthy_output(true, HEALTHY), Ok(()));
        assert_eq!(healthy_output(true, INSTALL), Err(()));
        assert_eq!(healthy_output(false, REMOVE), Err(()));
        assert_eq!(healthy_output(true, REMOVE), Err(()));
        assert_eq!(healthy_output(false, HEALTHY), Err(()));
    }

    #[test]
    fn all_actions_and_nonzero_summary_values_require_recovery() {
        let healthy = std::str::from_utf8(HEALTHY).unwrap();
        for name in [
            "to-install",
            "to-remove",
            "to-upgrade",
            "to-downgrade",
            "to-reinstall",
            "to-change-vendor",
            "to-change-arch",
            "unexpected-action",
        ] {
            let xml = healthy.replace(
                "</install-summary>",
                &format!("<{name}/></install-summary>"),
            );
            assert_eq!(healthy_output(true, xml.as_bytes()), Err(()), "{name}");
        }
        for field in [
            "packages-to-change",
            "download-size",
            "space-usage-diff",
            "space-usage-installed",
            "space-usage-removed",
        ] {
            for value in ["1", "-1", "invalid", "9223372036854775808"] {
                let xml =
                    healthy.replace(&format!("{field}=\"0\""), &format!("{field}=\"{value}\""));
                assert_eq!(
                    healthy_output(true, xml.as_bytes()),
                    Err(()),
                    "{field}={value}"
                );
            }
            let xml = healthy.replace(&format!(" {field}=\"0\""), "");
            assert_eq!(healthy_output(true, xml.as_bytes()), Err(()));
        }
    }

    #[test]
    fn malformed_truncated_duplicate_and_error_documents_fail_closed() {
        let healthy = std::str::from_utf8(HEALTHY).unwrap();
        for xml in [
            "".into(),
            "<stream/>".into(),
            healthy.replace("<stream>", "<stream><?xml version='1.0'?>"),
            format!("<?xml version='1.0'?>{healthy}"),
            "not XML".into(),
            healthy.replace("</stream>", ""),
            healthy.replace("</stream>", "</other>"),
            format!("{healthy}{healthy}"),
            healthy.replace(
                "<stream>",
                "<!DOCTYPE stream [<!ENTITY bad SYSTEM 'file:///etc/passwd'>]><stream>",
            ),
            healthy.replace(
                "</stream>",
                "<message type=\"error\">failed</message></stream>",
            ),
            healthy
                .replace("<install-summary", "<nested><install-summary")
                .replace("</install-summary>", "</install-summary></nested>"),
            healthy.replace(
                "packages-to-change=\"0\"",
                "packages-to-change=\"0\" packages-to-change=\"0\"",
            ),
            healthy.replace(
                "</stream>",
                "<install-summary packages-to-change=\"0\"/></stream>",
            ),
            healthy.replace("</stream>", "<message>&unknown;</message></stream>"),
        ] {
            assert_eq!(healthy_output(true, xml.as_bytes()), Err(()));
        }
        assert_eq!(healthy_output(true, &[0xff, 0xfe]), Err(()));
        assert_eq!(healthy_output(true, &vec![b' '; MAX_OUTPUT + 1]), Err(()));
    }

    #[test]
    fn command_is_always_a_dry_run_and_bounds_captured_output() {
        use std::os::unix::fs::PermissionsExt;
        let root =
            std::env::temp_dir().join(format!("lyra-dependency-probe-{}", std::process::id()));
        std::fs::create_dir(&root).unwrap();
        let script = root.join("probe");
        let xml = std::str::from_utf8(HEALTHY).unwrap();
        std::fs::write(&script,format!("#!/bin/sh\ntest \"$*\" = '--xmlout --non-interactive --no-refresh verify --dry-run --details' || exit 99\ncat <<'XML'\n{xml}\nXML\n")).unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(verify_with(&script), Ok(()));
        std::fs::write(
            &script,
            format!("#!/bin/sh\nhead -c {} /dev/zero\n", MAX_OUTPUT + 2),
        )
        .unwrap();
        assert_eq!(verify_with(&script), Err(()));
        std::fs::remove_dir_all(root).unwrap();
    }
}
