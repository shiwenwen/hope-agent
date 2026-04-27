import { useState } from "react"
import { useTranslation } from "react-i18next"
import { Brain, Loader2, Settings } from "lucide-react"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { Button } from "@/components/ui/button"
import { cn } from "@/lib/utils"
import {
  embeddingProviderLabel,
  openEmbeddingModelSettings,
  type EmbeddingModelConfig,
} from "@/types/embedding-models"

interface EmbeddingActivationDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  embeddingModels: EmbeddingModelConfig[]
  /** Returns whether activation succeeded (close on success). */
  onConfirm: (modelConfigId: string) => Promise<boolean>
}

/**
 * Shown when the user toggles vector search ON without a remembered embedding
 * model. The first selection both enables vector search AND spawns the initial
 * reembed job — there's no separate "save" step that strands the user in a
 * half-on state. If no embedding models are configured at all the dialog
 * collapses to a single "go configure" CTA.
 */
export default function EmbeddingActivationDialog({
  open,
  onOpenChange,
  embeddingModels,
  onConfirm,
}: EmbeddingActivationDialogProps) {
  const { t } = useTranslation()
  const [selectedId, setSelectedId] = useState<string | null>(null)
  const [submitting, setSubmitting] = useState(false)

  const isEmpty = embeddingModels.length === 0

  async function handleConfirm() {
    if (!selectedId || submitting) return
    setSubmitting(true)
    try {
      const success = await onConfirm(selectedId)
      if (success) {
        setSelectedId(null)
        onOpenChange(false)
      }
    } finally {
      setSubmitting(false)
    }
  }

  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        if (!next) setSelectedId(null)
        onOpenChange(next)
      }}
    >
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>{t("settings.embeddingActivation.title")}</DialogTitle>
          <DialogDescription>
            {isEmpty
              ? t("settings.embeddingActivation.empty")
              : t("settings.embeddingActivation.description")}
          </DialogDescription>
        </DialogHeader>

        {isEmpty ? (
          <div className="flex justify-center py-4">
            <Button
              variant="default"
              size="sm"
              onClick={() => {
                openEmbeddingModelSettings()
                onOpenChange(false)
              }}
            >
              <Settings className="mr-1.5 h-3.5 w-3.5" />
              {t("settings.embeddingActivation.gotoConfig")}
            </Button>
          </div>
        ) : (
          <div className="max-h-[40vh] space-y-1.5 overflow-y-auto">
            {embeddingModels.map((model) => {
              const active = selectedId === model.id
              return (
                <button
                  key={model.id}
                  type="button"
                  onClick={() => setSelectedId(model.id)}
                  className={cn(
                    "flex w-full items-start gap-3 rounded-lg border px-3 py-2.5 text-left transition-colors",
                    active
                      ? "border-primary bg-primary/10 text-foreground"
                      : "border-border hover:bg-secondary",
                  )}
                >
                  <Brain
                    className={cn(
                      "mt-0.5 h-4 w-4 shrink-0",
                      active ? "text-primary" : "text-muted-foreground",
                    )}
                  />
                  <div className="min-w-0 flex-1">
                    <div className="text-sm font-medium">{model.name}</div>
                    <div className="mt-0.5 text-xs text-muted-foreground">
                      {embeddingProviderLabel(model)}
                      {model.apiModel ? ` · ${model.apiModel}` : ""}
                      {model.apiDimensions ? ` · ${model.apiDimensions}d` : ""}
                    </div>
                  </div>
                </button>
              )
            })}
          </div>
        )}

        <DialogFooter>
          <Button
            variant="outline"
            size="sm"
            onClick={() => onOpenChange(false)}
            disabled={submitting}
          >
            {t("common.cancel")}
          </Button>
          {!isEmpty && (
            <Button
              size="sm"
              onClick={() => void handleConfirm()}
              disabled={!selectedId || submitting}
            >
              {submitting && <Loader2 className="mr-1.5 h-3.5 w-3.5 animate-spin" />}
              {t("settings.embeddingActivation.confirm")}
            </Button>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
