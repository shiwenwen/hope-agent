export type EvalExperimentStatus =
  | "queued"
  | "planning"
  | "running"
  | "cancelling"
  | "completed"
  | "failed"
  | "cancelled"
  | "interrupted"

export type EvalIntegrity =
  | "local_diagnostic"
  | "protected_verified"
  | "protected_unknown_assets"
  | "unverified_import"
  | "legacy_local"

export interface EvalControlHello {
  protocolVersion: string
  productVersion: string
  runnerDigest: string
  assetRootDigest: string
  versionLockDigest: string
  os: string
  arch: string
  adapters: string[]
}

export interface EvalReadiness {
  available: boolean
  canRun: boolean
  remoteRunEnabled: boolean
  signedImportAvailable: boolean
  hello?: EvalControlHello
  issues: string[]
  signedImportIssues: string[]
}

export interface EvalAppProfile {
  schemaVersion: "eval-app-profile.v1"
  id: string
  version: string
  title: string
  description: string
  baseTier: "nightly" | "weekly" | "monthly"
  suites: Array<{ suiteId: string; caseTags: string[] }>
  allowedArms: string[]
  armMode: "one_control_per_case" | "all_allowed"
  defaultRepetitions?: number
  useSuiteRepetitions: boolean
  maxTrials: number
  maxModels: number
  maxConcurrency: number
  maxCostUsd: number
  allowCustom: boolean
}

export interface EvalCredentialOption {
  credentialProfileRef: string
  label: string
}

export interface EvalModelOption {
  providerId: string
  modelId: string
  label: string
  providerLabel: string
  credentialProfileLabel?: string
  credentialProfiles: EvalCredentialOption[]
  supportsIsolatedEval: boolean
  costKnown: boolean
  warnings: string[]
}

export interface EvalCatalog {
  readiness: EvalReadiness
  profiles: EvalAppProfile[]
  suites: Array<{
    id: string
    version: string
    capability: string
    cases: Array<{
      id: string
      title: string
      tags: string[]
      arms: string[]
      timeoutSeconds: number
    }>
  }>
  models: EvalModelOption[]
}

export interface EvalCampaignBudget {
  maxWallSeconds?: number
  maxModelCalls?: number
  maxInputTokens?: number
  maxOutputTokens?: number
  maxCostUsd?: number
  maxToolCalls?: number
  maxAgents?: number
  maxConcurrency?: number
}

export interface EvalAppRunRequest {
  schemaVersion: "eval-app-run-request.v1"
  profileId: string
  suiteSelections: Array<{
    suiteId: string
    caseIds: string[]
    arms: string[]
    repetitions?: number
  }>
  models: Array<{
    providerId: string
    modelId: string
    credentialProfileRef?: string
    reasoningEffort?: string
    maxOutputTokens?: number
  }>
  campaignBudget: EvalCampaignBudget
  debugRetention: "metrics_only" | "redacted" | "full_local"
  consent: { modelCosts: boolean; syntheticToolExecution: boolean }
}

export interface EvalAppPlan {
  schemaVersion: "eval-app-plan.v1"
  experimentId: string
  planDigest: string
  reference: string
  dirty: boolean
  appVersion: string
  profileId: string
  profileVersion: string
  campaigns: Array<{
    campaignId: string
    model: { providerId: string; modelId: string }
    resolvedPlan: { trials: unknown[]; suites: Array<{ id: string; cases: unknown[] }> }
  }>
  campaignBudget: EvalCampaignBudget
  runtimeEnvironment: {
    os: string
    arch: string
    networkEnforcement: "unverified" | "enforced"
  }
}

export interface EvalPreview {
  plan: EvalAppPlan
  estimatedTrials: number
  maxCostUsd?: number
  maxWallSeconds?: number
}

export interface EvalLocalExportResult {
  experimentId: string
  outputPath: string
  bundleSha256: string
  campaignCount: number
  signed: false
  releaseEligible: false
}

export interface EvalExperimentRecord {
  id: string
  kind: "hope_core" | "coding" | "domain"
  profileId: string
  source: "local_app" | "local_cli" | "github_actions" | "dedicated_runner"
  integrity: EvalIntegrity
  status: EvalExperimentStatus
  reference: string
  dirty: boolean
  appVersion: string
  planDigest?: string
  parentExperimentId?: string
  createdAt: string
  startedAt?: string
  completedAt?: string
  totalTrials: number
  completedTrials: number
  passedTrials: number
  failedTrials: number
  infraErrorTrials: number
  maxCostUsd?: number
  observedCostUsd?: number
  pinned: boolean
  signatureStatus?: "verified" | "verified_retired" | "verified_now_revoked" | "verified_key_missing" | "unsigned" | string
  error?: string
}

export interface EvalCampaignRecord {
  id: string
  experimentId: string
  modelDigest: string
  providerConfigDigest: string
  status: EvalExperimentStatus
  evidenceArtifactSha256?: string
  aggregateStatus?: string
  totalTrials: number
  passedTrials: number
  failedTrials: number
  infraErrorTrials: number
  durationMs?: number
  costUsd?: number
}

