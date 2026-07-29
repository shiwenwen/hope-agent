import { Frame, Monitor, Smartphone, Tablet } from "lucide-react"
import type { KeyboardEvent as ReactKeyboardEvent, PointerEvent as ReactPointerEvent } from "react"
import { useTranslation } from "react-i18next"
import { DeferredNumberInput } from "@/components/ui/deferred-number-input"
import {
  Select,
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectLabel,
  SelectSeparator,
  SelectTrigger,
} from "@/components/ui/select"
import {
  CUSTOM_VIEWPORT_LIMITS,
  DEVICE_VIEWPORT_PRESETS,
  findDeviceViewportPreset,
  type DeviceGroup,
  type PreviewDevice,
  type ViewportResizeAxis,
  type ViewportSize,
} from "@/components/design/designViewport"

interface DesignViewportControlProps {
  value: PreviewDevice
  customSize: ViewportSize
  onValueChange: (value: PreviewDevice) => void
  onCustomSizeCommit: (size: ViewportSize) => void
}

const GROUPS: DeviceGroup[] = ["desktop", "tablet", "mobile"]

export function DesignViewportControl({
  value,
  customSize,
  onValueChange,
  onCustomSizeCommit,
}: DesignViewportControlProps) {
  const { t } = useTranslation()
  const preset = findDeviceViewportPreset(value)
  const group = preset?.group
  const TriggerIcon =
    value === "custom"
      ? Frame
      : group === "mobile"
        ? Smartphone
        : group === "tablet"
          ? Tablet
          : Monitor
  const triggerLabel =
    value === "auto"
      ? t("design.deviceAuto", "自动")
      : value === "custom"
        ? t("common.custom", "自定义")
        : (preset?.label ?? t("design.deviceDesktop", "桌面"))
  const triggerSize =
    value === "custom"
      ? `${customSize.width}×${customSize.height}`
      : preset
        ? `${preset.width}${preset.height ? `×${preset.height}` : ""}`
        : null

  return (
    <div className="flex items-center gap-1">
      <Select value={value} onValueChange={(next) => onValueChange(next as PreviewDevice)}>
        <SelectTrigger
          className="h-6 w-auto min-w-[8.5rem] gap-1.5 px-2 text-xs"
          aria-label={`${t("design.deviceDesktop", "桌面")} / ${t("design.deviceTablet", "平板")} / ${t("design.deviceMobile", "手机")}`}
        >
          <TriggerIcon className="h-3.5 w-3.5 shrink-0" />
          <span className="max-w-28 truncate">{triggerLabel}</span>
          {triggerSize && (
            <span className="tabular-nums text-[10px] text-muted-foreground">{triggerSize}</span>
          )}
        </SelectTrigger>
        <SelectContent className="w-64">
          <SelectItem value="auto">{t("design.deviceAuto", "自动")}</SelectItem>
          <SelectSeparator />
          {GROUPS.map((presetGroup, groupIndex) => {
            const groupPresets = DEVICE_VIEWPORT_PRESETS.filter(
              (candidate) => candidate.group === presetGroup,
            )
            const groupLabel =
              presetGroup === "desktop"
                ? t("design.deviceDesktop", "桌面")
                : presetGroup === "tablet"
                  ? t("design.deviceTablet", "平板")
                  : t("design.deviceMobile", "手机")
            return (
              <SelectGroup key={presetGroup}>
                {groupIndex > 0 && <SelectSeparator />}
                <SelectLabel>{groupLabel}</SelectLabel>
                {groupPresets.map((candidate) => (
                  <SelectItem key={candidate.id} value={candidate.id}>
                    <span className="flex w-full items-center justify-between gap-5">
                      <span>{candidate.label}</span>
                      <span className="tabular-nums text-[11px] text-muted-foreground">
                        {candidate.width}
                        {candidate.height ? `×${candidate.height}` : ""}
                      </span>
                    </span>
                  </SelectItem>
                ))}
              </SelectGroup>
            )
          })}
          <SelectSeparator />
          <SelectItem value="custom">{t("common.custom", "自定义")}</SelectItem>
        </SelectContent>
      </Select>

      {value === "custom" && (
        <div className="flex h-6 items-center rounded-md border border-border/60 px-1">
          <DeferredNumberInput
            value={customSize.width}
            min={CUSTOM_VIEWPORT_LIMITS.minWidth}
            max={CUSTOM_VIEWPORT_LIMITS.maxWidth}
            aria-label={t("design.insp.width", "宽")}
            className="h-5 w-12 border-0 bg-transparent px-1 text-center text-[11px] tabular-nums shadow-none"
            onValueCommit={(width) => onCustomSizeCommit({ ...customSize, width })}
          />
          <span className="text-[10px] text-muted-foreground">×</span>
          <DeferredNumberInput
            value={customSize.height}
            min={CUSTOM_VIEWPORT_LIMITS.minHeight}
            max={CUSTOM_VIEWPORT_LIMITS.maxHeight}
            aria-label={t("design.insp.height", "高")}
            className="h-5 w-12 border-0 bg-transparent px-1 text-center text-[11px] tabular-nums shadow-none"
            onValueCommit={(height) => onCustomSizeCommit({ ...customSize, height })}
          />
        </div>
      )}
    </div>
  )
}

