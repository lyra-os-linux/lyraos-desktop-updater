//! Read-only identity checks for an explicitly requested Btrfs rollback.
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SubvolumeIdentity {
    pub id: u64,
    pub uuid: String,
    pub parent_uuid: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RollbackGoal {
    pub source_snapshot_number: u64,
    pub source_snapshot: SubvolumeIdentity,
    /// None is a persisted intent whose boot selection has not been confirmed.
    pub boot_snapshot: Option<SubvolumeIdentity>,
}

impl RollbackGoal {
    pub fn valid(&self, snapshot_number: Option<u64>) -> bool {
        self.source_snapshot_number > 0
            && snapshot_number == Some(self.source_snapshot_number)
            && self.source_snapshot.valid()
            && self.boot_snapshot.as_ref().is_none_or(|boot| {
                boot.valid()
                    && boot.id != self.source_snapshot.id
                    && boot.uuid != self.source_snapshot.uuid
                    && boot.parent_uuid.as_deref() == Some(&self.source_snapshot.uuid)
            })
    }
}

impl SubvolumeIdentity {
    fn valid(&self) -> bool {
        self.id > 5 && valid_uuid(&self.uuid) && self.parent_uuid.as_deref().is_none_or(valid_uuid)
    }
}

fn valid_uuid(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(i, c)| {
            if [8, 13, 18, 23].contains(&i) {
                c == b'-'
            } else {
                c.is_ascii_hexdigit() && !c.is_ascii_uppercase()
            }
        })
}

pub fn snapshot_path(number: u64) -> PathBuf {
    Path::new("/.snapshots")
        .join(number.to_string())
        .join("snapshot")
}

fn btrfs(arguments: &[&str], path: &Path) -> Result<String, &'static str> {
    let output = Command::new("btrfs")
        .args(arguments)
        .arg(path)
        .env_clear()
        .env("PATH", "/usr/sbin:/usr/bin:/sbin:/bin")
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .output()
        .map_err(|_| "BTRFS_PROBE_FAILED")?;
    if !output.status.success() {
        return Err("BTRFS_PROBE_FAILED");
    }
    String::from_utf8(output.stdout).map_err(|_| "BTRFS_PROBE_FAILED")
}

pub fn inspect_subvolume(path: &Path) -> Result<SubvolumeIdentity, &'static str> {
    parse_subvolume(&btrfs(&["subvolume", "show", "--raw"], path)?)
}

pub fn containing_subvolume_id(path: &Path) -> Result<u64, &'static str> {
    btrfs(&["inspect-internal", "rootid"], path)?
        .trim()
        .parse()
        .map_err(|_| "BTRFS_PROBE_FAILED")
}

pub fn default_subvolume_id() -> Result<u64, &'static str> {
    let output = btrfs(&["subvolume", "get-default"], Path::new("/"))?;
    let mut fields = output.split_whitespace();
    if fields.next() != Some("ID") {
        return Err("BTRFS_PROBE_FAILED");
    }
    fields
        .next()
        .and_then(|id| id.parse().ok())
        .ok_or("BTRFS_PROBE_FAILED")
}

pub fn parse_subvolume(output: &str) -> Result<SubvolumeIdentity, &'static str> {
    let mut fields = std::collections::BTreeMap::new();
    for line in output.lines() {
        if let Some((key, value)) = line.trim().split_once(':')
            && matches!(key, "UUID" | "Parent UUID" | "Subvolume ID")
            && fields.insert(key, value.trim()).is_some()
        {
            return Err("BTRFS_IDENTITY_INVALID");
        }
    }
    let uuid = fields
        .get("UUID")
        .ok_or("BTRFS_IDENTITY_INVALID")?
        .to_string();
    let parent = *fields.get("Parent UUID").ok_or("BTRFS_IDENTITY_INVALID")?;
    let id = fields
        .get("Subvolume ID")
        .and_then(|id| id.parse().ok())
        .ok_or("BTRFS_IDENTITY_INVALID")?;
    let value = SubvolumeIdentity {
        id,
        uuid,
        parent_uuid: (parent != "-").then(|| parent.to_string()),
    };
    if !value.valid() {
        return Err("BTRFS_IDENTITY_INVALID");
    }
    Ok(value)
}

pub fn verify_rollback_boot(goal: &RollbackGoal) -> Result<(), &'static str> {
    let expected = goal
        .boot_snapshot
        .as_ref()
        .ok_or("ROLLBACK_INTENT_INCOMPLETE")?;
    if inspect_subvolume(&snapshot_path(goal.source_snapshot_number))? != goal.source_snapshot
        || inspect_subvolume(Path::new("/"))? != *expected
        || default_subvolume_id()? != expected.id
    {
        return Err("ROLLBACK_BOOT_IDENTITY_MISMATCH");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    const UUID: &str = "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee";
    #[test]
    fn native_identity_parser_rejects_ambiguous_and_malformed_output() {
        let input = format!(
            "@/.snapshots/2/snapshot\nUUID: {UUID}\nParent UUID: -\nSubvolume ID: 258\nFlags: readonly\n"
        );
        assert_eq!(
            parse_subvolume(&input).unwrap(),
            SubvolumeIdentity {
                id: 258,
                uuid: UUID.into(),
                parent_uuid: None
            }
        );
        for broken in [
            input.replace("258", "5"),
            input.replace(UUID, "not-a-uuid"),
            input.replace("Parent UUID: -\n", ""),
            format!("{input}UUID: {UUID}\n"),
        ] {
            assert!(parse_subvolume(&broken).is_err());
        }
    }
    #[test]
    fn rollback_requires_a_distinct_clone_of_the_exact_source() {
        let mut goal = RollbackGoal {
            source_snapshot_number: 2,
            source_snapshot: SubvolumeIdentity {
                id: 258,
                uuid: UUID.into(),
                parent_uuid: None,
            },
            boot_snapshot: None,
        };
        assert!(goal.valid(Some(2)));
        assert!(!goal.valid(Some(3)));
        goal.boot_snapshot = Some(SubvolumeIdentity {
            id: 260,
            uuid: "bbbbbbbb-bbbb-cccc-dddd-eeeeeeeeeeee".into(),
            parent_uuid: Some(UUID.into()),
        });
        assert!(goal.valid(Some(2)));
        goal.boot_snapshot.as_mut().unwrap().parent_uuid = None;
        assert!(!goal.valid(Some(2)));
        goal.boot_snapshot = Some(goal.source_snapshot.clone());
        assert!(!goal.valid(Some(2)));
    }
}
