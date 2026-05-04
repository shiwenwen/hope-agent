import { useCallback, useEffect, useState } from "react"
import { useTranslation } from "react-i18next"
import { AlertTriangle, Download, RefreshCw, Power, BellOff, Clock, X } from "lucide-react"
import { toast } from "sonner"

import { Button } from "@/components/ui/button"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { getTransport } from "@/lib/transport-provider"
import { parsePayload } from "@/lib/transport"
import { logger } from "@/lib/logger"
import {
  LOCAL_MODEL_ALERT_EVENT,
  type LocalModelMissingAlert,
  type MissingAlertAlternative,
} from "@/types/local-model-jobs"
import type { ModelCandidate } from "@/types/local-llm"
import type { OllamaEmbeddingModel } from "@/components/settings/memory-panel/types"

export default function MissingModelDialog() {
  const { t } = useTranslation()
  const [alert, setAlert] = useState<LocalModelMissingAlert | null>(null)

  useEffect(() => {
    const unlisten = getTransport().listen(LOCAL_MODEL_ALERT_EVENT, (raw: unknown) => {
      try {
        const payload = parsePayload<LocalModelMissingAlert>(raw)
        setAlert(payload)
      } catch (e) {
        logger.warn(
          "local-model",
          "MissingModelDialog::listen",
          "Failed to parse missing_alert payload",
          e,
        )
      }
    })
    return () => unlisten()
  }, [])

  const triggerWatchdog = useCallback(() => {
    void getTransport()
      .call("local_model_auto_maintenance_trigger")
      .catch((e) => {
        logger.warn("local-model", "trigger_watchdog", "trigger failed", e)
      })
  }, [])

  /**
   * Run a dialog action with consistent UX:
   * 1. await the side-effect
   * 2. if it succeeds → fire the optional success toast and run any
   *    follow-up; if it throws → show the error toast and keep the
   *    dialog open so the user can pick a different option
   * 3. close the dialog only on success
   *
   * Caller-supplied `errorKey` controls which i18n message wraps `{{error}}`
   * — different actions need different copy ("install failed" vs "switch
   * failed" etc.).
   */
  const runAction = useCallback(
    async (
      action: () => Promise<void>,
      opts: {
        errorKey: string
        successMessage?: string
        afterSuccess?: () => void
        closeOnError?: boolean
        logSource?: string
      },
    ) => {
      try {
        await action()
        if (opts.successMessage) toast.success(opts.successMessage)
        opts.afterSuccess?.()
        setAlert(null)
      } catch (e) {
        logger.warn("local-model", opts.logSource ?? "MissingModelDialog", "action failed", e)
        toast.error(t(opts.errorKey, { error: String(e) }))
        if (opts.closeOnError) setAlert(null)
      }
    },
    [t],
  )

  const handleRedownload = useCallback(() => {
    if (!alert) return
    void runAction(
      async () => {
        if (alert.kind === "chat") {
          // Use the full catalog rather than `local_llm_recommend_model`,
          // which filters by current hardware budget. A user who previously
          // installed a 27B/35B model on a beefier machine and migrated to
          // a smaller one would see canRedownload=true (catalog hit) but
          // the model would be missing from `recommend_model`'s output.
          const catalog = await getTransport().call<ModelCandidate[]>(
            "local_llm_chat_catalog",
          )
          const candidate = catalog.find((c) => c.id === alert.missingModelId)
          if (!candidate) {
            throw new Error(t("settings.localModelMaintenance.errors.candidateNotFound"))
          }
          await getTransport().call("local_model_job_start_chat_model", { model: candidate })
        } else {
          const list = await getTransport().call<OllamaEmbeddingModel[]>(
            "local_embedding_list_models",
          )
          const model = list.find((m) => m.id === alert.missingModelId)
          if (!model) {
            throw new Error(t("settings.localModelMaintenance.errors.candidateNotFound"))
          }
          await getTransport().call("local_model_job_start_embedding", { model })
        }
      },
      {
        errorKey: "settings.localModelMaintenance.errors.installFailed",
        successMessage: t("settings.localModelMaintenance.toast.redownloadStarted", {
          name: alert.missingDisplayName,
        }),
        logSource: "handleRedownload",
      },
    )
  }, [alert, runAction, t])

  const handleSwitchTo = useCallback(
    (alt: MissingAlertAlternative) => {
      if (!alert) return
      void runAction(
        async () => {
          if (alert.kind === "chat" && alt.providerId) {
            await getTransport().call("set_active_model", {
              providerId: alt.providerId,
              modelId: alt.modelId,
            })
          } else if (alert.kind === "embedding" && alt.embeddingConfigId) {
            await getTransport().call("memory_embedding_set_default", {
              modelConfigId: alt.embeddingConfigId,
              // ReembedMode is `#[serde(rename_all = "snake_case")]` on the
              // Rust side — the wire value must be snake_case.
              mode: "keep_existing",
            })
          } else {
            throw new Error("Alternative missing required ids")
          }
        },
        {
          errorKey: "settings.localModelMaintenance.errors.switchFailed",
          successMessage: t("settings.localModelMaintenance.toast.switched", {
            name: alt.displayName,
          }),
          afterSuccess: triggerWatchdog,
          logSource: "handleSwitchTo",
        },
      )
    },
    [alert, runAction, t, triggerWatchdog],
  )

  const handleDisableEmbedding = useCallback(() => {
    void runAction(
      () => getTransport().call("memory_embedding_disable").then(() => undefined),
      {
        errorKey: "settings.localModelMaintenance.errors.disableFailed",
        successMessage: t("settings.localModelMaintenance.toast.embeddingDisabled"),
        afterSuccess: triggerWatchdog,
        logSource: "handleDisableEmbedding",
      },
    )
  }, [runAction, t, triggerWatchdog])

  const handleDismissTemporary = useCallback(() => {
    if (!alert) return
    void runAction(
      () =>
        getTransport()
          .call("local_model_alert_dismiss_temporary", { modelId: alert.missingModelId })
          .then(() => undefined),
      {
        // Soft fail: dismiss errors don't block the user from picking a
        // different option, so close anyway.
        errorKey: "settings.localModelMaintenance.errors.disableFailed",
        closeOnError: true,
        logSource: "handleDismissTemporary",
      },
    )
  }, [alert, runAction])

  const handleSilenceSession = useCallback(() => {
    if (!alert) return
    void runAction(
      () =>
        getTransport()
          .call("local_model_alert_silence_session", { modelId: alert.missingModelId })
          .then(() => undefined),
      {
        errorKey: "settings.localModelMaintenance.errors.disableFailed",
        closeOnError: true,
        logSource: "handleSilenceSession",
      },
    )
  }, [alert, runAction])

  const handleDisableAutoMaintenance = useCallback(() => {
    void runAction(
      () => getTransport().call("local_model_auto_maintenance_disable").then(() => undefined),
      {
        errorKey: "settings.localModelMaintenance.errors.disableAutoMaintenanceFailed",
        successMessage: t("settings.localModelMaintenance.toast.autoMaintenanceDisabled"),
        logSource: "handleDisableAutoMaintenance",
      },
    )
  }, [runAction, t])

  if (!alert) return null

  const kindLabel =
    alert.kind === "chat"
      ? t("settings.localModelMaintenance.kindChat")
      : t("settings.localModelMaintenance.kindEmbedding")

  return (
    <Dialog open={!!alert} onOpenChange={(open) => !open && setAlert(null)}>
      <DialogContent className="max-w-lg">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <AlertTriangle className="size-5 text-amber-500" />
            {t("settings.localModelMaintenance.missingDialog.title", { kind: kindLabel })}
          </DialogTitle>
          <DialogDescription>
            {t("settings.localModelMaintenance.missingDialog.description", {
              name: alert.missingDisplayName,
            })}
          </DialogDescription>
        </DialogHeader>

        <div className="flex flex-col gap-2 py-2">
          {alert.canRedownload && (
            <Button variant="default" className="justify-start" onClick={handleRedownload}>
              <Download className="size-4" />
              {t("settings.localModelMaintenance.actions.redownload", {
                name: alert.missingDisplayName,
              })}
            </Button>
          )}
          {alert.alternatives.map((alt) => (
            <Button
              key={alt.modelId + (alt.providerId ?? "") + (alt.embeddingConfigId ?? "")}
              variant="outline"
              className="justify-start"
              onClick={() => handleSwitchTo(alt)}
            >
              <RefreshCw className="size-4" />
              {t("settings.localModelMaintenance.actions.switchTo", { name: alt.displayName })}
            </Button>
          ))}
          {alert.canDisableEmbedding && (
            <Button variant="outline" className="justify-start" onClick={handleDisableEmbedding}>
              <Power className="size-4" />
              {t("settings.localModelMaintenance.actions.disableEmbedding")}
            </Button>
          )}
        </div>

        <DialogFooter className="flex-wrap gap-1.5 sm:justify-end">
          <Button variant="ghost" size="sm" onClick={handleDismissTemporary}>
            <Clock className="size-3.5" />
            {t("settings.localModelMaintenance.actions.dismissTemporary")}
          </Button>
          <Button variant="ghost" size="sm" onClick={handleSilenceSession}>
            <X className="size-3.5" />
            {t("settings.localModelMaintenance.actions.silenceSession")}
          </Button>
          <Button variant="ghost" size="sm" onClick={handleDisableAutoMaintenance}>
            <BellOff className="size-3.5" />
            {t("settings.localModelMaintenance.actions.disableAutoMaintenance")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
