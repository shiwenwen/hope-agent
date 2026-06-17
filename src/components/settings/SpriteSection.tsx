import { useCallback, useEffect, useRef, useState } from "react"
import { useTranslation } from "react-i18next"
import { Cat, Check, ChevronDown, Loader2 } from "lucide-react"

import { AnimatedCollapse } from "@/components/ui/animated-presence"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Switch } from "@/components/ui/switch"
import { getTransport } from "@/lib/transport-provider"
import { cn } from "@/lib/utils"
import { logger } from "@/lib/logger"
import type { SpriteConfig, SpriteSenses, SpriteTriggers } from "@/types/knowledge"

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
  const [saving, setSaving] = useState(false)
  const [saveStatus, setSaveStatus] = useState<"idle" | "saved" | "failed">("idle")
  // Master-switch persistence is in-flight: disable the Switch (deter a racing
  // double-click) and let the config:changed listener skip its enabled-resync so
  // an unrelated config write can't flicker the toggle mid-flight.
  const [toggling, setToggling] = useState(false)
  const togglingRef = useRef(false)
  const toggleReqRef = useRef(0)

  useEffect(() => {
    getTransport()
      .call<SpriteConfig>("sprite_config_get_cmd")
      .then((c) => {
        setCfg(c)
        setSnapshot(tuningJson(c))
      })
      .catch((e) => logger.warn("knowledge", "SpriteSection::load", "load failed", e))
    // The chat-bar toggle (or another window) can flip `enabled` while this
    // panel is open — sync just that field, never the unsaved tuning edits.
    // NOTE: the EventBus `config:changed` payload's `category` is unreliable for
    // routing — the main write path always tags it `"app"` (the real category
    // only reaches the hooks subsystem; user-config/rollback paths use other
    // values) — so do NOT filter on category; re-pull on any config change.
    return getTransport().listen("config:changed", () => {
      // Skip while our own toggle is mid-flight: its handler sets the truth, and
      // an unrelated config write here would otherwise flicker the switch.
      if (togglingRef.current) return
      getTransport()
        .call<SpriteConfig>("sprite_config_get_cmd")
        .then((c) => setCfg((prev) => (prev ? { ...prev, enabled: c.enabled } : c)))
        .catch(() => {})
    })
  }, [])

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
        } finally {
          if (toggleReqRef.current === req) {
            togglingRef.current = false
            setToggling(false)
          }
        }
      })()
    },
    [cfg],
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
    } finally {
      setSaving(false)
    }
  }, [cfg, saving])

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
                <Input
                  type="number"
                  min={3}
                  max={60}
                  value={cfg.idleEditSecs}
                  onChange={(e) =>
                    patch({ idleEditSecs: Math.min(60, Math.max(3, Number(e.target.value) || 8)) })
                  }
                  className="h-7 w-16 text-xs"
                />
              </Row>

              <Row
                label={t("settings.sprite.minChange", "Minimum change (chars)")}
                desc={t("settings.sprite.minChangeDesc", "Only react after you've written at least this much since last time.")}
              >
                <Input
                  type="number"
                  min={20}
                  max={2000}
                  value={cfg.minChangeChars}
                  onChange={(e) =>
                    patch({
                      minChangeChars: Math.min(2000, Math.max(20, Number(e.target.value) || 80)),
                    })
                  }
                  className="h-7 w-20 text-xs"
                />
              </Row>

              <Row
                label={t("settings.sprite.cooldown", "Cooldown (s)")}
                desc={t("settings.sprite.cooldownDesc", "Minimum seconds between suggestions.")}
              >
                <Input
                  type="number"
                  min={10}
                  max={3600}
                  value={cfg.cooldownSecs}
                  onChange={(e) =>
                    patch({ cooldownSecs: Math.min(3600, Math.max(10, Number(e.target.value) || 45)) })
                  }
                  className="h-7 w-16 text-xs"
                />
              </Row>

              <Row
                label={t("settings.sprite.maxPerHour", "Max per hour")}
                desc={t("settings.sprite.maxPerHourDesc", "Hard cap on LLM calls per note each hour.")}
              >
                <Input
                  type="number"
                  min={1}
                  max={60}
                  value={cfg.maxPerSessionPerHour}
                  onChange={(e) =>
                    patch({
                      maxPerSessionPerHour: Math.min(60, Math.max(1, Number(e.target.value) || 12)),
                    })
                  }
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
                <Input
                  type="number"
                  min={15}
                  max={600}
                  value={cfg.periodicSecs}
                  onChange={(e) =>
                    patch({ periodicSecs: Math.min(600, Math.max(15, Number(e.target.value) || 120)) })
                  }
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
                <Input
                  type="number"
                  min={40}
                  max={4000}
                  value={cfg.pasteMinChars}
                  onChange={(e) =>
                    patch({ pasteMinChars: Math.min(4000, Math.max(40, Number(e.target.value) || 180)) })
                  }
                  className="h-7 w-20 text-xs"
                />
              </Row>

              <Row
                label={t("settings.sprite.maxTokens", "Max tokens per suggestion")}
                desc={t("settings.sprite.maxTokensDesc", "Upper bound on each suggestion's length (cost).")}
              >
                <Input
                  type="number"
                  min={64}
                  max={1200}
                  value={cfg.maxTokens}
                  onChange={(e) =>
                    patch({ maxTokens: Math.min(1200, Math.max(64, Number(e.target.value) || 400)) })
                  }
                  className="h-7 w-20 text-xs"
                />
              </Row>

              <Row
                label={t("settings.sprite.timeout", "Suggestion timeout (s)")}
                desc={t("settings.sprite.timeoutDesc", "Give up on a suggestion that takes longer than this.")}
              >
                <Input
                  type="number"
                  min={5}
                  max={60}
                  value={cfg.timeoutSecs}
                  onChange={(e) =>
                    patch({ timeoutSecs: Math.min(60, Math.max(5, Number(e.target.value) || 20)) })
                  }
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
