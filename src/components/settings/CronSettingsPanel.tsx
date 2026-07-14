import { useState, useEffect, useCallback, useRef } from "react"
import { getTransport } from "@/lib/transport-provider"
import { useTranslation } from "react-i18next"
import { NumberInput } from "@/components/ui/number-input"
import { Loader2, Check } from "lucide-react"

/**
 * Scheduled-task (cron) global settings. Lifted out of CronCalendarView's top
 * bar into a dedicated Settings section so the cron panel header stays compact
 * and future cron config has room to grow.
 *
 * Behavior matches the original toolbar exactly: inputs are disabled until a
 * real `get_cron_config` load (so a failed fetch never persists the hard-coded
 * defaults over stored values); each commit clamps to the backend band and
 * writes the WHOLE CronConfig (save_cron_config replaces the struct, so a
 * partial body would reset the other fields to their defaults). For the job
 * timeout, 0 means no cron-level timeout; positive values clamp to the backend
 * band. Three-state save feedback auto-clears after 2s.
 */
export default function CronSettingsPanel() {
  const { t } = useTranslation()

  const [maxConcurrent, setMaxConcurrent] = useState<number>(5)
  const [mcInput, setMcInput] = useState<string>("5")
  const [jobTimeout, setJobTimeout] = useState<number>(0)
  const [jtInput, setJtInput] = useState<string>("0")
  const [atGrace, setAtGrace] = useState<number>(300)
  const [agInput, setAgInput] = useState<string>("300")
  const [saving, setSaving] = useState(false)
  const [saveStatus, setSaveStatus] = useState<"idle" | "saved" | "failed">("idle")
  const [loaded, setLoaded] = useState(false)
  const statusTimer = useRef<ReturnType<typeof setTimeout> | null>(null)

  useEffect(() => {
    getTransport()
      .call<{ maxConcurrent?: number; jobTimeoutSecs?: number; atGraceSecs?: number }>(
        "get_cron_config",
      )
      .then((c) => {
        if (c && typeof c.maxConcurrent === "number") {
          setMaxConcurrent(c.maxConcurrent)
          setMcInput(String(c.maxConcurrent))
        }
        if (c && typeof c.jobTimeoutSecs === "number") {
          setJobTimeout(c.jobTimeoutSecs)
          setJtInput(String(c.jobTimeoutSecs))
        }
        if (c && typeof c.atGraceSecs === "number") {
          setAtGrace(c.atGraceSecs)
          setAgInput(String(c.atGraceSecs))
        }
        setLoaded(true)
      })
      .catch(() => {
        // Keep loaded=false so the inputs stay disabled: never persist the
        // hard-coded defaults over the stored config on a transient load failure.
      })
  }, [])

  useEffect(() => {
    return () => {
      if (statusTimer.current) clearTimeout(statusTimer.current)
    }
  }, [])

  const persistCron = useCallback(
    async (cfg: { maxConcurrent: number; jobTimeoutSecs: number; atGraceSecs: number }) => {
      setSaving(true)
      setSaveStatus("idle")
      try {
        await getTransport().call("save_cron_config", { config: cfg })
        setSaveStatus("saved")
      } catch {
        setSaveStatus("failed")
      } finally {
        setSaving(false)
        if (statusTimer.current) clearTimeout(statusTimer.current)
        statusTimer.current = setTimeout(() => setSaveStatus("idle"), 2000)
      }
    },
    [],
  )

  const commitMaxConcurrent = useCallback(async () => {
    const raw = mcInput.trim()
    if (raw === "" || Number.isNaN(Number(raw))) {
      setMcInput(String(maxConcurrent))
      return
    }
    const n = Math.max(0, Math.min(1000, Math.floor(Number(raw))))
    setMcInput(String(n))
    if (n === maxConcurrent) return
    setMaxConcurrent(n)
    await persistCron({ maxConcurrent: n, jobTimeoutSecs: jobTimeout, atGraceSecs: atGrace })
  }, [mcInput, maxConcurrent, jobTimeout, atGrace, persistCron])

  const commitJobTimeout = useCallback(async () => {
    const raw = jtInput.trim()
    if (raw === "" || Number.isNaN(Number(raw))) {
      setJtInput(String(jobTimeout))
      return
    }
    const value = Math.floor(Number(raw))
    const n = value <= 0 ? 0 : Math.max(30, Math.min(7200, value))
    setJtInput(String(n))
    if (n === jobTimeout) return
    setJobTimeout(n)
    await persistCron({ maxConcurrent, jobTimeoutSecs: n, atGraceSecs: atGrace })
  }, [jtInput, jobTimeout, maxConcurrent, atGrace, persistCron])

  const commitAtGrace = useCallback(async () => {
    const raw = agInput.trim()
    if (raw === "" || Number.isNaN(Number(raw))) {
      setAgInput(String(atGrace))
      return
    }
    const n = Math.max(0, Math.min(604800, Math.floor(Number(raw))))
    setAgInput(String(n))
    if (n === atGrace) return
    setAtGrace(n)
    await persistCron({ maxConcurrent, jobTimeoutSecs: jobTimeout, atGraceSecs: n })
  }, [agInput, atGrace, maxConcurrent, jobTimeout, persistCron])

  const rows: {
    label: string
    hint: string
    min: number
    max: number
    value: string
    onChange: (v: string) => void
    onCommit: () => void
  }[] = [
    {
      label: t("cron.maxConcurrent"),
      hint: t("cron.maxConcurrentHint"),
      min: 0,
      max: 1000,
      value: mcInput,
      onChange: setMcInput,
      onCommit: commitMaxConcurrent,
    },
    {
      label: t("cron.jobTimeout"),
      hint: t("cron.jobTimeoutHint"),
      min: 0,
      max: 7200,
      value: jtInput,
      onChange: setJtInput,
      onCommit: commitJobTimeout,
    },
    {
      label: t("cron.atGrace"),
      hint: t("cron.atGraceHint"),
      min: 0,
      max: 604800,
      value: agInput,
      onChange: setAgInput,
      onCommit: commitAtGrace,
    },
  ]

  return (
    <div className="flex-1 overflow-y-auto p-6 space-y-6">
      <div className="flex items-center justify-between">
        <h2 className="text-lg font-semibold">{t("settings.cron")}</h2>
        <div className="h-4 w-4">
          {saving ? (
            <Loader2 className="h-4 w-4 animate-spin text-muted-foreground" />
          ) : saveStatus === "saved" ? (
            <Check className="h-4 w-4 text-green-500" />
          ) : saveStatus === "failed" ? (
            <span className="text-sm font-semibold text-red-500">!</span>
          ) : null}
        </div>
      </div>

      <div className="space-y-5 max-w-2xl">
        {rows.map((row) => (
          <div key={row.label} className="flex items-start justify-between gap-6">
            <div className="min-w-0">
              <div className="text-sm font-medium">{row.label}</div>
              <p className="mt-1 text-xs text-muted-foreground">{row.hint}</p>
            </div>
            <NumberInput
              min={row.min}
              max={row.max}
              disabled={!loaded}
              className="h-8 w-24 shrink-0 text-sm"
              value={row.value}
              onChange={(e) => row.onChange(e.target.value)}
              onBlur={row.onCommit}
              onKeyDown={(e) => {
                if (e.key === "Enter") (e.target as HTMLInputElement).blur()
              }}
            />
          </div>
        ))}
      </div>
    </div>
  )
}
