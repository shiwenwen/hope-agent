import { useEffect, useSyncExternalStore } from "react"
import { isTauriMode, TRANSPORT_EVENT_RESYNC_REQUIRED } from "@/lib/transport"
import { getActiveHttpTransport, useTransportRevision } from "@/lib/transport-provider"
import type { HttpTransport } from "@/lib/transport-http"
import { logger } from "@/lib/logger"

export type ServerUpdateCapability = "automatic" | "docker_redeploy" | "manual" | "desktop_only"

export type ServerUpdateJobStatus = "running" | "awaiting_restart" | "succeeded" | "failed"

export interface ServerUpdateSnapshot {
  serverInstanceId: string
  currentVersion: string
  latestVersion: string
  hasUpdate: boolean
  installSource: string
  recommendedPath: "tauri" | "package_manager" | "self_contained" | "manual_prompt"
  capability: ServerUpdateCapability
  notes?: string | null
  pubDate?: string | null
  bareBinaryAvailable: boolean
  stagedVersion?: string | null
  checkedAt: string
  manualInstructions?: string | null
}

export interface ServerUpdateJob {
  jobId: string
  fromVersion: string
  targetVersion: string
  path: ServerUpdateSnapshot["recommendedPath"]
  status: ServerUpdateJobStatus
  phase: string
  percent?: number | null
  createdAt: string
  updatedAt: string
  error?: string | null
}

export interface ServerUpdateStatus {
  serverInstanceId: string
  currentVersion: string
  snapshot?: ServerUpdateSnapshot | null
  activeJob?: ServerUpdateJob | null
  recentJobs: ServerUpdateJob[]
}

export interface ServerInstallPlan {
  planId?: string | null
  serverInstanceId: string
  currentVersion: string
  targetVersion: string
  path: ServerUpdateSnapshot["recommendedPath"]
  capability: ServerUpdateCapability
  expiresAt?: string | null
  confirmation: string
  manualInstructions?: string | null
}

export interface ServerAutoUpdatePolicy {
  checkEnabled: boolean
  notify: boolean
}

interface StoreSnapshot {
  target: string | null
  status: ServerUpdateStatus | null
  autoUpdatePolicy: ServerAutoUpdatePolicy | null
  loading: boolean
  checking: boolean
  error: string | null
}

let snapshot: StoreSnapshot = {
  target: null,
  status: null,
  autoUpdatePolicy: null,
  loading: false,
  checking: false,
  error: null,
}
const listeners = new Set<() => void>()
let boundTransport: HttpTransport | null = null
let cleanupEvents: (() => void) | null = null
let refreshPromise: Promise<ServerUpdateStatus | null> | null = null

function emit(patch: Partial<StoreSnapshot>) {
  snapshot = { ...snapshot, ...patch }
  for (const listener of listeners) listener()
}

function subscribe(listener: () => void) {
  listeners.add(listener)
  return () => listeners.delete(listener)
}

function getSnapshot() {
  return snapshot
}

function describeError(error: unknown): string {
  return error instanceof Error ? error.message : String(error)
}

async function refreshStatus(check = false): Promise<ServerUpdateStatus | null> {
  const transport = boundTransport
  if (!transport) return null
  if (refreshPromise && !check) return refreshPromise
  const request = (async () => {
    emit(check ? { checking: true, error: null } : { loading: true, error: null })
    try {
      const status = await transport.requestAppUpdate<ServerUpdateStatus>(
        check ? "/api/app-update/check" : "/api/app-update/status",
        check ? { method: "POST" } : undefined,
      )
      if (boundTransport === transport) emit({ status })
      return status
    } catch (error) {
      if (boundTransport === transport) emit({ error: describeError(error) })
      throw error
    } finally {
      if (boundTransport === transport) emit(check ? { checking: false } : { loading: false })
    }
  })()
  if (!check) refreshPromise = request
  try {
    return await request
  } finally {
    if (refreshPromise === request) refreshPromise = null
  }
}

async function refreshAutoUpdatePolicy(): Promise<ServerAutoUpdatePolicy | null> {
  const transport = boundTransport
  if (!transport) return null
  try {
    const config = await transport.call<ServerAutoUpdatePolicy>("get_auto_update_config")
    if (typeof config.checkEnabled !== "boolean" || typeof config.notify !== "boolean") {
      throw new Error("Invalid server auto-update policy")
    }
    const policy = { checkEnabled: config.checkEnabled, notify: config.notify }
    if (boundTransport === transport) emit({ autoUpdatePolicy: policy })
    return policy
  } catch (error) {
    if (boundTransport === transport) emit({ autoUpdatePolicy: null })
    throw error
  }
}

