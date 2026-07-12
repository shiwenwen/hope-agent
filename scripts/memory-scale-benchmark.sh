#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

echo "== Retrieval Planner source-fusion benchmark =="
cargo test -p ha-core --release --lib \
  agent::retrieval_planner::tests::benchmark_source_fusion_with_one_hundred_thousand_candidates \
  --locked -- --ignored --nocapture --test-threads=1

echo
echo "== SQLite memory retrieval benchmark =="
cargo test -p ha-core --release --test memory_retrieval_scale \
  --locked -- --ignored --nocapture --test-threads=1
