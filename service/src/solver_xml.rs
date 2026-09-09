use lyra_upgrade_core::{PackageAction, PackageChange, SolverResult};
use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};

#[derive(Debug)]
pub enum SolverXmlError {
    Xml(quick_xml::Error),
    InvalidNumber(&'static str),
    MissingSummary,
    InvalidPackage(&'static str),
    ConflictingPackage,
    ArchitectureChange,
}

impl From<quick_xml::Error> for SolverXmlError {
    fn from(error: quick_xml::Error) -> Self {
        Self::Xml(error)
    }
}

pub fn parse_solver_xml(
    xml: &str,
    metadata_valid_repositories: Vec<String>,
    estimated_snapshot_bytes: u64,
) -> Result<SolverResult, SolverXmlError> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    let mut path: Vec<Vec<u8>> = Vec::new();
    let mut changes = std::collections::BTreeMap::new();
    let mut problems = Vec::new();
    let mut summary = None;
    let mut in_error_message = false;

    loop {
        let event = reader.read_event()?;
        match &event {
            Event::Start(element) | Event::Empty(element) => {
                let name = element.name();
                if name.as_ref() == b"install-summary" {
                    if summary.is_some() || path.as_slice() != [b"stream".to_vec()] {
                        return Err(SolverXmlError::InvalidPackage(
                            "duplicate or misplaced summary",
                        ));
                    }
                    summary = Some(parse_summary(&reader, element)?);
                }
                if name.as_ref() == b"solvable"
                    && path.len() == 3
                    && path[1] == b"install-summary"
                    && let Some(action) = section_action(&path[2])
                    && let Some(change) = parse_solvable(&reader, element, action)?
                {
                    let key = (
                        change.name.clone(),
                        change.architecture.clone(),
                        change.current_version.clone(),
                        change.proposed_version.clone(),
                        change.repository_alias.clone(),
                    );
                    let existing = changes.entry(key).or_insert_with(|| change.clone());
                    if existing.action != change.action {
                        if existing.action == PackageAction::Reinstall {
                            existing.action = change.action;
                        } else if change.action != PackageAction::Reinstall {
                            return Err(SolverXmlError::ConflictingPackage);
                        }
                    }
                }
                if name.as_ref() == b"message" {
                    in_error_message =
                        attribute(&reader, element, b"type")?.is_some_and(|kind| kind == "error");
                    if in_error_message {
                        problems.push("zypper reported an error".into());
                    }
                }
                if matches!(event, Event::Start(_)) {
                    if path.len() >= 32 {
                        return Err(SolverXmlError::InvalidPackage("excessive nesting"));
                    }
                    path.push(name.as_ref().to_vec());
                }
            }
            Event::Text(text) if in_error_message => {
                let value = text.decode().map_err(quick_xml::Error::Encoding)?;
                if !value.trim().is_empty() {
                    problems.push(value.trim().to_string());
                }
            }
            Event::End(element) => {
                path.pop();
                if element.name().as_ref() == b"message" {
                    in_error_message = false;
                }
            }
            Event::DocType(_) => return Err(SolverXmlError::InvalidPackage("DTD not allowed")),
            Event::Eof => break,
            _ => {}
        }
    }
    let summary = summary.ok_or(SolverXmlError::MissingSummary)?;
    let mut changes: Vec<PackageChange> = changes.into_values().collect();
    if !path.is_empty()
        || changes.len() as u64 != summary.packages_to_change
        || changes.iter().any(|change| {
            change.action == PackageAction::Reinstall
                && change.current_version != change.proposed_version
        })
    {
        return Err(SolverXmlError::ConflictingPackage);
    }
    changes.sort();
    problems.sort();
    problems.dedup();
    Ok(SolverResult {
        schema_version: 1,
        successful: problems.is_empty(),
        problems,
        metadata_valid_repositories,
        changes,
        download_bytes: summary.download_bytes,
        transaction_size_increase: summary.space_usage_diff.max(0) as u64,
        estimated_snapshot_bytes: estimated_snapshot_bytes.max(summary.snapshot_bytes),
        reboot_required: summary.need_reboot,
    })
}

fn section_action(section: &[u8]) -> Option<PackageAction> {
    match section {
        b"to-install" => Some(PackageAction::Install),
        b"to-remove" => Some(PackageAction::Remove),
        b"to-upgrade" | b"to-upgrade-change-arch" => Some(PackageAction::Upgrade),
        b"to-downgrade" | b"to-downgrade-change-arch" => Some(PackageAction::Downgrade),
        b"to-reinstall" | b"to-change-arch" | b"to-change-vendor" => Some(PackageAction::Reinstall),
        _ => None,
    }
}

struct Summary {
    packages_to_change: u64,
    download_bytes: u64,
    space_usage_diff: i64,
    snapshot_bytes: u64,
    need_reboot: bool,
}

fn parse_summary(
    reader: &Reader<&[u8]>,
    element: &BytesStart<'_>,
) -> Result<Summary, SolverXmlError> {
    Ok(Summary {
        packages_to_change: required_number(reader, element, b"packages-to-change")?,
        download_bytes: required_number(reader, element, b"download-size")?,
        space_usage_diff: required_signed_number(reader, element, b"space-usage-diff")?,
        snapshot_bytes: required_number(reader, element, b"space-usage-installed")?
            .saturating_add(required_number(reader, element, b"space-usage-removed")?),
        need_reboot: attribute(reader, element, b"need-reboot")?
            .is_some_and(|value| matches!(value.as_str(), "true" | "1")),
    })
}

fn parse_solvable(
    reader: &Reader<&[u8]>,
    element: &BytesStart<'_>,
    action: PackageAction,
) -> Result<Option<PackageChange>, SolverXmlError> {
    match attribute(reader, element, b"type")?.as_deref() {
        Some("pattern" | "product" | "patch" | "application") => return Ok(None),
        Some("package") => {}
        _ => return Err(SolverXmlError::InvalidPackage("unsupported solvable type")),
    }
    let name = attribute(reader, element, b"name")?
        .filter(|value| !value.is_empty())
        .ok_or(SolverXmlError::InvalidPackage("name"))?;
    let architecture = attribute(reader, element, b"arch")?
        .filter(|value| !value.is_empty())
        .ok_or(SolverXmlError::InvalidPackage("architecture"))?;
    if attribute(reader, element, b"arch-old")?.is_some_and(|old| old != architecture) {
        // The v1 contract cannot identify the old architecture. Never guess
        // an installed package when the transaction violates our arch policy.
        return Err(SolverXmlError::ArchitectureChange);
    }
    let version = attribute(reader, element, b"edition")?
        .filter(|value| !value.is_empty())
        .ok_or(SolverXmlError::InvalidPackage("edition"))?;
    let old = attribute(reader, element, b"edition-old")?;
    let (current_version, proposed_version) = match action {
        PackageAction::Remove => (Some(version), None),
        PackageAction::Install => (None, Some(version)),
        PackageAction::Reinstall => (
            Some(old.ok_or(SolverXmlError::InvalidPackage("edition-old"))?),
            Some(version),
        ),
        _ => (
            Some(old.ok_or(SolverXmlError::InvalidPackage("edition-old"))?),
            Some(version),
        ),
    };
    Ok(Some(PackageChange {
        name,
        architecture,
        action,
        current_version,
        proposed_version,
        current_vendor: None,
        proposed_vendor: None,
        repository_alias: if action == PackageAction::Remove {
            None
        } else {
            attribute(reader, element, b"repository")?
        },
        download_bytes: 0,
        installed_size_before: 0,
        installed_size_after: 0,
    }))
}

fn required_number(
    reader: &Reader<&[u8]>,
    element: &BytesStart<'_>,
    name: &'static [u8],
) -> Result<u64, SolverXmlError> {
    attribute(reader, element, name)?
        .and_then(|value| value.parse().ok())
        .ok_or(SolverXmlError::InvalidNumber("unsigned summary field"))
}

fn required_signed_number(
    reader: &Reader<&[u8]>,
    element: &BytesStart<'_>,
    name: &'static [u8],
) -> Result<i64, SolverXmlError> {
    attribute(reader, element, name)?
        .and_then(|value| value.parse().ok())
        .ok_or(SolverXmlError::InvalidNumber("signed summary field"))
}

fn attribute(
    reader: &Reader<&[u8]>,
    element: &BytesStart<'_>,
    name: &[u8],
) -> Result<Option<String>, SolverXmlError> {
    for item in element.attributes().with_checks(true) {
        let item = item.map_err(quick_xml::Error::InvalidAttr)?;
        if item.key.as_ref() == name {
            return Ok(Some(
                item.decode_and_unescape_value(reader.decoder())?
                    .into_owned(),
            ));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vendor_only_change_is_not_lost() {
        let xml = r#"<stream><install-summary download-size="4096" space-usage-diff="0" space-usage-installed="4096" space-usage-removed="4096" packages-to-change="1"><to-change-vendor><solvable type="package" name="critical-package" edition="1-1" edition-old="1-1" arch="x86_64" arch-old="x86_64" repository="repo-target"/></to-change-vendor></install-summary></stream>"#;
        let result = parse_solver_xml(xml, vec!["repo-target".into()], 0).unwrap();
        assert_eq!(result.changes.len(), 1);
        assert_eq!(result.changes[0].action, PackageAction::Reinstall);
        assert_eq!(result.changes[0].current_version.as_deref(), Some("1-1"));
    }

    #[test]
    fn parses_dry_run_summary_and_changes() {
        let xml = r#"<?xml version='1.0'?><stream><install-summary download-size="4096" space-usage-diff="2048" space-usage-installed="2048" space-usage-removed="0" packages-to-change="1" need-restart="false" need-reboot="true"><to-upgrade><solvable status="other-version" type="package" name="firefox" edition="2" edition-old="1" arch="x86_64" repository="repo-oss"/></to-upgrade></install-summary></stream>"#;
        let result = parse_solver_xml(xml, vec!["repo-oss".into()], 8192).unwrap();
        assert!(result.successful);
        assert_eq!(result.download_bytes, 4096);
        assert_eq!(result.changes[0].action, PackageAction::Upgrade);
        assert!(result.reboot_required);
    }
}
