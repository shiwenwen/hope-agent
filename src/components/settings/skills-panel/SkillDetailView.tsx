import { useState } from "react"
import { invoke } from "@tauri-apps/api/core"
import { useTranslation } from "react-i18next"
import { cn } from "@/lib/utils"
import { IconTip } from "@/components/ui/tooltip"
import { Input } from "@/components/ui/input"
import { Switch } from "@/components/ui/switch"
import {
  ArrowLeft,
  Check,
  ExternalLink,
  File,
  Folder,
  Trash2,
} from "lucide-react"
import type { SkillDetail, SkillInstallSpec } from "./types"

function formatFileSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`
}

function InstallSpecRow({
  spec,
  skillName,
  specIndex,
}: {
  spec: SkillInstallSpec
  skillName: string
  specIndex: number
}) {
  const { t } = useTranslation()
  const [installing, setInstalling] = useState(false)
  const [result, setResult] = useState<{ ok: boolean; message: string } | null>(null)

  const label =
    spec.label || `${spec.kind}: ${spec.formula || spec.package || spec.go_module || "?"}`

  async function handleInstall() {
    setInstalling(true)
    setResult(null)
    try {
      const output = await invoke<string>("install_skill_dependency", {
        skillName,
        specIndex,
      })
      setResult({ ok: true, message: output })
    } catch (e) {
      setResult({ ok: false, message: String(e) })
    } finally {
      setInstalling(false)
    }
  }

  return (
    <div className="flex items-center gap-2">
      <span className="text-[10px] px-1.5 py-0.5 rounded bg-secondary text-muted-foreground font-mono">
        {spec.kind}
      </span>
      <span className="text-xs text-foreground/80 flex-1 truncate">{label}</span>
      <button
        className={cn(
          "text-[10px] px-2 py-0.5 rounded transition-colors font-medium",
          installing
            ? "bg-muted text-muted-foreground cursor-wait"
            : result?.ok
              ? "bg-green-500/10 text-green-600"
              : result && !result.ok
                ? "bg-destructive/10 text-destructive"
                : "bg-primary/10 text-primary hover:bg-primary/20",
        )}
        onClick={handleInstall}
        disabled={installing}
      >
        {installing
          ? t("settings.skillInstalling")
          : result?.ok
            ? t("settings.skillInstallSuccess")
            : result && !result.ok
              ? t("settings.skillInstallFailed")
              : t("settings.skillInstall")}
      </button>
    </div>
  )
}

interface SkillDetailViewProps {
  skill: SkillDetail
  envStatus: Record<string, Record<string, boolean>>
  envValues: Record<string, string>
  envDirty: Record<string, boolean>
  envSaving: Record<string, boolean>
  onBack: () => void
  onToggleSkill: (name: string, enabled: boolean) => void
  onOpenDir: (path: string) => void
  onEnvValueChange: (key: string, value: string) => void
  onSaveEnvVar: (key: string) => void
  onRemoveEnvVar: (key: string) => void
}

export default function SkillDetailView({
  skill,
  envStatus,
  envValues,
  envDirty,
  envSaving,
  onBack,
  onToggleSkill,
  onOpenDir,
  onEnvValueChange,
  onSaveEnvVar,
  onRemoveEnvVar,
}: SkillDetailViewProps) {
  const { t } = useTranslation()
  const requiresEnv = skill.requires?.env ?? []

  return (
    <div className="flex-1 flex flex-col min-h-0 overflow-y-auto p-6">
      <div className="max-w-4xl">
        <button
          onClick={onBack}
          className="flex items-center gap-1.5 text-sm text-muted-foreground hover:text-foreground transition-colors mb-4"
        >
          <ArrowLeft className="h-4 w-4" />
          <span>{t("settings.skills")}</span>
        </button>

        {/* Header */}
        <div className="mb-4">
          <div className="flex items-center gap-3">
            <h2 className="text-lg font-semibold text-foreground">{skill.name}</h2>
            <Switch
              checked={skill.enabled}
              onCheckedChange={(v) => onToggleSkill(skill.name, v)}
            />
          </div>
          <p className="text-xs text-muted-foreground mt-1">{skill.description}</p>
          <div className="flex items-center gap-2 mt-2">
            <span className="text-[10px] px-1.5 py-0.5 rounded bg-secondary text-muted-foreground font-medium">
              {skill.source}
            </span>
            <button
              className="flex items-center gap-1 text-[10px] text-muted-foreground hover:text-foreground transition-colors"
              onClick={() => onOpenDir(skill.base_dir)}
              title={skill.base_dir}
            >
              <ExternalLink className="h-3 w-3" />
              <span className="truncate max-w-[300px]">{skill.base_dir}</span>
            </button>
          </div>
        </div>

        {/* Environment Variables Configuration */}
        {requiresEnv.length > 0 && (
          <div className="mb-4">
            <h3 className="text-xs font-semibold text-muted-foreground uppercase tracking-wider mb-1">
              {t("settings.skillEnvVars")}
            </h3>
            <p className="text-xs text-muted-foreground mb-3">
              {t("settings.skillEnvVarsDesc")}
            </p>
            <div className="space-y-2">
              {requiresEnv.map((envKey) => {
                const currentValue = envValues[envKey] ?? ""
                const isDirty = envDirty[envKey] ?? false
                const isSaving = envSaving[envKey] ?? false
                const isConfigured = envStatus[skill.name]?.[envKey] ?? false

                return (
                  <div key={envKey} className="flex items-center gap-2">
                    {/* Status indicator */}
                    <div
                      className={cn(
                        "h-2 w-2 rounded-full shrink-0",
                        isConfigured ? "bg-green-500" : "bg-orange-400",
                      )}
                      title={
                        isConfigured
                          ? t("settings.skillEnvConfigured")
                          : t("settings.skillEnvNotConfigured")
                      }
                    />
                    {/* Label */}
                    <code
                      className="text-xs text-foreground/80 w-44 shrink-0 truncate"
                      title={envKey}
                    >
                      {envKey}
                    </code>
                    {/* Input */}
                    <Input
                      type="password"
                      className="h-7 text-xs flex-1 min-w-0"
                      placeholder={t("settings.skillEnvPlaceholder", { key: envKey })}
                      value={currentValue}
                      onChange={(e) => onEnvValueChange(envKey, e.target.value)}
                      onKeyDown={(e) => {
                        if (e.key === "Enter" && isDirty) onSaveEnvVar(envKey)
                      }}
                    />
                    {/* Save button */}
                    <IconTip label={t("settings.skillEnvSave")}>
                      <button
                        className={cn(
                          "shrink-0 p-1 rounded transition-colors",
                          isDirty && !isSaving
                            ? "text-primary hover:bg-primary/10"
                            : "text-muted-foreground/30 cursor-default",
                        )}
                        onClick={() => isDirty && onSaveEnvVar(envKey)}
                        disabled={!isDirty || isSaving}
                      >
                        <Check className="h-3.5 w-3.5" />
                      </button>
                    </IconTip>
                    {/* Clear button */}
                    <IconTip label={t("settings.skillEnvClear")}>
                      <button
                        className={cn(
                          "shrink-0 p-1 rounded transition-colors",
                          currentValue
                            ? "text-muted-foreground hover:text-destructive hover:bg-destructive/10"
                            : "text-muted-foreground/30 cursor-default",
                        )}
                        onClick={() => currentValue && onRemoveEnvVar(envKey)}
                        disabled={!currentValue}
                      >
                        <Trash2 className="h-3.5 w-3.5" />
                      </button>
                    </IconTip>
                  </div>
                )
              })}
            </div>
          </div>
        )}

        {/* Advanced Info: anyBins, always, invocation policy, command dispatch, install */}
        {(skill.requires?.any_bins?.length ||
          skill.requires?.always ||
          skill.user_invocable !== undefined ||
          skill.disable_model_invocation !== undefined ||
          skill.command_dispatch ||
          (skill.install && skill.install.length > 0)) && (
          <div className="mb-4">
            <h3 className="text-xs font-semibold text-muted-foreground uppercase tracking-wider mb-2">
              {t("settings.skillInvocationPolicy")}
            </h3>
            <div className="flex flex-wrap gap-2">
              {skill.requires?.always && (
                <span className="text-[10px] px-2 py-0.5 rounded-full bg-green-500/10 text-green-600 font-medium">
                  {t("settings.skillAlways")}
                </span>
              )}
              {skill.requires?.any_bins && skill.requires.any_bins.length > 0 && (
                <span className="text-[10px] px-2 py-0.5 rounded-full bg-blue-500/10 text-blue-600 font-medium">
                  {t("settings.skillAnyBins")}: {skill.requires.any_bins.join(" | ")}
                </span>
              )}
              {skill.user_invocable === false && (
                <span className="text-[10px] px-2 py-0.5 rounded-full bg-orange-500/10 text-orange-600 font-medium">
                  {t("settings.skillUserInvocable")}: ✗
                </span>
              )}
              {skill.disable_model_invocation === true && (
                <span className="text-[10px] px-2 py-0.5 rounded-full bg-orange-500/10 text-orange-600 font-medium">
                  {t("settings.skillModelInvocable")}: ✗
                </span>
              )}
              {skill.command_dispatch && (
                <span className="text-[10px] px-2 py-0.5 rounded-full bg-purple-500/10 text-purple-600 font-medium">
                  {t("settings.skillCommandDispatch")}: {skill.command_dispatch}
                  {skill.command_tool ? ` → ${skill.command_tool}` : ""}
                </span>
              )}
            </div>

            {/* Install specs */}
            {skill.install && skill.install.length > 0 && (
              <div className="mt-3">
                <h4 className="text-[10px] font-medium text-muted-foreground uppercase tracking-wider mb-1.5">
                  {t("settings.skillInstall")}
                </h4>
                <div className="space-y-1.5">
                  {skill.install.map((spec, idx) => (
                    <InstallSpecRow
                      key={idx}
                      spec={spec}
                      skillName={skill.name}
                      specIndex={idx}
                    />
                  ))}
                </div>
              </div>
            )}
          </div>
        )}

        {/* Files in skill directory */}
        {skill.files.length > 0 && (
          <div className="mb-4">
            <h3 className="text-xs font-semibold text-muted-foreground uppercase tracking-wider mb-2">
              {t("settings.skillFiles")}
            </h3>
            <div className="rounded-lg border border-border overflow-hidden">
              {skill.files.map((file) => (
                <div
                  key={file.name}
                  className="flex items-center gap-2 px-3 py-1.5 text-xs border-b border-border/50 last:border-b-0 bg-secondary/20"
                >
                  {file.is_dir ? (
                    <Folder className="h-3.5 w-3.5 text-primary/60 shrink-0" />
                  ) : (
                    <File className="h-3.5 w-3.5 text-muted-foreground shrink-0" />
                  )}
                  <span className="flex-1 text-foreground/80 truncate">
                    {file.name}
                    {file.is_dir ? "/" : ""}
                  </span>
                  {!file.is_dir && (
                    <span className="text-muted-foreground/60 shrink-0">
                      {formatFileSize(file.size)}
                    </span>
                  )}
                </div>
              ))}
            </div>
          </div>
        )}

        {/* SKILL.md content */}
        <div className="border-t border-border pt-4">
          <h3 className="text-xs font-semibold text-muted-foreground uppercase tracking-wider mb-2">
            SKILL.md
          </h3>
          <pre className="text-xs text-foreground/80 whitespace-pre-wrap leading-relaxed bg-secondary/30 rounded-lg p-4">
            {skill.content}
          </pre>
        </div>
      </div>
    </div>
  )
}
