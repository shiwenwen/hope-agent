/**
 * Quick Import for third-party skill catalogs.
 *
 * Probes known locations on the user's machine — Claude Code user-level
 * skills, Claude Code plugins, the Anthropic Agent Skills marketplace,
 * and OpenClaw / Hermes Agent clones — and lets the user one-click each
 * found path into [`extra_skills_dirs`](../../../crates/ha-core/src/config/mod.rs).
 *
 * Read-only discovery; the actual write reuses `add_extra_skills_dir`.
 */

import { useCallback, useEffect, useState } from "react"
import { useTranslation } from "react-i18next"
import { Loader2, FolderOpen, Check, AlertTriangle, X } from "lucide-react"
import { toast } from "sonner"

import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { Button } from "@/components/ui/button"
import { IconTip } from "@/components/ui/tooltip"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import type { PresetSkillSource } from "../types"

interface Props {
  open: boolean
  onClose: () => void
  onImported: () => void
}

export default function QuickImportDialog({ open, onClose, onImported }: Props) {
  const { t } = useTranslation()
  const [sources, setSources] = useState<PresetSkillSource[] | null>(null)
  const [loading, setLoading] = useState(false)
  const [importingPath, setImportingPath] = useState<string | null>(null)

  const refresh = useCallback(async () => {
    setLoading(true)
    try {
      const list = await getTransport().call<PresetSkillSource[]>(
        "discover_preset_skill_sources",
      )
      setSources(list)
    } catch (e) {
      logger.error("settings", "QuickImportDialog::discover", "discover failed", e)
      toast.error(t("settings.skillsImport.error.discover"))
    } finally {
      setLoading(false)
    }
  }, [t])

  useEffect(() => {
    if (!open) return
    void refresh()
  }, [open, refresh])

  async function handleImport(path: string) {
    setImportingPath(path)
    try {
      await getTransport().call("add_extra_skills_dir", { dir: path })
      onImported()
      await refresh()
    } catch (e) {
      logger.error("settings", "QuickImportDialog::import", "Failed to add dir", e)
      toast.error(t("settings.skillsImport.error.add"))
    } finally {
      setImportingPath(null)
    }
  }

  return (
    <Dialog open={open} onOpenChange={(v) => !v && onClose()}>
      <DialogContent className="max-w-2xl">
        <DialogHeader>
          <DialogTitle>{t("settings.skillsImport.dialogTitle")}</DialogTitle>
          <DialogDescription>{t("settings.skillsImport.dialogDescription")}</DialogDescription>
        </DialogHeader>

        <div className="max-h-[60vh] overflow-y-auto space-y-4 py-1">
          {loading && (
            <div className="flex items-center justify-center py-12">
              <Loader2 className="h-5 w-5 animate-spin text-muted-foreground" />
            </div>
          )}

          {!loading &&
            sources?.map((source) => {
              const found = source.candidates.filter((c) => c.exists)
              return (
                <div key={source.id} className="rounded-lg border border-border p-3">
                  <div className="flex items-center gap-2 mb-2">
                    <span className="text-sm font-medium text-foreground">
                      {t(source.labelKey)}
                    </span>
                    {source.warningKey && (
                      <IconTip label={t(source.warningKey)}>
                        <AlertTriangle className="h-3.5 w-3.5 text-orange-400 shrink-0" />
                      </IconTip>
                    )}
                  </div>

                  {found.length === 0 ? (
                    <div className="text-xs text-muted-foreground italic px-1">
                      {t("settings.skillsImport.notFound")}
                    </div>
                  ) : (
                    <div className="space-y-1">
                      {found.map((c) => {
                        const busy = importingPath === c.path
                        return (
                          <div
                            key={c.path}
                            className="flex items-center gap-2 px-2 py-1.5 rounded bg-secondary/30 text-xs"
                          >
                            <FolderOpen className="h-3.5 w-3.5 text-muted-foreground shrink-0" />
                            <code
                              className="flex-1 truncate text-foreground/80"
                              title={c.path}
                            >
                              {c.path}
                            </code>
                            <span className="text-[10px] px-1.5 py-0.5 rounded bg-secondary text-muted-foreground font-medium shrink-0">
                              {t("settings.skillsImport.skillCount", {
                                count: c.skillCount,
                              })}
                            </span>
                            {c.alreadyAdded ? (
                              <span className="flex items-center gap-1 text-[10px] text-green-600 shrink-0 px-1.5">
                                <Check className="h-3 w-3" />
                                {t("settings.skillsImport.alreadyAdded")}
                              </span>
                            ) : (
                              <Button
                                size="sm"
                                variant="ghost"
                                className="h-6 px-2 text-xs"
                                disabled={busy}
                                onClick={() => handleImport(c.path)}
                              >
                                {busy ? (
                                  <Loader2 className="h-3 w-3 animate-spin" />
                                ) : (
                                  t("settings.skillsImport.add")
                                )}
                              </Button>
                            )}
                          </div>
                        )
                      })}
                    </div>
                  )}
                </div>
              )
            })}
        </div>

        <DialogFooter>
          <Button variant="outline" onClick={onClose}>
            <X className="h-4 w-4 mr-1.5" />
            {t("common.close")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
