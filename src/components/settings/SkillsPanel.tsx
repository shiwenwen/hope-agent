import { useState, useEffect } from "react"
import { invoke } from "@tauri-apps/api/core"
import { useTranslation } from "react-i18next"
import { cn } from "@/lib/utils"
import { Switch } from "@/components/ui/switch"
import {
  ArrowLeft,
  ChevronRight,
  ExternalLink,
  File,
  Folder,
  FolderOpen,
  Puzzle,
  X,
} from "lucide-react"
import type { SkillSummary } from "./types"

interface SkillFileInfo {
  name: string
  size: number
  is_dir: boolean
}

interface SkillDetail {
  name: string
  description: string
  source: string
  file_path: string
  base_dir: string
  content: string
  enabled: boolean
  files: SkillFileInfo[]
}

function formatFileSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`
}

export default function SkillsPanel() {
  const { t } = useTranslation()
  const [skills, setSkills] = useState<SkillSummary[]>([])
  const [extraDirs, setExtraDirs] = useState<string[]>([])
  const [selectedSkill, setSelectedSkill] = useState<SkillDetail | null>(null)
  const [loading, setLoading] = useState(true)
  const [skillEnvCheck, setSkillEnvCheck] = useState(true)

  async function reload() {
    try {
      const [list, dirs, envCheck] = await Promise.all([
        invoke<SkillSummary[]>("get_skills"),
        invoke<string[]>("get_extra_skills_dirs"),
        invoke<boolean>("get_skill_env_check"),
      ])
      setSkills(list)
      setExtraDirs(dirs)
      setSkillEnvCheck(envCheck)
    } catch (e) {
      console.error("Failed to load skills:", e)
    } finally {
      setLoading(false)
    }
  }

  useEffect(() => { reload() }, [])

  async function handleOpenDir(path: string) {
    try {
      await invoke("open_directory", { path })
    } catch (e) {
      console.error("Failed to open directory:", e)
    }
  }

  async function handleAddDir() {
    try {
      const { open } = await import("@tauri-apps/plugin-dialog")
      const selected = await open({ directory: true, multiple: false })
      if (selected) {
        await invoke("add_extra_skills_dir", { dir: selected })
        await reload()
      }
    } catch (e) {
      console.error("Failed to add skills directory:", e)
    }
  }

  async function handleRemoveDir(dir: string) {
    try {
      await invoke("remove_extra_skills_dir", { dir })
      await reload()
    } catch (e) {
      console.error("Failed to remove skills directory:", e)
    }
  }

  async function handleToggleSkill(name: string, enabled: boolean) {
    try {
      await invoke("toggle_skill", { name, enabled })
      // Update local state immediately
      setSkills((prev) =>
        prev.map((s) => (s.name === name ? { ...s, enabled } : s))
      )
      if (selectedSkill?.name === name) {
        setSelectedSkill((prev) => prev ? { ...prev, enabled } : prev)
      }
    } catch (e) {
      console.error("Failed to toggle skill:", e)
    }
  }

  async function handleSelectSkill(name: string) {
    try {
      const detail = await invoke<SkillDetail>("get_skill_detail", { name })
      setSelectedSkill(detail)
    } catch (e) {
      console.error("Failed to load skill detail:", e)
    }
  }

  // ── Skill Detail View ──────────────────────────────────────────
  if (selectedSkill) {
    return (
      <div className="flex-1 flex flex-col min-h-0 overflow-y-auto p-6">
        <div className="max-w-4xl">
          <button
            onClick={() => setSelectedSkill(null)}
            className="flex items-center gap-1.5 text-sm text-muted-foreground hover:text-foreground transition-colors mb-4"
          >
            <ArrowLeft className="h-4 w-4" />
            <span>{t("settings.skills")}</span>
          </button>

          {/* Header */}
          <div className="mb-4">
            <div className="flex items-center gap-3">
              <h2 className="text-lg font-semibold text-foreground">{selectedSkill.name}</h2>
              <Switch
                checked={selectedSkill.enabled}
                onCheckedChange={(v) => handleToggleSkill(selectedSkill.name, v)}
              />
            </div>
            <p className="text-xs text-muted-foreground mt-1">{selectedSkill.description}</p>
            <div className="flex items-center gap-2 mt-2">
              <span className="text-[10px] px-1.5 py-0.5 rounded bg-secondary text-muted-foreground font-medium">
                {selectedSkill.source}
              </span>
              <button
                className="flex items-center gap-1 text-[10px] text-muted-foreground hover:text-foreground transition-colors"
                onClick={() => handleOpenDir(selectedSkill.base_dir)}
                title={selectedSkill.base_dir}
              >
                <ExternalLink className="h-3 w-3" />
                <span className="truncate max-w-[300px]">{selectedSkill.base_dir}</span>
              </button>
            </div>
          </div>

          {/* Files in skill directory */}
          {selectedSkill.files.length > 0 && (
            <div className="mb-4">
              <h3 className="text-xs font-semibold text-muted-foreground uppercase tracking-wider mb-2">
                {t("settings.skillFiles")}
              </h3>
              <div className="rounded-lg border border-border overflow-hidden">
                {selectedSkill.files.map((file) => (
                  <div
                    key={file.name}
                    className="flex items-center gap-2 px-3 py-1.5 text-xs border-b border-border/50 last:border-b-0 bg-secondary/20"
                  >
                    {file.is_dir
                      ? <Folder className="h-3.5 w-3.5 text-primary/60 shrink-0" />
                      : <File className="h-3.5 w-3.5 text-muted-foreground shrink-0" />
                    }
                    <span className="flex-1 text-foreground/80 truncate">{file.name}{file.is_dir ? "/" : ""}</span>
                    {!file.is_dir && (
                      <span className="text-muted-foreground/60 shrink-0">{formatFileSize(file.size)}</span>
                    )}
                  </div>
                ))}
              </div>
            </div>
          )}

          {/* SKILL.md content */}
          <div className="border-t border-border pt-4">
            <h3 className="text-xs font-semibold text-muted-foreground uppercase tracking-wider mb-2">SKILL.md</h3>
            <pre className="text-xs text-foreground/80 whitespace-pre-wrap leading-relaxed bg-secondary/30 rounded-lg p-4">
              {selectedSkill.content}
            </pre>
          </div>
        </div>
      </div>
    )
  }

  // ── Skills List View ───────────────────────────────────────────
  return (
    <div className="flex-1 min-h-0 overflow-y-auto p-6">
      <h2 className="text-lg font-semibold text-foreground mb-1">
        {t("settings.skills")}
      </h2>
      <p className="text-xs text-muted-foreground mb-4">
        {t("settings.skillsDesc")}
      </p>

      {/* Skill directories */}
      <div className="mb-5">
        <h3 className="text-xs font-semibold text-muted-foreground uppercase tracking-wider mb-2">
          {t("settings.skillsDirs")}
        </h3>
        <div className="space-y-1">
          {/* Default directory (clickable) */}
          <button
            className="flex items-center gap-2 px-3 py-2 rounded-lg bg-secondary/30 text-xs w-full text-left hover:bg-secondary/50 transition-colors"
            onClick={() => handleOpenDir("~/.opencomputer/skills/")}
          >
            <FolderOpen className="h-3.5 w-3.5 text-muted-foreground shrink-0" />
            <code className="flex-1 text-foreground/80 truncate">~/.opencomputer/skills/</code>
            <span className="text-[10px] px-1.5 py-0.5 rounded bg-secondary text-muted-foreground font-medium shrink-0">
              {t("settings.skillsDirDefault")}
            </span>
          </button>

          {/* Extra directories (clickable) */}
          {extraDirs.map((dir) => (
            <div key={dir} className="flex items-center gap-2 px-3 py-2 rounded-lg bg-secondary/30 text-xs group">
              <button
                className="flex items-center gap-2 flex-1 min-w-0 text-left hover:text-foreground transition-colors"
                onClick={() => handleOpenDir(dir)}
              >
                <FolderOpen className="h-3.5 w-3.5 text-muted-foreground shrink-0" />
                <code className="flex-1 text-foreground/80 truncate" title={dir}>{dir}</code>
              </button>
              <button
                className="text-muted-foreground/50 hover:text-destructive transition-colors shrink-0 opacity-0 group-hover:opacity-100"
                onClick={() => handleRemoveDir(dir)}
                title={t("settings.skillsDirRemove")}
              >
                <X className="h-3.5 w-3.5" />
              </button>
            </div>
          ))}
        </div>

        {/* Import directory button */}
        <button
          className="mt-2 flex items-center gap-1.5 text-xs text-primary hover:text-primary/80 transition-colors px-3 py-1.5"
          onClick={handleAddDir}
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
          <span className="ml-1.5 text-muted-foreground/60 font-normal normal-case">({skills.length})</span>
        )}
      </h3>

      {/* Env check toggle */}
      <div className="flex items-center justify-between px-1 mb-5">
        <div>
          <div className="text-sm text-foreground">{t("settings.agentSkillEnvCheck")}</div>
          <div className="text-xs text-muted-foreground">{t("settings.agentSkillEnvCheckDesc")}</div>
        </div>
        <Switch
          checked={skillEnvCheck}
          onCheckedChange={async (v) => {
            setSkillEnvCheck(v)
            await invoke("set_skill_env_check", { enabled: v })
          }}
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
          {skills.map((skill) => (
            <div
              key={skill.name}
              className={cn(
                "flex items-center gap-3 w-full px-3 py-2.5 rounded-lg text-sm transition-colors group",
                skill.enabled
                  ? "text-foreground hover:bg-secondary/60"
                  : "text-muted-foreground/50 hover:bg-secondary/40"
              )}
            >
              {/* Toggle */}
              <Switch
                checked={skill.enabled}
                onCheckedChange={(v) => handleToggleSkill(skill.name, v)}
                onClick={(e) => e.stopPropagation()}
              />

              {/* Name + description (clickable → detail) */}
              <button
                className="flex-1 text-left min-w-0"
                onClick={() => handleSelectSkill(skill.name)}
              >
                <div className={cn("font-medium truncate", !skill.enabled && "line-through")}>{skill.name}</div>
                <div className="text-xs text-muted-foreground truncate">{skill.description}</div>
              </button>

              {/* Source tag */}
              <span className="text-[10px] px-1.5 py-0.5 rounded bg-secondary text-muted-foreground font-medium shrink-0">
                {skill.source}
              </span>

              {/* Open directory */}
              <button
                className="shrink-0 text-muted-foreground/40 hover:text-muted-foreground transition-colors opacity-0 group-hover:opacity-100"
                onClick={(e) => { e.stopPropagation(); handleOpenDir(skill.base_dir) }}
                title={skill.base_dir}
              >
                <FolderOpen className="h-3.5 w-3.5" />
              </button>

              <ChevronRight
                className="h-4 w-4 text-muted-foreground/30 shrink-0 group-hover:text-muted-foreground/60 transition-colors cursor-pointer"
                onClick={() => handleSelectSkill(skill.name)}
              />
            </div>
          ))}
        </div>
      )}
    </div>
  )
}
