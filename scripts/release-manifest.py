#!/usr/bin/env python3
"""Validate, canonicalize and optionally sign a Lyra release-upgrade manifest."""

from __future__ import annotations

import argparse
import datetime as dt
import json
import os
from pathlib import Path
import re
import subprocess
from urllib.parse import urlsplit


FIELDS = {
    "schema_version",
    "sequence",
    "status",
    "valid_from",
    "valid_until",
    "source",
    "target",
    "minimum_updater_version",
    "minimum_free_space_bytes",
    "repositories",
    "allowed_removals",
    "allowed_vendor_transitions",
    "lockstep_packages",
}
IDENTITY_FIELDS = {"version", "edition", "architecture", "build_id"}
REPOSITORY_FIELDS = {
    "alias",
    "base_url",
    "signing_key_url",
    "signing_key_fingerprint",
    "priority",
}
PACKAGE = re.compile(r"^[A-Za-z0-9+._-]+$")
ALIAS = re.compile(r"^[A-Za-z0-9._:-]+$")
FINGERPRINT = re.compile(r"^[0-9A-F]{40}$")
VERSION = re.compile(r"^[0-9]+\.[0-9]+(?:\.[0-9]+)?(?:-[A-Za-z0-9][A-Za-z0-9.-]*)?$")
UPDATER_VERSION = re.compile(r"^[0-9]+\.[0-9]+(?:\.[0-9]+)?$")


class ManifestError(ValueError):
    pass


def require_exact_fields(value: object, expected: set[str], label: str) -> dict:
    if not isinstance(value, dict) or set(value) != expected:
        raise ManifestError(f"{label} fields differ from schema v1")
    return value


def timestamp(value: object, label: str) -> dt.datetime:
    if not isinstance(value, str):
        raise ManifestError(f"{label} must be an RFC3339 timestamp")
    try:
        parsed = dt.datetime.fromisoformat(value.replace("Z", "+00:00"))
    except ValueError as error:
        raise ManifestError(f"{label} must be an RFC3339 timestamp") from error
    if parsed.tzinfo is None:
        raise ManifestError(f"{label} must include a timezone")
    return parsed


def https_url(value: object, label: str) -> None:
    if not isinstance(value, str):
        raise ManifestError(f"{label} must be HTTPS")
    parsed = urlsplit(value)
    if (
        parsed.scheme != "https"
        or not parsed.hostname
        or parsed.username is not None
        or parsed.password is not None
        or parsed.query
        or parsed.fragment
    ):
        raise ManifestError(f"{label} must be an HTTPS URL without credentials or query")


def validate_identity(value: object, label: str) -> dict:
    identity = require_exact_fields(value, IDENTITY_FIELDS, label)
    if not isinstance(identity["version"], str) or not VERSION.fullmatch(identity["version"]):
        raise ManifestError(f"{label}.version is invalid")
    if identity["edition"] != "desktop" or identity["architecture"] != "x86_64":
        raise ManifestError(f"{label} must identify Desktop x86_64")
    if not isinstance(identity["build_id"], str) or not identity["build_id"].strip():
        raise ManifestError(f"{label}.build_id is empty")
    return identity


def version_base(value: str) -> tuple[int, int, int]:
    numeric = value.split("-", 1)[0].split(".")
    numeric.extend(["0"] * (3 - len(numeric)))
    return tuple(int(component) for component in numeric)


def legacy_source(value: str) -> bool:
    return value in {"27.02", "27.06", "28.02"} or value.startswith(("2026.08", "27.02-"))


