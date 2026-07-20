#!/usr/bin/env bash
# 下一代记忆系统专项 smoke:
# - Rust: memory 核心单测 + Retrieval Planner trace + Agent memory bridge + Dreaming golden fixtures + backup/restore 安全链
# - Frontend: Memory Center / Answer Memory Chips / Workspace diagnostics / Knowledge focus/preview /
#   editor contracts / external provider / owner feedback 纯函数回归（分组运行，降低单次 Vitest 资源峰值）
# - Shared: i18n key 完整性 + TypeScript 类型闭合
#
# 用法:
#   scripts/memory-smoke-test.sh
#   HA_MEMORY_SMOKE_SKIP_RUST=1 scripts/memory-smoke-test.sh
#   HA_MEMORY_SMOKE_SKIP_FRONTEND=1 scripts/memory-smoke-test.sh
#   HA_MEMORY_SMOKE_SKIP_TYPECHECK=1 scripts/memory-smoke-test.sh
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

if [[ "${HA_MEMORY_SMOKE_SKIP_RUST:-0}" != "1" ]]; then
  echo "==> Rust memory unit tests (ha-core --lib memory::)"
  cargo test -p ha-core --lib memory:: --locked

  echo
  echo "==> Rust Retrieval Planner trace tests"
  cargo test -p ha-core --lib retrieval_planner --locked

  echo
  echo "==> Rust Agent memory bridge tests"
  cargo test -p ha-core --lib procedure_memory_suffix --locked
  cargo test -p ha-core --lib graph_edges_to_candidate_refs --locked

  echo
  echo "==> Rust Dreaming deterministic golden fixtures (standalone eval runner)"
  eval_ref="$(git rev-parse HEAD)"
  eval_tmp="$(mktemp -d)"
  cargo run -p ha-eval --locked -- \
    plan --tier weekly --ref "$eval_ref" --output "$eval_tmp/plan.json"
  cargo run -p ha-eval --locked -- \
    run --plan "$eval_tmp/plan.json" --suite memory-dreaming --shard 1/2 \
    --output "$eval_tmp/dreaming-1.json"
  cargo run -p ha-eval --locked -- \
    run --plan "$eval_tmp/plan.json" --suite memory-dreaming --shard 2/2 \
    --output "$eval_tmp/dreaming-2.json"
  jq -e 'all(.cases[]; .status == "passed")' \
    "$eval_tmp/dreaming-1.json" "$eval_tmp/dreaming-2.json" >/dev/null

  echo
  echo "==> Rust memory backup/restore safety integration"
  cargo test -p ha-core --test memory_backup_restore --locked
else
  echo "==> Rust memory checks skipped (HA_MEMORY_SMOKE_SKIP_RUST=1)"
fi

