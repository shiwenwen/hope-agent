import { useCallback, useEffect, useState } from "react"
import { useTranslation } from "react-i18next"
import { toast } from "sonner"
import { Check, ChevronDown, Loader2, Sparkles, Wand2 } from "lucide-react"

import { AnimatedCollapse } from "@/components/ui/animated-presence"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { DeferredNumberInput } from "@/components/ui/deferred-number-input"
import { Switch } from "@/components/ui/switch"
import { getTransport } from "@/lib/transport-provider"
import { cn } from "@/lib/utils"
import { logger } from "@/lib/logger"
import type {
  MaintenanceConfig,
  MaintenanceReport,
  MaintenanceTasks,
} from "@/types/knowledge"

const TASK_KEYS: Array<keyof MaintenanceTasks> = [
  "autoLink",
  "orphanRescue",
  "frontmatterFill",
  "dedupMerge",
  "knowledgeGap",
  "autoTag",
  "mocUpkeep",
  "memoryToNote",
]

/**
 * Layer-2 autonomous maintenance settings (WS6). Collapsible section under
 * Settings → Knowledge Space. Toggles the background scheduler + per-task
 * switches + auto-approve; proposals are reviewed in the knowledge view's
 * maintenance panel (this only configures generation).
 */
export default function KnowledgeMaintenanceSection() {
  const { t } = useTranslation()
  const [open, setOpen] = useState(false)
  const [cfg, setCfg] = useState<MaintenanceConfig | null>(null)
  const [snapshot, setSnapshot] = useState("")
  const [saving, setSaving] = useState(false)
  const [saveStatus, setSaveStatus] = useState<"idle" | "saved" | "failed">("idle")
  const [running, setRunning] = useState(false)

  useEffect(() => {
    getTransport()
      .call<MaintenanceConfig>("kb_maintenance_config_get_cmd")
      .then((c) => {
        setCfg(c)
        setSnapshot(JSON.stringify(c))
      })
      .catch((e) =>
        logger.warn("knowledge", "KnowledgeMaintenanceSection::load", "load failed", e),
      )
  }, [])

  const dirty = cfg != null && JSON.stringify(cfg) !== snapshot

  const save = useCallback(async () => {
    if (!cfg || saving) return
    setSaving(true)
    try {
      const saved = await getTransport().call<MaintenanceConfig>("kb_maintenance_config_set_cmd", {
        config: cfg,
      })
      setCfg(saved)
      setSnapshot(JSON.stringify(saved))
      setSaveStatus("saved")
      setTimeout(() => setSaveStatus("idle"), 2000)
    } catch (e) {
      logger.warn("knowledge", "KnowledgeMaintenanceSection::save", "save failed", e)
      setSaveStatus("failed")
      setTimeout(() => setSaveStatus("idle"), 2000)
    } finally {
      setSaving(false)
    }
  }, [cfg, saving])

  const runNow = useCallback(async () => {
    if (running) return
    setRunning(true)
    try {
      const report = await getTransport().call<MaintenanceReport>("kb_maintenance_run_cmd", {})
      if (report.note) {
        toast.message(
          t("settings.knowledgeMaintenance.cycleSkipped", "Skipped: {{note}}", {
            note: report.note,
          }),
        )
      } else {
        toast.success(
          t("settings.knowledgeMaintenance.cycleDone", "Generated {{n}} proposal(s)", {
            n: report.generated,
          }),
        )
      }
    } catch (e) {
      logger.warn("knowledge", "KnowledgeMaintenanceSection::runNow", "run failed", e)
      toast.error(t("settings.knowledgeMaintenance.runFailed", "Couldn't run maintenance"))
    } finally {
      setRunning(false)
    }
  }, [running, t])

  const patch = (p: Partial<MaintenanceConfig>) => setCfg((c) => (c ? { ...c, ...p } : c))

  return (
    <div className="rounded-lg border border-border/60 bg-card/40">
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        className="flex w-full items-center gap-2 px-4 py-3 text-left"
      >
        <Wand2 className="h-4 w-4 text-primary" />
        <span className="text-sm font-medium">
          {t("settings.knowledgeMaintenance.title", "Autonomous maintenance")}
        </span>
        {cfg?.enabled && (
          <span className="rounded-full bg-primary/10 px-1.5 text-[10px] text-primary">
            {t("common.on", "On")}
          </span>
        )}
        <ChevronDown
          className={cn("ml-auto h-4 w-4 text-muted-foreground transition-transform", open && "rotate-180")}
        />
      </button>

      <AnimatedCollapse open={open}>
        <div className="space-y-4 border-t border-border/60 px-4 py-3">
          <p className="text-[11px] leading-relaxed text-muted-foreground">
            {t(
              "settings.knowledgeMaintenance.intro",
              "Periodically scan knowledge spaces and queue note-maintenance suggestions (linking, tagging, dedup, MOCs, …) for your review. Nothing is written until you approve a suggestion.",
            )}
          </p>

          {cfg && (
            <>
              <Row
                label={t("settings.knowledgeMaintenance.enabled", "Enable background maintenance")}
                desc={t(
                  "settings.knowledgeMaintenance.enabledDesc",
                  "Run scans on the schedule below. With this off, only manual scans work.",
                )}
              >
                <Switch
                  checked={cfg.enabled}
                  onCheckedChange={(v) => patch({ enabled: v })}
                />
              </Row>

              <Row
                label={t("settings.knowledgeMaintenance.idle", "Run when idle")}
                desc={t(
                  "settings.knowledgeMaintenance.idleDesc",
                  "Scan after the app has been idle for the given minutes.",
                )}
              >
                <div className="flex items-center gap-2">
                  {cfg.idleTrigger.enabled && (
                    <DeferredNumberInput
                      min={1}
                      value={cfg.idleTrigger.idleMinutes}
                      onValueCommit={(value) =>
                        patch({
                          idleTrigger: {
                            ...cfg.idleTrigger,
                            idleMinutes: value,
                          },
                        })
                      }
                      className="h-7 w-16 text-xs"
                    />
                  )}
                  <Switch
                    checked={cfg.idleTrigger.enabled}
                    onCheckedChange={(v) =>
                      patch({ idleTrigger: { ...cfg.idleTrigger, enabled: v } })
                    }
                  />
                </div>
              </Row>

              <Row
                label={t("settings.knowledgeMaintenance.cron", "Run on a schedule")}
                desc={t(
                  "settings.knowledgeMaintenance.cronDesc",
                  "6-field cron (sec min hour day month weekday).",
                )}
              >
                <div className="flex items-center gap-2">
                  {cfg.cronTrigger.enabled && (
                    <Input
                      value={cfg.cronTrigger.cronExpr}
                      onChange={(e) =>
                        patch({ cronTrigger: { ...cfg.cronTrigger, cronExpr: e.target.value } })
                      }
                      className="h-7 w-32 font-mono text-xs"
                    />
                  )}
                  <Switch
                    checked={cfg.cronTrigger.enabled}
                    onCheckedChange={(v) =>
                      patch({ cronTrigger: { ...cfg.cronTrigger, enabled: v } })
                    }
                  />
                </div>
              </Row>

              <Row
                label={t("settings.knowledgeMaintenance.autoApprove", "Auto-apply suggestions")}
                desc={t(
                  "settings.knowledgeMaintenance.autoApproveDesc",
                  "Skip review and write approved-free changes to your notes automatically. Use with care.",
                )}
              >
                <Switch
                  checked={cfg.autoApprove}
                  onCheckedChange={(v) => patch({ autoApprove: v })}
                />
              </Row>

              <Row
                label={t("settings.knowledgeMaintenance.maxProposals", "Max suggestions per scan")}
                desc=""
              >
                <DeferredNumberInput
                  min={1}
                  max={200}
                  value={cfg.maxProposalsPerCycle}
                  onValueCommit={(value) => patch({ maxProposalsPerCycle: value })}
                  className="h-7 w-16 text-xs"
                />
              </Row>

              <div>
                <div className="mb-1.5 text-xs font-medium text-muted-foreground">
                  {t("settings.knowledgeMaintenance.tasks", "Tasks")}
                </div>
                <div className="grid grid-cols-2 gap-x-4 gap-y-1.5">
                  {TASK_KEYS.map((key) => (
                    <label key={key} className="flex items-center justify-between gap-2 text-xs">
                      <span className="truncate">
                        {t(`settings.knowledgeMaintenance.task.${key}`, key)}
                      </span>
                      <Switch
                        checked={cfg.tasks[key]}
                        onCheckedChange={(v) => patch({ tasks: { ...cfg.tasks, [key]: v } })}
                      />
                    </label>
                  ))}
                </div>
              </div>

              <div className="flex items-center justify-between gap-2 border-t border-border/60 pt-3">
                <Button variant="outline" size="sm" disabled={running} onClick={() => void runNow()}>
                  {running ? (
                    <Loader2 className="mr-1.5 h-3.5 w-3.5 animate-spin" />
                  ) : (
                    <Sparkles className="mr-1.5 h-3.5 w-3.5" />
                  )}
                  {t("settings.knowledgeMaintenance.runNow", "Scan now")}
                </Button>
                <Button
                  size="sm"
                  disabled={!dirty || saving}
                  onClick={() => void save()}
                  className={cn(saveStatus === "failed" && "bg-destructive hover:bg-destructive/90")}
                >
                  {saving ? (
                    <Loader2 className="mr-1.5 h-3.5 w-3.5 animate-spin" />
                  ) : saveStatus === "saved" ? (
                    <Check className="mr-1.5 h-3.5 w-3.5 text-emerald-300" />
                  ) : null}
                  {t("common.save", "Save")}
                </Button>
              </div>
            </>
          )}
        </div>
      </AnimatedCollapse>
    </div>
  )
}

function Row({
  label,
  desc,
  children,
}: {
  label: string
  desc: string
  children: React.ReactNode
}) {
  return (
    <div className="flex items-start justify-between gap-3">
      <div className="min-w-0">
        <div className="text-xs font-medium">{label}</div>
        {desc && <div className="mt-0.5 text-[11px] text-muted-foreground">{desc}</div>}
      </div>
      <div className="shrink-0">{children}</div>
    </div>
  )
}
