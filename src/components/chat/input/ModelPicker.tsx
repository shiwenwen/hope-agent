import { useState, useRef, useEffect, useCallback, type CSSProperties } from "react"
import { useTranslation } from "react-i18next"
import { cn } from "@/lib/utils"
import { Check, ChevronDown, ChevronRight } from "lucide-react"
import { Slider } from "@/components/ui/slider"
import { FloatingMenu } from "@/components/ui/floating-menu"
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip"
import { Switch } from "@/components/ui/switch"
import ProviderIcon from "@/components/common/ProviderIcon"
import ModelCapabilityBadges from "@/components/chat/ModelCapabilityBadges"
import {
  modelSupportsInputTypes,
  type UnsupportedModelBehavior,
} from "@/components/chat/model-capabilities"
import type { AvailableModel, ActiveModel } from "@/types/chat"
import { getEffortOptionsForModel, modelSupportsThinking } from "@/types/chat"

const MODEL_SUBMENU_WIDTH = 280
const TEMPERATURE_SUBMENU_WIDTH = 220
const SUBMENU_GAP = 6
const VIEWPORT_MARGIN = 8

type SubmenuPlacement = "right" | "left" | "top" | "bottom"

export interface ModelPickerProps {
  availableModels: AvailableModel[]
  activeModel: ActiveModel | null
  reasoningEffort: string
  onModelChange: (key: string, options?: { applyToAgentDefault?: boolean }) => void
  onEffortChange: (effort: string, options?: { applyToAgentDefault?: boolean }) => void
  onEffortReset?: () => void
  currentModelInfo?: AvailableModel
  unavailablePreference?: string | null
  sessionTemperature?: number | null
  onSessionTemperatureChange?: (
    temp: number | null,
    options?: { applyToAgentDefault?: boolean },
  ) => void
  /** Every listed type must be supported for a model to remain selectable. */
  requiredInputTypes?: string[]
  /** Unsupported models are disabled by default, or can be removed entirely. */
  unsupportedBehavior?: UnsupportedModelBehavior
}

