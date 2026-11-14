import { useEffect, useRef, useState } from "react"
import { useTranslation } from "react-i18next"
import { Loader2, RefreshCw } from "lucide-react"
import { toast } from "sonner"
import { Button } from "@/components/ui/button"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import type { DesignBaselineRevision, DesignQualityRun } from "@/types/design"

type Finding = { lens: string; severity: string; message: string }
type BaselineState = { revisionId: string | null; revisions: DesignBaselineRevision[] }
type Diff = DesignQualityRun["diffs"][number]

/** Mounted only for one artifact/connection and one open review dialog. */
export function DesignQualityReview({
  artifactId,
  onClose,
}: {
  artifactId: string
  onClose: () => void
}) {
  const { t } = useTranslation()
  const tx = getTransport()
  const scopeRef = useRef<{ transport: typeof tx } | null>(null)
  const [attempt, setAttempt] = useState(0)
  const [loading, setLoading] = useState(true)
  const [busy, setBusy] = useState(false)
  const [run, setRun] = useState<DesignQualityRun | null>(null)
  const [findings, setFindings] = useState<Finding[] | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [selected, setSelected] = useState<number | null>(null)
  const [baseline, setBaseline] = useState<BaselineState>({ revisionId: null, revisions: [] })
  const [changed, setChanged] = useState(false)

  useEffect(() => {
    const scope = { transport: tx }
    scopeRef.current = scope
    setLoading(true)
    setRun(null)
    setFindings(null)
    setError(null)
    setSelected(null)
    setChanged(false)
    setBaseline({ revisionId: null, revisions: [] })
    void Promise.allSettled([
      tx.call<Finding[]>("review_design_artifact_cmd", { id: artifactId }),
      tx.call<DesignQualityRun>("run_design_visual_regression_cmd", { artifactId }),
    ]).then(([staticResult, visualResult]) => {
      if (scopeRef.current !== scope || getTransport() !== tx) return
      if (staticResult.status === "fulfilled") setFindings(staticResult.value)
      if (visualResult.status === "fulfilled" && visualResult.value.artifactId === artifactId) {
        setRun(visualResult.value)
        setBaseline({
          revisionId: visualResult.value.baselineRevision,
          revisions: visualResult.value.baselineRevisions,
        })
      } else {
        setError(
          String(
            visualResult.status === "rejected" ? visualResult.reason : "artifact identity mismatch",
          ).includes("visual_scenarios_incomplete")
            ? "design.review.visualScenarioIncomplete"
            : "design.review.visualUnavailable",
        )
      }
      if (staticResult.status === "rejected") setError("design.err.load")
      setLoading(false)
    })
    return () => {
      scopeRef.current = null
    }
  }, [artifactId, tx, attempt])

  const mutate = async (revisionId?: string) => {
    const scope = scopeRef.current
    if (
      !run ||
      !scope ||
      scope.transport !== tx ||
      busy ||
      loading ||
      getTransport() !== tx ||
      (!revisionId && changed)
    )
      return
    const isCurrent = () => scopeRef.current === scope && getTransport() === tx
    setBusy(true)
    try {
      const value = revisionId
        ? await tx.call<BaselineState>("restore_design_visual_baseline_cmd", {
            artifactId,
            input: { artifactId, revisionId, expectedRevisionId: baseline.revisionId },
          })
        : await tx.call<BaselineState>("accept_design_visual_baseline_cmd", {
            input: {
              artifactId,
              reviewId: run.reviewId,
              expectedArtifactHash: run.artifactHash,
              expectedScenariosHash: run.scenariosHash,
              expectedBaselineRevision: run.baselineRevision,
            },
          })
      if (!isCurrent()) return
      setBaseline(value)
      setChanged(true)
      toast.success(
        t(revisionId ? "design.review.baselineRestored" : "design.review.baselineAccepted"),
      )
    } catch (e) {
      if (!isCurrent()) return
      logger.warn("design", "DesignQualityReview::baseline", "baseline operation failed", e)
      toast.error(t("design.review.baselineFailed"))
    } finally {
      if (isCurrent()) setBusy(false)
    }
  }

  const close = () => {
    scopeRef.current = null
    onClose()
  }
  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!open) close()
      }}
    >
      <DialogContent className="max-h-[90vh] max-w-3xl overflow-y-auto">
        <DialogHeader>
          <DialogTitle>{t("design.review.qualityTitle", "质量审查")}</DialogTitle>
          <DialogDescription>
            {t("design.review.captureHint", "查看本轮保存的截图；接受时使用同一批画面。")}
          </DialogDescription>
        </DialogHeader>
        {loading && (
          <p role="status" className="flex items-center gap-2 text-sm">
            <Loader2 className="h-4 w-4 animate-spin" />
            {t("common.loading", "加载中")}
          </p>
        )}
        {error && (
          <p role="alert" className="text-sm text-destructive">
            {t(error)}
          </p>
        )}
        {run && (
          <div className="space-y-3">
            <p className="text-xs text-muted-foreground">
              {run.deterministicPassed
                ? t("design.review.visualPassed")
                : t("design.review.visualNeedsBaseline")}
            </p>
            <div
              className="grid grid-cols-2 gap-2 sm:grid-cols-3"
              aria-label={t("design.review.visualTitle")}
            >
              {run.diffs.map((diff, index) => (
                <Button
                  key={`${diff.scenarioId}:${diff.viewport.width}x${diff.viewport.height}`}
                  variant="outline"
                  className="h-auto flex-col items-start px-2 py-1.5 text-xs"
                  aria-pressed={selected === index}
                  onClick={() => setSelected(index)}
                >
                  <span className="max-w-full truncate">{diff.scenarioTitle}</span>
                  <span>
                    {diff.viewport.width}×{diff.viewport.height} ·{" "}
                    {diff.changedRatio == null
                      ? t("design.review.noBaseline")
                      : `${(diff.changedRatio * 100).toFixed(2)}%`}
                  </span>
                </Button>
              ))}
            </div>
            {selected !== null && (
              <CapturePair
                key={`${run.reviewId}:${selected}`}
                artifactId={artifactId}
                run={run}
                diff={run.diffs[selected]}
              />
            )}
            <p className="break-all text-[10px] text-muted-foreground">
              {t("design.review.captureVersion", "截图版本")} {run.artifactHash.slice(0, 12)} ·{" "}
              {run.reviewId}
            </p>
            <div className="flex flex-wrap gap-2">
              <Button
                size="sm"
                disabled={busy || loading || changed || !run.reviewId}
                onClick={() => void mutate()}
              >
                {t("design.review.acceptBaseline")}
              </Button>
              <Button
                size="sm"
                variant="outline"
                disabled={busy || loading}
                onClick={() => setAttempt((value) => value + 1)}
              >
                <RefreshCw className="mr-1 h-3 w-3" />
                {t("design.review.rerun", "重新审查")}
              </Button>
            </div>
            {changed && (
              <p role="status" className="text-xs text-muted-foreground">
                {t("design.review.baselineChanged", "基线已更新；重新审查可比较新基线。")}
              </p>
            )}
            {baseline.revisions.some((r) => r.id !== baseline.revisionId) && (
              <div className="space-y-1">
                <p className="text-xs font-medium">
                  {t("design.review.baselineHistory", "已保留的基线")}
                </p>
                {baseline.revisions
                  .filter((r) => r.id !== baseline.revisionId)
                  .map((r) => (
                    <Button
                      key={r.id}
                      size="sm"
                      variant="ghost"
                      className="mr-1 text-xs"
                      disabled={busy || !baseline.revisionId}
                      onClick={() => void mutate(r.id)}
                    >
                      {t("design.review.restoreBaseline", "恢复基线")} ·{" "}
                      {new Date(r.createdAt).toLocaleString()} · {r.id.slice(0, 8)}
                    </Button>
                  ))}
              </div>
            )}
          </div>
        )}
        {findings && (
          <ul className="space-y-1 text-xs">
            {findings.length === 0 ? (
              <li>{t("design.review.allClear")}</li>
            ) : (
              findings.map((f, i) => (
                <li key={i} className="rounded border p-2">
                  {t(`design.review.lens.${f.lens}`, f.lens)} · {f.message}
                </li>
              ))
            )}
          </ul>
        )}
        <Button variant="ghost" onClick={close}>
          {t("common.close", "关闭")}
        </Button>
      </DialogContent>
    </Dialog>
  )
}

