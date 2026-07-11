import { useCallback, useEffect, useRef, useState } from "react"
import { useTranslation } from "react-i18next"
import { toast } from "sonner"
import { AlertCircle, Cat, Check, ChevronDown, Loader2, RotateCcw } from "lucide-react"

import { AnimatedCollapse } from "@/components/ui/animated-presence"
import { Button } from "@/components/ui/button"
import { DeferredNumberInput } from "@/components/ui/deferred-number-input"
import { Switch } from "@/components/ui/switch"
import { ModelChainEditor, type ModelChainRef } from "@/components/ui/model-chain-editor"
import type { AvailableModel } from "@/components/ui/model-selector"
import { getTransport } from "@/lib/transport-provider"
import { cn } from "@/lib/utils"
import { logger } from "@/lib/logger"
import type { SpriteConfig, SpriteSenses, SpriteTriggers } from "@/types/knowledge"
import {
  spriteSettingsErrorToast,
  type SpriteSettingsErrorToast,
} from "./spriteSettingsFeedback"

const SENSE_KEYS: Array<keyof SpriteSenses> = [
  "doc",
  "edit",
  "conversation",
  "memory",
  "awareness",
]

const TRIGGER_KEYS: Array<keyof SpriteTriggers> = [
  "editIdle",
  "noteOpen",
  "conversation",
  "periodic",
  "paste",
]

// Dirty-tracking excludes `enabled`: the master switch auto-persists on toggle
// (in sync with the chat-bar toggle), so flipping it never marks the tuning
// fields as unsaved.
function tuningJson(c: SpriteConfig): string {
  const copy: Record<string, unknown> = { ...c }
  delete copy.enabled
  return JSON.stringify(copy)
}

/**
 * Knowledge-space sprite / inspiration mode settings (Phase 2). Collapsible
 * section under Settings → Knowledge Space. Master enable + trigger/throttle
 * params + per-sense toggles. The per-note sprite toggle lives in the chat
 * panel header; this is the global capability switch + tuning.
 */
