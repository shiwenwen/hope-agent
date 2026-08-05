import { useState } from "react"
import { useTranslation } from "react-i18next"
import { Download, Loader2, RefreshCw, Server, X } from "lucide-react"
import MarkdownRenderer from "@/components/common/MarkdownRenderer"
import { Button } from "@/components/ui/button"
import { Progress } from "@/components/ui/progress"
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog"
import {
  checkServerUpdate,
  confirmServerUpdate,
  prepareServerUpdate,
  useServerUpdateStore,
  type ServerInstallPlan,
} from "@/lib/serverUpdater"

export default function ServerUpdateNotice({
  variant = "notice",
}: {
  variant?: "notice" | "panel"
}) {
  const { t } = useTranslation()
  const { target, status, autoUpdatePolicy, checking, error } = useServerUpdateStore()
  const [dismissed, setDismissed] = useState<string | null>(null)
  const [plan, setPlan] = useState<ServerInstallPlan | null>(null)
  const [preparing, setPreparing] = useState(false)
  const [confirming, setConfirming] = useState(false)
  const available = status?.snapshot
  const job = status?.activeJob
  const latestJob = status?.recentJobs[0]
  const failedJob = latestJob?.status === "failed" ? latestJob : undefined
  const identity = target
    ? failedJob
      ? `${target}|failed|${failedJob.jobId}`
      : available
        ? `${target}|${available.latestVersion}`
        : target
    : null

  if (!target) return null
  if (variant === "notice") {
    if (autoUpdatePolicy?.notify !== true) return null
    if (!job && ((!available?.hasUpdate && !failedJob) || dismissed === identity)) return null
  }

  async function prepare() {
    setPreparing(true)
    try {
      setPlan(await prepareServerUpdate())
    } catch {
      // The shared store renders the server's structured error below the card.
    } finally {
      setPreparing(false)
    }
  }

  async function confirm() {
    if (!plan?.planId) return
    setConfirming(true)
    try {
      await confirmServerUpdate(plan.planId)
      setPlan(null)
    } catch {
      // Keep the confirmation open so the user can review/retry.
    } finally {
      setConfirming(false)
    }
  }

  const manual = plan && plan.capability !== "automatic"
  const confirmationText = plan
    ? plan.capability === "docker_redeploy"
      ? t("askUserDialog.appUpdate.manual.dockerText", {
          from: plan.currentVersion,
          to: plan.targetVersion,
        })
      : manual
        ? t("askUserDialog.appUpdate.manual.text", {
            from: plan.currentVersion,
            to: plan.targetVersion,
            installSource: available?.installSource,
          })
        : t(
            plan.path === "package_manager"
              ? "askUserDialog.appUpdate.install.text.packageManager"
              : "askUserDialog.appUpdate.install.text.selfContained",
            {
              from: plan.currentVersion,
              to: plan.targetVersion,
              installSource: available?.installSource,
              notes: available?.notes ? `\n\n${available.notes}` : "",
            },
          )
    : ""

  return (
    <>
      <div
        className={
          variant === "notice"
            ? "fixed bottom-6 right-6 z-50 w-[380px] animate-in slide-in-from-bottom-5 fade-in duration-300"
            : "w-full"
        }
      >
        <div
          className={
            variant === "notice"
              ? "relative rounded-2xl border border-sky-500/25 bg-card p-4 shadow-xl dark:bg-zinc-900/95"
              : "relative rounded-[24px] border border-sky-500/20 bg-card px-6 py-5"
          }
        >
          {variant === "notice" && !job && (
            <button
              type="button"
              aria-label={t("common.close")}
              className="absolute right-3 top-3 rounded-lg p-1.5 text-muted-foreground transition-colors hover:bg-secondary hover:text-foreground"
              onClick={() => setDismissed(identity)}
            >
              <X className="h-3.5 w-3.5" />
            </button>
          )}
          <div className="flex items-start gap-3.5 pr-5">
            <div className="mt-0.5 flex h-10 w-10 shrink-0 items-center justify-center rounded-xl bg-sky-500/10 text-sky-600 dark:text-sky-400">
              <Server className="h-5 w-5" />
            </div>
            <div className="min-w-0 flex-1">
              <p className="text-xs font-medium uppercase tracking-wide text-sky-600 dark:text-sky-400">
                Hope Agent Server · {new URL(target).host}
              </p>
              <p className="mt-1 text-sm font-semibold text-foreground">
                {failedJob && !job
                  ? t("about.updateInstallFailed")
                  : job
                    ? job.status === "awaiting_restart"
                      ? t("about.updateInstalled")
                      : t("about.updateInstalling", { version: job.targetVersion })
                    : available?.hasUpdate
                      ? t("about.updateAvailable", { version: available.latestVersion })
                      : t("about.updateUpToDate", { version: status?.currentVersion })}
              </p>
              {job ? (
                <div className="mt-3">
                  <div className="flex justify-between text-[11px] text-muted-foreground">
                    <span>{job.phase}</span>
                    {job.percent != null && <span>{job.percent}%</span>}
                  </div>
                  <Progress
                    value={job.percent}
                    indeterminate={job.percent == null}
                    className="mt-1.5 bg-sky-500/15 [&>div]:bg-sky-500"
                  />
                </div>
              ) : available?.hasUpdate ? (
                <>
                  {available?.notes && (
                    <div className="update-notes-markdown mt-2 max-h-28 overflow-auto text-xs leading-relaxed text-muted-foreground">
                      <MarkdownRenderer content={available.notes} />
                    </div>
                  )}
                  <div className="mt-4 flex justify-end gap-2">
                    <Button
                      size="sm"
                      variant="ghost"
                      className="h-8 rounded-full text-xs"
                      disabled={checking}
                      onClick={() => void checkServerUpdate().catch(() => undefined)}
                    >
                      <RefreshCw
                        className={`mr-1.5 h-3.5 w-3.5 ${checking ? "animate-spin" : ""}`}
                      />
                      {t("about.updateCheck")}
                    </Button>
                    <Button
                      size="sm"
                      className="h-8 rounded-full bg-sky-600 px-4 text-xs text-white hover:bg-sky-700"
                      disabled={preparing}
                      onClick={() => void prepare()}
                    >
                      {preparing ? (
                        <Loader2 className="mr-1.5 h-3.5 w-3.5 animate-spin" />
                      ) : (
                        <Download className="mr-1.5 h-3.5 w-3.5" />
                      )}
                      {t("about.updateShort")}
                    </Button>
                  </div>
                </>
              ) : (
                <div className="mt-4 flex justify-end">
                  <Button
                    size="sm"
                    variant="outline"
                    className="h-8 rounded-full text-xs"
                    disabled={checking}
                    onClick={() => void checkServerUpdate().catch(() => undefined)}
                  >
                    <RefreshCw className={`mr-1.5 h-3.5 w-3.5 ${checking ? "animate-spin" : ""}`} />
                    {checking ? t("about.updateChecking") : t("about.updateCheck")}
                  </Button>
                </div>
              )}
              {failedJob?.error && !job && (
                <p className="mt-2 break-words text-[11px] text-destructive">{failedJob.error}</p>
              )}
              {error && <p className="mt-2 break-words text-[11px] text-destructive">{error}</p>}
            </div>
          </div>
        </div>
      </div>

      <AlertDialog open={Boolean(plan)} onOpenChange={(open) => !open && setPlan(null)}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>
              {t("askUserDialog.appUpdate.install.header", {
                from: plan?.currentVersion,
                to: plan?.targetVersion,
              })}
            </AlertDialogTitle>
            <AlertDialogDescription className="whitespace-pre-wrap">
              {confirmationText}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t("common.cancel")}</AlertDialogCancel>
            {!manual && (
              <AlertDialogAction
                disabled={confirming}
                onClick={(event) => {
                  event.preventDefault()
                  void confirm()
                }}
              >
                {confirming && <Loader2 className="mr-2 h-4 w-4 animate-spin" />}
                {t("askUserDialog.appUpdate.install.confirm")}
              </AlertDialogAction>
            )}
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </>
  )
}
