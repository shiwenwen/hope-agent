#!/usr/bin/env bash
#
# Resolve the Cloudflare R2 S3 endpoint from `R2_ACCOUNT_ID`, with the
# operator-error validation that turns Cloudflare's opaque failures into
# actionable messages.
#
# Shared by every workflow that talks to the R2 bucket
# (update-linux-repo.yml, mirror-release-r2.yml). Extracted so the
# validation lives in ONE place: it exists entirely to produce good error
# text, and two copies drift into two different sets of half-right hints.
#
# Inputs (env):
#   R2_ACCOUNT_ID      — 32-hex Cloudflare account id, OR a pasted full S3
#                        endpoint host / URL (jurisdiction variants ok).
#   R2_ACCESS_KEY_ID   — optional; only used to catch the account-id /
#                        access-key-id mixup with a specific message.
#
# Outputs:
#   Appends RCLONE_CONFIG_R2_ENDPOINT + CF_ACCOUNT_ID to $GITHUB_ENV when
#   that variable is set (the CI path); otherwise prints both as
#   `KEY=value` lines so the script is runnable and testable locally.
#
# Exit 0 = resolved. Exit 1 = misconfigured (message names the fix).

set -euo pipefail

: "${R2_ACCOUNT_ID:?R2_ACCOUNT_ID must be set}"

# Accept either the bare 32-hex Cloudflare account id OR a pasted full S3
# endpoint (host or URL, incl. jurisdiction variants like
# <id>.eu.r2.cloudflarestorage.com). Strip whitespace + scheme, then:
#   - if it already names the R2 endpoint host, use it verbatim
#     (preserves any .eu. jurisdiction label);
#   - otherwise treat it as a bare account id and build the host,
#     validating it is 32 lowercase hex first.
# This turns the common "pasted the whole endpoint URL" mistake — which
# otherwise surfaces as an opaque `tls: handshake failure` — into either a
# correct endpoint or a clear, actionable error.
raw="$(printf '%s' "$R2_ACCOUNT_ID" | tr -d '[:space:]')"
raw="${raw#http://}"; raw="${raw#https://}"
# Cloudflare's bucket "S3 API" panel shows the endpoint WITH the bucket
# path (…r2.cloudflarestorage.com/<bucket>). Strip any path so pasting
# that whole string resolves to the endpoint host.
raw="${raw%%/*}"

if [[ "$raw" == *".r2.cloudflarestorage.com" ]]; then
  host="$raw"
else
  id="$raw"
  if [[ ! "$id" =~ ^[0-9a-f]{32}$ ]]; then
    echo "::error::R2_ACCOUNT_ID must be the 32-char hex Cloudflare account id (or the full S3 endpoint host). Got something that normalizes to length ${#id}. Do NOT paste the whole 'https://<id>.r2.cloudflarestorage.com' URL — paste only the account id shown on the R2 overview page. See linux-repo/README.md."
    exit 1
  fi
  # Common mixup: the R2 API token's Access Key ID is ALSO 32 hex and sits
  # right next to the Account ID on the token screen. If someone pasted the
  # Access Key ID into R2_ACCOUNT_ID, the endpoint host is wrong and
  # Cloudflare rejects it at TLS (handshake failure), not with a 403. Catch
  # the identical-value case with a clear message.
  akid="$(printf '%s' "${R2_ACCESS_KEY_ID:-}" | tr -d '[:space:]')"
  if [[ -n "$akid" && "$id" == "$akid" ]]; then
    echo "::error::R2_ACCOUNT_ID equals R2_ACCESS_KEY_ID — you pasted the API token's Access Key ID into R2_ACCOUNT_ID. They are DIFFERENT values: the Account ID is the 32-hex id in the dashboard URL dash.cloudflare.com/<ACCOUNT_ID> and in https://<ACCOUNT_ID>.r2.cloudflarestorage.com. Re-set R2_ACCOUNT_ID to the account id. See linux-repo/README.md."
    exit 1
  fi
  host="${id}.r2.cloudflarestorage.com"
fi

# Bare 32-hex account id (first label of the host) for Wrangler's
# CLOUDFLARE_ACCOUNT_ID on the bridge path.
out_endpoint="RCLONE_CONFIG_R2_ENDPOINT=https://${host}"
out_account="CF_ACCOUNT_ID=${host%%.*}"

if [[ -n "${GITHUB_ENV:-}" ]]; then
  echo "$out_endpoint" >> "$GITHUB_ENV"
  echo "$out_account" >> "$GITHUB_ENV"
else
  echo "$out_endpoint"
  echo "$out_account"
fi
echo "R2 endpoint host: ${host}"
