//! Vendor identities from RPM headers and the exact RPM-MD cache used by zypper.
//! Call only after a successful, signature-checking refresh/solver invocation.
//! Repository aliases are selectors, never vendor identities.
use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::{self, BufRead, BufReader, Seek};
use std::path::{Component, Path};
use std::process::{Command, Stdio};

use lyra_upgrade_core::{PackageAction, SolverResult, valid_vendor};
use quick_xml::{NsReader, events::Event, name::ResolveResult};
use sha2::{Digest, Sha256, Sha512};

const COMMON: &str = "http://linux.duke.edu/metadata/common";
const RPM: &str = "http://linux.duke.edu/metadata/rpm";
const REPO: &str = "http://linux.duke.edu/metadata/repo";
const MAX_XML_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const RPM_QUERY: &str = "<package>%{NAME:xml}%{EPOCHNUM:xml}%{VERSION:xml}%{RELEASE:xml}%|ARCH?{%{ARCH:xml}}:{<string/>}|%|VENDOR?{%{VENDOR:xml}}:{<string/>}|</package>\n";

type Identity = (String, String, String); // name, edition (including epoch), arch
pub type VendorResult<T> = Result<T, VendorMetadataError>;

#[derive(Debug)]
pub enum VendorMetadataError {
    Io(io::Error),
    Invalid(&'static str),
    MissingIdentity { package: String },
}
impl From<io::Error> for VendorMetadataError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

/// Populate every required side atomically. No guessed, stale or partial vendors.
pub fn enrich_solver_vendors(solver: &mut SolverResult, raw_cache: &Path) -> VendorResult<()> {
    let output = Command::new("rpm")
        .args(["-qa", "--queryformat", RPM_QUERY])
        .env_clear()
        .env("PATH", "/usr/sbin:/usr/bin:/sbin:/bin")
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .output()?;
    if !output.status.success() {
        return Err(VendorMetadataError::Invalid("RPM query failed"));
    }
    let mut xml = b"<rpmdb>".to_vec();
    xml.extend(output.stdout);
    xml.extend(b"</rpmdb>");
    enrich_from_rpm_xml(solver, raw_cache, &xml)
}

/// Also used by the native qualification harness with a private RPM database.
pub fn enrich_from_rpm_xml(
    solver: &mut SolverResult,
    raw_cache: &Path,
    rpm_xml: &[u8],
) -> VendorResult<()> {
    let installed = installed_vendors(rpm_xml)?;
    let mut candidate_sets: BTreeMap<String, BTreeSet<Identity>> = BTreeMap::new();
    for change in &solver.changes {
        if change.action != PackageAction::Remove {
            let alias = change
                .repository_alias
                .as_deref()
                .filter(|alias| {
                    valid_alias(alias)
                        && solver
                            .metadata_valid_repositories
                            .iter()
                            .any(|item| item == alias)
                })
                .ok_or(VendorMetadataError::Invalid("unverified repository alias"))?;
            let version = change
                .proposed_version
                .as_ref()
                .ok_or(VendorMetadataError::Invalid("missing candidate version"))?;
            candidate_sets.entry(alias.to_owned()).or_default().insert((
                change.name.clone(),
                version.clone(),
                change.architecture.clone(),
            ));
        }
    }
    let mut candidates = BTreeMap::new();
    for (alias, wanted) in candidate_sets {
        let root = raw_cache.canonicalize()?;
        let repo = root.join(&alias).canonicalize()?;
        if !repo.starts_with(&root) {
            return Err(VendorMetadataError::Invalid(
                "repository cache escapes root",
            ));
        }
        candidates.insert(alias, repository_vendors(&repo, &wanted)?);
    }
    let mut changes = solver.changes.clone();
    for change in &mut changes {
        let identity = |version: &Option<String>| -> VendorResult<Identity> {
            Ok((
                change.name.clone(),
                version
                    .clone()
                    .ok_or(VendorMetadataError::Invalid("missing package version"))?,
                change.architecture.clone(),
            ))
        };
        change.current_vendor = if change.action == PackageAction::Install {
            None
        } else {
            Some(lookup(&installed, &identity(&change.current_version)?)?)
        };
        change.proposed_vendor = if change.action == PackageAction::Remove {
            None
        } else {
            let catalog = candidates
                .get(change.repository_alias.as_ref().unwrap())
                .ok_or(VendorMetadataError::Invalid("missing repository catalog"))?;
            Some(lookup(catalog, &identity(&change.proposed_version)?)?)
        };
    }
    solver.changes = changes;
    Ok(())
}

fn valid_alias(value: &str) -> bool {
    !value.is_empty()
        && value != "."
        && value != ".."
        && value.len() <= 128
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"._:-".contains(&c))
}

