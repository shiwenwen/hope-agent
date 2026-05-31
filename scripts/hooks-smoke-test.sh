#!/usr/bin/env bash
# Hooks 子系统冒烟测试:内联单测 + 集成测试 + 官方脚本兼容套件。
# 覆盖 matcher / parse / 决策聚合 / condition / config / scope / 5 个 runner /
# transcript,以及 config→reload→dispatch 全链、project-scope 闸、continue:false
# 聚合、字段级官方对齐。
#
# 用法:  scripts/hooks-smoke-test.sh
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

echo "==> jq 检查(兼容套件 hooks_compat 需要)"
if command -v jq >/dev/null 2>&1; then
  jq --version
else
  echo "⚠ jq 未安装 → hooks_compat 会自动跳过(不算失败)。装:brew install jq"
fi

echo
echo "==> 内联单测 (cargo test -p ha-core --lib hooks::)"
cargo test -p ha-core --lib hooks::

echo
echo "==> 集成测试 (hooks_e2e / project_scope / continue_false / compat)"
cargo test -p ha-core \
  --test hooks_e2e \
  --test hooks_project_scope \
  --test hooks_pre_tool_continue_false \
  --test hooks_compat

echo
echo "✅ hooks 自动化测试全部通过"