function bindToActiveTransport() {
  const transport = getActiveHttpTransport()
  if (transport === boundTransport) return
  cleanupEvents?.()
  cleanupEvents = null
  boundTransport = transport
  refreshPromise = null
  if (!transport) {
    emit({
      target: null,
      status: null,
      autoUpdatePolicy: null,
      loading: false,
      checking: false,
      error: null,
    })
    return
  }
  emit({
    target: transport.getBaseUrl(),
    status: null,
    autoUpdatePolicy: null,
    loading: true,
    error: null,
  })
  const refreshStatusFromEvent = () => {
    void refreshStatus().catch((error) => {
      logger.warn("updater", "serverUpdater::refresh", "server update status failed", error)
    })
  }
  const refreshPolicyFromEvent = () => {
    void refreshAutoUpdatePolicy().catch((error) => {
      logger.warn("updater", "serverUpdater::policy", "server update policy failed", error)
    })
  }
  const refreshAllFromEvent = () => {
    refreshStatusFromEvent()
    refreshPolicyFromEvent()
  }
  const unlisten = [
    transport.listen(TRANSPORT_EVENT_RESYNC_REQUIRED, refreshAllFromEvent),
    transport.listen("config:changed", refreshPolicyFromEvent),
    transport.listen("app_update:available", refreshStatusFromEvent),
    transport.listen("app_update:staged", refreshStatusFromEvent),
    transport.listen("app_update:completed", refreshStatusFromEvent),
    transport.listen("app_update:progress", (payload) => {
      if (!payload || typeof payload !== "object") return
      const event = payload as { job_id?: string; phase?: string; percent?: number }
      const status = snapshot.status
      if (!status?.activeJob || status.activeJob.jobId !== event.job_id) return
      emit({
        status: {
          ...status,
          activeJob: {
            ...status.activeJob,
            phase: event.phase ?? status.activeJob.phase,
            percent: event.percent ?? status.activeJob.percent,
          },
        },
      })
    }),
  ]
  cleanupEvents = () => unlisten.forEach((off) => off())
  void (async () => {
    const [status, policy] = await Promise.all([
      refreshStatus().catch((error) => {
        logger.warn("updater", "serverUpdater::bind", "initial server update status failed", error)
        return null
      }),
      refreshAutoUpdatePolicy().catch((error) => {
        logger.warn("updater", "serverUpdater::bind", "initial server update policy failed", error)
        return null
      }),
    ])
    // Establish an initial snapshot only when the remote server permits
    // automatic checks. Manual checks remain available from the panel.
    if (boundTransport === transport && status && !status.snapshot && policy?.checkEnabled) {
      await refreshStatus(true).catch((error) => {
        logger.warn("updater", "serverUpdater::bind", "initial server update check failed", error)
      })
    }
  })()
}

export function useServerUpdateStore(): StoreSnapshot {
  const revision = useTransportRevision()
  useEffect(() => bindToActiveTransport(), [revision])
  return useSyncExternalStore(subscribe, getSnapshot, getSnapshot)
}

export async function checkServerUpdate(): Promise<ServerUpdateStatus | null> {
  bindToActiveTransport()
  return refreshStatus(true)
}

export async function prepareServerUpdate(): Promise<ServerInstallPlan> {
  bindToActiveTransport()
  const transport = boundTransport
  const current = snapshot.status
  const available = current?.snapshot
  if (!transport || !current || !available?.hasUpdate) {
    throw new Error("No server update is available")
  }
  // Manual/Docker paths are informational only. The durable status already
  // carries the server-selected capability, so showing its instructions does
  // not need to create an executable plan (or require update authority).
  if (available.capability !== "automatic") {
    return {
      planId: null,
      serverInstanceId: current.serverInstanceId,
      currentVersion: current.currentVersion,
      targetVersion: available.latestVersion,
      path: available.recommendedPath,
      capability: available.capability,
      expiresAt: null,
      confirmation: available.manualInstructions ?? "",
      manualInstructions: available.manualInstructions,
    }
  }
  try {
    return await transport.requestAppUpdate<ServerInstallPlan>("/api/app-update/prepare", {
      method: "POST",
      body: {
        currentVersion: current.currentVersion,
        targetVersion: available.latestVersion,
        serverInstanceId: current.serverInstanceId,
      },
    })
  } catch (error) {
    emit({ error: describeError(error) })
    throw error
  }
}

export async function confirmServerUpdate(planId: string): Promise<ServerUpdateJob> {
  const transport = boundTransport
  if (!transport) throw new Error("Server connection changed")
  let job: ServerUpdateJob
  try {
    job = await transport.requestAppUpdate<ServerUpdateJob>("/api/app-update/confirm", {
      method: "POST",
      body: { planId },
    })
  } catch (error) {
    emit({ error: describeError(error) })
    throw error
  }
  const current = snapshot.status
  if (current) emit({ status: { ...current, activeJob: job } })
  void verifyRestart(transport, job.targetVersion)
  return job
}

async function verifyRestart(transport: HttpTransport, targetVersion: string) {
  const deadline = Date.now() + 2 * 60_000
  while (Date.now() < deadline && boundTransport === transport) {
    try {
      const health = await transport.probeHealth(AbortSignal.timeout(5_000))
      if (health.version.replace(/^v/, "") === targetVersion.replace(/^v/, "")) {
        await refreshStatus().catch(() => null)
        const sameOrigin =
          typeof window !== "undefined" &&
          new URL(transport.getBaseUrl()).origin === window.location.origin
        if (!isTauriMode() && sameOrigin) {
          window.setTimeout(() => window.location.reload(), 600)
        }
        return
      }
      const durable = await refreshStatus().catch(() => null)
      if (durable?.recentJobs[0]?.status === "failed") return
    } catch {
      // Expected while the service manager replaces/restarts the process.
    }
    await new Promise((resolve) => window.setTimeout(resolve, 1500))
  }
  if (boundTransport === transport) {
    emit({ error: `Server did not come back on version ${targetVersion}` })
    await refreshStatus().catch(() => null)
  }
}
