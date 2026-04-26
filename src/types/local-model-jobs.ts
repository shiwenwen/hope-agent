export type LocalModelJobKind = "chat_model" | "embedding_model"

export type LocalModelJobStatus =
  | "running"
  | "cancelling"
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
  error?: string | null
  resultJson?: unknown | null
  createdAt: number
  updatedAt: number
  completedAt?: number | null
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
}

export const LOCAL_MODEL_JOB_EVENTS = {
  created: "local_model_job:created",
  updated: "local_model_job:updated",
  log: "local_model_job:log",
  completed: "local_model_job:completed",
} as const

export function isLocalModelJobActive(job: LocalModelJobSnapshot): boolean {
  return job.status === "running" || job.status === "cancelling"
}

export function isLocalModelJobTerminal(job: LocalModelJobSnapshot): boolean {
  return !isLocalModelJobActive(job)
}

const PHASE_KEY: Record<string, string> = {
  queued: "localModelJobs.phases.queued",
  "checking-ollama": "localModelJobs.phases.checkingOllama",
  starting: "settings.localLlm.phases.starting",
  "download-installer": "settings.localLlm.phases.downloadInstaller",
  authorize: "settings.localLlm.phases.authorize",
  "install-ollama": "settings.localLlm.phases.installOllama",
  "start-ollama": "localModelJobs.phases.startOllama",
  "pulling manifest": "settings.localLlm.phases.pullingManifest",
  downloading: "settings.localLlm.phases.downloading",
  "verifying digest": "settings.localLlm.phases.verifying",
  "writing manifest": "settings.localLlm.phases.writingManifest",
  success: "settings.localLlm.phases.success",
  "register-provider": "settings.localLlm.phases.registerProvider",
  "configure-embedding": "settings.localEmbedding.phases.configureEmbedding",
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
    percent: job.percent ?? null,
  }
}
