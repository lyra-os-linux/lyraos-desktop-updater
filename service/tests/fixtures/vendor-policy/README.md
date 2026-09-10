# Native zypper vendor fixtures

Generated on 2026-09-09 with `scripts/check-vendor-policy-native.py` on Leap 16.1.
The script builds scriptless RPMs, installs only their headers in a private
RPM database, signs a local RPM-MD repository, performs a signature-checking
refresh and captures real `zypper --xmlout dist-upgrade --dry-run --details`.
The original installed vendor remains unchanged after both dry-runs.

- `vendor-only`: 1-1 → 1-1, Lyra Fixture A → Lyra Fixture B; zypper emits both
  `to-reinstall` and `to-change-vendor` for one RPM change.
- `upgrade-vendor`: 1-1 → 2-1 with the same vendor transition; both `to-upgrade`
  and `to-change-vendor` describe one change.
- `summary.xml` keeps the native install-summary subtree inside a stream;
  transient refresh/progress messages are omitted. Vendor names do not appear
  in this zypper XML; `installed.xml` and the checked RPM-MD cache supply them.
- The installed XML includes a GPG key record with absent arch/vendor, which
  must not be treated as a package vendor or break enumeration.

Ordinary regression tests consume these committed fixtures without root or
network. They cover exact/directed allowlists, unknown identities, epoch/arch/
repository matching, ambiguous metadata, escaped vendors, checksum failures,
compressed metadata, plan/manifest drift and the shared staging/offline gate.

To regenerate in a disposable user namespace (normal user, never host root):

```sh
unshare --user --map-root-user python3 scripts/check-vendor-policy-native.py --output /tmp/lyra-vendor-native-result
LYRA_VENDOR_FIXTURES=/tmp/lyra-vendor-native-result cargo test -p lyra-upgrade-service --test vendor_policy --locked
```

Prerequisites: Python 3, rpmbuild/rpm, zypper, GnuPG, util-linux, gzip, xz, zstd.
Use a fresh output path. The script confines RPM and zypper state to its private
root and repository; it does not execute an update on the host. This qualifies
the solver/identity boundary, not a complete offline upgrade/recovery boot.
