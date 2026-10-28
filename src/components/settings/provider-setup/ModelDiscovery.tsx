import { useEffect, useRef, useState } from "react"
import { useTranslation } from "react-i18next"
import { Loader2, Search } from "lucide-react"
import { Button } from "@/components/ui/button"
import { NumberInput } from "@/components/ui/number-input"
import { SearchInput } from "@/components/ui/search-input"
import { Switch } from "@/components/ui/switch"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { getTransport } from "@/lib/transport-provider"
import type { ModelConfig, ProviderConfig } from "./types"

interface Candidate {
  id: string
  name: string
}
interface DiscoveryResult {
  models: Candidate[]
  truncated: boolean
}
interface Selection {
  context: string
  output: string
}

function validLimits(value: Selection) {
  const context = Number(value.context)
  const output = Number(value.output)
  return (
    Number.isSafeInteger(context) &&
    context > 0 &&
    context <= 0xffffffff &&
    Number.isSafeInteger(output) &&
    output > 0 &&
    output <= context
  )
}

/** A discovery result is only a draft. Appending requires explicit capability
 * confirmation and budgets; persistence still belongs to the parent's Save. */
export function ModelDiscovery({
  config,
  onAdd,
}: {
  config: ProviderConfig
  onAdd: (models: ModelConfig[]) => void
}) {
  const { t } = useTranslation()
  const [open, setOpen] = useState(false)
  const [loading, setLoading] = useState(false)
  const [result, setResult] = useState<DiscoveryResult | null>(null)
  const [error, setError] = useState(false)
  const [query, setQuery] = useState("")
  const [selected, setSelected] = useState(new Map<string, Selection>())
  const [confirmed, setConfirmed] = useState(false)
  const generation = useRef(0)
  // Request-local only; never render/log this credential-bearing identity.
  const connection = JSON.stringify([
    config.id,
    config.apiType,
    config.baseUrl,
    config.apiKey,
    config.authProfiles,
    config.userAgent,
    config.allowPrivateNetwork,
  ])
  useEffect(() => {
    const invalidateRequests = () => {
      generation.current++
    }
    invalidateRequests()
    setLoading(false)
    setResult(null)
    setSelected(new Map())
    setConfirmed(false)
    setError(false)
    setOpen(false)
    return invalidateRequests
  }, [connection])

  const existing = new Set(config.models.map((model) => model.id.trim()))
  const candidates = result?.models.filter((model) => !existing.has(model.id)) ?? []
  const selections = [...selected].filter(([id]) => candidates.some((model) => model.id === id))
  const canAdd =
    confirmed && selections.length > 0 && selections.every(([, limits]) => validLimits(limits))

  async function discover() {
    const request = ++generation.current
    setLoading(true)
    setError(false)
    setResult(null)
    setQuery("")
    setSelected(new Map())
    setConfirmed(false)
    setOpen(true)
    try {
      const discovered = await getTransport().call<DiscoveryResult>("discover_provider_models", {
        config,
      })
      if (request === generation.current) setResult(discovered)
    } catch {
      // Provider responses and transport errors can contain sensitive echoes.
      if (request === generation.current) setError(true)
    } finally {
      if (request === generation.current) setLoading(false)
    }
  }

  function addSelected() {
    if (!canAdd) return
    const models = selections.map(
      ([id, limits]): ModelConfig => ({
        id,
        name: candidates.find((model) => model.id === id)!.name,
        contextWindow: Number(limits.context),
        maxTokens: Number(limits.output),
        inputTypes: ["text"],
        reasoning: false,
        costInput: null,
        costOutput: null,
      }),
    )
    onAdd(models)
    setOpen(false)
    setSelected(new Map())
    setConfirmed(false)
  }

  return (
    <>
      <Button
        type="button"
        variant="secondary"
        size="sm"
        disabled={!config.baseUrl.trim() || config.apiType === "codex" || loading}
        onClick={() => void discover()}
      >
        {loading ? (
          <Loader2 className="mr-1.5 h-3.5 w-3.5 animate-spin" />
        ) : (
          <Search className="mr-1.5 h-3.5 w-3.5" />
        )}
        {t("provider.discovery.action")}
      </Button>
      <Dialog
        open={open}
        onOpenChange={(value) => {
          setOpen(value)
          if (!value) {
            generation.current++
            setLoading(false)
          }
        }}
      >
        <DialogContent
          className="max-h-[85vh] max-w-2xl overflow-y-auto"
          onPointerDownOutside={(event) => event.preventDefault()}
        >
          <DialogHeader>
            <DialogTitle>{t("provider.discovery.action")}</DialogTitle>
            <DialogDescription>{t("provider.discovery.description")}</DialogDescription>
          </DialogHeader>
          {loading && (
            <p role="status" className="text-sm text-muted-foreground">
              {t("common.loading")}
            </p>
          )}
          {error && (
            <p role="alert" className="text-sm text-destructive">
              {t("provider.discovery.error")}
            </p>
          )}
          {result && (
            <>
              {result.truncated && (
                <p role="status" className="text-xs text-muted-foreground">
                  {t("provider.discovery.truncated")}
                </p>
              )}
              <SearchInput
                aria-label={t("provider.discovery.search")}
                placeholder={t("provider.discovery.search")}
                value={query}
                onChange={(event) => setQuery(event.target.value)}
              />
              {candidates.length === 0 && (
                <p className="text-sm text-muted-foreground">{t("provider.discovery.empty")}</p>
              )}
              <div className="max-h-[45vh] space-y-3 overflow-y-auto">
                {candidates
                  .filter((model) =>
                    `${model.id} ${model.name}`.toLowerCase().includes(query.toLowerCase()),
                  )
                  .map((model) => {
                    const limits = selected.get(model.id)
                    const update = (value: Partial<Selection>) => {
                      setSelected((old) =>
                        new Map(old).set(model.id, { ...old.get(model.id)!, ...value }),
                      )
                      setConfirmed(false)
                    }
                    return (
                      <div key={model.id} className="space-y-2 rounded-lg border border-border p-3">
                        <div className="flex items-center justify-between gap-3">
                          <div className="min-w-0 text-xs">
                            <p className="break-all font-medium">{model.name}</p>
                            <p className="break-all text-muted-foreground">{model.id}</p>
                          </div>
                          <Switch
                            aria-label={t("provider.discovery.select", { id: model.id })}
                            checked={Boolean(limits)}
                            onCheckedChange={(checked) => {
                              setSelected((old) => {
                                const next = new Map(old)
                                if (checked) next.set(model.id, { context: "", output: "" })
                                else next.delete(model.id)
                                return next
                              })
                              setConfirmed(false)
                            }}
                          />
                        </div>
                        {limits && (
                          <div className="grid grid-cols-2 gap-3">
                            <label className="space-y-1 text-xs">
                              {t("model.contextWindow")}
                              <NumberInput
                                aria-label={t("provider.discovery.context", { id: model.id })}
                                min={1}
                                max={0xffffffff}
                                step={1}
                                value={limits.context}
                                onChange={(event) => update({ context: event.target.value })}
                              />
                            </label>
                            <label className="space-y-1 text-xs">
                              {t("model.maxTokens")}
                              <NumberInput
                                aria-label={t("provider.discovery.output", { id: model.id })}
                                min={1}
                                max={0xffffffff}
                                step={1}
                                value={limits.output}
                                onChange={(event) => update({ output: event.target.value })}
                              />
                            </label>
                          </div>
                        )}
                      </div>
                    )
                  })}
              </div>
              {selections.length > 0 && (
                <label className="flex items-start gap-3 text-xs text-muted-foreground">
                  <Switch
                    aria-label={t("provider.discovery.confirm")}
                    checked={confirmed}
                    onCheckedChange={setConfirmed}
                  />
                  {t("provider.discovery.confirm")}
                </label>
              )}
            </>
          )}
          <DialogFooter>
            <Button
              type="button"
              variant="secondary"
              onClick={() => {
                generation.current++
                setLoading(false)
                setOpen(false)
              }}
            >
              {t("common.cancel")}
            </Button>
            {error && (
              <Button type="button" variant="secondary" onClick={() => void discover()}>
                {t("common.retry")}
              </Button>
            )}
            <Button type="button" disabled={!canAdd || loading} onClick={addSelected}>
              {t("provider.discovery.add", { count: selections.length })}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </>
  )
}
