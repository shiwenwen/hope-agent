#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

eval_ref="$(git rev-parse HEAD)"
eval_tmp="$(mktemp -d)"
trap 'rm -rf "$eval_tmp"' EXIT

echo "== Deterministic memory retrieval capability eval =="
cargo run -p ha-eval --release --locked -- \
  plan --tier weekly --ref "$eval_ref" --output "$eval_tmp/plan.json"
cargo run -p ha-eval --release --locked -- \
  run --plan "$eval_tmp/plan.json" \
  --suite memory-retrieval-scale --shard 1/1 \
  --output "$eval_tmp/memory-retrieval.json"
jq '.cases[] | {caseId, status, durationMs, checks}' "$eval_tmp/memory-retrieval.json"
jq -e 'all(.cases[]; .status == "passed")' "$eval_tmp/memory-retrieval.json" >/dev/null
