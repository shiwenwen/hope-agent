import { useState, useEffect, useCallback } from "react"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import type { SkillSummary } from "../types"
import type { SkillDetail } from "./types"
import SkillListView from "./SkillListView"
import SkillDetailView from "./SkillDetailView"

export default function SkillsPanel() {
  const [skills, setSkills] = useState<SkillSummary[]>([])
  const [extraDirs, setExtraDirs] = useState<string[]>([])
  const [selectedSkill, setSelectedSkill] = useState<SkillDetail | null>(null)
  const [loading, setLoading] = useState(true)
  const [skillEnvCheck, setSkillEnvCheck] = useState(true)
  // Per-skill env status: skill_name -> { env_var -> is_configured }
  const [envStatus, setEnvStatus] = useState<Record<string, Record<string, boolean>>>({})
  // Env var values for the currently selected skill detail (masked from backend)
  const [envValues, setEnvValues] = useState<Record<string, string>>({})
  // Tracks which env vars the user has edited (dirty state)
  const [envDirty, setEnvDirty] = useState<Record<string, boolean>>({})
  // Saving state per key
  const [envSaving, setEnvSaving] = useState<Record<string, boolean>>({})

  const reload = useCallback(async () => {
    try {
      const [list, dirs, envCheck, status] = await Promise.all([
        getTransport().call<SkillSummary[]>("get_skills"),
        getTransport().call<string[]>("get_extra_skills_dirs"),
        getTransport().call<boolean>("get_skill_env_check"),
        getTransport().call<Record<string, Record<string, boolean>>>("get_skills_env_status"),
      ])
      setSkills(list)
      setExtraDirs(dirs)
      setSkillEnvCheck(envCheck)
      setEnvStatus(status)
    } catch (e) {
      logger.error("settings", "SkillsPanel::load", "Failed to load skills", e)
    } finally {
      setLoading(false)
    }
  }, [])

  useEffect(() => {
    reload()
  }, [reload])

  async function handleOpenDir(path: string) {
    try {
      await getTransport().call("open_directory", { path })
    } catch (e) {
      logger.error("settings", "SkillsPanel::openDir", "Failed to open directory", e)
    }
  }

  async function handleAddDir() {
    try {
      const { open } = await import("@tauri-apps/plugin-dialog")
      const selected = await open({ directory: true, multiple: false })
      if (selected) {
        await getTransport().call("add_extra_skills_dir", { dir: selected })
        await reload()
      }
    } catch (e) {
      logger.error("settings", "SkillsPanel::addDir", "Failed to add skills directory", e)
    }
  }

  async function handleRemoveDir(dir: string) {
    try {
      await getTransport().call("remove_extra_skills_dir", { dir })
      await reload()
    } catch (e) {
      logger.error("settings", "SkillsPanel::removeDir", "Failed to remove skills directory", e)
    }
  }

  async function handleToggleSkill(name: string, enabled: boolean) {
    try {
      await getTransport().call("toggle_skill", { name, enabled })
      // Update local state immediately
      setSkills((prev) => prev.map((s) => (s.name === name ? { ...s, enabled } : s)))
      if (selectedSkill?.name === name) {
        setSelectedSkill((prev) => (prev ? { ...prev, enabled } : prev))
      }
    } catch (e) {
      logger.error("settings", "SkillsPanel::toggle", "Failed to toggle skill", e)
    }
  }

  async function handleSelectSkill(name: string) {
    try {
      const [detail, maskedEnv] = await Promise.all([
        getTransport().call<SkillDetail>("get_skill_detail", { name }),
        getTransport().call<Record<string, string>>("get_skill_env", { name }),
      ])
      setSelectedSkill(detail)
      setEnvValues(maskedEnv)
      setEnvDirty({})
      setEnvSaving({})
    } catch (e) {
      logger.error("settings", "SkillsPanel::detail", "Failed to load skill detail", e)
    }
  }

  async function handleSaveEnvVar(key: string) {
    if (!selectedSkill) return
    const value = envValues[key] ?? ""
    setEnvSaving((prev) => ({ ...prev, [key]: true }))
    try {
      await getTransport().call("set_skill_env_var", { skill: selectedSkill.name, key, value })
      // Re-fetch the masked value
      const maskedEnv = await getTransport().call<Record<string, string>>("get_skill_env", {
        name: selectedSkill.name,
      })
      setEnvValues(maskedEnv)
      setEnvDirty((prev) => ({ ...prev, [key]: false }))
      // Refresh env status
      const status = await getTransport().call<Record<string, Record<string, boolean>>>("get_skills_env_status")
      setEnvStatus(status)
    } catch (e) {
      logger.error("settings", "SkillsPanel::saveEnv", "Failed to save env var", e)
    } finally {
      setEnvSaving((prev) => ({ ...prev, [key]: false }))
    }
  }

  async function handleRemoveEnvVar(key: string) {
    if (!selectedSkill) return
    try {
      await getTransport().call("remove_skill_env_var", { skill: selectedSkill.name, key })
      setEnvValues((prev) => {
        const next = { ...prev }
        delete next[key]
        return next
      })
      setEnvDirty((prev) => ({ ...prev, [key]: false }))
      // Refresh env status
      const status = await getTransport().call<Record<string, Record<string, boolean>>>("get_skills_env_status")
      setEnvStatus(status)
    } catch (e) {
      logger.error("settings", "SkillsPanel::removeEnv", "Failed to remove env var", e)
    }
  }

  function handleEnvValueChange(key: string, value: string) {
    setEnvValues((prev) => ({ ...prev, [key]: value }))
    setEnvDirty((prev) => ({ ...prev, [key]: true }))
  }

  async function handleSetSkillEnvCheck(v: boolean) {
    setSkillEnvCheck(v)
    await getTransport().call("set_skill_env_check", { enabled: v })
  }

  // ── Skill Detail View ──────────────────────────────────────────
  if (selectedSkill) {
    return (
      <SkillDetailView
        skill={selectedSkill}
        envStatus={envStatus}
        envValues={envValues}
        envDirty={envDirty}
        envSaving={envSaving}
        onBack={() => setSelectedSkill(null)}
        onToggleSkill={handleToggleSkill}
        onOpenDir={handleOpenDir}
        onEnvValueChange={handleEnvValueChange}
        onSaveEnvVar={handleSaveEnvVar}
        onRemoveEnvVar={handleRemoveEnvVar}
      />
    )
  }

  // ── Skills List View ───────────────────────────────────────────
  return (
    <SkillListView
      skills={skills}
      extraDirs={extraDirs}
      loading={loading}
      skillEnvCheck={skillEnvCheck}
      envStatus={envStatus}
      onToggleSkill={handleToggleSkill}
      onSelectSkill={handleSelectSkill}
      onOpenDir={handleOpenDir}
      onAddDir={handleAddDir}
      onRemoveDir={handleRemoveDir}
      onSetSkillEnvCheck={handleSetSkillEnvCheck}
    />
  )
}