def validate(document: object) -> dict:
    manifest = require_exact_fields(document, FIELDS, "manifest")
    if manifest["schema_version"] != 1:
        raise ManifestError("unsupported schema_version")
    if isinstance(manifest["sequence"], bool) or not isinstance(manifest["sequence"], int) or manifest["sequence"] < 1:
        raise ManifestError("sequence must be a positive integer")
    if manifest["status"] not in {"testing", "available", "paused", "withdrawn"}:
        raise ManifestError("invalid status")
    valid_from = timestamp(manifest["valid_from"], "valid_from")
    valid_until = timestamp(manifest["valid_until"], "valid_until")
    if valid_from >= valid_until:
        raise ManifestError("valid_until must be later than valid_from")
    source = validate_identity(manifest["source"], "source")
    target = validate_identity(manifest["target"], "target")
    if source == target:
        raise ManifestError("source and target must differ")
    if legacy_source(target["version"]) or (
        not legacy_source(source["version"])
        and version_base(target["version"]) <= version_base(source["version"])
    ):
        raise ManifestError("target version must be newer than source version")
    if not isinstance(manifest["minimum_updater_version"], str) or not UPDATER_VERSION.fullmatch(manifest["minimum_updater_version"]):
        raise ManifestError("minimum_updater_version is invalid")
    minimum_space = manifest["minimum_free_space_bytes"]
    if isinstance(minimum_space, bool) or not isinstance(minimum_space, int) or minimum_space < 1:
        raise ManifestError("minimum_free_space_bytes must be positive")

    repositories = manifest["repositories"]
    if not isinstance(repositories, list) or not repositories:
        raise ManifestError("repositories must not be empty")
    aliases: set[str] = set()
    for index, item in enumerate(repositories):
        repository = require_exact_fields(item, REPOSITORY_FIELDS, f"repositories[{index}]")
        alias = repository["alias"]
        if not isinstance(alias, str) or not ALIAS.fullmatch(alias) or alias in aliases:
            raise ManifestError("repository aliases must be valid and unique")
        aliases.add(alias)
        https_url(repository["base_url"], f"repositories[{index}].base_url")
        https_url(repository["signing_key_url"], f"repositories[{index}].signing_key_url")
        fingerprint = repository["signing_key_fingerprint"]
        if not isinstance(fingerprint, str) or not FINGERPRINT.fullmatch(fingerprint):
            raise ManifestError("repository fingerprint must contain 40 uppercase hexadecimal digits")
        priority = repository["priority"]
        if isinstance(priority, bool) or not isinstance(priority, int) or not 1 <= priority <= 200:
            raise ManifestError("repository priority must be between 1 and 200")

    removals = manifest["allowed_removals"]
    if not isinstance(removals, list) or any(not isinstance(name, str) or not PACKAGE.fullmatch(name) for name in removals):
        raise ManifestError("allowed_removals contains an invalid package")
    transitions = manifest["allowed_vendor_transitions"]
    if not isinstance(transitions, list):
        raise ManifestError("allowed_vendor_transitions must be an array")
    for transition in transitions:
        transition = require_exact_fields(transition, {"from", "to"}, "vendor transition")
        if any(not isinstance(transition[key], str) or not transition[key] for key in ("from", "to")):
            raise ManifestError("vendor transition values must not be empty")
    groups = manifest["lockstep_packages"]
    if not isinstance(groups, list):
        raise ManifestError("lockstep_packages must be an array")
    for group in groups:
        if not isinstance(group, list) or len(group) < 2 or len(set(group)) != len(group):
            raise ManifestError("lockstep groups require at least two unique packages")
        if any(not isinstance(name, str) or not PACKAGE.fullmatch(name) for name in group):
            raise ManifestError("lockstep group contains an invalid package")
    return manifest


def canonical_bytes(document: dict) -> bytes:
    return (json.dumps(document, indent=2, sort_keys=True, ensure_ascii=False) + "\n").encode()


def write_new(path: Path, payload: bytes) -> None:
    parent = path.parent
    if parent.is_symlink() or not parent.is_dir() or parent.stat().st_uid != os.getuid():
        raise ManifestError(f"unsafe output directory: {parent}")
    if path.exists() or path.is_symlink():
        raise ManifestError(f"refusing to replace {path}")
    descriptor = os.open(
        path,
        os.O_WRONLY | os.O_CREAT | os.O_EXCL | getattr(os, "O_NOFOLLOW", 0),
        0o644,
    )
    with os.fdopen(descriptor, "wb") as stream:
        stream.write(payload)
        stream.flush()
        os.fsync(stream.fileno())


def sign(path: Path, signature: Path, fingerprint: str) -> None:
    if not FINGERPRINT.fullmatch(fingerprint):
        raise ManifestError("signing key must be a full uppercase fingerprint")
    if signature.exists() or signature.is_symlink():
        raise ManifestError(f"refusing to replace {signature}")
    subprocess.run(
        [
            "gpg",
            "--batch",
            "--yes",
            "--armor",
            "--detach-sign",
            "--local-user",
            fingerprint,
            "--output",
            str(signature),
            "--",
            str(path),
        ],
        check=True,
    )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("input", type=Path)
    parser.add_argument("--output-dir", required=True, type=Path)
    parser.add_argument("--signing-key", help="full GPG fingerprint; omit for validation only")
    args = parser.parse_args()
    document = validate(json.loads(args.input.read_text(encoding="utf-8")))
    args.output_dir.mkdir(mode=0o755, parents=True, exist_ok=True)
    output = args.output_dir / "releases-v1.json"
    write_new(output, canonical_bytes(document))
    if args.signing_key:
        sign(output, output.with_suffix(".json.asc"), args.signing_key)
    print(output)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