function CapturePair({
  artifactId,
  run,
  diff,
}: {
  artifactId: string
  run: DesignQualityRun
  diff: Diff
}) {
  const { t } = useTranslation()
  const tx = getTransport()
  const [images, setImages] = useState<{ current: string; baseline?: string } | null>(null)
  const [failed, setFailed] = useState(false)
  useEffect(() => {
    let alive = true
    const read = async (side: "current" | "baseline", expected: string) => {
      const value = await tx.call<{ dataUrl: string; imageHash: string }>(
        "get_design_quality_image_cmd",
        {
          artifactId,
          input: {
            artifactId,
            reviewId: run.reviewId,
            scenarioId: diff.scenarioId ?? "default",
            viewport: diff.viewport,
            side,
          },
        },
      )
      if (value.imageHash !== expected || !value.dataUrl.startsWith("data:image/png;base64,"))
        throw new Error("capture identity mismatch")
      return value.dataUrl
    }
    void Promise.all([
      read("current", diff.currentHash),
      diff.baselineHash ? read("baseline", diff.baselineHash) : Promise.resolve(undefined),
    ])
      .then(([current, baseline]) => {
        if (alive && getTransport() === tx) setImages({ current, baseline })
      })
      .catch(() => {
        if (alive && getTransport() === tx) setFailed(true)
      })
    return () => {
      alive = false
    }
  }, [artifactId, tx, run.reviewId, diff])
  if (failed)
    return (
      <p role="alert" className="text-xs text-destructive">
        {t("design.review.captureFailed", "截图不可用，请重新审查。")}
      </p>
    )
  if (!images)
    return (
      <p role="status" className="text-xs">
        {t("common.loading", "加载中")}
      </p>
    )
  return (
    <div className="grid grid-cols-2 gap-2">
      <figure>
        <figcaption className="mb-1 text-xs">
          {t("design.review.currentCapture", "本轮截图")}
        </figcaption>
        <img
          className="max-h-[40vh] w-full object-contain"
          src={images.current}
          alt={`${diff.scenarioTitle} ${t("design.review.currentCapture", "本轮截图")}`}
        />
      </figure>
      <figure>
        <figcaption className="mb-1 text-xs">
          {t("design.review.baselineCapture", "审查时基线")}
        </figcaption>
        {images.baseline ? (
          <img
            className="max-h-[40vh] w-full object-contain"
            src={images.baseline}
            alt={`${diff.scenarioTitle} ${t("design.review.baselineCapture", "审查时基线")}`}
          />
        ) : (
          <p className="text-xs text-muted-foreground">{t("design.review.noBaseline")}</p>
        )}
      </figure>
    </div>
  )
}