export interface EvalTrialRecord {
  id: string
  campaignId: string
  suiteId: string
  caseId: string
  arm: string
  outcome: string
  attempt: number
  durationMs: number
  modelCalls: number
  toolCalls: number
  inputTokens?: number
  outputTokens?: number
  costUsd?: number
  failureClass?: string
}

export interface EvalExperimentDetail {
  experiment: EvalExperimentRecord
  campaigns: EvalCampaignRecord[]
  trials: EvalTrialRecord[]
}

export interface EvalTrialDetail {
  record: EvalTrialRecord
  result?: {
    trialId: string
    outcome: string
    failureClass?: string
    error?: string
    warnings: string[]
    timings: {
      wallMs: number
      environmentSetupMs: number
      environmentCleanupMs: number
      modelActiveMs: number
      toolActiveMs: number
      queueWaitMs: number
      approvalWaitMs: number
      environmentWaitMs: number
      criticalPathMs: number
      ttftMs?: number
    }
    tokens: {
      input?: number
      output?: number
      cacheRead?: number
      cacheWrite?: number
      reasoning?: number
      usageSource?: string
    }
    cost: {
      totalUsd?: number
      agentUsd?: number
      simulatorUsd?: number
      judgeUsd?: number
      priceSnapshotDigest?: string
    }
    tools: {
      attempted: number
      logicalCalls: number
      succeeded: number
      failed: number
      cancelled: number
      retries: number
      parseErrors: number
      invalid: number
      duplicate: number
      unusedResults: number
      effective: number
    }
    orchestration: {
      modelCalls: number
      modelRetries: number
      failovers: number
      loopIterations: number
      replans: number
      checkpoints: number
      resumes: number
      spawnedAgents: number
      maxAgentDepth: number
      maxConcurrency: number
      handoffs: number
      coordinationTokens?: number
      childActiveMs: number
      asyncJobs: number
      duplicateInjections: number
      orphanedChildren: number
    }
    milestones: EvalTrialCheck[]
    invariants: EvalTrialCheck[]
    judgeChecks: EvalTrialCheck[]
    trace: { traceId: string; rootSpanId: string; spanCount: number; orphanSpanCount: number; closed: boolean }
    traceEvents: Array<{
      seq: number
      timestampMs: number
      event: string
      spanId: string
      parentSpanId: string
      key?: string
      status: string
      durationMs: number
      attributes: Record<string, string | number | boolean | null>
    }>
  }
}

export interface EvalTrialCheck {
  id: string
  passed: boolean
  blocking: boolean
  detail: string
  metric?: number
  artifactHashes: string[]
}

export type EvalCompatibility = "incompatible" | "diagnostic_only" | "functional" | "exact"
export type EvalCompatibilityMetric =
  | "functional"
  | "tokens"
  | "wall_time"
  | "tool_calls"
  | "usd_cost"
  | "multi_agent"

export type EvalTrendMetric =
  | "task_success"
  | "end_to_end_yield"
  | "any_pass_at_k"
  | "all_pass_at_k"
  | "infra_error"
  | "policy_failure"
  | "budget_exhausted"
  | "false_completion"
  | "wall_time"
  | "tool_calls"
  | "tokens"
  | "usd_cost"
  | "multi_agent_uplift"

export interface EvalCompatibilityAssessment {
  compatibility: EvalCompatibility
  reasons: string[]
}

export interface EvalMetricComparison {
  metric: EvalCompatibilityMetric
  compatibility: EvalCompatibilityAssessment
  baselineValue?: number
  candidateValue?: number
  delta?: number
  deltaPercent?: number
}

export interface EvalCompareResult {
  baselineExperimentId: string
  candidateExperimentId: string
  comparisons: EvalCampaignComparison[]
}

export interface EvalCampaignComparison {
  baselineCampaignId: string
  candidateCampaignId: string
  baselineModelDigest: string
  candidateModelDigest: string
  metrics: EvalMetricComparison[]
}

export interface EvalTrendPoint {
  experimentId: string
  campaignId: string
  modelDigest: string
  reference: string
  completedAt: string
  metric: EvalTrendMetric
  metricValue?: number
  compatibility: EvalCompatibilityAssessment
  successRate: number
  endToEndYield: number
  infraErrorRate: number
  policyFailureRate: number
  budgetExhaustedRate: number
  falseCompletionRate: number
  anyPassRate?: number
  allPassRate?: number
  multiAgentUpliftPp?: number
  medianWallMs?: number
  totalToolCalls: number
  totalInputTokens?: number
  totalOutputTokens?: number
  totalCostUsd?: number
}

export interface EvalImportResult {
  importId: string
  experimentId: string
  integrity: EvalIntegrity
  keyId?: string
  evidenceSha256: string
  alreadyImported: boolean
}

export interface EvalBaselineRecord {
  id: string
  experimentId: string
  tier: "nightly" | "weekly" | "release" | "monthly"
  approvedBy: string
  approvedAt: string
  note?: string
}

export interface EvalAnnotationRecord {
  id: string
  experimentId: string
  campaignId?: string
  trialId?: string
  text: string
  createdAt: string
}