export default function ModelPicker({
  availableModels,
  activeModel,
  reasoningEffort,
  onModelChange,
  onEffortChange,
  onEffortReset,
  currentModelInfo,
  unavailablePreference,
  sessionTemperature,
  onSessionTemperatureChange,
  requiredInputTypes,
  unsupportedBehavior = "disable",
}: ModelPickerProps) {
  const { t } = useTranslation()
  const [showMenu, setShowMenu] = useState(false)
  const [openPanel, setOpenPanel] = useState<"model" | "temperature" | null>(null)
  const [submenuPlacement, setSubmenuPlacement] = useState<SubmenuPlacement>("right")
  const [submenuStyle, setSubmenuStyle] = useState<CSSProperties>()
  const [applyToAgentDefault, setApplyToAgentDefault] = useState(false)
  // `null` means there is no active slider drag; outside a drag the control
  // derives directly from the current Session prop, so no syncing effect is
  // needed when switching conversations.
  const [temperatureDraft, setTemperatureDraft] = useState<number | null>(null)
  const menuRef = useRef<HTMLDivElement>(null)
  const rootMenuRef = useRef<HTMLDivElement>(null)
  const modelSubmenuRef = useRef<HTMLDivElement>(null)
  const temperatureSubmenuRef = useRef<HTMLDivElement>(null)

  const placeSubmenu = useCallback((panel: "model" | "temperature") => {
    const root = rootMenuRef.current
    if (!root) {
      setSubmenuPlacement("right")
      setSubmenuStyle(undefined)
      return
    }

    const submenuWidth = panel === "model" ? MODEL_SUBMENU_WIDTH : TEMPERATURE_SUBMENU_WIDTH
    const rootRect = root.getBoundingClientRect()
    const requiredSideSpace = SUBMENU_GAP + submenuWidth
    const rightSpace = window.innerWidth - VIEWPORT_MARGIN - rootRect.right
    const bottomOffset = Math.max(VIEWPORT_MARGIN, window.innerHeight - rootRect.bottom)
    if (rightSpace >= requiredSideSpace) {
      setSubmenuPlacement("right")
      setSubmenuStyle({
        bottom: bottomOffset,
        left: rootRect.right + SUBMENU_GAP,
      })
      return
    }

    const leftSpace = rootRect.left - VIEWPORT_MARGIN
    if (leftSpace >= requiredSideSpace) {
      setSubmenuPlacement("left")
      setSubmenuStyle({
        bottom: bottomOffset,
        right: window.innerWidth - rootRect.left + SUBMENU_GAP,
      })
      return
    }

    const topSpace = rootRect.top - VIEWPORT_MARGIN
    const bottomSpace = window.innerHeight - VIEWPORT_MARGIN - rootRect.bottom
    setSubmenuPlacement(topSpace >= bottomSpace ? "top" : "bottom")

    // Keep a vertical fallback inside the viewport even when the root menu is
    // close to either horizontal edge.
    const maxViewportLeft = Math.max(
      VIEWPORT_MARGIN,
      window.innerWidth - VIEWPORT_MARGIN - submenuWidth,
    )
    const viewportLeft = Math.min(
      Math.max(rootRect.left, VIEWPORT_MARGIN),
      maxViewportLeft,
    )
    setSubmenuStyle(
      topSpace >= bottomSpace
        ? {
            bottom: window.innerHeight - rootRect.top + SUBMENU_GAP,
            left: viewportLeft,
          }
        : {
            left: viewportLeft,
            top: rootRect.bottom + SUBMENU_GAP,
          },
    )
  }, [])

  const setNestedPanel = useCallback(
    (panel: "model" | "temperature" | null) => {
      if (panel) placeSubmenu(panel)
      setOpenPanel(panel)
    },
    [placeSubmenu],
  )

  const handleRootItemMouseEnter = useCallback(
    (panel: "model" | "temperature" | null) => {
      // A vertically positioned submenu overlaps the pointer's route through
      // the root menu. Keep the current panel pinned while the pointer travels;
      // users can still switch or close it by clicking a root item.
      if ((submenuPlacement === "top" || submenuPlacement === "bottom") && openPanel !== null) {
        return
      }
      setNestedPanel(panel)
    },
    [openPanel, setNestedPanel, submenuPlacement],
  )

  useEffect(() => {
    function handleClickOutside(e: MouseEvent) {
      const target = e.target as Node
      const clickedInside = [menuRef, modelSubmenuRef, temperatureSubmenuRef].some((ref) =>
        ref.current?.contains(target),
      )
      if (!clickedInside) {
        setShowMenu(false)
        setOpenPanel(null)
      }
    }
    if (showMenu) {
      document.addEventListener("mousedown", handleClickOutside)
      return () => document.removeEventListener("mousedown", handleClickOutside)
    }
  }, [showMenu])

  useEffect(() => {
    if (!showMenu || !openPanel) return
    const updatePlacement = () => placeSubmenu(openPanel)
    updatePlacement()
    window.addEventListener("resize", updatePlacement)
    window.addEventListener("scroll", updatePlacement, true)
    return () => {
      window.removeEventListener("resize", updatePlacement)
      window.removeEventListener("scroll", updatePlacement, true)
    }
  }, [openPanel, placeSubmenu, showMenu])

  const supportsThinking = modelSupportsThinking(currentModelInfo)
  const effortOptions = getEffortOptionsForModel(currentModelInfo, t)
  const effortLabel =
    effortOptions.find((o) => o.value === reasoningEffort)?.label ?? reasoningEffort
  const modelLabel = currentModelInfo?.modelName ?? t("chat.selectModel")
  const displayedTemperature = temperatureDraft ?? sessionTemperature
  const temperatureLabel =
    displayedTemperature != null
      ? displayedTemperature.toFixed(2)
      : t("settings.temperatureDefault")

  const visibleModels =
    unsupportedBehavior === "hide"
      ? availableModels.filter((model) =>
          modelSupportsInputTypes(model.inputTypes, requiredInputTypes),
        )
      : availableModels

  const modelGroups = Array.from(
    visibleModels.reduce((groups, model) => {
      const existing = groups.get(model.providerId)
      if (existing) {
        existing.models.push(model)
      } else {
        groups.set(model.providerId, {
          providerName: model.providerName,
          models: [model],
        })
      }
      return groups
    }, new Map<string, { providerName: string; models: AvailableModel[] }>()),
  )

  return (
    <div className="relative shrink-0" ref={menuRef}>
      <Tooltip>
        <TooltipTrigger asChild>
          <button
            type="button"
            onClick={() => {
              const nextOpen = !showMenu
              setShowMenu(nextOpen)
              if (nextOpen) {
                setApplyToAgentDefault(false)
                setTemperatureDraft(null)
              }
              setOpenPanel(null)
            }}
            className="flex max-w-[220px] items-center gap-1 overflow-hidden bg-transparent px-2 py-1 text-xs font-medium text-muted-foreground transition-colors hover:bg-secondary hover:text-foreground rounded-lg cursor-pointer"
          >
            {currentModelInfo && (
              <ProviderIcon
                providerKey={currentModelInfo.providerId}
                providerName={currentModelInfo.providerName}
                size={14}
                className="shrink-0"
                color
              />
            )}
            <span className="min-w-0 truncate">{modelLabel}</span>
            {supportsThinking && (
              <span
                className={cn(
                  "shrink-0 whitespace-nowrap",
                  reasoningEffort !== "none" && "text-foreground/80",
                )}
              >
                {effortLabel}
              </span>
            )}
            <ChevronDown className="h-3.5 w-3.5 shrink-0 opacity-60" />
          </button>
        </TooltipTrigger>
        {/* Full, untruncated model name on hover — the trigger caps at
            max-w-[220px] and truncates, so this is the only way to read a
            long name without opening the menu. */}
        <TooltipContent side="top">
          {currentModelInfo?.providerName
            ? `${currentModelInfo.providerName} · ${modelLabel}`
            : modelLabel}
          {supportsThinking && reasoningEffort !== "none" ? ` · ${effortLabel}` : ""}
        </TooltipContent>
      </Tooltip>

      <FloatingMenu
        open={showMenu}
        elementRef={rootMenuRef}
        className="w-[260px] overflow-visible p-1.5"
        onEscapeKeyDown={() => {
          setShowMenu(false)
          setNestedPanel(null)
        }}
      >
        <div className="max-h-[min(420px,calc(100vh-96px))] overflow-y-auto overscroll-contain pr-0.5">
          {unavailablePreference && (
            <div className="mb-1.5 rounded-md bg-amber-500/10 px-2.5 py-2 text-[11px] leading-snug text-amber-700 dark:text-amber-300">
              {t("chat.modelPicker.unavailablePreference", {
                model: unavailablePreference,
                defaultValue: `首选模型 ${unavailablePreference} 当前不可用，正在临时使用可用模型。`,
              })}
            </div>
          )}
          <div className="px-2.5 pb-1 pt-1 text-[11px] font-semibold text-muted-foreground">
            {t("settings.localModels.filters.thinking")}
          </div>
          {supportsThinking ? (
            <div className="flex flex-col gap-0.5">
              {effortOptions.map((opt) => (
                <button
                  key={opt.value}
                  type="button"
                  className={cn(
                    "flex w-full items-center justify-between gap-3 rounded-md px-2.5 py-1.5 text-left text-[13px] transition-all duration-150",
                    reasoningEffort === opt.value
                      ? "bg-secondary text-foreground font-medium shadow-sm"
                      : "text-foreground/80 hover:bg-secondary/60 hover:text-foreground",
                  )}
                  onMouseEnter={() => handleRootItemMouseEnter(null)}
                  onClick={() => {
                    onEffortChange(opt.value, { applyToAgentDefault })
                    setShowMenu(false)
                    setNestedPanel(null)
                  }}
                >
                  <span className="truncate">{opt.label}</span>
                  {reasoningEffort === opt.value && (
                    <Check className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
                  )}
                </button>
              ))}
            </div>
          ) : (
            <p className="px-2.5 pb-1 pt-0.5 text-[11px] leading-relaxed text-muted-foreground/70">
              {t("chat.reasoningDisabledHint")}
            </p>
          )}
          {onEffortReset && (
            <button
              type="button"
              className="mt-0.5 w-full rounded-md px-2.5 py-1.5 text-left text-[11px] text-primary hover:bg-secondary/60"
              onClick={() => {
                onEffortReset()
                setShowMenu(false)
                setOpenPanel(null)
              }}
            >
              {t("chat.modelPicker.restoreAgentDefault", "恢复 Agent 默认")}
            </button>
          )}

          <div className="-mx-1 my-1.5 h-px bg-border-soft" />

          {visibleModels.length > 0 && (
            <button
              type="button"
              aria-haspopup="menu"
              aria-expanded={openPanel === "model"}
              className={cn(
                "flex w-full items-center justify-between gap-3 rounded-md px-2.5 py-1.5 text-left text-[13px] transition-all duration-150",
                openPanel === "model"
                  ? "bg-secondary text-foreground shadow-sm"
                  : "text-foreground/80 hover:bg-secondary/60 hover:text-foreground",
              )}
              onMouseEnter={() => handleRootItemMouseEnter("model")}
              onClick={() => setNestedPanel(openPanel === "model" ? null : "model")}
            >
              <span className="truncate">{modelLabel}</span>
              <ChevronRight className="h-3.5 w-3.5 shrink-0 opacity-50" />
            </button>
          )}

          <button
            type="button"
            aria-haspopup="menu"
            aria-expanded={openPanel === "temperature"}
            className={cn(
              "flex w-full items-center justify-between gap-3 rounded-md px-2.5 py-1.5 text-left text-[13px] transition-all duration-150",
              openPanel === "temperature"
                ? "bg-secondary text-foreground shadow-sm"
                : "text-foreground/80 hover:bg-secondary/60 hover:text-foreground",
            )}
            onMouseEnter={() => handleRootItemMouseEnter("temperature")}
            onClick={() => setNestedPanel(openPanel === "temperature" ? null : "temperature")}
          >
            <span className="truncate">{t("settings.temperature")}</span>
            <span className="ml-auto shrink-0 text-xs text-muted-foreground tabular-nums">
              {temperatureLabel}
            </span>
            <ChevronRight className="h-3.5 w-3.5 shrink-0 opacity-50" />
          </button>

          <div className="-mx-1 my-1.5 h-px bg-border-soft" />
          <div className="flex items-center gap-3 px-2.5 py-2">
            <div className="min-w-0 flex-1">
              <div className="text-[13px] font-medium text-foreground">
                {t("chat.modelPicker.applyToAgentDefault", "同时设为 Agent 默认")}
              </div>
              <div className="text-[11px] leading-snug text-muted-foreground">
                {t("chat.modelPicker.applyToAgentDefaultDesc", "只影响该 Agent 之后新建的会话")}
              </div>
            </div>
            <Switch
              checked={applyToAgentDefault}
              onCheckedChange={setApplyToAgentDefault}
              aria-label={t("chat.modelPicker.applyToAgentDefault", "同时设为 Agent 默认")}
            />
          </div>
        </div>

        <FloatingMenu
          open={openPanel === "model"}
          role="menu"
          strategy="fixed"
          portal
          elementRef={modelSubmenuRef}
          positionClassName=""
          originClassName={cn(
            submenuPlacement === "right" && "origin-left",
            submenuPlacement === "left" && "origin-right",
            submenuPlacement === "top" && "origin-bottom-left",
            submenuPlacement === "bottom" && "origin-top-left",
          )}
          className={cn(
            submenuPlacement === "right" && "ha-menu-from-left",
            submenuPlacement === "left" && "ha-menu-from-right",
            submenuPlacement === "top" && "ha-menu-from-top",
            submenuPlacement === "bottom" && "ha-menu-from-bottom",
            "w-[280px] p-1.5",
          )}
          style={submenuStyle}
        >
          <div className="max-h-[min(360px,calc(100vh-112px))] overflow-y-auto overscroll-contain">
            {modelGroups.map(([providerId, group]) => (
              <div key={providerId} className="py-0.5">
                <div className="flex items-center gap-1.5 px-2.5 py-1 text-[10px] font-semibold uppercase tracking-wide text-muted-foreground/70">
                  <ProviderIcon
                    providerKey={providerId}
                    providerName={group.providerName}
                    size={13}
                    color
                  />
                  <span className="truncate">{group.providerName}</span>
                </div>
                <div className="flex flex-col gap-0.5">
                  {group.models.map((model) => {
                    const selected =
                      activeModel?.providerId === model.providerId &&
                      activeModel?.modelId === model.modelId
                    const unsupported = !modelSupportsInputTypes(
                      model.inputTypes,
                      requiredInputTypes,
                    )
                    return (
                      <button
                        key={`${model.providerId}::${model.modelId}`}
                        type="button"
                        className={cn(
                          "flex w-full items-center justify-between gap-3 rounded-md px-2.5 py-1.5 text-left text-[13px] transition-all duration-150",
                          unsupported
                            ? "cursor-not-allowed bg-muted/30 text-muted-foreground/45 opacity-60"
                            : selected
                              ? "bg-secondary text-foreground font-medium shadow-sm"
                              : "text-foreground/80 hover:bg-secondary/60 hover:text-foreground",
                        )}
                        disabled={unsupported}
                        onClick={() => {
                          onModelChange(`${model.providerId}::${model.modelId}`, {
                            applyToAgentDefault,
                          })
                          setShowMenu(false)
                          setNestedPanel(null)
                        }}
                      >
                        <span className="min-w-0 flex-1 truncate">{model.modelName}</span>
                        <ModelCapabilityBadges inputTypes={model.inputTypes} />
                        {selected && (
                          <Check className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
                        )}
                      </button>
                    )
                  })}
                </div>
              </div>
            ))}
          </div>
        </FloatingMenu>

        <FloatingMenu
          open={openPanel === "temperature"}
          role="menu"
          strategy="fixed"
          portal
          elementRef={temperatureSubmenuRef}
          positionClassName=""
          originClassName={cn(
            submenuPlacement === "right" && "origin-left",
            submenuPlacement === "left" && "origin-right",
            submenuPlacement === "top" && "origin-bottom-left",
            submenuPlacement === "bottom" && "origin-top-left",
          )}
          className={cn(
            submenuPlacement === "right" && "ha-menu-from-left",
            submenuPlacement === "left" && "ha-menu-from-right",
            submenuPlacement === "top" && "ha-menu-from-top",
            submenuPlacement === "bottom" && "ha-menu-from-bottom",
            "w-[220px] p-3",
          )}
          style={submenuStyle}
        >
          <div className="mb-2 flex items-center justify-between gap-3">
            <span className="text-[11px] font-medium text-muted-foreground">
              {t("settings.temperature")}
            </span>
            <span className="text-xs font-mono text-foreground tabular-nums">
              {temperatureLabel}
            </span>
          </div>
          <Slider
            min={0}
            max={200}
            step={1}
            value={[Math.round((displayedTemperature ?? 1) * 100)]}
            onValueChange={([v]) => {
              setTemperatureDraft(v / 100)
            }}
            onValueCommit={([v]) => {
              setTemperatureDraft(null)
              onSessionTemperatureChange?.(v / 100, { applyToAgentDefault })
            }}
            disabled={!onSessionTemperatureChange}
          />
          <div className="mt-2 flex items-center justify-between gap-3">
            <span className="text-[10px] text-muted-foreground/60">
              {t("settings.temperaturePrecise")}
            </span>
            <button
              type="button"
              className="text-[10px] text-primary transition-colors hover:text-primary/80 disabled:pointer-events-none disabled:opacity-50"
              disabled={!onSessionTemperatureChange}
              onClick={() => {
                setTemperatureDraft(null)
                onSessionTemperatureChange?.(null)
                setShowMenu(false)
                setNestedPanel(null)
              }}
            >
              {t("settings.temperatureReset")}
            </button>
          </div>
        </FloatingMenu>
      </FloatingMenu>
    </div>
  )
}
