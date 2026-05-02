import { useState } from "react"
import { useTranslation } from "react-i18next"
import { cn } from "@/lib/utils"
import { Check } from "lucide-react"
import ProviderIcon from "@/components/common/ProviderIcon"

interface ModelPickerData {
  models: {
    providerId: string
    providerName: string
    modelId: string
    modelName: string
  }[]
  activeProviderId?: string
  activeModelId?: string
}

interface ModelPickerCardProps {
  data: ModelPickerData
  onSelect: (providerId: string, modelId: string) => void
}

export default function ModelPickerCard({ data, onSelect }: ModelPickerCardProps) {
  const { t } = useTranslation()
  const [switchedKey, setSwitchedKey] = useState<string | null>(null)

  // Group models by provider
  const groups = (() => {
    const map = new Map<
      string,
      { providerName: string; providerId: string; models: ModelPickerData["models"] }
    >()
    for (const m of data.models) {
      const key = m.providerId
      if (!map.has(key)) {
        map.set(key, { providerName: m.providerName, providerId: m.providerId, models: [] })
      }
      map.get(key)!.models.push(m)
    }
    return Array.from(map.values())
  })()

  const handleClick = (providerId: string, modelId: string) => {
    setSwitchedKey(`${providerId}::${modelId}`)
    onSelect(providerId, modelId)
  }

  return (
    <div className="w-full max-w-lg rounded-xl border border-border bg-card shadow-sm overflow-hidden">
      <div className="px-4 py-2.5 border-b border-border bg-muted/30">
        <span className="text-sm font-medium text-foreground">
          {t("slashCommands.models.cardTitle", "Available Models")}
        </span>
      </div>
      <div className="p-3 space-y-3">
        {groups.map((group) => (
          <div key={group.providerId}>
            <div className="flex items-center gap-1.5 mb-1.5 px-1">
              <ProviderIcon providerName={group.providerName} size={14} color />
              <span className="text-xs font-medium text-muted-foreground">
                {group.providerName}
              </span>
            </div>
            <div className="flex flex-wrap gap-1.5">
              {group.models.map((m) => {
                const key = `${m.providerId}::${m.modelId}`
                const isActive =
                  data.activeProviderId === m.providerId && data.activeModelId === m.modelId
                const justSwitched = switchedKey === key && !isActive

                return (
                  <button
                    key={key}
                    onClick={() => handleClick(m.providerId, m.modelId)}
                    disabled={isActive}
                    className={cn(
                      "inline-flex items-center gap-1 px-2.5 py-1 rounded-md text-xs font-medium transition-colors",
                      "border cursor-pointer",
                      isActive
                        ? "bg-primary/10 border-primary/30 text-primary cursor-default"
                        : justSwitched
                          ? "bg-green-500/10 border-green-500/30 text-green-600 dark:text-green-400"
                          : "bg-background border-border text-foreground hover:bg-accent hover:border-accent-foreground/20",
                    )}
                  >
                    {(isActive || justSwitched) && <Check className="size-3" />}
                    {m.modelName}
                  </button>
                )
              })}
            </div>
          </div>
        ))}
      </div>
    </div>
  )
}