export default function SpriteSection() {
  const { t } = useTranslation()
  const [open, setOpen] = useState(false)
  const [cfg, setCfg] = useState<SpriteConfig | null>(null)
  const [snapshot, setSnapshot] = useState("")
  const [loadError, setLoadError] = useState<SpriteSettingsErrorToast | null>(null)
  const [saving, setSaving] = useState(false)
  const [saveStatus, setSaveStatus] = useState<"idle" | "saved" | "failed">("idle")
  // Master-switch persistence is in-flight: disable the Switch (deter a racing
  // double-click) and let the config:changed listener skip its enabled-resync so
  // an unrelated config write can't flicker the toggle mid-flight.
  const [toggling, setToggling] = useState(false)
  const togglingRef = useRef(false)
  const toggleReqRef = useRef(0)
  const [availableModels, setAvailableModels] = useState<AvailableModel[]>([])

  useEffect(() => {
    getTransport()
      .call<AvailableModel[]>("get_available_models")
      .then(setAvailableModels)
      .catch((e) => logger.warn("knowledge", "SpriteSection::loadModels", "load failed", e))
  }, [])

  const applyConfig = useCallback((c: SpriteConfig) => {
    setCfg(c)
    setSnapshot(tuningJson(c))
    setLoadError(null)
  }, [])

  const reload = useCallback(async () => {
    try {
      const c = await getTransport().call<SpriteConfig>("sprite_config_get_cmd")
      applyConfig(c)
    } catch (e) {
      logger.warn("knowledge", "SpriteSection::reload", "load failed", e)
      setLoadError(spriteSettingsErrorToast("load", t, e))
    }
  }, [applyConfig, t])

  useEffect(() => {
    let cancelled = false
    getTransport()
      .call<SpriteConfig>("sprite_config_get_cmd")
      .then((c) => {
        if (cancelled) return
        applyConfig(c)
      })
      .catch((e) => {
        logger.warn("knowledge", "SpriteSection::load", "load failed", e)
        if (!cancelled) setLoadError(spriteSettingsErrorToast("load", t, e))
      })
    // The chat-bar toggle (or another window) can flip `enabled` while this
    // panel is open — sync just that field, never the unsaved tuning edits.
    // NOTE: the EventBus `config:changed` payload's `category` is unreliable for
    // routing — the main write path always tags it `"app"` (the real category
    // only reaches the hooks subsystem; user-config/rollback paths use other
    // values) — so do NOT filter on category; re-pull on any config change.
    const unlisten = getTransport().listen("config:changed", () => {
      // Skip while our own toggle is mid-flight: its handler sets the truth, and
      // an unrelated config write here would otherwise flicker the switch.
      if (togglingRef.current) return
      getTransport()
        .call<SpriteConfig>("sprite_config_get_cmd")
        .then((c) => setCfg((prev) => (prev ? { ...prev, enabled: c.enabled } : c)))
        .catch(() => {})
    })
    return () => {
      cancelled = true
      unlisten()
    }
  }, [applyConfig, t])

  const dirty = cfg != null && tuningJson(cfg) !== snapshot

  // Master switch: persist immediately (optimistic), mirroring the chat-bar
  // toggle. Outside the Save/dirty flow so it stays in sync from either place.
  const toggleEnabled = useCallback(
    (on: boolean) => {
      if (!cfg) return
      const req = ++toggleReqRef.current
      togglingRef.current = true
      setToggling(true)
      // Optimistic: flip only the local `enabled` (tuning edits untouched).
      setCfg((c) => (c ? { ...c, enabled: on } : c))
      void (async () => {
        try {
          // Persist ONLY `enabled` on top of the latest PERSISTED config so this
          // panel's unsaved tuning edits are never silently flushed to disk. If
          // we can't read a clean disk base, abort (the catch rolls back) rather
          // than fall back to the local dirty cfg and write tuning edits.
          const base = await getTransport().call<SpriteConfig>("sprite_config_get_cmd")
          const saved = await getTransport().call<SpriteConfig>("sprite_config_set_cmd", {
            config: { ...base, enabled: on },
          })
          // Only the latest toggle wins (a stale rapid double-click is ignored).
          if (toggleReqRef.current === req) {
            setCfg((prev) => (prev ? { ...prev, enabled: saved.enabled } : saved))
          }
        } catch (e) {
          logger.warn("knowledge", "SpriteSection::toggle", "save failed", e)
          // Roll back so the switch reflects the persisted (unchanged) state.
          if (toggleReqRef.current === req) {
            setCfg((prev) => (prev ? { ...prev, enabled: !on } : prev))
          }
          const failure = spriteSettingsErrorToast("toggle", t, e)
          toast.error(
            failure.title,
            failure.description ? { description: failure.description } : undefined,
          )
        } finally {
          if (toggleReqRef.current === req) {
            togglingRef.current = false
            setToggling(false)
          }
        }
      })()
    },
    [cfg, t],
  )

  const save = useCallback(async () => {
    if (!cfg || saving) return
    setSaving(true)
    try {
      // `enabled` is toggled from the chat bar, not here — preserve the latest
      // value so saving these tuning fields doesn't clobber it.
      let enabled = cfg.enabled
      try {
        enabled = (await getTransport().call<SpriteConfig>("sprite_config_get_cmd")).enabled
      } catch {
        /* fall back to local */
      }
      const saved = await getTransport().call<SpriteConfig>("sprite_config_set_cmd", {
        config: { ...cfg, enabled },
      })
      setCfg(saved)
      setSnapshot(tuningJson(saved))
      setSaveStatus("saved")
      setTimeout(() => setSaveStatus("idle"), 2000)
    } catch (e) {
      logger.warn("knowledge", "SpriteSection::save", "save failed", e)
      setSaveStatus("failed")
      setTimeout(() => setSaveStatus("idle"), 2000)
      const failure = spriteSettingsErrorToast("save", t, e)
      toast.error(
        failure.title,
        failure.description ? { description: failure.description } : undefined,
      )
    } finally {
      setSaving(false)
    }
  }, [cfg, saving, t])

  const patch = (p: Partial<SpriteConfig>) => setCfg((c) => (c ? { ...c, ...p } : c))

  return (
    <div className="rounded-lg border border-border/60 bg-card/40">
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        className="flex w-full items-center gap-2 px-4 py-3 text-left"
      >
        <Cat className="h-4 w-4 text-primary" />
        <span className="text-sm font-medium">{t("settings.sprite.title", "Sprite / inspiration mode")}</span>
        {cfg?.enabled && (
          <span className="rounded-full bg-primary/10 px-1.5 text-[10px] text-primary">
            {t("common.on", "On")}
          </span>
        )}
        <ChevronDown
          className={cn(
            "ml-auto h-4 w-4 text-muted-foreground transition-transform",
            open && "rotate-180",
          )}
        />
      </button>

      <AnimatedCollapse open={open}>
        <div className="space-y-4 border-t border-border/60 px-4 py-3">
          <p className="text-[11px] leading-relaxed text-muted-foreground">
            {t(
              "settings.sprite.intro",
              "A proactive companion that watches the note you're editing and, when you pause, may offer a gentle suggestion in the chat panel. Each suggestion is a bounded LLM call — throttled, and never in incognito sessions.",
            )}
          </p>

          {loadError && (
            <div className="flex items-start gap-2 rounded-md border border-amber-500/25 bg-amber-500/10 px-3 py-2 text-xs text-amber-800 dark:text-amber-200">
              <AlertCircle className="mt-0.5 h-3.5 w-3.5 shrink-0" />
              <div className="min-w-0 flex-1">
                <div className="font-medium">{loadError.title}</div>
                {loadError.description ? (
                  <div className="mt-0.5 whitespace-pre-wrap text-amber-800/80 dark:text-amber-100/80">
                    {loadError.description}
                  </div>
                ) : null}
                <Button
                  type="button"
                  variant="ghost"
                  size="sm"
                  className="mt-1 h-7 px-2 text-xs"
                  onClick={() => void reload()}
                >
                  <RotateCcw className="mr-1.5 h-3.5 w-3.5" />
                  {t("common.retry", "Retry")}
                </Button>
              </div>
            </div>
          )}

          {cfg && (
            <>
              <Row
                label={t("settings.sprite.enabled", "Enable sprite mode")}
                desc={t(
                  "settings.sprite.enabledDesc",
                  "Master switch — also toggleable from the chat panel header.",
                )}
              >
                <Switch
                  checked={cfg.enabled}
                  disabled={toggling}
                  onCheckedChange={(v) => toggleEnabled(v)}
                />
              </Row>

              <Row
                label={t("settings.sprite.proactive", "More proactive")}
                desc={t(
                  "settings.sprite.proactiveDesc",
                  "Lean toward offering a line rather than staying quiet. Turn off for a more restrained sprite.",
                )}
              >
                <Switch
                  checked={cfg.proactive}
                  onCheckedChange={(v) => patch({ proactive: v })}
                />
              </Row>

              <div>
                <div className="mb-1.5 text-xs font-medium">
                  {t("settings.sprite.triggers", "When it chimes in")}
                </div>
                <div className="grid grid-cols-2 gap-1.5">
                  {TRIGGER_KEYS.map((key) => (
                    <label
                      key={key}
                      className="flex items-center justify-between gap-2 rounded-md border border-border/50 px-2 py-1.5"
                    >
                      <span className="truncate text-[11px]">
                        {t(`settings.sprite.trigger.${key}`, key)}
                      </span>
                      <Switch
                        checked={cfg.triggers[key]}
                        onCheckedChange={(v) => patch({ triggers: { ...cfg.triggers, [key]: v } })}
                      />
                    </label>
                  ))}
                </div>
              </div>

              <Row
                label={t("settings.sprite.idleEdit", "Speak after editing pause (s)")}
                desc={t("settings.sprite.idleEditDesc", "Seconds of editing inactivity before it may chime in.")}
              >
                <DeferredNumberInput
                  min={3}
                  max={60}
                  value={cfg.idleEditSecs}
                  onValueCommit={(value) => patch({ idleEditSecs: value })}
                  className="h-7 w-16 text-xs"
                />
              </Row>

              <Row
                label={t("settings.sprite.minChange", "Minimum change (chars)")}
                desc={t("settings.sprite.minChangeDesc", "Only react after you've written at least this much since last time.")}
              >
                <DeferredNumberInput
                  min={20}
                  max={2000}
                  value={cfg.minChangeChars}
                  onValueCommit={(value) => patch({ minChangeChars: value })}
                  className="h-7 w-20 text-xs"
                />
              </Row>

              <Row
                label={t("settings.sprite.cooldown", "Cooldown (s)")}
                desc={t("settings.sprite.cooldownDesc", "Minimum seconds between suggestions.")}
              >
                <DeferredNumberInput
                  min={10}
                  max={3600}
                  value={cfg.cooldownSecs}
                  onValueCommit={(value) => patch({ cooldownSecs: value })}
                  className="h-7 w-16 text-xs"
                />
              </Row>

              <Row
                label={t("settings.sprite.maxPerHour", "Max per hour")}
                desc={t("settings.sprite.maxPerHourDesc", "Hard cap on LLM calls per note each hour.")}
              >
                <DeferredNumberInput
                  min={1}
                  max={60}
                  value={cfg.maxPerSessionPerHour}
                  onValueCommit={(value) => patch({ maxPerSessionPerHour: value })}
                  className="h-7 w-16 text-xs"
                />
              </Row>

              <Row
                label={t("settings.sprite.periodic", "Periodic interval (s)")}
                desc={t(
                  "settings.sprite.periodicDesc",
                  "How often it may chime in during a continuous writing streak (the \"periodic\" trigger).",
                )}
              >
                <DeferredNumberInput
                  min={15}
                  max={600}
                  value={cfg.periodicSecs}
                  onValueCommit={(value) => patch({ periodicSecs: value })}
                  className="h-7 w-16 text-xs"
                />
              </Row>

              <Row
                label={t("settings.sprite.pasteMin", "Paste size (chars)")}
                desc={t(
                  "settings.sprite.pasteMinDesc",
                  "A single insert at least this large counts as a paste and triggers immediately.",
                )}
              >
                <DeferredNumberInput
                  min={40}
                  max={4000}
                  value={cfg.pasteMinChars}
                  onValueCommit={(value) => patch({ pasteMinChars: value })}
                  className="h-7 w-20 text-xs"
                />
              </Row>

              <Row
                label={t("settings.sprite.maxTokens", "Max tokens per suggestion")}
                desc={t("settings.sprite.maxTokensDesc", "Upper bound on each suggestion's length (cost).")}
              >
                <DeferredNumberInput
                  min={64}
                  max={1200}
                  value={cfg.maxTokens}
                  onValueCommit={(value) => patch({ maxTokens: value })}
                  className="h-7 w-20 text-xs"
                />
              </Row>

              <Row
                label={t("settings.sprite.timeout", "Suggestion timeout (s)")}
                desc={t("settings.sprite.timeoutDesc", "Give up on a suggestion that takes longer than this.")}
              >
                <DeferredNumberInput
                  min={5}
                  max={60}
                  value={cfg.timeoutSecs}
                  onValueCommit={(value) => patch({ timeoutSecs: value })}
                  className="h-7 w-16 text-xs"
                />
              </Row>

              <div>
                <div className="mb-1.5 text-xs font-medium">
                  {t("settings.sprite.senses", "What the sprite senses")}
                </div>
                <div className="grid grid-cols-2 gap-1.5">
                  {SENSE_KEYS.map((key) => (
                    <label
                      key={key}
                      className="flex items-center justify-between gap-2 rounded-md border border-border/50 px-2 py-1.5"
                    >
                      <span className="truncate text-[11px]">
                        {t(`settings.sprite.sense.${key}`, key)}
                      </span>
                      <Switch
                        checked={cfg.senses[key]}
                        onCheckedChange={(v) => patch({ senses: { ...cfg.senses, [key]: v } })}
                      />
                    </label>
                  ))}
                </div>
              </div>

              <div className="space-y-1">
                <div className="text-xs font-medium">{t("settings.sprite.model", "Model")}</div>
                <div className="text-[11px] text-muted-foreground">
                  {t(
                    "settings.sprite.modelDesc",
                    "Sprite's suggestion call is fire-and-forget — a full fallback chain is fine, not just a single model.",
                  )}
                </div>
                <ModelChainEditor
                  value={cfg.modelOverride ?? null}
                  onChange={(next: ModelChainRef | null) => patch({ modelOverride: next })}
                  availableModels={availableModels}
                  inheritLabel={t("settings.sprite.modelDefault", "Follow automation default")}
                />
              </div>

              <div className="flex items-center justify-end gap-2 border-t border-border/60 pt-3">
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
