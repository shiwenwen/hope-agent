import { useTranslation } from "react-i18next"
import { cn } from "@/lib/utils"
import { IconTip } from "@/components/ui/tooltip"
import { Switch } from "@/components/ui/switch"
import {
  AlertTriangle,
  ChevronRight,
  FolderOpen,
  Puzzle,
  Settings2,
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
}: SkillListViewProps) {
  const { t } = useTranslation()

  function hasEnvWarning(skillName: string): boolean {
    const status = envStatus[skillName]
    if (!status) return false
    return Object.values(status).some((v) => !v)
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
          <button
            className="flex items-center gap-2 px-3 py-2 rounded-lg bg-secondary/30 text-xs w-full text-left hover:bg-secondary/50 transition-colors"
            onClick={() => onOpenDir("~/.opencomputer/skills/")}
          >
            <FolderOpen className="h-3.5 w-3.5 text-muted-foreground shrink-0" />
            <code className="flex-1 text-foreground/80 truncate">~/.opencomputer/skills/</code>
            <span className="text-[10px] px-1.5 py-0.5 rounded bg-secondary text-muted-foreground font-medium shrink-0">
              {t("settings.skillsDirDefault")}
            </span>
          </button>

          {/* Extra directories (clickable) */}
          {extraDirs.map((dir) => (
            <div
              key={dir}
              className="flex items-center gap-2 px-3 py-2 rounded-lg bg-secondary/30 text-xs group"
            >
              <button
                className="flex items-center gap-2 flex-1 min-w-0 text-left hover:text-foreground transition-colors"
                onClick={() => onOpenDir(dir)}
              >
                <FolderOpen className="h-3.5 w-3.5 text-muted-foreground shrink-0" />
                <code className="flex-1 text-foreground/80 truncate" title={dir}>
                  {dir}
                </code>
              </button>
              <IconTip label={t("settings.skillsDirRemove")}>
                <button
                  className="text-muted-foreground/50 hover:text-destructive transition-colors shrink-0 opacity-0 group-hover:opacity-100"
                  onClick={() => onRemoveDir(dir)}
                >
                  <X className="h-3.5 w-3.5" />
                </button>
              </IconTip>
            </div>
          ))}
        </div>

        {/* Import directory button */}
        <button
          className="mt-2 flex items-center gap-1.5 text-xs text-primary hover:text-primary/80 transition-colors px-3 py-1.5"
          onClick={onAddDir}
        >
          <FolderOpen className="h-3.5 w-3.5" />
          <span>{t("settings.skillsDirAdd")}</span>
        </button>
      </div>

      {/* Divider */}
      <div className="border-t border-border mb-4" />

      {/* Skills list */}
      <h3 className="text-xs font-semibold text-muted-foreground uppercase tracking-wider mb-2">
        {t("settings.skillsList")}
        {!loading && skills.length > 0 && (
          <span className="ml-1.5 text-muted-foreground/60 font-normal normal-case">
            ({skills.length})
          </span>
        )}
      </h3>

      {/* Env check toggle */}
      <div className="flex items-center justify-between px-1 mb-5">
        <div>
          <div className="text-sm text-foreground">{t("settings.agentSkillEnvCheck")}</div>
          <div className="text-xs text-muted-foreground">
            {t("settings.agentSkillEnvCheckDesc")}
          </div>
        </div>
        <Switch
          checked={skillEnvCheck}
          onCheckedChange={onSetSkillEnvCheck}
        />
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
        <div className="space-y-1">
          {skills.map((skill) => {
            const showWarning = hasEnvWarning(skill.name)
            const hasEnvConfig = skill.requires_env.length > 0

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
                <button
                  className="flex-1 text-left min-w-0"
                  onClick={() => onSelectSkill(skill.name)}
                >
                  <div className="flex items-center gap-1.5">
                    <span
                      className={cn("font-medium truncate", !skill.enabled && "line-through")}
                    >
                      {skill.name}
                    </span>
                    {/* Warning icon for unconfigured env vars */}
                    {showWarning && (
                      <IconTip label={t("settings.skillEnvNotConfigured")}>
                        <span className="shrink-0">
                          <AlertTriangle className="h-3.5 w-3.5 text-orange-400" />
                        </span>
                      </IconTip>
                    )}
                  </div>
                  <div className="text-xs text-muted-foreground truncate">
                    {skill.description}
                  </div>
                  {/* Status badges */}
                  <div className="flex items-center gap-1 mt-0.5">
                    {skill.always && (
                      <span className="text-[9px] px-1 py-0 rounded bg-green-500/10 text-green-600 font-medium">
                        {t("settings.skillAlways")}
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
                  </div>
                </button>

                {/* Source tag */}
                <span className="text-[10px] px-1.5 py-0.5 rounded bg-secondary text-muted-foreground font-medium shrink-0">
                  {skill.source}
                </span>

                {/* Settings button for skills with env requirements */}
                {hasEnvConfig && (
                  <IconTip label={t("settings.skillEnvVars")}>
                    <button
                      className={cn(
                        "shrink-0 transition-colors",
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
                    </button>
                  </IconTip>
                )}

                {/* Open directory */}
                <button
                  className="shrink-0 text-muted-foreground/40 hover:text-muted-foreground transition-colors opacity-0 group-hover:opacity-100"
                  onClick={(e) => {
                    e.stopPropagation()
                    onOpenDir(skill.base_dir)
                  }}
                  title={skill.base_dir}
                >
                  <FolderOpen className="h-3.5 w-3.5" />
                </button>

                <ChevronRight
                  className="h-4 w-4 text-muted-foreground/30 shrink-0 group-hover:text-muted-foreground/60 transition-colors cursor-pointer"
                  onClick={() => onSelectSkill(skill.name)}
                />
              </div>
            )
          })}
        </div>
      )}
    </div>
  )
}
