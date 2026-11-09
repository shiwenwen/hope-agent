#!/usr/bin/env bash
# Shared by the two R2 publish paths. No bucket credentials are needed here.
set -euo pipefail

# Reviewed 2026-10-29: https://downloads.rclone.org/v1.75.2/SHA256SUMS
# Keep helper variables outside the RCLONE_* flag environment namespace.
R2_RCLONE_PIN=v1.75.2
R2_RCLONE_SHA256=349ac8fba6ff65d6247043f1750cdcb518ec5d500ef91463a10d37c0ccdf3702
R2_RCLONE_BYTES=31615572
destination="${1:?usage: install-rclone.sh DESTINATION_DIRECTORY}"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

curl --proto '=https' --proto-redir '=https' -fsSL -o "$tmp/rclone.zip" \
  "https://downloads.rclone.org/${R2_RCLONE_PIN}/rclone-${R2_RCLONE_PIN}-linux-amd64.zip"
[[ "$(wc -c < "$tmp/rclone.zip" | tr -d '[:space:]')" == "$R2_RCLONE_BYTES" ]] || {
  echo '::error::rclone archive size mismatch' >&2
  exit 1
}
printf '%s  %s\n' "$R2_RCLONE_SHA256" "$tmp/rclone.zip" | sha256sum -c - >/dev/null
unzip -q "$tmp/rclone.zip" -d "$tmp"
binary="$tmp/rclone-${R2_RCLONE_PIN}-linux-amd64/rclone"
chmod 755 "$binary"
version_output="$("$binary" version)"
[[ "${version_output%%$'\n'*}" == "rclone ${R2_RCLONE_PIN}" ]] || {
  echo '::error::rclone executable version mismatch' >&2
  exit 1
}
flags_help="$("$binary" help flags)"
for flag in --metadata --metadata-set --checksum --s3-no-check-bucket; do
  grep -Eq -- "(^|[[:space:]])${flag}([=[:space:]]|$)" <<<"$flags_help" || {
    echo "::error::rclone ${R2_RCLONE_PIN} does not support ${flag}" >&2
    exit 1
  }
done

# Publish only after all offline checks succeed. A failure leaves any previous
# executable untouched; mv stays on the destination filesystem.
mkdir -p "$destination"
candidate="$(mktemp "$destination/.rclone.XXXXXX")"
trap 'rm -rf "$tmp"; rm -f "$candidate"' EXIT
install -m 755 "$binary" "$candidate"
mv -f "$candidate" "$destination/rclone"
echo "rclone ${R2_RCLONE_PIN}: size, SHA-256, version and flags verified"
