import { useTranslation } from "react-i18next"
import * as DropdownMenu from "@radix-ui/react-dropdown-menu"
import { Check, ChevronRight, ChevronDown } from "lucide-react"
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
  const modelsByProvider = (() => {
    const groups: Record<string, typeof availableModels> = {}
    availableModels.forEach((m) => {
      if (!groups[m.providerName]) groups[m.providerName] = []
      groups[m.providerName].push(m)
    })
    return groups
  })()

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
          className="z-50 min-w-[12rem] overflow-hidden rounded-md border bg-popover p-1 text-popover-foreground shadow-md data-[state=open]:animate-in data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=open]:fade-in-0 data-[state=closed]:zoom-out-95 data-[state=open]:zoom-in-95 data-[side=bottom]:slide-in-from-top-2 data-[side=left]:slide-in-from-right-2 data-[side=right]:slide-in-from-left-2 data-[side=top]:slide-in-from-bottom-2"
          sideOffset={4}
          align="start"
        >
          {Object.entries(modelsByProvider).map(([providerName, models]) => (
            <DropdownMenu.Sub key={providerName}>
              <DropdownMenu.SubTrigger className="flex cursor-default select-none items-center rounded-sm px-2 py-1.5 text-sm outline-none focus:bg-accent data-[state=open]:bg-accent">
                {providerName}
                <ChevronRight className="ml-auto h-4 w-4" />
              </DropdownMenu.SubTrigger>
              <DropdownMenu.Portal>
                <DropdownMenu.SubContent
                  className="z-50 min-w-[8rem] overflow-hidden rounded-md border bg-popover p-1 text-popover-foreground shadow-lg data-[state=open]:animate-in data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=open]:fade-in-0 data-[state=closed]:zoom-out-95 data-[state=open]:zoom-in-95 data-[side=bottom]:slide-in-from-top-2 data-[side=left]:slide-in-from-right-2 data-[side=right]:slide-in-from-left-2 data-[side=top]:slide-in-from-bottom-2"
                  sideOffset={4}
                  alignOffset={-4}
                >
                  {models.map((m) => {
                    const isSelected =
                      m.providerId === selectedProviderId && m.modelId === selectedModelId
                    return (
                      <DropdownMenu.Item
                        key={`${m.providerId}::${m.modelId}`}
                        className="relative flex cursor-default select-none items-center rounded-sm px-2 py-1.5 text-sm outline-none focus:bg-accent focus:text-accent-foreground data-[disabled]:pointer-events-none data-[disabled]:opacity-50"
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