// None also records duplicate identities; even equal vendors cannot disambiguate
// two RPM headers with the same NEVRA.
type Catalog = BTreeMap<Identity, Option<String>>;
fn lookup(catalog: &Catalog, identity: &Identity) -> VendorResult<String> {
    catalog
        .get(identity)
        .and_then(Option::as_ref)
        .filter(|v| valid_vendor(v))
        .cloned()
        .ok_or_else(|| VendorMetadataError::MissingIdentity {
            package: identity.0.clone(),
        })
}
fn insert(catalog: &mut Catalog, key: Identity, vendor: String) {
    if catalog.insert(key.clone(), Some(vendor)).is_some() {
        catalog.insert(key, None);
    }
}
fn edition(epoch: &str, version: &str, release: &str) -> VendorResult<String> {
    let epoch: u64 = epoch
        .parse()
        .map_err(|_| VendorMetadataError::Invalid("invalid RPM epoch"))?;
    if version.is_empty() || release.is_empty() {
        return Err(VendorMetadataError::Invalid("missing RPM version/release"));
    }
    Ok(if epoch == 0 {
        format!("{version}-{release}")
    } else {
        format!("{epoch}:{version}-{release}")
    })
}

fn installed_vendors(xml: &[u8]) -> VendorResult<Catalog> {
    let mut catalog = Catalog::new();
    let mut fields = Vec::new();
    walk(xml, "", "rpmdb", |parents, node| {
        if parents.len() == 2 && parents[1].is("", "package") && node.ns.is_empty() {
            if !matches!(node.name.as_str(), "string" | "integer") {
                return Err(VendorMetadataError::Invalid("invalid RPM field"));
            }
            fields.push(node.text.clone());
        }
        if parents.len() == 1 && node.is("", "package") {
            if fields.len() != 6 {
                return Err(VendorMetadataError::Invalid("incomplete RPM header"));
            }
            insert(
                &mut catalog,
                (
                    fields[0].clone(),
                    edition(&fields[1], &fields[2], &fields[3])?,
                    fields[4].clone(),
                ),
                fields[5].clone(),
            );
            fields.clear();
        }
        Ok(())
    })?;
    Ok(catalog)
}

#[derive(Default)]
struct Primary {
    href: String,
    algorithm: String,
    checksum: String,
}
fn primary_record(xml: &[u8]) -> VendorResult<Primary> {
    let mut records = Vec::new();
    let mut record = Primary::default();
    walk(xml, REPO, "repomd", |parents, node| {
        if parents.len() == 2
            && parents[1].is(REPO, "data")
            && parents[1].attr("type")? == "primary"
        {
            if node.is(REPO, "location") {
                if !record.href.is_empty() {
                    return Err(VendorMetadataError::Invalid("duplicate primary location"));
                }
                record.href = node.attr("href")?.to_owned();
            } else if node.is(REPO, "checksum") {
                if !record.checksum.is_empty() {
                    return Err(VendorMetadataError::Invalid("duplicate primary checksum"));
                }
                record.algorithm = node.attr("type")?.to_owned();
                record.checksum = node.text.clone();
            }
        }
        if parents.len() == 1 && node.is(REPO, "data") && node.attr("type")? == "primary" {
            records.push(std::mem::take(&mut record));
        }
        Ok(())
    })?;
    if records.len() != 1 {
        return Err(VendorMetadataError::Invalid(
            "missing/ambiguous primary metadata",
        ));
    }
    Ok(records.remove(0))
}

