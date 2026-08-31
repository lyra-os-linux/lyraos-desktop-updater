#!/usr/bin/env python3
"""Validate guest observations for a controlled upgrade and rollback rehearsal."""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import stat


MAX_INPUT = 1024 * 1024
TOP_FIELDS = {"schema", "status", "mode", "installation_uuid", "boot_id", "session", "release", "upgrade"}
RELEASE_FIELDS = {"id", "version_id", "edition", "architecture", "build_id"}
UPGRADE_FIELDS = {
    "package_version", "operation_id", "operation_state", "operation_sequence",
    "source_version", "target_version", "snapshot_recorded",
}


class ObservationError(ValueError):
    pass


def read_regular(path: Path) -> str:
    metadata = path.lstat()
    if (
        stat.S_ISLNK(metadata.st_mode)
        or not stat.S_ISREG(metadata.st_mode)
        or metadata.st_uid != os.getuid()
        or metadata.st_size > MAX_INPUT
    ):
        raise ObservationError(f"unsafe input: {path}")
    return path.read_text(encoding="utf-8")


def validate_observation(value: object, installation_uuid: str) -> dict:
    if not isinstance(value, dict) or set(value) != TOP_FIELDS:
        raise ObservationError("guest observation fields differ from schema 1")
    if (
        value["schema"] != 1
        or value["status"] != "observed"
        or value["mode"] != "guest-upgrade-state"
        or value["installation_uuid"] != installation_uuid
        or value["session"] != "installed"
    ):
        raise ObservationError("guest observation identity or mode is invalid")
    release, upgrade = value["release"], value["upgrade"]
    if not isinstance(release, dict) or set(release) != RELEASE_FIELDS:
        raise ObservationError("release observation fields differ from schema 1")
    if not isinstance(upgrade, dict) or set(upgrade) != UPGRADE_FIELDS:
        raise ObservationError("upgrade observation fields differ from schema 1")
    if release["id"] != "lyra-os" or release["edition"] != "desktop" or release["architecture"] != "x86_64":
        raise ObservationError("guest does not identify Lyra Desktop x86_64")
    if not isinstance(value["boot_id"], str) or not value["boot_id"]:
        raise ObservationError("boot ID is missing")
    return value


def aggregate(trace_path: Path, observations_path: Path, baseline: tuple[str, str], target: tuple[str, str]) -> dict:
    trace = json.loads(read_regular(trace_path))
    if (
        not isinstance(trace, dict)
        or trace.get("schema") != 1
        or trace.get("status") != "in-progress"
        or not isinstance(trace.get("installation_uuid"), str)
        or not isinstance(trace.get("launches"), list)
        or trace.get("qemu_launch_count") != len(trace["launches"])
    ):
        raise ObservationError("rehearsal trace is incomplete")
    installation_uuid = trace["installation_uuid"]
    values = []
    for line in read_regular(observations_path).splitlines():
        if line.strip():
            values.append(validate_observation(json.loads(line), installation_uuid))
    if len(values) < 3:
        raise ObservationError("baseline, target and rollback observations are required")

    expected = [baseline, target, baseline]
    selected = values[-3:]
    actual = [(item["release"]["version_id"], item["release"]["build_id"]) for item in selected]
    if actual != expected:
        raise ObservationError("observed release sequence is not baseline-target-baseline")
    if len({item["boot_id"] for item in selected}) != 3:
        raise ObservationError("each phase must come from a distinct boot")
    target_upgrade = selected[1]["upgrade"]
    if (
        target_upgrade["operation_state"] != "Completed"
        or target_upgrade["source_version"] != baseline[0]
        or target_upgrade["target_version"] != target[0]
        or target_upgrade["snapshot_recorded"] is not True
    ):
        raise ObservationError("target boot does not prove a completed snapshotted upgrade")
    installed_launches = sum(1 for launch in trace["launches"] if launch.get("mode") == "installed")
    if installed_launches < 3:
        raise ObservationError("trace does not contain three installed VM launches")
    return {
        "schema": 1,
        "status": "observed",
        "mode": "upgrade-rehearsal-observations",
        "phase": "rollback-observed",
        "installation_uuid": installation_uuid,
        "boot_ids": [item["boot_id"] for item in selected],
        "baseline": {"version": baseline[0], "build_id": baseline[1]},
        "target": {"version": target[0], "build_id": target[1]},
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--trace", required=True, type=Path)
    parser.add_argument("--observations", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--baseline-version", required=True)
    parser.add_argument("--baseline-build-id", required=True)
    parser.add_argument("--target-version", required=True)
    parser.add_argument("--target-build-id", required=True)
    args = parser.parse_args()
    if args.output.exists() or args.output.is_symlink():
        parser.error("output must not already exist")
    try:
        result = aggregate(
            args.trace,
            args.observations,
            (args.baseline_version, args.baseline_build_id),
            (args.target_version, args.target_build_id),
        )
    except (ObservationError, OSError, json.JSONDecodeError) as error:
        parser.error(str(error))
    descriptor = os.open(args.output, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(descriptor, "w", encoding="utf-8") as stream:
        json.dump(result, stream, indent=2, sort_keys=True)
        stream.write("\n")
    print(args.output)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
