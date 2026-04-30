import { useEffect, useMemo, useState } from "react"
import { useTranslation } from "react-i18next"
import { toast } from "sonner"
import { Plus, RotateCcw, Trash2, Loader2 } from "lucide-react"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"

/**
 * Reusable list editor for the three pattern lists (protected paths /
 * dangerous commands / edit commands). Talks to a tuple of Tauri/HTTP
 * commands that follow the standard {get,set,reset} naming.
 */
interface PatternListEditorProps {
  /** Section title shown above the list (already i18n'd by the caller). */
  title: string
  /** Section description. */
  description: string
  /** Inline placeholder for the "add new pattern" input. */
  inputPlaceholder: string
  /** Tauri/HTTP command names — same on both transports. */
  getCmd: string
  setCmd: string
  resetCmd: string
}

interface ListPayload {
  current: string[]
  defaults: string[]
}

export default function PatternListEditor({
  title,
  description,
  inputPlaceholder,
  getCmd,
  setCmd,
  resetCmd,
}: PatternListEditorProps) {
  const { t } = useTranslation()
  const [patterns, setPatterns] = useState<string[]>([])
  const [defaults, setDefaults] = useState<string[]>([])
  const [draft, setDraft] = useState("")
  const [loading, setLoading] = useState(true)
  const [busy, setBusy] = useState(false)

  const isPristine = useMemo(() => {
    if (patterns.length !== defaults.length) return false
    return patterns.every((p, i) => p === defaults[i])
  }, [patterns, defaults])

  useEffect(() => {
    let cancelled = false
    getTransport()
      .call<ListPayload>(getCmd)
      .then((p) => {
        if (cancelled) return
        setPatterns(p.current)
        setDefaults(p.defaults)
      })
      .catch((e) => logger.error("settings", "approvalPanel", `${getCmd} failed`, e))
      .finally(() => {
        if (!cancelled) setLoading(false)
      })
    return () => {
      cancelled = true
    }
  }, [getCmd])

  const persist = async (next: string[]) => {
    setBusy(true)
    try {
      await getTransport().call(setCmd, { patterns: next })
      setPatterns(next)
    } catch (e) {
      logger.error("settings", "approvalPanel", `${setCmd} failed`, e)
      toast.error(t("settings.approvalPanel.saveFailed"))
    } finally {
      setBusy(false)
    }
  }

  const addPattern = async () => {
    const value = draft.trim()
    if (!value) return
    if (patterns.includes(value)) {
      setDraft("")
      return
    }
    await persist([...patterns, value])
    setDraft("")
  }

  const removePattern = async (idx: number) => {
    const next = patterns.filter((_, i) => i !== idx)
    await persist(next)
  }

  const restoreDefaults = async () => {
    setBusy(true)
    try {
      const next = await getTransport().call<string[]>(resetCmd)
      setPatterns(next)
      toast.success(t("settings.approvalPanel.restoredDefaults"))
    } catch (e) {
      logger.error("settings", "approvalPanel", `${resetCmd} failed`, e)
      toast.error(t("settings.approvalPanel.saveFailed"))
    } finally {
      setBusy(false)
    }
  }

  return (
    <section className="rounded-lg border border-border/50 bg-card/40 p-4">
      <header className="flex items-start justify-between gap-3 mb-3">
        <div>
          <h3 className="text-sm font-medium text-foreground">{title}</h3>
          <p className="text-xs text-muted-foreground mt-0.5">{description}</p>
        </div>
        <Button
          variant="outline"
          size="sm"
          onClick={restoreDefaults}
          disabled={busy || isPristine}
          className="shrink-0 h-7 text-xs"
        >
          <RotateCcw className="h-3 w-3 mr-1" />
          {t("settings.approvalPanel.restoreDefaults")}
        </Button>
      </header>

      <div className="flex gap-2 mb-3">
        <Input
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          placeholder={inputPlaceholder}
          onKeyDown={(e) => {
            if (e.key === "Enter") {
              e.preventDefault()
              void addPattern()
            }
          }}
          disabled={busy}
          className="text-xs h-8"
        />
        <Button onClick={addPattern} size="sm" disabled={busy || !draft.trim()} className="h-8">
          <Plus className="h-3.5 w-3.5 mr-1" />
          {t("settings.approvalPanel.add")}
        </Button>
      </div>

      <div className="rounded-md border border-border/40 overflow-hidden bg-background/40">
        {loading ? (
          <div className="flex items-center justify-center py-6 text-xs text-muted-foreground">
            <Loader2 className="h-3.5 w-3.5 mr-2 animate-spin" />
            {t("settings.approvalPanel.loading")}
          </div>
        ) : patterns.length === 0 ? (
          <div className="text-center py-6 text-xs text-muted-foreground">
            {t("settings.approvalPanel.empty")}
          </div>
        ) : (
          patterns.map((pat, idx) => (
            <div
              key={`${pat}-${idx}`}
              className={`flex items-center justify-between px-3 py-1.5 gap-2 text-xs ${
                idx > 0 ? "border-t border-border/30" : ""
              }`}
            >
              <code className="font-mono text-foreground/90 truncate flex-1">{pat}</code>
              <Button
                variant="ghost"
                size="icon"
                className="h-6 w-6 text-muted-foreground hover:text-destructive"
                onClick={() => removePattern(idx)}
                disabled={busy}
              >
                <Trash2 className="h-3 w-3" />
              </Button>
            </div>
          ))
        )}
      </div>
    </section>
  )
}
