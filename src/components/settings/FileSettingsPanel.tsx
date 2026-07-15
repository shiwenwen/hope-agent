import { useEffect, useMemo, useState } from "react"
import { useTranslation } from "react-i18next"
import { AlertTriangle, Check, Loader2, RotateCcw } from "lucide-react"
import { toast } from "sonner"

import { Button } from "@/components/ui/button"
import { DeferredNumberInput } from "@/components/ui/deferred-number-input"
import {
  DEFAULT_FILESYSTEM_CONFIG,
  MAX_MAX_CHAT_ATTACHMENT_MB,
  MAX_MAX_DOCUMENT_PREVIEW_MB,
  MAX_MAX_TEXT_EDIT_MB,
  MAX_MAX_TEXT_PREVIEW_MB,
  MAX_MAX_WORKSPACE_UPLOAD_MB,
  MIN_MAX_CHAT_ATTACHMENT_MB,
  MIN_MAX_DOCUMENT_PREVIEW_MB,
  MIN_MAX_TEXT_EDIT_MB,
  MIN_MAX_TEXT_PREVIEW_MB,
  MIN_MAX_WORKSPACE_UPLOAD_MB,
  normalizeFilesystemConfig,
  patchFilesystemConfig,
  useFilesystemConfig,
  type FilesystemConfig,
} from "@/lib/filesystemConfig"
import { useTransport } from "@/lib/transport-provider"

type LimitKey = Exclude<keyof FilesystemConfig, "allowRemoteWrites">

const LIMITS: Array<{
  key: LimitKey
  label: string
  description: string
  min: number
  max: number
}> = [
  {
    key: "maxChatAttachmentMb",
    label: "settings.files.maxChatAttachment",
    description: "settings.files.maxChatAttachmentDesc",
    min: MIN_MAX_CHAT_ATTACHMENT_MB,
    max: MAX_MAX_CHAT_ATTACHMENT_MB,
  },
  {
    key: "maxWorkspaceUploadMb",
    label: "settings.files.maxWorkspaceUpload",
    description: "settings.files.maxWorkspaceUploadDesc",
    min: MIN_MAX_WORKSPACE_UPLOAD_MB,
    max: MAX_MAX_WORKSPACE_UPLOAD_MB,
  },
  {
    key: "maxTextPreviewMb",
    label: "settings.files.maxTextPreview",
    description: "settings.files.maxTextPreviewDesc",
    min: MIN_MAX_TEXT_PREVIEW_MB,
    max: MAX_MAX_TEXT_PREVIEW_MB,
  },
  {
    key: "maxTextEditMb",
    label: "settings.files.maxTextEdit",
    description: "settings.files.maxTextEditDesc",
    min: MIN_MAX_TEXT_EDIT_MB,
    max: MAX_MAX_TEXT_EDIT_MB,
  },
  {
    key: "maxDocumentPreviewMb",
    label: "settings.files.maxDocumentPreview",
    description: "settings.files.maxDocumentPreviewDesc",
    min: MIN_MAX_DOCUMENT_PREVIEW_MB,
    max: MAX_MAX_DOCUMENT_PREVIEW_MB,
  },
]

export default function FileSettingsPanel() {
  const { t } = useTranslation()
  const transport = useTransport()
  const { config, loading } = useFilesystemConfig()
  const [draft, setDraft] = useState(config)
  const [saving, setSaving] = useState(false)
  const [saved, setSaved] = useState(false)

  useEffect(() => setDraft(config), [config])
  const normalized = useMemo(() => normalizeFilesystemConfig(draft), [draft])
  const dirty = JSON.stringify(normalized) !== JSON.stringify(config)
  const largeUpload =
    normalized.maxChatAttachmentMb > 100 || normalized.maxWorkspaceUploadMb > 100
  const runtime = transport.fileRuntime()

  const save = async () => {
    setSaving(true)
    try {
      const next = await patchFilesystemConfig(transport, {
        maxChatAttachmentMb: normalized.maxChatAttachmentMb,
        maxWorkspaceUploadMb: normalized.maxWorkspaceUploadMb,
        maxTextPreviewMb: normalized.maxTextPreviewMb,
        maxTextEditMb: normalized.maxTextEditMb,
        maxDocumentPreviewMb: normalized.maxDocumentPreviewMb,
      })
      setDraft(next)
      setSaved(true)
      window.setTimeout(() => setSaved(false), 1800)
    } catch (error) {
      toast.error(error instanceof Error ? error.message : String(error))
    } finally {
      setSaving(false)
    }
  }

  const reset = () => {
    setDraft({ ...DEFAULT_FILESYSTEM_CONFIG, allowRemoteWrites: config.allowRemoteWrites })
  }

  return (
    <div className="flex-1 overflow-y-auto p-6">
      <div className="w-full space-y-6">
        <div>
          <h2 className="mb-1 text-lg font-semibold">{t("settings.files.title")}</h2>
          <p className="text-xs text-muted-foreground">{t("settings.files.description")}</p>
        </div>

        <div className="rounded-lg border border-border bg-muted/20 px-3 py-2.5">
          <div className="text-sm font-medium">
            {runtime.workspaceHost === "local"
              ? t("settings.files.runtimeLocal")
              : t("settings.files.runtimeServer")}
          </div>
          <p className="mt-0.5 text-xs text-muted-foreground">
            {runtime.workspaceHost === "local"
              ? t("settings.files.runtimeLocalDesc")
              : t("settings.files.runtimeServerDesc")}
          </p>
        </div>

        <div className="divide-y divide-border rounded-lg border border-border">
          {LIMITS.map((limit) => (
            <div key={limit.key} className="space-y-1.5 px-3 py-3">
              <div className="flex items-center justify-between gap-4">
                <span className="text-sm font-medium">{t(limit.label)}</span>
                <div className="flex items-center gap-2">
                  <DeferredNumberInput
                    min={limit.min}
                    max={limit.key === "maxTextEditMb" ? normalized.maxTextPreviewMb : limit.max}
                    value={normalized[limit.key]}
                    disabled={loading || saving}
                    onValueCommit={(value) =>
                      setDraft((current) =>
                        normalizeFilesystemConfig({ ...current, [limit.key]: value }),
                      )
                    }
                    className="h-8 w-24 text-sm"
                  />
                  <span className="w-7 text-xs text-muted-foreground">MiB</span>
                </div>
              </div>
              <p className="text-xs text-muted-foreground">
                {t(limit.description, { min: limit.min, max: limit.max })}
              </p>
            </div>
          ))}
        </div>

        {largeUpload && (
          <div className="flex gap-2 rounded-lg border border-amber-500/30 bg-amber-500/5 px-3 py-2.5 text-xs text-amber-700 dark:text-amber-300">
            <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0" />
            <span>{t("settings.files.largeUploadWarning")}</span>
          </div>
        )}

        <div className="flex justify-end gap-2">
          <Button variant="outline" size="sm" onClick={reset} disabled={saving}>
            <RotateCcw className="mr-1.5 h-3.5 w-3.5" />
            {t("common.restoreDefaults", "Restore defaults")}
          </Button>
          <Button size="sm" onClick={() => void save()} disabled={!dirty || saving}>
            {saving ? (
              <Loader2 className="mr-1.5 h-3.5 w-3.5 animate-spin" />
            ) : saved ? (
              <Check className="mr-1.5 h-3.5 w-3.5" />
            ) : null}
            {saved ? t("common.saved") : t("common.save")}
          </Button>
        </div>
      </div>
    </div>
  )
}
