import type { TFunction } from "i18next"
import { formatBytes, formatDurationCompact } from "@/lib/format"

export interface JobTransferLineOptions {
  unit: "bytes" | "count"
  completed: number | null | undefined
  total: number | null | undefined
  speedBps?: number | null
  etaSeconds?: number | null
  t: TFunction
}

/**
 * 把 [`LocalModelJobSnapshot`] 的传输状态格式化成单行 `"X / Y · N/s · ≈ ETA"`。
 * `unit === "count"` 走 `embedding.reembedJob.{progress,perSecond}` 渲染「X / Y 条
 * 记忆 · N 条/秒」；`bytes` 走 `formatBytes(maxUnit:"GB")`。两个 formatter 内部
 * 共享相同的 `parts.join(" · ")` 串接 + ETA `formatDurationCompact` 逻辑，
 * `InstallProgressDialog` 与 `LocalModelsPanel.jobTransferSummary` 都消费此函数。
 *
 * 返回 `null` 表示完全没有进度数据（completed/total 都没有），调用方应不渲染此行。
 */
export function formatJobTransferLine(opts: JobTransferLineOptions): string | null {
  const { unit, completed, total, speedBps, etaSeconds, t } = opts
  if (completed == null && total == null) return null

  const parts: string[] = []

  if (completed != null && total != null) {
    if (unit === "count") {
      parts.push(t("settings.embedding.reembedJob.progress", { done: completed, total }))
    } else {
      parts.push(
        `${formatBytes(completed, { maxUnit: "GB" })} / ${formatBytes(total, { maxUnit: "GB" })}`,
      )
    }
  } else if (completed != null) {
    parts.push(unit === "count" ? String(completed) : formatBytes(completed, { maxUnit: "GB" }))
  }

  if (speedBps != null && speedBps > 0) {
    if (unit === "count") {
      parts.push(t("settings.embedding.reembedJob.perSecond", { count: Math.round(speedBps) }))
    } else {
      parts.push(`${formatBytes(speedBps, { maxUnit: "GB" })}/s`)
    }
  }

  if (etaSeconds != null && Number.isFinite(etaSeconds)) {
    parts.push(`≈ ${formatDurationCompact(etaSeconds)}`)
  }

  return parts.join(" · ")
}
