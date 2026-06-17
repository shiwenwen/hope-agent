#!/usr/bin/env bash
#
# Generate a stable self-signed macOS code-signing certificate for Hope Agent.
#
# WHY ------------------------------------------------------------------------
# GitHub Actions currently builds the macOS app ad-hoc signed, so its code
# signature (cdhash) changes on every release. macOS binds TCC permissions
# (Screen Recording, Accessibility, ...) to the signature's *designated
# requirement*; for an ad-hoc binary that requirement IS the cdhash, so every
# auto-update silently revokes already-granted permissions — the toggle stays
# on in System Settings but the app reports "not authorized".
#
# Signing every build with ONE fixed certificate makes the designated
# requirement key on the (constant) certificate + bundle id instead of the
# (changing) cdhash, so a permission granted once survives all future updates.
#
# This is NOT a substitute for an Apple Developer ID: Gatekeeper still warns on
# first launch (right-click > Open, or `xattr -dr com.apple.quarantine`). It
# only fixes permission persistence. See docs/macos-self-signing.md.
#
# USAGE ----------------------------------------------------------------------
#   bash scripts/macos-selfsign-cert.sh
#
# Run ONCE on macOS. Paste the printed values into the GitHub repo secrets, then
# keep the certificate forever — re-running this makes a NEW identity and breaks
# the persistence (users would have to re-grant once more).
#
# The output contains private-key material. Do not commit it or paste it into
# chat — only into Settings > Secrets and variables > Actions.
# ---------------------------------------------------------------------------
set -euo pipefail

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "This script must run on macOS (needs the system openssl + .p12 format)." >&2
  exit 1
fi

# Stable identity name — also the APPLE_SIGNING_IDENTITY secret. Override only
# if you have a reason to; it must never change once a release has shipped.
IDENTITY="${HOPE_SIGN_IDENTITY:-Hope Agent Self-Signed}"

# LibreSSL ships with macOS at /usr/bin/openssl and emits a legacy-encrypted
# .p12 that the CI runner's `security import` reads without extra flags (a
# Homebrew OpenSSL 3 .p12 can need -legacy on older importers).
OPENSSL=/usr/bin/openssl

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

p12_pass="$("$OPENSSL" rand -base64 24)"
keychain_pass="$("$OPENSSL" rand -base64 24)"

cat > "$tmp/req.cnf" <<EOF
[req]
distinguished_name = dn
x509_extensions    = codesign
prompt             = no
[dn]
CN = ${IDENTITY}
[codesign]
basicConstraints   = critical,CA:FALSE
keyUsage           = critical,digitalSignature
extendedKeyUsage   = critical,codeSigning
EOF

# 10-year self-signed leaf with a code-signing EKU.
"$OPENSSL" req -x509 -newkey rsa:2048 -nodes -days 3650 \
  -keyout "$tmp/key.pem" -out "$tmp/cert.pem" -config "$tmp/req.cnf"

"$OPENSSL" pkcs12 -export \
  -inkey "$tmp/key.pem" -in "$tmp/cert.pem" \
  -name "$IDENTITY" -out "$tmp/cert.p12" -passout "pass:${p12_pass}"

cert_b64="$(base64 < "$tmp/cert.p12" | tr -d '\n')"

cat <<EOF

────────────────────────────────────────────────────────────────────────────
Self-signed code-signing certificate generated (valid 10 years).

Add these four GitHub repo secrets
(Settings ▸ Secrets and variables ▸ Actions ▸ New repository secret):

APPLE_CERTIFICATE
${cert_b64}

APPLE_CERTIFICATE_PASSWORD
${p12_pass}

APPLE_SIGNING_IDENTITY
${IDENTITY}

KEYCHAIN_PASSWORD
${keychain_pass}
────────────────────────────────────────────────────────────────────────────

Next:
  1. Add the four secrets above.
  2. Cut a release (or run the release workflow with dry_run=true to verify the
     macOS lane signs without notarization).
  3. On each Mac, after installing the first signed build, grant the desired
     permission once more — it then persists across all later updates.

Keep this certificate forever. Do NOT re-run this script for a new identity.
EOF
