# Package migration component qualification — 2026-09-27

[Structured results](result.json) pin the tested service, offline worker and
boot verifier binaries, and the seven guest logs. These are local 0.2.7 builds,
not an OBS RPM or the real portal migration.

Both scenarios fetched an HTTPS offer signed by an ephemeral guest-only key.
The production service refused unconfirmed Start, recomputed the exact plan,
downloaded the signed fixture RPM and created a real Snapper snapshot. A second
cold boot had no HTTPS server or network NIC. Altering signed JSON or cached RPM
bytes produced `NeedsRecovery` without changing installed package identity;
restoring the test fixture allowed the exact offline transaction.

The success scenario verified the next systemd boot, exact target inventory,
unchanged product identity/repositories and sequence 7→8. The recovery scenario
injected an unexpected package version after application. The boot verifier
reported `POST_BOOT_INVENTORY_FAILED`, then the service's explicit rollback
selected a real Snapper clone. A fourth cold boot verified clone/source identity
and original inventory, ending in `Completed`/`rollback-verified` with sequence 7.

The native harness additionally verifies cached application after deleting the
repository payload, idempotent repeat refusal and refusal of an identical RPM
edition that would require forced reinstallation. Optional language presence,
unexpected solver actions and partial application are covered by Rust tests.

## Reproduction

On a compatible openSUSE host with RPM build/signing tools, zypper, GnuPG,
QEMU, Btrfs/Snapper tools and the kernel modules available:

```sh
cargo build --locked -p lyra-upgrade-service -p lyra-upgrade-offline -p lyra-upgrade-verify --examples --bins
unshare --user --map-root-user python3 scripts/check-package-migration-native.py \
  --output /tmp/migration-native --adapter target/debug/examples/package-migration-native
python3 scripts/check-package-migration-vm.py \
  --kernel /boot/vmlinuz-6.12.0-160100.5-default \
  --modules-dir /usr/lib/modules/6.12.0-160100.5-default \
  --fixtures /tmp/migration-native/fixtures/upgrade-vendor \
  --binary-dir target/debug --log-dir /tmp/migration-success --scenario success --accel kvm
python3 scripts/check-package-migration-vm.py \
  --kernel /boot/vmlinuz-6.12.0-160100.5-default \
  --modules-dir /usr/lib/modules/6.12.0-160100.5-default \
  --fixtures /tmp/migration-native/fixtures/upgrade-vendor \
  --binary-dir target/debug --log-dir /tmp/migration-rollback --scenario rollback --accel kvm
```

Each output directory must be new. Guest disks and ephemeral keys are disposed
after the run; only logs and binary digests remain. The disk serial and kernel
marker are checked before formatting. No host disk, account database, NIC or
production signing credential is passed to QEMU.

## Limits

The service receives a fixture authenticated UID: this is not an interactive
Polkit test. Secure Boot is represented by a disabled probe, and the GRUB file is
a fixture; unexpected bootloader regeneration fails. No running desktop,
portal behavior, actual OBS RPM or ISO was qualified here. Production manifest
signing, staging artifact qualification, portal tests with/without translations,
promotion and the exact candidate ISO remain delivery gates.