interface DesignViewportResizeHandlesProps {
  size: ViewportSize
  onResizeStart: (axis: ViewportResizeAxis, event: ReactPointerEvent<HTMLDivElement>) => void
  onSizeCommit: (size: ViewportSize) => void
}

export function DesignViewportResizeHandles({
  size,
  onResizeStart,
  onSizeCommit,
}: DesignViewportResizeHandlesProps) {
  const { t } = useTranslation()
  const widthLabel = `${t("common.custom", "自定义")} · ${t("design.insp.width", "宽")}`
  const heightLabel = `${t("common.custom", "自定义")} · ${t("design.insp.height", "高")}`
  const handleKeyDown = (
    axis: Exclude<ViewportResizeAxis, "both">,
    event: ReactKeyboardEvent<HTMLDivElement>,
  ) => {
    let next: ViewportSize | null = null
    if (axis === "width") {
      if (event.key === "ArrowLeft") next = { ...size, width: size.width - 10 }
      if (event.key === "ArrowRight") next = { ...size, width: size.width + 10 }
      if (event.key === "Home") next = { ...size, width: CUSTOM_VIEWPORT_LIMITS.minWidth }
      if (event.key === "End") next = { ...size, width: CUSTOM_VIEWPORT_LIMITS.maxWidth }
    } else {
      if (event.key === "ArrowUp") next = { ...size, height: size.height - 10 }
      if (event.key === "ArrowDown") next = { ...size, height: size.height + 10 }
      if (event.key === "Home") next = { ...size, height: CUSTOM_VIEWPORT_LIMITS.minHeight }
      if (event.key === "End") next = { ...size, height: CUSTOM_VIEWPORT_LIMITS.maxHeight }
    }
    if (!next) return
    event.preventDefault()
    onSizeCommit(next)
  }
  const handleCornerKeyDown = (event: ReactKeyboardEvent<HTMLDivElement>) => {
    let next: ViewportSize | null = null
    if (event.key === "ArrowLeft") next = { ...size, width: size.width - 10 }
    if (event.key === "ArrowRight") next = { ...size, width: size.width + 10 }
    if (event.key === "ArrowUp") next = { ...size, height: size.height - 10 }
    if (event.key === "ArrowDown") next = { ...size, height: size.height + 10 }
    if (event.key === "Home") {
      next = {
        width: CUSTOM_VIEWPORT_LIMITS.minWidth,
        height: CUSTOM_VIEWPORT_LIMITS.minHeight,
      }
    }
    if (event.key === "End") {
      next = {
        width: CUSTOM_VIEWPORT_LIMITS.maxWidth,
        height: CUSTOM_VIEWPORT_LIMITS.maxHeight,
      }
    }
    if (!next) return
    event.preventDefault()
    onSizeCommit(next)
  }

  return (
    <>
      <div
        role="separator"
        aria-label={widthLabel}
        aria-orientation="vertical"
        aria-valuemin={CUSTOM_VIEWPORT_LIMITS.minWidth}
        aria-valuemax={CUSTOM_VIEWPORT_LIMITS.maxWidth}
        aria-valuenow={size.width}
        tabIndex={0}
        onPointerDown={(event) => onResizeStart("width", event)}
        onKeyDown={(event) => handleKeyDown("width", event)}
        className="group absolute -right-2 top-7 bottom-7 z-30 w-4 cursor-ew-resize touch-none"
      >
        <span className="absolute right-1 top-1/2 h-10 w-1 -translate-y-1/2 rounded-full bg-foreground/20 transition-colors group-hover:bg-primary/70" />
      </div>
      <div
        role="separator"
        aria-label={heightLabel}
        aria-orientation="horizontal"
        aria-valuemin={CUSTOM_VIEWPORT_LIMITS.minHeight}
        aria-valuemax={CUSTOM_VIEWPORT_LIMITS.maxHeight}
        aria-valuenow={size.height}
        tabIndex={0}
        onPointerDown={(event) => onResizeStart("height", event)}
        onKeyDown={(event) => handleKeyDown("height", event)}
        className="group absolute -bottom-2 left-7 right-7 z-30 h-4 cursor-ns-resize touch-none"
      >
        <span className="absolute bottom-1 left-1/2 h-1 w-10 -translate-x-1/2 rounded-full bg-foreground/20 transition-colors group-hover:bg-primary/70" />
      </div>
      <div
        role="button"
        aria-label={`${widthLabel} × ${heightLabel}`}
        tabIndex={0}
        onPointerDown={(event) => onResizeStart("both", event)}
        onKeyDown={handleCornerKeyDown}
        className="group absolute -bottom-2 -right-2 z-40 h-5 w-5 cursor-nwse-resize touch-none"
      >
        <span className="absolute bottom-1 right-1 h-2.5 w-2.5 rounded-br border-b-2 border-r-2 border-foreground/30 transition-colors group-hover:bg-primary/15" />
      </div>
    </>
  )
}
