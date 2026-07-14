import * as React from "react"
import { useTranslation } from "react-i18next"
import * as DropdownMenu from "@radix-ui/react-dropdown-menu"
import { Check, ChevronRight, ChevronDown } from "lucide-react"
import {
  FLOATING_MENU_RADIX_MOTION_CLASS,
  FLOATING_MENU_SURFACE_CLASS,
} from "@/components/ui/floating-menu"
import { FLAT_CONTROL_SURFACE_CLASS } from "@/components/ui/control-surface"
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
  /** Vision-only mode: models without image input support stay visible but
   *  grayed out (not selectable) with a "no image support" hint. Omitting the
   *  prop keeps behavior byte-identical for existing callers. */
  requireVision?: boolean
  /** Optional "clear selection" item pinned at the top of the menu (e.g.
   *  「跟随默认模型」). Rendered only when both are provided; selecting it
   *  calls `onClear`. Omitting keeps behavior byte-identical. */
  clearLabel?: string
  onClear?: () => void
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
  requireVision,
  clearLabel,
  onClear,
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
          FLAT_CONTROL_SURFACE_CLASS,
          "flex h-9 w-full items-center justify-between gap-1.5 whitespace-nowrap px-2.5 py-1.5 text-sm placeholder:text-muted-foreground [&>span]:line-clamp-1",
          className,
        )}
      >
        <span className={selectedModel ? "text-foreground" : "text-muted-foreground"}>
          {selectedModel
            ? `${selectedModel.providerName} / ${selectedModel.modelName}`
            : placeholder || t("settings.selectDefaultModel", "Select model")}
        </span>
        <ChevronDown className="h-3 w-3 shrink-0 text-muted-foreground/70" />
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
          {clearLabel && onClear && (
            <DropdownMenu.Item
              className="relative flex cursor-default select-none items-center rounded-md px-2.5 py-1.5 text-[13px] text-foreground/80 outline-none transition-colors duration-150 focus:bg-secondary/60 focus:text-foreground"
              onSelect={onClear}
            >
              <span className="absolute right-2 flex h-3.5 w-3.5 items-center justify-center">
                {!selectedModel && <Check className="h-4 w-4" />}
              </span>
              <span className="pr-6">{clearLabel}</span>
            </DropdownMenu.Item>
          )}
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
                    // Vision gate: gray out (not hide) so users see WHY their
                    // usual model is unavailable instead of it vanishing.
                    // Empty inputTypes = "unconfigured, assume capable" — mirrors
                    // the backend's `model_supports_vision` three-state semantics
                    // (custom provider entries default to an empty list).
                    const visionBlocked =
                      !!requireVision &&
                      m.inputTypes.length > 0 &&
                      !m.inputTypes.includes("image")
                    return (
                      <DropdownMenu.Item
                        key={`${m.providerId}::${m.modelId}`}
                        disabled={visionBlocked}
                        className="relative flex cursor-default select-none items-center rounded-md px-2.5 py-1.5 text-[13px] text-foreground/80 outline-none transition-colors duration-150 focus:bg-secondary/60 focus:text-foreground data-[disabled]:pointer-events-none data-[disabled]:opacity-50"
                        onSelect={() => onChange(m.providerId, m.modelId)}
                      >
                        <span className="absolute right-2 flex h-3.5 w-3.5 items-center justify-center">
                          {isSelected && <Check className="h-4 w-4" />}
                        </span>
                        <span className={visionBlocked ? "pr-1" : "pr-6"}>{m.modelName}</span>
                        {visionBlocked && (
                          <span className="ml-auto shrink-0 pr-6 text-[10px] text-muted-foreground/70">
                            {t("common.modelNoVision", "不支持图片")}
                          </span>
                        )}
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
