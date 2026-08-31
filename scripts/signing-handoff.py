#!/usr/bin/env python3
"""Create a deterministic, non-secret handoff for release-manifest signing."""

from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import sys


ROOT = Path(__file__).resolve().parents[1]
SIGNING_FINGERPRINT = "01B63EEDBE6B079126A0116EFA7353A131ECEFEB"


def manifest_module():
    path = ROOT / "scripts/release-manifest.py"
    spec = importlib.util.spec_from_file_location("release_manifest_handoff", path)
    assert spec and spec.loader
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


def prepare(manifest_path: Path, output: Path) -> dict:
    if output.exists() or output.is_symlink():
        raise ValueError("output directory must not already exist")
    module = manifest_module()
    source = manifest_path.read_bytes()
    document = module.validate(json.loads(source))
    canonical = module.canonical_bytes(document)
    if source != canonical:
        raise ValueError("manifest input is not canonical")
    output.mkdir(mode=0o700, parents=True)
    manifest = output / "releases-v1.json"
    module.write_new(manifest, canonical)
    digest = hashlib.sha256(canonical).hexdigest()
    request = {
        "schema": 1,
        "status": "awaiting-signature",
        "manifest_filename": manifest.name,
        "signature_filename": "releases-v1.json.asc",
        "manifest_sha256": digest,
        "signing_key_fingerprint": SIGNING_FINGERPRINT,
        "signature_format": "OpenPGP ASCII-armored detached signature",
    }
    module.write_new(
        output / "signing-request.json",
        (json.dumps(request, indent=2, sort_keys=True) + "\n").encode(),
    )
    module.write_new(output / "SHA256SUMS", f"{digest}  releases-v1.json\n".encode())
    return request


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("manifest", type=Path)
    parser.add_argument("--output-dir", required=True, type=Path)
    args = parser.parse_args()
    try:
        request = prepare(args.manifest, args.output_dir)
    except (OSError, ValueError, json.JSONDecodeError) as error:
        parser.error(str(error))
    print(json.dumps(request, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
