import { useCallback, useEffect, useState } from "react"
import { useTranslation } from "react-i18next"
import { Loader2, FolderOpen, Plus, Sparkles, X } from "lucide-react"

import { getTransport } from "@/lib/transport-provider"
import { isTauriMode } from "@/lib/transport"
import { logger } from "@/lib/logger"
import { cn } from "@/lib/utils"
import { IconTip } from "@/components/ui/tooltip"
import { Switch } from "@/components/ui/switch"
import QuickImportDialog from "@/components/settings/skills-panel/QuickImportDialog"
import ServerDirectoryBrowser from "@/components/chat/input/ServerDirectoryBrowser"
import { useDirectoryPicker } from "@/components/chat/input/useDirectoryPicker"

/**
 * Subset of `SkillSummary` returned by the `get_skills` Tauri/HTTP command.
 * We only need name / description / source / always here; everything else
 * (env status, tool restrictions, lifecycle, etc.) is handled by the full
 * Settings → Skills panel.
 */
interface SkillInfo {
  name: string
  description?: string | null
  source?: string | null
  always?: boolean
}

interface SkillsStepProps {
  /** Names (keys) of currently-disabled skills; wizard reassigns this on Next. */
  initialDisabled: string[]
  onChange: (disabled: string[]) => void
}

/**
 * Step 7 — allow-list bundled/imported skills and, optionally, import
 * extra skill directories exactly the way Settings → Skills does.
 *
 * `always: true` skills skip environment / dependency checks, but they are
 * still user-toggleable. Settings -> Skills and Agent skill filters apply the
 * same way, so the wizard should not create a separate "locked" model.
 */
