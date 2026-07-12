export const reviewerDecisionLabels = [
  "Evidence is real, current, and not a deterministic substitute.",
  "Required coverage is complete.",
  "No governance capability was bypassed or weakened.",
]

export const strictProofRequirements = {
  tauri_manual_gui_smoke: {
    title: "Tauri Desktop Manual GUI Smoke",
    auditTitle: "Tauri desktop manual GUI smoke evidence exists",
    allowedEvidenceKinds: ["real"],
    defaultEvidenceKind: "real",
    coverage: [
      "input_plus_menu",
      "goal_plan_mutex",
      "workflow_menu",
      "loop_create_status",
      "workspace_default_advanced",
      "responsive_layout",
      "key_locales",
    ],
    artifactSlug: "tauri-manual-gui-smoke",
    prompt: "Record real Tauri desktop smoke for Goal, Loop, Workflow, Workspace, responsive layout, and key locales.",
    missingMeans:
      "Need real desktop smoke for input menus, Goal/Plan mutual exclusion, Workflow menu, Loop creation/status, Workspace default/advanced sections, responsive layout, and key locales.",
  },
  real_restart_resume_matrix: {
    title: "Real Restart / Resume Matrix",
    auditTitle: "Real restart/resume matrix evidence exists",
    allowedEvidenceKinds: ["real"],
    defaultEvidenceKind: "real",
    coverage: [
      "active_goal_restart",
      "running_workflow_restart",
      "dynamic_loop_restart",
      "approval_waiting_restart",
      "background_job_waiting_restart",
    ],
    artifactSlug: "restart-resume-matrix",
    prompt:
      "Record real cross-process restart/resume samples for active Goal, running Workflow, dynamic Loop, approval waiting, and background job waiting.",
    missingMeans:
      "Need real cross-process samples for active Goal, running Workflow, dynamic Loop, approval waiting, and background job waiting.",
  },
  real_soak: {
    title: "Real Wall-clock Soak",
    auditTitle: "Real wall-clock soak evidence exists",
    allowedEvidenceKinds: ["real"],
    defaultEvidenceKind: "real",
    coverage: ["wall_clock", "goal_continuation", "loop_reschedule", "workflow_recovery"],
    artifactSlug: "wall-clock-soak",
    prompt: "Record a real wall-clock soak spanning Goal continuation, Loop reschedule, and Workflow recovery.",
    missingMeans: "Need a real wall-clock sample that spans Goal continuation, Loop reschedule, and Workflow recovery.",
  },
  connector_readback: {
    title: "Connector Execution + Read-back",
    auditTitle: "Connector execution + read-back evidence exists",
    allowedEvidenceKinds: ["real", "sandbox"],
    defaultEvidenceKind: "sandbox",
    coverage: ["connector_execution", "post_action_readback", "approval_or_sandbox", "rollback_or_recovery"],
    artifactSlug: "connector-readback",
    prompt: "Record connector execution plus post-action read-back verification.",
    missingMeans: "Need real or sandbox connector execution plus post-action read-back verification.",
  },
  codex_claude_comparison: {
    title: "Hope / Claude Code / Codex Comparison",
    auditTitle: "Hope / Claude Code / Codex comparison evidence exists",
    allowedEvidenceKinds: ["real"],
    defaultEvidenceKind: "real",
    coverage: [
      "hope_result",
      "claude_code_result",
      "codex_result",
      "validation_quality",
      "time_token_cost",
      "recovery_notes",
    ],
    artifactSlug: "hope-claude-codex-comparison",
    prompt: "Record a real coding UI task comparison across Hope, Claude Code, and Codex.",
    missingMeans:
      "Need at least one 30-minute-level real coding UI task comparison with completion, interruptions, validation quality, time/token/cost, and recovery notes.",
  },
}

export const strictProofClosureOrder = [
  "tauri_manual_gui_smoke",
  "real_restart_resume_matrix",
  "real_soak",
  "connector_readback",
  "codex_claude_comparison",
]
