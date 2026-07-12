import * as React from "react"
import { useTranslation } from "react-i18next"
import * as DropdownMenu from "@radix-ui/react-dropdown-menu"
import { Check, ChevronRight, ChevronDown } from "lucide-react"
import {
  FLOATING_MENU_RADIX_MOTION_CLASS,
  FLOATING_MENU_SURFACE_CLASS,
} from "@/components/ui/floating-menu"
import { cn } from "@/lib/utils"

export interface AvailableModel {
  providerId: string
  providerName: string
  apiType: string
  modelId: string
  modelName: string
  inputTypes: string[]
  contextWindow: number
  maxTokens: number
  reasoning: boolean
}

export interface ModelSelectorProps {
  value: string // Format: "providerId{separator}modelId"
  onChange: (providerId: string, modelId: string) => void
  availableModels: AvailableModel[]
  placeholder?: string
  className?: string
  disabled?: boolean
  defaultOpen?: boolean
  onOpenChange?: (open: boolean) => void
  /** Separator used in value string, default "::" */
  separator?: string
}

export function ModelSelector({
  value,
  onChange,
  availableModels,
  placeholder,
  className,
  disabled,
  defaultOpen,
  onOpenChange,
  separator = "::",
}: ModelSelectorProps) {
  const { t } = useTranslation()

  // Find the selected model to display its name
  const [selectedProviderId, selectedModelId] = value ? value.split(separator) : ["", ""]
  const selectedModel = availableModels.find(
    (m) => m.providerId === selectedProviderId && m.modelId === selectedModelId,
  )

  // Group models by provider
  const modelsByProvider = React.useMemo(() => {
    const groups: Record<string, typeof availableModels> = {}
    availableModels.forEach((m) => {
      if (!groups[m.providerName]) groups[m.providerName] = []
      groups[m.providerName].push(m)
    })
    return groups
  }, [availableModels])

  return (
    <DropdownMenu.Root defaultOpen={defaultOpen} onOpenChange={onOpenChange}>
      <DropdownMenu.Trigger
        disabled={disabled}
        className={cn(
          "flex h-9 w-full items-center justify-between whitespace-nowrap rounded-md border border-border bg-background px-3 py-2 text-sm shadow-sm placeholder:text-muted-foreground hover:bg-secondary/50 focus:outline-none focus:border-ring disabled:cursor-not-allowed disabled:opacity-50 [&>span]:line-clamp-1",
          className,
        )}
      >
        <span className={selectedModel ? "text-foreground" : "text-muted-foreground"}>
          {selectedModel
            ? `${selectedModel.providerName} / ${selectedModel.modelName}`
            : placeholder || t("settings.selectDefaultModel", "Select model")}
        </span>
        <ChevronDown className="h-4 w-4 opacity-50" />
      </DropdownMenu.Trigger>
      <DropdownMenu.Portal>
        <DropdownMenu.Content
          className={cn(
            "z-50 min-w-[12rem] overflow-hidden p-1.5",
            FLOATING_MENU_SURFACE_CLASS,
            FLOATING_MENU_RADIX_MOTION_CLASS,
          )}
          sideOffset={6}
          align="start"
        >
          {Object.entries(modelsByProvider).map(([providerName, models]) => (
            <DropdownMenu.Sub key={providerName}>
              <DropdownMenu.SubTrigger className="flex cursor-default select-none items-center rounded-md px-2.5 py-1.5 text-[13px] text-foreground/80 outline-none transition-colors duration-150 focus:bg-secondary/60 focus:text-foreground data-[state=open]:bg-secondary data-[state=open]:text-foreground data-[state=open]:shadow-sm">
                {providerName}
                <ChevronRight className="ml-auto h-4 w-4" />
              </DropdownMenu.SubTrigger>
              <DropdownMenu.Portal>
                <DropdownMenu.SubContent
                  className={cn(
                    "z-50 min-w-[8rem] overflow-hidden p-1.5",
                    FLOATING_MENU_SURFACE_CLASS,
                    FLOATING_MENU_RADIX_MOTION_CLASS,
                  )}
                  sideOffset={6}
                  alignOffset={-4}
                >
                  {models.map((m) => {
                    const isSelected =
                      m.providerId === selectedProviderId && m.modelId === selectedModelId
                    return (
                      <DropdownMenu.Item
                        key={`${m.providerId}::${m.modelId}`}
                        className="relative flex cursor-default select-none items-center rounded-md px-2.5 py-1.5 text-[13px] text-foreground/80 outline-none transition-colors duration-150 focus:bg-secondary/60 focus:text-foreground data-[disabled]:pointer-events-none data-[disabled]:opacity-50"
                        onSelect={() => onChange(m.providerId, m.modelId)}
                      >
                        <span className="absolute right-2 flex h-3.5 w-3.5 items-center justify-center">
                          {isSelected && <Check className="h-4 w-4" />}
                        </span>
                        <span className="pr-6">{m.modelName}</span>
                      </DropdownMenu.Item>
                    )
                  })}
                </DropdownMenu.SubContent>
              </DropdownMenu.Portal>
            </DropdownMenu.Sub>
          ))}
        </DropdownMenu.Content>
      </DropdownMenu.Portal>
    </DropdownMenu.Root>
  )
}
