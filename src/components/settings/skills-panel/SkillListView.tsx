import { useTranslation } from "react-i18next"
import { cn } from "@/lib/utils"
import { IconTip } from "@/components/ui/tooltip"
import { Switch } from "@/components/ui/switch"
import { Button } from "@/components/ui/button"
import {
  AlertTriangle,
  ChevronRight,
  FolderOpen,
  Puzzle,
  Settings2,
  Sparkles,
  X,
} from "lucide-react"
import type { SkillSummary } from "../types"

interface SkillListViewProps {
  skills: SkillSummary[]
  extraDirs: string[]
  loading: boolean
  skillEnvCheck: boolean
  envStatus: Record<string, Record<string, boolean>>
  onToggleSkill: (name: string, enabled: boolean) => void
  onSelectSkill: (name: string) => void
  onOpenDir: (path: string) => void
  onAddDir: () => void
  onRemoveDir: (dir: string) => void
  onSetSkillEnvCheck: (v: boolean) => void
  onQuickImport?: () => void
}

export default function SkillListView({
  skills,
  extraDirs,
  loading,
  skillEnvCheck,
  envStatus,
  onToggleSkill,
  onSelectSkill,
  onOpenDir,
  onAddDir,
  onRemoveDir,
  onSetSkillEnvCheck,
  onQuickImport,
}: SkillListViewProps) {
  const { t } = useTranslation()

  function hasEnvWarning(skillName: string): boolean {
    const status = envStatus[skillName]
    if (!status) return false
    return Object.values(status).some((v) => !v)
  }

  const bundledSkills = skills.filter((s) => s.source === "bundled")
  const userSkills = skills.filter((s) => s.source !== "bundled")

  function renderSkillRow(skill: SkillSummary) {
    const showWarning = hasEnvWarning(skill.name)
    const hasEnvConfig = skill.requires_env.length > 0
    const display = skill.display

    return (
      <div
        key={skill.name}
        className={cn(
          "flex items-center gap-3 w-full px-3 py-2.5 rounded-lg text-sm transition-colors group",
          skill.enabled
            ? "text-foreground hover:bg-secondary/60"
            : "text-muted-foreground/50 hover:bg-secondary/40",
        )}
      >
        {/* Toggle */}
        <Switch
          checked={skill.enabled}
          onCheckedChange={(v) => onToggleSkill(skill.name, v)}
          onClick={(e) => e.stopPropagation()}
        />

        {/* Name + description (clickable -> detail) */}
        <Button
          variant="ghost"
          className="h-auto flex-1 min-w-0 flex-col items-start justify-start px-0 py-0 text-left font-normal hover:bg-transparent"
          onClick={() => onSelectSkill(skill.name)}
        >
          <div className="flex items-center gap-1.5 w-full">
            {display?.emoji && (
              <span className="shrink-0 text-base leading-none" aria-hidden>
                {display.emoji}
              </span>
            )}
            <span className={cn("font-medium truncate", !skill.enabled && "line-through")}>
              {skill.name}
            </span>
            {display?.version && (
              <IconTip
                label={
                  display.author ? `v${display.version} · ${display.author}` : `v${display.version}`
                }
              >
                <span className="text-[10px] text-muted-foreground/70 shrink-0">
                  v{display.version}
                </span>
              </IconTip>
            )}
            {/* Warning icon for unconfigured env vars */}
            {showWarning && (
              <IconTip label={t("settings.skillEnvNotConfigured")}>
                <span className="shrink-0">
                  <AlertTriangle className="h-3.5 w-3.5 text-orange-400" />
                </span>
              </IconTip>
            )}
          </div>
          <div className="text-xs text-muted-foreground truncate">{skill.description}</div>
          {/* Status badges */}
          <div className="flex items-center gap-1 mt-0.5 flex-wrap">
            {skill.always && (
              <span className="text-[9px] px-1 py-0 rounded bg-green-500/10 text-green-600 font-medium">
                {t("settings.skillSkipsRequirements")}
              </span>
            )}
            {skill.has_install && (
              <span className="text-[9px] px-1 py-0 rounded bg-blue-500/10 text-blue-600 font-medium">
                {t("settings.skillInstall")}
              </span>
            )}
            {skill.disable_model_invocation && (
              <span className="text-[9px] px-1 py-0 rounded bg-orange-500/10 text-orange-600 font-medium">
                {t("settings.skillModelInvocable")}: ✗
              </span>
            )}
            {display?.is_proprietary && (
              <IconTip
                label={t("settings.skillExtras.licenseWarning", { license: display.license })}
              >
                <span className="text-[9px] px-1 py-0 rounded bg-amber-500/10 text-amber-600 font-medium cursor-help">
                  {display.license_label ?? "Proprietary"}
                </span>
              </IconTip>
            )}
            {display?.tags?.slice(0, 3).map((tag) => (
              <span
                key={tag}
                className="text-[9px] px-1 py-0 rounded bg-secondary/60 text-muted-foreground"
              >
                {tag}
              </span>
            ))}
          </div>
        </Button>

        {/* Source tag */}
        <span className="text-[10px] px-1.5 py-0.5 rounded bg-secondary text-muted-foreground font-medium shrink-0">
          {skill.source}
        </span>

        {/* Settings button for skills with env requirements */}
        {hasEnvConfig && (
          <IconTip label={t("settings.skillEnvVars")}>
            <Button
              variant="ghost"
              size="icon"
              className={cn(
                "h-7 w-7 shrink-0",
                showWarning
                  ? "text-orange-400 hover:text-orange-500"
                  : "text-muted-foreground/40 hover:text-muted-foreground opacity-0 group-hover:opacity-100",
              )}
              onClick={(e) => {
                e.stopPropagation()
                onSelectSkill(skill.name)
              }}
            >
              <Settings2 className="h-3.5 w-3.5" />
            </Button>
          </IconTip>
        )}

        {/* Open directory */}
        <IconTip label={skill.base_dir}>
          <Button
            variant="ghost"
            size="icon"
            className="h-7 w-7 shrink-0 text-muted-foreground/40 hover:text-muted-foreground opacity-0 group-hover:opacity-100"
            onClick={(e) => {
              e.stopPropagation()
              onOpenDir(skill.base_dir)
            }}
          >
            <FolderOpen className="h-3.5 w-3.5" />
          </Button>
        </IconTip>

        <ChevronRight
          className="h-4 w-4 text-muted-foreground/30 shrink-0 group-hover:text-muted-foreground/60 transition-colors cursor-pointer"
          onClick={() => onSelectSkill(skill.name)}
        />
      </div>
    )
  }

  return (
    <div className="flex-1 min-h-0 overflow-y-auto p-6">
      <h2 className="text-lg font-semibold text-foreground mb-1">{t("settings.skills")}</h2>
      <p className="text-xs text-muted-foreground mb-4">{t("settings.skillsDesc")}</p>

      {/* Skill directories */}
      <div className="mb-5">
        <h3 className="text-xs font-semibold text-muted-foreground uppercase tracking-wider mb-2">
          {t("settings.skillsDirs")}
        </h3>
        <div className="space-y-1">
          {/* Default directory (clickable) */}
          <Button
            variant="ghost"
            className="h-auto w-full justify-start gap-2 rounded-lg bg-secondary/30 px-3 py-2 text-xs font-normal hover:bg-secondary/50"
            onClick={() => onOpenDir("~/.hope-agent/skills/")}
          >
            <FolderOpen className="h-3.5 w-3.5 text-muted-foreground shrink-0" />
            <code className="flex-1 text-left text-foreground/80 truncate">~/.hope-agent/skills/</code>
            <span className="text-[10px] px-1.5 py-0.5 rounded bg-secondary text-muted-foreground font-medium shrink-0">
              {t("settings.skillsDirDefault")}
            </span>
          </Button>

          {/* Shared directory (~/.agents/skills/, cross-tool convention) */}
          <IconTip label={t("settings.skillsDirSharedDesc")}>
            <Button
              variant="ghost"
              className="h-auto w-full justify-start gap-2 rounded-lg bg-secondary/30 px-3 py-2 text-xs font-normal hover:bg-secondary/50"
              onClick={() => onOpenDir("~/.agents/skills/")}
            >
              <FolderOpen className="h-3.5 w-3.5 text-muted-foreground shrink-0" />
              <code className="flex-1 text-left text-foreground/80 truncate">~/.agents/skills/</code>
              <span className="text-[10px] px-1.5 py-0.5 rounded bg-secondary text-muted-foreground font-medium shrink-0">
                {t("settings.skillsDirShared")}
              </span>
            </Button>
          </IconTip>

          {/* Extra directories (clickable) */}
          {extraDirs.map((dir) => (
            <div
              key={dir}
              className="flex items-center gap-2 px-3 py-2 rounded-lg bg-secondary/30 text-xs group"
            >
              <Button
                variant="ghost"
                className="h-auto flex-1 min-w-0 justify-start gap-2 px-0 py-0 text-left font-normal hover:bg-transparent hover:text-foreground"
                onClick={() => onOpenDir(dir)}
              >
                <FolderOpen className="h-3.5 w-3.5 text-muted-foreground shrink-0" />
                <IconTip label={dir}>
                  <code className="flex-1 text-foreground/80 truncate">{dir}</code>
                </IconTip>
              </Button>
              <IconTip label={t("settings.skillsDirRemove")}>
                <Button
                  variant="ghost"
                  size="icon"
                  className="h-6 w-6 shrink-0 text-muted-foreground/50 opacity-0 group-hover:opacity-100 hover:text-destructive"
                  onClick={() => onRemoveDir(dir)}
                >
                  <X className="h-3.5 w-3.5" />
                </Button>
              </IconTip>
            </div>
          ))}
        </div>

        {/* Import directory buttons */}
        <div className="mt-2 flex items-center gap-3">
          <Button
            variant="ghost"
            size="sm"
            className="h-auto gap-1.5 px-3 py-1.5 text-xs font-normal text-primary hover:bg-transparent hover:text-primary/80"
            onClick={onAddDir}
          >
            <FolderOpen className="h-3.5 w-3.5" />
            <span>{t("settings.skillsDirAdd")}</span>
          </Button>
          {onQuickImport && (
            <Button
              variant="ghost"
              size="sm"
              className="h-auto gap-1.5 px-3 py-1.5 text-xs font-normal text-primary hover:bg-transparent hover:text-primary/80"
              onClick={onQuickImport}
            >
              <Sparkles className="h-3.5 w-3.5" />
              <span>{t("settings.skillsImport.button")}</span>
            </Button>
          )}
        </div>
      </div>

      {/* Divider */}
      <div className="border-t border-border mb-4" />

      {/* Env check toggle */}
      <div className="flex items-center justify-between px-1 mb-5">
        <div>
          <div className="text-sm text-foreground">{t("settings.agentSkillEnvCheck")}</div>
          <div className="text-xs text-muted-foreground">
            {t("settings.agentSkillEnvCheckDesc")}
          </div>
        </div>
        <Switch checked={skillEnvCheck} onCheckedChange={onSetSkillEnvCheck} />
      </div>

      <div className="border-t border-border mb-4" />

      {loading ? (
        <div className="flex items-center justify-center py-12">
          <div className="animate-spin h-5 w-5 border-2 border-foreground border-t-transparent rounded-full" />
        </div>
      ) : skills.length === 0 ? (
        <div className="text-center py-12">
          <Puzzle className="h-10 w-10 text-muted-foreground/30 mx-auto mb-3" />
          <p className="text-sm text-muted-foreground">{t("settings.noSkills")}</p>
          <p className="text-xs text-muted-foreground/70 mt-1">{t("settings.noSkillsHint")}</p>
        </div>
      ) : (
        <div className="space-y-5">
          {bundledSkills.length > 0 && (
            <div>
              <h3 className="text-xs font-semibold text-muted-foreground uppercase tracking-wider mb-2">
                {t("settings.skillsListBundled")}
                <span className="ml-1.5 text-muted-foreground/60 font-normal normal-case">
                  ({bundledSkills.length})
                </span>
              </h3>
              <div className="space-y-1">{bundledSkills.map(renderSkillRow)}</div>
            </div>
          )}

          {userSkills.length > 0 && (
            <div>
              <h3 className="text-xs font-semibold text-muted-foreground uppercase tracking-wider mb-2">
                {t("settings.skillsListUser")}
                <span className="ml-1.5 text-muted-foreground/60 font-normal normal-case">
                  ({userSkills.length})
                </span>
              </h3>
              <div className="space-y-1">{userSkills.map(renderSkillRow)}</div>
            </div>
          )}
        </div>
      )}
    </div>
  )
}