fn repository_vendors(repo: &Path, wanted: &BTreeSet<Identity>) -> VendorResult<Catalog> {
    let repomd_path = repo.join("repodata/repomd.xml").canonicalize()?;
    if !repomd_path.starts_with(repo) {
        return Err(VendorMetadataError::Invalid("repomd escapes repository"));
    }
    let repomd = std::fs::read(&repomd_path)?;
    let primary = primary_record(&repomd)?;
    let relative = Path::new(&primary.href);
    if primary.href.is_empty()
        || relative
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err(VendorMetadataError::Invalid("unsafe primary location"));
    }
    let path = repo.join(relative).canonicalize()?;
    if !path.starts_with(repo) {
        return Err(VendorMetadataError::Invalid(
            "primary location escapes repository",
        ));
    }
    let mut file = File::open(&path)?;
    verify_checksum(&mut file, &primary)?;
    file.rewind()?;
    let result = match path.extension().and_then(|value| value.to_str()) {
        Some("xml") => primary_vendors(BufReader::new(file), wanted),
        Some(extension @ ("gz" | "xz" | "zst")) => {
            let program = match extension {
                "gz" => "gzip",
                "xz" => "xz",
                _ => "zstd",
            };
            // Feed the already verified descriptor, never reopen a mutable path.
            let mut child = Command::new(program)
                .arg("-dc")
                .stdin(file)
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .env_clear()
                .env("PATH", "/usr/sbin:/usr/bin:/sbin:/bin")
                .spawn()?;
            let result = primary_vendors(BufReader::new(child.stdout.take().unwrap()), wanted);
            if result.is_err() {
                let _ = child.kill();
            }
            let status = child.wait()?;
            if !status.success() {
                return Err(VendorMetadataError::Invalid("primary decompression failed"));
            }
            result
        }
        _ => Err(VendorMetadataError::Invalid(
            "unsupported primary compression",
        )),
    }?;
    if std::fs::read(&repomd_path)? != repomd {
        return Err(VendorMetadataError::Invalid(
            "metadata changed during lookup",
        ));
    }
    Ok(result)
}

fn verify_checksum(file: &mut File, primary: &Primary) -> VendorResult<()> {
    let actual = match primary.algorithm.as_str() {
        "sha256" => {
            let mut hash = Sha256::new();
            io::copy(file, &mut hash)?;
            format!("{:x}", hash.finalize())
        }
        "sha512" => {
            let mut hash = Sha512::new();
            io::copy(file, &mut hash)?;
            format!("{:x}", hash.finalize())
        }
        _ => return Err(VendorMetadataError::Invalid("unsupported primary checksum")),
    };
    if actual != primary.checksum {
        return Err(VendorMetadataError::Invalid("primary checksum mismatch"));
    }
    Ok(())
}

fn primary_vendors(input: impl BufRead, wanted: &BTreeSet<Identity>) -> VendorResult<Catalog> {
    let mut catalog = Catalog::new();
    let mut fields: BTreeMap<&str, String> = BTreeMap::new();
    walk(input, COMMON, "metadata", |parents, node| {
        let field = if parents.len() == 2 && parents[1].is(COMMON, "package") {
            match (node.ns.as_str(), node.name.as_str()) {
                (COMMON, "name") => Some(("name", node.text.clone())),
                (COMMON, "arch") => Some(("arch", node.text.clone())),
                (COMMON, "version") => Some((
                    "edition",
                    edition(node.attr("epoch")?, node.attr("ver")?, node.attr("rel")?)?,
                )),
                _ => None,
            }
        } else if parents.len() == 3
            && parents[1].is(COMMON, "package")
            && parents[2].is(COMMON, "format")
            && node.is(RPM, "vendor")
        {
            Some(("vendor", node.text.clone()))
        } else {
            None
        };
        if let Some((key, value)) = field
            && fields.insert(key, value).is_some()
        {
            return Err(VendorMetadataError::Invalid(
                "duplicate primary package field",
            ));
        }
        if parents.len() == 1 && node.is(COMMON, "package") {
            let get = |key| {
                fields
                    .get(key)
                    .cloned()
                    .ok_or(VendorMetadataError::Invalid("incomplete primary package"))
            };
            let identity = (get("name")?, get("edition")?, get("arch")?);
            if wanted.contains(&identity) {
                if node.attr("type")? != "rpm" {
                    return Err(VendorMetadataError::Invalid("non-RPM candidate"));
                }
                insert(
                    &mut catalog,
                    identity,
                    fields.get("vendor").cloned().unwrap_or_default(),
                );
            }
            fields.clear();
        }
        Ok(())
    })?;
    Ok(catalog)
}

struct Node {
    ns: String,
    name: String,
    attrs: BTreeMap<String, String>,
    text: String,
}
impl Node {
    fn is(&self, ns: &str, name: &str) -> bool {
        self.ns == ns && self.name == name
    }
    fn attr(&self, name: &str) -> VendorResult<&str> {
        self.attrs
            .get(name)
            .map(String::as_str)
            .ok_or(VendorMetadataError::Invalid("missing XML attribute"))
    }
    fn append(&mut self, text: &str) -> VendorResult<()> {
        if matches!(
            self.name.as_str(),
            "name" | "arch" | "vendor" | "checksum" | "string" | "integer"
        ) {
            if self.text.len() + text.len() > 4096 {
                return Err(VendorMetadataError::Invalid("oversized identity field"));
            }
            self.text.push_str(text);
        }
        Ok(())
    }
}

