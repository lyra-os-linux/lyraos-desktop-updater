#!/usr/bin/env bash
# Build the reproducible sources consumed by the lyra-upgrade OBS package.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$(readlink -f "$0")")" && pwd)"
UPGRADE_DIR="$(dirname "$SCRIPT_DIR")"
REPO_ROOT="$(dirname "$UPGRADE_DIR")"
OUTPUT_DIR="${1:-$SCRIPT_DIR/output}"

for command in cargo git gpg sha256sum tar zstd; do
  if ! command -v "$command" >/dev/null 2>&1; then
    echo "required command not found: $command" >&2
    exit 1
  fi
done

if [ -n "$(git -C "$REPO_ROOT" status --porcelain --untracked-files=normal)" ]; then
  echo "OBS sources require a clean committed working tree" >&2
  exit 1
fi

VERSION="$(awk -F '"' '/^version = / { print $2; exit }' "$UPGRADE_DIR/Cargo.toml")"
if [[ ! "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  echo "could not read the lyra-upgrade semantic version" >&2
  exit 1
fi

COMMIT="$(git -C "$REPO_ROOT" rev-parse HEAD)"
SOURCE_EPOCH="$(git -C "$REPO_ROOT" show -s --format=%ct "$COMMIT")"
PREFIX="lyra-upgrade-$VERSION"
TEMPORARY="$(mktemp -d /tmp/lyra-upgrade-source.XXXXXX)"

cleanup() {
  case "$TEMPORARY" in
    /tmp/lyra-upgrade-source.*) rm -rf -- "$TEMPORARY" ;;
    *) echo "refusing to remove unexpected temporary path: $TEMPORARY" >&2 ;;
  esac
}
trap cleanup EXIT

mkdir -p "$OUTPUT_DIR" "$TEMPORARY/source/$PREFIX" "$TEMPORARY/vendor-layer"
git -C "$REPO_ROOT" archive --format=tar "$COMMIT:upgrade" |
  tar -xf - -C "$TEMPORARY/source/$PREFIX"
git -C "$REPO_ROOT" show "$COMMIT:LICENSE" >"$TEMPORARY/source/$PREFIX/LICENSE"

make_archive() {
  local source_dir="$1"
  local member="$2"
  local destination="$3"
  local temporary_archive="$destination.new"
  rm -f -- "$temporary_archive"
  tar \
    --sort=name \
    --mtime="@$SOURCE_EPOCH" \
    --owner=0 \
    --group=0 \
    --numeric-owner \
    --pax-option=delete=atime,delete=ctime \
    -C "$source_dir" \
    -cf - "$member" |
    # Level 15 remains compact while avoiding the memory/time spike of -19
    # on the large Tauri vendor tree. One thread keeps output reproducible.
    zstd --quiet --threads=1 -15 -o "$temporary_archive"
  mv -f -- "$temporary_archive" "$destination"
}

SOURCE_ARCHIVE="$OUTPUT_DIR/$PREFIX.tar.zst"
make_archive "$TEMPORARY/source" "$PREFIX" "$SOURCE_ARCHIVE"

mkdir -p "$TEMPORARY/vendor-layer/.cargo"
(
  cd "$TEMPORARY/source/$PREFIX"
  cargo vendor --locked "$TEMPORARY/vendor-layer/vendor" \
    >"$TEMPORARY/vendor-layer/.cargo/config.toml"
)
VENDOR_ARCHIVE="$OUTPUT_DIR/vendor.tar.zst"
make_archive "$TEMPORARY/vendor-layer" . "$VENDOR_ARCHIVE"

LOCK_SHA256="$(sha256sum "$UPGRADE_DIR/Cargo.lock" | awk '{print $1}')"
cat >"$OUTPUT_DIR/build-source.txt.new" <<EOF
commit=$COMMIT
source_epoch=$SOURCE_EPOCH
cargo_lock_sha256=$LOCK_SHA256
EOF
mv -f -- "$OUTPUT_DIR/build-source.txt.new" "$OUTPUT_DIR/build-source.txt"

gpg --batch --yes --dearmor \
  --output "$OUTPUT_DIR/release-signing-key.gpg.new" \
  "$REPO_ROOT/docs/release-signing-key.asc"
mv -f -- "$OUTPUT_DIR/release-signing-key.gpg.new" \
  "$OUTPUT_DIR/release-signing-key.gpg"

sha256sum \
  "$SOURCE_ARCHIVE" \
  "$VENDOR_ARCHIVE" \
  "$OUTPUT_DIR/build-source.txt" \
  "$OUTPUT_DIR/release-signing-key.gpg" \
  >"$OUTPUT_DIR/SHA256SUMS.new"
mv -f -- "$OUTPUT_DIR/SHA256SUMS.new" "$OUTPUT_DIR/SHA256SUMS"

printf '%s\n' \
  "$SOURCE_ARCHIVE" \
  "$VENDOR_ARCHIVE" \
  "$OUTPUT_DIR/build-source.txt" \
  "$OUTPUT_DIR/release-signing-key.gpg" \
  "$OUTPUT_DIR/SHA256SUMS"