if [[ "${HA_MEMORY_SMOKE_SKIP_FRONTEND:-0}" != "1" ]]; then
  echo
  echo "==> Frontend memory-focused Vitest suite"
  FRONTEND_TEST_PATHS=()
  FRONTEND_TEST_GROUPS=()

  validate_frontend_test_paths() {
    local group_name="$1"
    shift
    local path
    local i
    for path in "$@"; do
      if [[ ! -f "$path" ]]; then
        echo "Missing frontend memory smoke test in ${group_name}: ${path}" >&2
        exit 1
      fi
      for ((i = 0; i < ${#FRONTEND_TEST_PATHS[@]}; i++)); do
        if [[ "${FRONTEND_TEST_PATHS[$i]}" == "$path" ]]; then
          echo "Duplicate frontend memory smoke test in ${group_name}: ${path}" >&2
          echo "Already listed in ${FRONTEND_TEST_GROUPS[$i]}" >&2
          exit 1
        fi
      done
      FRONTEND_TEST_PATHS+=("$path")
      FRONTEND_TEST_GROUPS+=("$group_name")
    done
  }

  run_frontend_vitest_group() {
    local group_name="$1"
    shift
    validate_frontend_test_paths "$group_name" "$@"
    echo
    echo "==> Frontend memory-focused Vitest suite: ${group_name}"
    pnpm vitest run "$@"
  }

  run_frontend_vitest_group "core trace and editor contracts" \
    src/components/chat/chatUtils.test.ts \
    src/components/chat/message/memoryTraceFormat.test.ts \
    src/components/chat/workspace/workspaceMemoryDiagnostics.test.ts \
    src/components/knowledge/knowledgeFocus.test.ts \
    src/components/knowledge/cm/livePreviewExtensions.test.ts \
    src/components/knowledge/noteEmbedFeedback.test.ts \
    src/components/knowledge/noteSourceReferenceFeedback.test.ts \
    src/components/knowledge/outline.test.ts \
    src/components/knowledge/previewHighlight.test.ts \
    src/components/knowledge/transclusionParse.test.ts \
    src/components/chat/message/MessageBubble.memoryTrace.test.tsx \
    src/lib/diagnosticRedaction.test.ts \
    src/lib/openExternalUrl.test.ts

  run_frontend_vitest_group "knowledge and chat feedback surfaces" \
    src/components/dashboard/dreaming/claimReviewActionFeedback.test.ts \
    src/components/dashboard/dreaming/dreamingOperationFeedback.test.ts \
    src/components/knowledge/knowledgeFocusFeedback.test.ts \
    src/components/knowledge/knowledgeCompileFeedback.test.ts \
    src/components/knowledge/knowledgeEmbeddingBadgeFeedback.test.ts \
    src/components/knowledge/knowledgeGraphFeedback.test.ts \
    src/components/knowledge/knowledgeJobsFeedback.test.ts \
    src/components/knowledge/knowledgeMaintenanceFeedback.test.ts \
    src/components/knowledge/knowledgeSourceFeedback.test.ts \
    src/components/knowledge/knowledgeViewFeedback.test.ts \
    src/components/knowledge/chat/knowledgeChatFeedback.test.ts \
    src/components/knowledge/chat/knowledgeQueryFilingFeedback.test.ts \
    src/components/knowledge/chat/knowledgeQuickRewriteFeedback.test.ts \
    src/components/knowledge/sprite/knowledgeSpriteFeedback.test.ts \
    src/components/chat/chatFocusFeedback.test.ts \
    src/components/chat/chatKnowledgeReferenceFeedback.test.ts \
    src/components/chat/file-mention/FileMentionMenu.test.tsx \
    src/components/chat/note-mention/NoteMentionMenu.test.tsx \
    src/components/chat/note-mention/noteMentionFeedback.test.ts \
    src/components/chat/project/ProjectKnowledgeSection.test.tsx \
    src/components/chat/project/projectFocusFeedback.test.ts \
    src/components/chat/project/projectKnowledgeFeedback.test.ts \
    src/components/chat/workspace/workspaceKnowledgeFeedback.test.ts \
    src/components/chat/workspace/workspaceSourceFeedback.test.ts \
    src/components/settings/KnowledgeMaintenanceSection.test.tsx \
    src/components/settings/KnowledgePanel.test.tsx \
    src/components/settings/knowledgeMaintenanceSettingsFeedback.test.ts \
    src/components/settings/knowledgePanelFeedback.test.ts \
    src/components/settings/SpriteSection.test.tsx \
    src/components/settings/spriteSettingsFeedback.test.ts

  run_frontend_vitest_group "agent and memory settings controls" \
    src/components/settings/agent-panel/AgentEditView.test.tsx \
    src/components/settings/agent-panel/AgentListView.test.tsx \
    src/components/settings/agent-panel/DefaultAgentSection.test.tsx \
    src/components/settings/agent-panel/activeMemoryPreset.test.ts \
    src/components/settings/agent-panel/activeMemorySummary.test.ts \
    src/components/settings/agent-panel/agentLoadOperationFeedback.test.ts \
    src/components/settings/agent-panel/tabs/MemoryTab.test.tsx \
    src/components/settings/embedding-models/embeddingModelFeedback.test.ts \
    src/components/settings/memory-panel/BudgetConfig.test.tsx \
    src/components/settings/memory-panel/CoreMemoryEditor.test.tsx \
    src/components/settings/memory-panel/defaultMemoryAgent.test.ts \
    src/components/settings/memory-panel/ExternalMemoryProviderCredentials.test.tsx \
    src/components/settings/memory-panel/ExternalMemoryProvidersConfig.test.tsx \
    src/components/settings/memory-panel/externalMemoryProviderOperationFeedback.test.ts \
    src/components/settings/memory-panel/externalMemoryProviderReadiness.test.ts \
    src/components/settings/memory-panel/HybridSearchConfig.test.tsx \
    src/components/settings/memory-panel/LocalEmbeddingAssistantCard.test.tsx \
    src/components/settings/memory-panel/memoryAdvancedConfigFeedback.test.ts \
    src/components/settings/memory-panel/memoryBudgetOperationFeedback.test.ts \
    src/components/settings/memory-panel/memoryCrudOperationFeedback.test.ts \
    src/components/settings/memory-panel/memoryEmbeddingFeedback.test.ts \
    src/components/settings/memory-panel/memoryExtractOperationFeedback.test.ts \
    src/components/settings/memory-panel/memoryOverviewOperationFeedback.test.ts \
    src/components/settings/memory-panel/memoryRepairOperationFeedback.test.ts \
    src/components/settings/memory-panel/memorySearchExplain.test.ts \
    src/components/settings/memory-panel/profileSnapshotOperationFeedback.test.ts \
    src/components/settings/memory-panel/scopeFocus.test.ts \
    src/components/settings/memory-panel/TemporalDecayConfig.test.tsx

  run_frontend_vitest_group "memory audit backup and health contracts" \
    src/components/settings/memory-panel/claimClipboardFeedback.test.ts \
    src/components/settings/memory-panel/claimConflictAudit.test.ts \
    src/components/settings/memory-panel/claimOwnerOperationFeedback.test.ts \
    src/components/settings/memory-panel/claimSearchExplain.test.ts \
    src/components/settings/memory-panel/coreMemoryOperationFeedback.test.ts \
    src/components/settings/memory-panel/dreamingSettingsOperationFeedback.test.ts \
    src/components/settings/memory-panel/memoryAuditOperationFeedback.test.ts \
    src/components/settings/memory-panel/memoryBackupLegacyRestoreFeedback.test.ts \
    src/components/settings/memory-panel/memoryBackupOperationFeedback.test.ts \
    src/components/settings/memory-panel/memoryBackupPreviewDiagnostics.test.ts \
    src/components/settings/memory-panel/memoryBackupPreviewSummary.test.ts \
    src/components/settings/memory-panel/memoryBackupRestoreOptions.test.ts \
    src/components/settings/memory-panel/memoryBackupRestorePlan.test.ts \
    src/components/settings/memory-panel/memoryBackupStructuredRestoreFeedback.test.ts \
    src/components/settings/memory-panel/memoryBackupUnlockFlow.test.ts \
    src/components/settings/memory-panel/memoryExperienceOperationFeedback.test.ts \
    src/components/settings/memory-panel/memoryAuditActivity.test.ts \
    src/components/settings/memory-panel/memoryFocus.test.ts \
    src/components/settings/memory-panel/memoryHealthFormat.test.ts \
    src/components/settings/memory-panel/memoryHealthRepairHints.test.ts \
    src/components/settings/memory-panel/memoryImportFeedback.test.ts \
    src/components/settings/memory-panel/memorySnapshotArtifactFormat.test.ts

  echo
  echo "==> i18n key completeness"
  node scripts/sync-i18n.mjs --check
else
  echo
  echo "==> Frontend memory checks skipped (HA_MEMORY_SMOKE_SKIP_FRONTEND=1)"
fi

if [[ "${HA_MEMORY_SMOKE_SKIP_TYPECHECK:-0}" != "1" ]]; then
  echo
  echo "==> TypeScript typecheck"
  pnpm typecheck
else
  echo
  echo "==> TypeScript typecheck skipped (HA_MEMORY_SMOKE_SKIP_TYPECHECK=1)"
fi

echo
echo "✅ memory smoke checks passed"