// Streaming reader: bounded depth/identity fields, expanded namespace names,
// escaped text and CDATA, no DTD/entity expansion or human-language output.
fn walk(
    input: impl BufRead,
    root_ns: &str,
    root_name: &str,
    mut closed: impl FnMut(&[Node], &Node) -> VendorResult<()>,
) -> VendorResult<()> {
    let mut reader = NsReader::from_reader(input.take(MAX_XML_BYTES));
    let mut stack: Vec<Node> = Vec::new();
    let mut buffer = Vec::new();
    let mut roots = 0;
    loop {
        let (ns, event) = reader
            .read_resolved_event_into(&mut buffer)
            .map_err(|_| VendorMetadataError::Invalid("malformed XML"))?;
        match event {
            Event::Start(ref element) | Event::Empty(ref element) => {
                let empty = matches!(event, Event::Empty(_));
                let ns = match ns {
                    ResolveResult::Bound(ns) => String::from_utf8_lossy(ns.as_ref()).into_owned(),
                    ResolveResult::Unbound => String::new(),
                    _ => return Err(VendorMetadataError::Invalid("unbound XML namespace")),
                };
                let mut node = Node {
                    ns,
                    name: String::from_utf8_lossy(element.local_name().as_ref()).into_owned(),
                    attrs: BTreeMap::new(),
                    text: String::new(),
                };
                for attribute in element.attributes() {
                    let attribute = attribute
                        .map_err(|_| VendorMetadataError::Invalid("invalid XML attribute"))?;
                    let value = attribute
                        .decode_and_unescape_value(reader.decoder())
                        .map_err(|_| VendorMetadataError::Invalid("invalid XML attribute value"))?
                        .into_owned();
                    if node
                        .attrs
                        .insert(
                            String::from_utf8_lossy(attribute.key.as_ref()).into_owned(),
                            value,
                        )
                        .is_some()
                    {
                        return Err(VendorMetadataError::Invalid("duplicate XML attribute"));
                    }
                }
                if stack.is_empty() {
                    roots += 1;
                    if roots != 1 || !node.is(root_ns, root_name) {
                        return Err(VendorMetadataError::Invalid("invalid XML root"));
                    }
                }
                if stack.len() >= 32 {
                    return Err(VendorMetadataError::Invalid("excessive XML depth"));
                }
                if empty {
                    closed(&stack, &node)?;
                } else {
                    stack.push(node);
                }
            }
            Event::End(_) => {
                let node = stack
                    .pop()
                    .ok_or(VendorMetadataError::Invalid("unbalanced XML"))?;
                closed(&stack, &node)?;
            }
            Event::Text(text) => {
                let text = text
                    .xml_content()
                    .map_err(|_| VendorMetadataError::Invalid("invalid XML text"))?;
                if let Some(node) = stack.last_mut() {
                    node.append(&text)?;
                } else if !text.trim().is_empty() {
                    return Err(VendorMetadataError::Invalid("text outside XML root"));
                }
            }
            Event::CData(text) => {
                if let Some(node) = stack.last_mut() {
                    node.append(
                        &text
                            .decode()
                            .map_err(|_| VendorMetadataError::Invalid("invalid CDATA"))?,
                    )?;
                }
            }
            Event::GeneralRef(reference) => {
                let name = reference
                    .decode()
                    .map_err(|_| VendorMetadataError::Invalid("invalid XML reference"))?;
                let resolved = quick_xml::escape::unescape(&format!("&{name};"))
                    .map_err(|_| VendorMetadataError::Invalid("unknown XML entity"))?
                    .into_owned();
                if let Some(node) = stack.last_mut() {
                    node.append(&resolved)?;
                }
            }
            Event::DocType(_) => return Err(VendorMetadataError::Invalid("DTD not allowed")),
            Event::Eof => break,
            _ => {}
        }
        buffer.clear();
    }
    if roots != 1 || !stack.is_empty() || reader.buffer_position() >= MAX_XML_BYTES {
        return Err(VendorMetadataError::Invalid("truncated XML"));
    }
    Ok(())
}
