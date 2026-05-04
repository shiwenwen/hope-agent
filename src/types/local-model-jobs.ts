export type LocalModelJobKind =
  | "chat_model"
  | "embedding_model"
  | "ollama_install"
  | "ollama_pull"
  | "ollama_preload"
  | "memory_reembed"

export type LocalModelJobStatus =
  | "running"
  | "cancelling"
  | "paused"
  | "completed"
  | "failed"
  | "interrupted"
  | "cancelled"

export interface LocalModelJobSnapshot {
  jobId: string
  kind: LocalModelJobKind
  modelId: string
  displayName: string
  status: LocalModelJobStatus
  phase: string
  percent?: number | null
  bytesCompleted?: number | null
  bytesTotal?: number | null
  error?: string | null
  resultJson?: unknown | null
  createdAt: number
  updatedAt: number
  completedAt?: number | null
  /**
   * 当本任务是另一个任务的「续作 / 后续步骤」时，指向触发它的那个任务的 `jobId`。
   * 当前唯一用法：embedding pull 任务结束后由 ha-core 派发的 `MemoryReembed`
   * 任务在此字段记录原 pull 任务 id，让 dialog 能自动接力到 reembed 进度。
   */
  successorForJobId?: string | null
}

export interface LocalModelJobLogEntry {
  jobId: string
  seq: number
  kind: string
  message: string
  createdAt: number
}

export interface ProgressFrame {
  phase: string
  message?: string
  percent?: number | null
  bytesCompleted?: number | null
  bytesTotal?: number | null
  /**
   * 本帧的进度数值单位语义。`bytes` 是默认（下载量），`count` 用于按条数计的
   * 任务（如 `memory_reembed` 把 bytesCompleted/bytesTotal 当成已处理 / 总记忆
   * 条数）。`InstallProgressDialog` 据此切换 formatter（条记忆 vs MB/GB）。
   */
  unit?: "bytes" | "count"
}

export const LOCAL_MODEL_JOB_EVENTS = {
  created: "local_model_job:created",
  updated: "local_model_job:updated",
  log: "local_model_job:log",
  completed: "local_model_job:completed",
} as const

/**
 * Backend `local_model:missing_alert` event payload. Mirrors
 * `crates/ha-core/src/local_llm/auto_maintainer.rs::LocalModelMissingAlert`.
 */
export interface LocalModelMissingAlert {
  kind: "chat" | "embedding"
  missingModelId: string
  missingDisplayName: string
  alternatives: MissingAlertAlternative[]
  canRedownload: boolean
  canDisableEmbedding: boolean
}

export interface MissingAlertAlternative {
  modelId: string
  displayName: string
  /** Set when `kind === "chat"` — needed for `set_active_model`. */
  providerId?: string | null
  /** Set when `kind === "embedding"` — needed for `set_memory_embedding_default`. */
  embeddingConfigId?: string | null
}

export const LOCAL_MODEL_ALERT_EVENT = "local_model:missing_alert" as const

export function isLocalModelJobActive(job: LocalModelJobSnapshot): boolean {
  return job.status === "running" || job.status === "cancelling"
}

export function isLocalModelJobVisible(job: LocalModelJobSnapshot): boolean {
  return (
    isLocalModelJobActive(job) ||
    job.status === "paused" ||
    job.status === "interrupted" ||
    job.status === "failed"
  )
}

export function isLocalModelJobResumable(job: LocalModelJobSnapshot): boolean {
  return job.status === "paused" || job.status === "interrupted" || job.status === "failed"
}

export function isLocalModelJobTerminal(job: LocalModelJobSnapshot): boolean {
  return !isLocalModelJobActive(job)
}

/**
 * 判断 `next` 是否应该作为 `current` 的接力任务被 dialog 自动跟进。
 * 典型场景：embedding pull 任务结束后由 ha-core 派发的 `MemoryReembed` 任务在
 * `successorForJobId` 字段引用原 pull job，此时 dialog 自动把 `currentJob` 切到
 * reembed 任务，让用户连续看到「下载 → 切换模型 → 重建记忆向量」整条流水线。
 *
 * 不要求 `current` 已 terminal —— 后端在父 job emit done/completed 之前就已
 * spawn successor 并 emit `created` 事件（[`local_embedding.rs::pull_and_activate_cancellable`]
 * 内 `save_and_set_default_for_model` 在 99% 帧之后立刻派发 reembed job，
 * 然后才发 100% 帧 + finish_job），事件到达前端的顺序可能反过来。如果
 * successor 已被创建，意味着 set_memory_embedding_default 已经成功返回，
 * 父任务必然会以 completed 收尾——`current.error` 始终为空。
 */
