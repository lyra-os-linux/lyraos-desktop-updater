use lyra_upgrade_core::{PackageAction, PackageChange, SolverResult};
use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};

#[derive(Debug)]
pub enum SolverXmlError {
    Xml(quick_xml::Error),
    InvalidNumber(&'static str),
    MissingSummary,
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
    let mut current_action = None;
    let mut changes = Vec::new();
    let mut problems = Vec::new();
    let mut summary = None;
    let mut in_error_message = false;

    loop {
        match reader.read_event()? {
            Event::Start(element) => match element.name().as_ref() {
                b"install-summary" => summary = Some(parse_summary(&reader, &element)?),
                b"to-install" => current_action = Some(PackageAction::Install),
                b"to-remove" => current_action = Some(PackageAction::Remove),
                b"to-upgrade" | b"to-upgrade-change-arch" => {
                    current_action = Some(PackageAction::Upgrade)
                }
                b"to-downgrade" | b"to-downgrade-change-arch" => {
                    current_action = Some(PackageAction::Downgrade)
                }
                b"to-reinstall" | b"to-change-arch" => {
                    current_action = Some(PackageAction::Reinstall)
                }
                b"solvable" if current_action.is_some() => {
                    changes.push(parse_solvable(&reader, &element, current_action.unwrap())?)
                }
                b"message" => {
                    in_error_message =
                        attribute(&reader, &element, b"type")?.is_some_and(|kind| kind == "error")
                }
                _ => {}
            },
            Event::Text(text) if in_error_message => {
                let value = text.decode().map_err(quick_xml::Error::Encoding)?;
                let value = value.trim();
                if !value.is_empty() {
                    problems.push(value.to_string());
                }
            }
            Event::Empty(element)
                if element.name().as_ref() == b"solvable" && current_action.is_some() =>
            {
                changes.push(parse_solvable(&reader, &element, current_action.unwrap())?)
            }
            Event::End(element) => match element.name().as_ref() {
                b"to-install"
                | b"to-remove"
                | b"to-upgrade"
                | b"to-downgrade"
                | b"to-upgrade-change-arch"
                | b"to-downgrade-change-arch"
                | b"to-reinstall"
                | b"to-change-arch" => current_action = None,
                b"message" => in_error_message = false,
                _ => {}
            },
            Event::Eof => break,
            _ => {}
        }
    }

    let summary = summary.ok_or(SolverXmlError::MissingSummary)?;
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

struct Summary {
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
) -> Result<PackageChange, SolverXmlError> {
    Ok(PackageChange {
        name: attribute(reader, element, b"name")?.unwrap_or_default(),
        architecture: attribute(reader, element, b"arch")?.unwrap_or_default(),
        action,
        current_version: attribute(reader, element, b"edition-old")?,
        proposed_version: attribute(reader, element, b"edition")?,
        current_vendor: None,
        proposed_vendor: None,
        repository_alias: attribute(reader, element, b"repository")?,
        download_bytes: 0,
        installed_size_before: 0,
        installed_size_after: 0,
    })
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
    fn parses_dry_run_summary_and_changes() {
        let xml = r#"<?xml version='1.0'?><stream><install-summary download-size="4096" space-usage-diff="2048" space-usage-installed="2048" space-usage-removed="0" packages-to-change="1" need-restart="false" need-reboot="true"><to-upgrade><solvable status="other-version" kind="package" name="firefox" edition="2" edition-old="1" arch="x86_64" repository="repo-oss"/></to-upgrade></install-summary></stream>"#;
        let result = parse_solver_xml(xml, vec!["repo-oss".into()], 8192).unwrap();
        assert!(result.successful);
        assert_eq!(result.download_bytes, 4096);
        assert_eq!(result.changes[0].action, PackageAction::Upgrade);
        assert!(result.reboot_required);
    }
}
