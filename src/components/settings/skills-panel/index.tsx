import { useEffect, useState, useEffectEvent } from "react"
import { useTranslation } from "react-i18next"
import { Tabs, TabsList, TabsTrigger, TabsContent } from "@/components/ui/tabs"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import {
  markDraftsSeen,
  refreshDraftSkillsStore,
  useDraftSkillsStore,
} from "@/hooks/useDraftSkillsStore"
import { SKILLS_EVENTS } from "@/types/skills"
import type { SkillSummary } from "../types"
import type { SkillDetail } from "./types"
import SkillListView from "./SkillListView"
import SkillEvolutionView from "./SkillEvolutionView"
import SkillDetailView from "./SkillDetailView"
import DraftReviewSection from "./DraftReviewSection"
import QuickImportDialog from "./QuickImportDialog"

export default function SkillsPanel() {
  const { t } = useTranslation()
  const { drafts } = useDraftSkillsStore()
  const [skills, setSkills] = useState<SkillSummary[]>([])
  const [draftPending, setDraftPending] = useState<
    Record<string, "activate" | "discard" | undefined>
  >({})
  const [extraDirs, setExtraDirs] = useState<string[]>([])
  const [selectedSkill, setSelectedSkill] = useState<SkillDetail | null>(null)
  const [loading, setLoading] = useState(true)
  const [quickImportOpen, setQuickImportOpen] = useState(false)
  const [skillEnvCheck, setSkillEnvCheck] = useState(true)
  const [autoReviewEnabled, setAutoReviewEnabled] = useState(true)
  const [autoReviewPromotion, setAutoReviewPromotion] = useState(false)
  // Per-skill env status: skill_name -> { env_var -> is_configured }
  const [envStatus, setEnvStatus] = useState<Record<string, Record<string, boolean>>>({})
  // Env var values for the currently selected skill detail (masked from backend)
  const [envValues, setEnvValues] = useState<Record<string, string>>({})
  // Tracks which env vars the user has edited (dirty state)
  const [envDirty, setEnvDirty] = useState<Record<string, boolean>>({})
  // Saving state per key
  const [envSaving, setEnvSaving] = useState<Record<string, boolean>>({})

  const reload = async () => {
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
  }
  const reloadEffectEvent = useEffectEvent(reload)

  useEffect(() => {
    reloadEffectEvent()
    const unlisten = getTransport().listen(SKILLS_EVENTS.autoReviewComplete, () => {
      reloadEffectEvent()
    })
    return unlisten
  }, [])

  // Whenever the panel is open and the draft list changes (mount, store
  // refresh, or live event), the user is implicitly "seeing" the current set —
  // clear the unseen-count badge that drives the IconSidebar / SettingsView dots.
  useEffect(() => {
    markDraftsSeen()
  }, [drafts])

  useEffect(() => {
    let cancelled = false
    Promise.all([
      getTransport().call<boolean>("get_skills_auto_review_enabled"),
      getTransport().call<boolean>("get_skills_auto_review_promotion"),
    ])
      .then(([enabled, promotion]) => {
        if (cancelled) return
        setAutoReviewEnabled(enabled)
        setAutoReviewPromotion(promotion)
      })
      .catch((e) => {
        logger.error(
          "settings",
          "SkillsPanel::loadAutoReview",
          "Failed to load auto-review settings",
          e,
        )
      })
    return () => {
      cancelled = true
    }
  }, [])

  const draftNames = new Set(drafts.map((d) => d.name))
  const visibleSkills = skills.filter((s) => !draftNames.has(s.name))

  async function handleActivateDraft(name: string) {
    setDraftPending((prev) => ({ ...prev, [name]: "activate" }))
    try {
      await getTransport().call("activate_draft_skill", { name })
      await reload()
      refreshDraftSkillsStore()
    } catch (e) {
      logger.error("settings", "SkillsPanel::activateDraft", "Failed to activate", e)
    } finally {
      setDraftPending((prev) => ({ ...prev, [name]: undefined }))
    }
  }

  async function handleDiscardDraft(name: string) {
    setDraftPending((prev) => ({ ...prev, [name]: "discard" }))
    try {
      await getTransport().call("discard_draft_skill", { name })
      await reload()
      refreshDraftSkillsStore()
    } catch (e) {
      logger.error("settings", "SkillsPanel::discardDraft", "Failed to discard", e)
    } finally {
      setDraftPending((prev) => ({ ...prev, [name]: undefined }))
    }
  }

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
      const status =
        await getTransport().call<Record<string, Record<string, boolean>>>("get_skills_env_status")
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
      const status =
        await getTransport().call<Record<string, Record<string, boolean>>>("get_skills_env_status")
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

  async function handleSetAutoReviewPromotion(v: boolean) {
    const previous = autoReviewPromotion
    setAutoReviewPromotion(v)
    try {
      await getTransport().call("set_skills_auto_review_promotion", { auto: v })
    } catch (e) {
      logger.error(
        "settings",
        "SkillsPanel::setAutoReviewPromotion",
        "Failed to update auto-review promotion",
        e,
      )
      setAutoReviewPromotion(previous)
    }
  }

  async function handleSetAutoReviewEnabled(v: boolean) {
    const previous = autoReviewEnabled
    setAutoReviewEnabled(v)
    try {
      await getTransport().call("set_skills_auto_review_enabled", { enabled: v })
    } catch (e) {
      logger.error(
        "settings",
        "SkillsPanel::setAutoReviewEnabled",
        "Failed to update auto-review enabled",
        e,
      )
      setAutoReviewEnabled(previous)
    }
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
    <div className="flex-1 min-h-0 overflow-hidden flex flex-col">
      {drafts.length > 0 && (
        <div className="px-6 pt-4">
          <DraftReviewSection
            drafts={drafts}
            pendingAction={draftPending}
            onActivate={handleActivateDraft}
            onDiscard={handleDiscardDraft}
            onSelectSkill={handleSelectSkill}
          />
        </div>
      )}
      <Tabs defaultValue="manage" className="flex-1 flex flex-col min-h-0">
        <div className="px-6 pt-4 shrink-0">
          <TabsList>
            <TabsTrigger value="manage">{t("settings.skillsTab.manage")}</TabsTrigger>
            <TabsTrigger value="evolution">{t("settings.skillsTab.evolution")}</TabsTrigger>
          </TabsList>
        </div>
        <TabsContent value="manage" className="flex-1 min-h-0 outline-none">
          <SkillListView
            skills={visibleSkills}
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
            onQuickImport={() => setQuickImportOpen(true)}
          />
        </TabsContent>
        <TabsContent value="evolution" className="flex-1 min-h-0 outline-none">
          <SkillEvolutionView
            autoReviewEnabled={autoReviewEnabled}
            autoReviewPromotion={autoReviewPromotion}
            onSetAutoReviewEnabled={handleSetAutoReviewEnabled}
            onSetAutoReviewPromotion={handleSetAutoReviewPromotion}
          />
        </TabsContent>
      </Tabs>
      <QuickImportDialog
        open={quickImportOpen}
        onClose={() => setQuickImportOpen(false)}
        onImported={reload}
      />
    </div>
  )
}