export function SkillsStep({ initialDisabled, onChange }: SkillsStepProps) {
  const { t } = useTranslation()
  const [skills, setSkills] = useState<SkillInfo[] | null>(null)
  const [extraDirs, setExtraDirs] = useState<string[]>([])
  const [disabled, setDisabled] = useState<Set<string>>(new Set(initialDisabled))
  const [error, setError] = useState<string | null>(null)
  const [importing, setImporting] = useState(false)
  const [quickImportOpen, setQuickImportOpen] = useState(false)

  const reload = async () => {
    try {
      const [list, dirs] = await Promise.all([
        getTransport().call<SkillInfo[]>("get_skills"),
        getTransport()
          .call<string[]>("get_extra_skills_dirs")
          .catch(() => [] as string[]),
      ])
      const sorted = [...list].sort((a, b) => a.name.localeCompare(b.name))
      setSkills(sorted)
      setExtraDirs(dirs)
      setError(null)
    } catch (e) {
      logger.warn("onboarding", "SkillsStep", "get_skills failed", e)
      setError(String(e))
      setSkills([])
    }
  }

  useEffect(() => {
    void reload()
  }, [])

  useEffect(() => {
    onChange(Array.from(disabled))
  }, [disabled]) // eslint-disable-line react-hooks/exhaustive-deps

  function toggle(name: string) {
    setDisabled((prev) => {
      const next = new Set(prev)
      if (next.has(name)) next.delete(name)
      else next.add(name)
      return next
    })
  }

  const addExtraDir = useCallback(async (dir: string) => {
    setImporting(true)
    try {
      await getTransport().call("add_extra_skills_dir", { dir })
      await reload()
    } catch (e) {
      logger.error("onboarding", "SkillsStep::addDir", "failed to add skills dir", e)
      setError(String(e))
    } finally {
      setImporting(false)
    }
  }, [])

  const {
    pick: handleImportDir,
    browserOpen: dirBrowserOpen,
    setBrowserOpen: setDirBrowserOpen,
    handleBrowserSelect: handleDirBrowserSelect,
  } = useDirectoryPicker({
    onPicked: (path) => {
      void addExtraDir(path)
    },
    errorTitle: t("settings.skillsDirPickFailed"),
    loggerSource: "SkillsStep::pickExtraDir",
  })

  async function handleRemoveDir(dir: string) {
    try {
      await getTransport().call("remove_extra_skills_dir", { dir })
      await reload()
    } catch (e) {
      logger.error("onboarding", "SkillsStep::removeDir", "failed to remove skills dir", e)
      setError(String(e))
    }
  }

  return (
    <div className="px-6 py-6 space-y-4 max-w-2xl mx-auto">
      <div className="text-center space-y-1">
        <h2 className="text-xl font-semibold">{t("onboarding.skills.title")}</h2>
        <p className="text-sm text-muted-foreground">{t("onboarding.skills.subtitle")}</p>
      </div>

      {/* Directories — mirror Settings → Skills compact layout */}
      <div className="space-y-1.5">
        <div className="text-xs font-semibold text-muted-foreground uppercase tracking-wider">
          {t("settings.skillsDirs")}
        </div>
        <div className="flex items-center gap-2 px-3 py-2 rounded-md bg-secondary/30 text-xs">
          <FolderOpen className="h-3.5 w-3.5 text-muted-foreground shrink-0" />
          <code className="flex-1 text-foreground/80 truncate">~/.hope-agent/skills/</code>
          <span className="text-[10px] px-1.5 py-0.5 rounded bg-secondary text-muted-foreground font-medium shrink-0">
            {t("settings.skillsDirDefault")}
          </span>
        </div>
        {extraDirs.map((dir) => (
          <div
            key={dir}
            className="group flex items-center gap-2 px-3 py-2 rounded-md bg-secondary/30 text-xs"
          >
            <FolderOpen className="h-3.5 w-3.5 text-muted-foreground shrink-0" />
            <IconTip label={dir}>
              <code className="flex-1 text-foreground/80 truncate">{dir}</code>
            </IconTip>
            <IconTip label={t("settings.skillsDirRemove")}>
              <button
                className="text-muted-foreground/50 hover:text-destructive transition-colors shrink-0 opacity-0 group-hover:opacity-100"
                onClick={() => void handleRemoveDir(dir)}
              >
                <X className="h-3.5 w-3.5" />
              </button>
            </IconTip>
          </div>
        ))}
        <div className="mt-1 flex items-center gap-3">
          <button
            type="button"
            onClick={() => void handleImportDir()}
            disabled={importing}
            className="flex items-center gap-1.5 text-xs text-primary hover:text-primary/80 transition-colors px-2 py-1 disabled:opacity-50"
          >
            {importing ? (
              <Loader2 className="h-3.5 w-3.5 animate-spin" />
            ) : (
              <Plus className="h-3.5 w-3.5" />
            )}
            <span>{t("settings.skillsDirAdd")}</span>
          </button>
          <IconTip label={t("settings.skillsImport.tooltip.distinguishOnboarding")}>
            <button
              type="button"
              onClick={() => setQuickImportOpen(true)}
              className="flex items-center gap-1.5 text-xs text-primary hover:text-primary/80 transition-colors px-2 py-1"
            >
              <Sparkles className="h-3.5 w-3.5" />
              <span>{t("settings.skillsImport.button")}</span>
            </button>
          </IconTip>
        </div>
      </div>

      {/* Skills list */}
      {skills === null && (
        <div className="flex items-center justify-center py-10 text-muted-foreground">
          <Loader2 className="h-5 w-5 animate-spin" />
        </div>
      )}

      {error && (
        <div className="rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2 text-sm text-destructive">
          {error}
        </div>
      )}

      {skills && skills.length === 0 && !error && (
        <div className="rounded-md border border-border px-4 py-6 text-center text-sm text-muted-foreground">
          {t("onboarding.skills.empty")}
        </div>
      )}

      {skills && skills.length > 0 && (
        <ul className="space-y-0.5 max-h-[360px] overflow-y-auto pr-1">
          {skills.map((s) => {
            const skipsRequirements = s.always === true
            const enabled = !disabled.has(s.name)
            const row = (
              <div
                className={cn(
                  "flex items-center gap-3 w-full px-3 py-2.5 rounded-lg text-sm transition-colors",
                  enabled
                    ? "text-foreground hover:bg-secondary/60 cursor-pointer"
                    : "text-muted-foreground/50 hover:bg-secondary/40 cursor-pointer",
                )}
                onClick={() => toggle(s.name)}
              >
                <Switch
                  checked={enabled}
                  onCheckedChange={() => toggle(s.name)}
                  onClick={(e) => e.stopPropagation()}
                />
                <div className="flex-1 min-w-0">
                  <div className="flex items-center gap-1.5">
                    <span className={cn("font-medium truncate", !enabled && "line-through")}>
                      {s.name}
                    </span>
                    {skipsRequirements && (
                      <span className="text-[9px] px-1 py-0 rounded bg-green-500/10 text-green-600 font-medium shrink-0">
                        {t("settings.skillSkipsRequirements")}
                      </span>
                    )}
                  </div>
                  {s.description && (
                    <div className="text-xs text-muted-foreground truncate">{s.description}</div>
                  )}
                </div>
                {s.source && (
                  <span className="text-[10px] px-1.5 py-0.5 rounded bg-secondary text-muted-foreground font-medium shrink-0">
                    {s.source}
                  </span>
                )}
              </div>
            )
            return <li key={s.name}>{row}</li>
          })}
        </ul>
      )}

      <QuickImportDialog
        open={quickImportOpen}
        onClose={() => setQuickImportOpen(false)}
        onImported={() => void reload()}
      />
      {!isTauriMode() && (
        <ServerDirectoryBrowser
          open={dirBrowserOpen}
          onOpenChange={setDirBrowserOpen}
          onSelect={handleDirBrowserSelect}
        />
      )}
    </div>
  )
}