export function isJobSuccessorOf(
  next: LocalModelJobSnapshot,
  current: LocalModelJobSnapshot | null,
): boolean {
  if (!current) return false
  if (next.jobId === current.jobId) return false
  if (next.successorForJobId !== current.jobId) return false
  return !current.error
}

export function localModelJobPercent(job: LocalModelJobSnapshot): number | null {
  if (job.percent != null) return job.percent
  const completed = job.bytesCompleted ?? null
  const total = job.bytesTotal ?? null
  if (completed == null || total == null || total <= 0) return null
  return Math.max(0, Math.min(100, (completed / total) * 100))
}

const PHASE_KEY: Record<string, string> = {
  queued: "localModelJobs.phases.queued",
  "checking-ollama": "localModelJobs.phases.checkingOllama",
  starting: "settings.localLlm.phases.starting",
  "download-installer": "settings.localLlm.phases.downloadInstaller",
  authorize: "settings.localLlm.phases.authorize",
  "install-ollama": "settings.localLlm.phases.installOllama",
  "start-ollama": "localModelJobs.phases.startOllama",
  "loading-model": "localModelJobs.phases.loadingModel",
  "loaded-waiting": "localModelJobs.phases.loadedWaiting",
  "verifying-load": "localModelJobs.phases.verifyingLoad",
  paused: "localModelJobs.phases.paused",
  "pulling manifest": "settings.localLlm.phases.pullingManifest",
  downloading: "settings.localLlm.phases.downloading",
  "verifying digest": "settings.localLlm.phases.verifying",
  "writing manifest": "settings.localLlm.phases.writingManifest",
  success: "settings.localLlm.phases.success",
  "register-provider": "settings.localLlm.phases.registerProvider",
  // Source of truth for these phase strings is in Rust:
  //   - `crates/ha-core/src/local_embedding.rs::PHASE_SWITCHING_EMBEDDING_MODEL`
  //   - `crates/ha-core/src/memory/reembed_job.rs::PHASE_REEMBED_KEEP / PHASE_REEMBED_FRESH`
  // Drift between the two sides silently breaks the localized phase label.
  "switching-embedding-model": "settings.localEmbedding.phases.switchingEmbeddingModel",
  // 老 phase 字符串，留映射兼容历史 DB row（已写入但 UI 还在显示）。
  "configure-embedding": "settings.localEmbedding.phases.switchingEmbeddingModel",
  "reembed-keep": "settings.embedding.reembedJob.phaseKeep",
  "reembed-fresh": "settings.embedding.reembedJob.phaseFresh",
  done: "settings.localLlm.phases.done",
}

export function phaseTranslationKey(phase: string | undefined): string | undefined {
  if (!phase) return undefined
  return PHASE_KEY[phase.toLowerCase()]
}

export function formatLocalModelJobLogLine(message: string, createdAt?: number): string {
  const date = createdAt ? new Date(createdAt * 1000) : new Date()
  return `[${date.toLocaleTimeString()}] ${message}`
}

export function localModelJobToProgressFrame(
  job: LocalModelJobSnapshot,
  phaseLabel: (phase: string | undefined) => string,
): ProgressFrame {
  return {
    phase: job.phase,
    message: phaseLabel(job.phase) || job.phase,
    percent: localModelJobPercent(job),
    bytesCompleted: job.bytesCompleted ?? null,
    bytesTotal: job.bytesTotal ?? null,
    // memory_reembed 的 bytesCompleted/Total 字段语义实际是「已处理 / 总记忆
    // 条数」，让 dialog 把字节 formatter 换成「X / Y 条记忆」渲染。
    unit: job.kind === "memory_reembed" ? "count" : "bytes",
  }
}
