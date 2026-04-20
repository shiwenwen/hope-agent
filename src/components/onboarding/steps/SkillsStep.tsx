import { useEffect, useState } from "react"
import { useTranslation } from "react-i18next"
import { Package, Loader2 } from "lucide-react"

import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"

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
 * Step 6 — bundled skills allow-list.
 *
 * Filter rule:
 *   - `source === "bundled"` — only skills shipped with the binary.
 *     Managed / project / shared skills are tuned in the full Settings
 *     panel, not here.
 *   - `always !== true` — core skills like `ha-settings` set
 *     `always: true` in their SKILL.md frontmatter and must never appear
 *     in a disable list, otherwise the model loses config-management
 *     access.
 */
export function SkillsStep({ initialDisabled, onChange }: SkillsStepProps) {
  const { t } = useTranslation()
  const [skills, setSkills] = useState<SkillInfo[] | null>(null)
  const [disabled, setDisabled] = useState<Set<string>>(new Set(initialDisabled))
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    void (async () => {
      try {
        const raw = await getTransport().call<SkillInfo[]>("get_skills")
        setSkills(raw.filter((s) => s.source === "bundled" && s.always !== true))
      } catch (e) {
        logger.warn("onboarding", "SkillsStep", "get_skills failed", e)
        setError(String(e))
        setSkills([])
      }
    })()
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

  return (
    <div className="px-6 py-6 space-y-4 max-w-xl mx-auto">
      <div className="text-center space-y-1">
        <h2 className="text-xl font-semibold">{t("onboarding.skills.title")}</h2>
        <p className="text-sm text-muted-foreground">{t("onboarding.skills.subtitle")}</p>
      </div>

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
        <ul className="space-y-2 max-h-[360px] overflow-y-auto pr-1">
          {skills.map((s) => {
            const off = disabled.has(s.name)
            return (
              <li key={s.name}>
                <button
                  type="button"
                  onClick={() => toggle(s.name)}
                  className={`w-full text-left rounded-md border px-3 py-2 flex items-start gap-3 transition-colors ${
                    off
                      ? "border-border bg-muted/40 opacity-70"
                      : "border-primary/30 bg-primary/5"
                  }`}
                >
                  <div
                    className={`mt-0.5 flex h-5 w-5 shrink-0 items-center justify-center rounded border ${
                      off
                        ? "border-muted-foreground/30"
                        : "border-primary bg-primary text-primary-foreground"
                    }`}
                    aria-hidden
                  >
                    {!off && <span className="text-xs">✓</span>}
                  </div>
                  <div className="min-w-0 flex-1">
                    <div className="flex items-center gap-1.5">
                      <Package className="h-3.5 w-3.5 text-muted-foreground" />
                      <span className="font-medium text-sm">{s.name}</span>
                    </div>
                    {s.description && (
                      <p className="text-xs text-muted-foreground mt-0.5 line-clamp-2">
                        {s.description}
                      </p>
                    )}
                  </div>
                </button>
              </li>
            )
          })}
        </ul>
      )}
    </div>
  )
}
