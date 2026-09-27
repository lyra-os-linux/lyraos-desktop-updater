# Signed package maintenance within one Lyra release

Updater 0.2.7 introduces `PackageMigration`: an explicit signed transaction that
keeps the complete Lyra product identity, including version and build ID. The
portal fix for Lyra 1.1 must use this route; it must not invent a newer Lyra
release just to pass the release-upgrade version check.

## Manifest and transaction

The optional `package_migration` array declares 1–64 unique package names, an
exact architecture, source/target RPM edition and vendor, repository alias,
target RPM SHA256, and `if_installed`. Example entry (not a complete or published manifest):

```json
{
  "name": "xdg-desktop-portal-gnome-lang",
  "architecture": "noarch",
  "from_version": "48.0-160100.2.1",
  "from_vendor": "SUSE LLC <https://www.suse.com/>",
  "to_version": "48.0-160100.2.1.lyra1.160101.1",
  "to_vendor": "obs://build.opensuse.org/home:rodrigosbrito",
  "repository_alias": "lyra-staging",
  "sha256": "ba84d6486f3b5907c86e0a38e17d93e939e0dbb674cd3ea4754deac256f50a28",
  "if_installed": true
}
```

At least one entry must be mandatory. Optional entries authorize upgrading an
installed package, never adding an absent one. Already migrated entries are
skipped; when all entries are current, planning returns
`MIGRATION_ALREADY_APPLIED`. Unknown source versions/vendors, mixed
architectures and ambiguous installed identities stop planning.
The already consumed manifest sequence produces the same explicit refusal;
lower sequences remain replay errors. Neither result permits another execution.

Every selected entry uses one declared repository. Vendor exceptions must be
scoped to names and directional vendor pairs present in this array. No removals
or independent lockstep groups are permitted: the pending array itself defines
the exact complete transaction. Source and target RPM editions must differ;
vendor-only replacement of an identical edition would require a forced reinstall
and is unsupported. Downgrades remain forbidden by both zypper and plan policy.

Planning, staging and offline application share the same exact
`zypper install --name --from ALIAS --no-recommends ... -- name.arch=edition`
selection. The complete solver result must match pending entries, rejecting
additional same-vendor updates, newly installed dependencies, removals,
omissions, architecture changes and unexpected versions. Missing, additional,
duplicate or altered RPM payloads fail digest verification after download and
again before offline application. Repository/RPM signature checks stay enabled.

The service preserves the signed document and detached signature byte for byte,
including documents without a trailing newline. The signed policy, confirmed
plan hash, installed inventory and exact identities are checked again before
execution. The root service recomputes the submitted plan after authorization.

## Execution and recovery

The UI presents a system correction for the current version, requests backup
acknowledgment and confirmation, and uses the existing authenticated service.
The transaction downloads, takes a Snapper snapshot, stages `/system-update`,
applies offline and verifies the next boot. It does not replace active repository
configuration. Non-boot package fixes do not invoke dracut/GRUB regeneration.

The post-application and post-boot inventories must match the exact expected
installed set. A successful boot alone advances the manifest replay sequence.
Failure preserves `NeedsRecovery`; explicit rollback verifies the restored RPM
inventory and snapshot identity even though the Lyra version remains 1.1.
Rollback leaves the replay sequence unchanged.

## Compatibility and qualification

This is an additive manifest-v1/plan-v3/protocol-v3/state-v1 capability gated by
`minimum_updater_version >= 0.2.7`. Older readers reject the unknown policy or
operation; they must never interpret it as an ordinary upgrade. The existing
`CheckRelease`, `PlanReleaseUpgrade` and `Start` requests carry the new operation.
Legacy manifests omit the new field and retain their canonical representation.
`null`, empty arrays and lower declared minimum versions are rejected.

Rust tests cover exact policy, optional packages, partial/repeated application,
confirmation binding, state/snapshot constraints, payload authentication,
signature-byte preservation and replay behavior. Python/JavaScript tests cover
producer validation and confirmation/UI behavior. Native harnesses are:

* `scripts/check-package-migration-native.py`: signed scriptless RPM fixtures,
  real zypper planning/download/cached application, identical-edition refusal.
* `scripts/check-package-migration-vm.py`: disposable Btrfs disk, HTTPS and
  ephemeral manifest key, production service/staging/offline/verifier, cold boots,
  tampered signature/payload rejection and explicit recovery scenario.

The VM uses an authenticated UID fixture, a disabled Secure Boot probe and a
GRUB file fixture. It does not qualify interactive Polkit, actual bootloader
regeneration, the portal RPMs, OBS builds or a candidate ISO. Those are separate
delivery gates. Do not promote the portal, change the image minimum or claim
parental protection from these component tests.

On 2026-09-27, 145 Rust tests, 35 Python/UI tests, strict Clippy and formatting
checks passed. The native RPM harness passed, and the final binaries passed
three cold boots for success and four for recovery. See
[results and reproduction](evidence/package-migration/README.md).
