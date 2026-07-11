import * as React from "react"
import * as TooltipPrimitive from "@radix-ui/react-tooltip"
import {
  FLOATING_MENU_RADIX_MOTION_CLASS,
  FLOATING_MENU_SURFACE_CLASS,
} from "@/components/ui/floating-menu"
import { cn } from "@/lib/utils"

const ATTRIBUTE_TOOLTIP_SELECTOR = "[data-ha-title-tip]"

interface AttributeTooltipSnapshot {
  element: HTMLElement
  label: string
  rect: Pick<DOMRect, "height" | "left" | "top" | "width">
  side: "top" | "bottom"
}

function readAttributeTooltip(element: HTMLElement): AttributeTooltipSnapshot | null {
  const label = element.getAttribute("data-ha-title-tip")?.trim()
  if (!label) return null
  const rect = element.getBoundingClientRect()
  if (rect.width <= 0 && rect.height <= 0) return null
  const roomAbove = rect.top
  const roomBelow = window.innerHeight - (rect.top + rect.height)
  return {
    element,
    label,
    rect: { height: rect.height, left: rect.left, top: rect.top, width: rect.width },
    side: roomAbove >= 48 || roomAbove >= roomBelow ? "top" : "bottom",
  }
}

function findAttributeTooltipElement(target: EventTarget | null): HTMLElement | null {
  return target instanceof Element
    ? (target.closest(ATTRIBUTE_TOOLTIP_SELECTOR) as HTMLElement | null)
    : null
}

function findDisabledAttributeTooltipAtPoint(
  elements: readonly HTMLElement[],
  x: number,
  y: number,
): HTMLElement | null {
  for (const element of elements) {
    const rect = element.getBoundingClientRect()
    if (x >= rect.left && x <= rect.right && y >= rect.top && y <= rect.bottom) return element
  }
  return null
}

/**
 * One delegated Tooltip instance for long/truncated text and legacy title-style
 * hints. Business nodes expose `data-ha-title-tip`; this bridge avoids mounting
 * dozens of Radix roots while keeping the same surface, collision handling and
 * keyboard behavior as the standard Tooltip primitive.
 */
function AttributeTooltipBridge({ delayDuration }: { delayDuration: number }) {
  const [active, setActive] = React.useState<AttributeTooltipSnapshot | null>(null)
  const [renderedSnapshot, setRenderedSnapshot] =
    React.useState<AttributeTooltipSnapshot | null>(null)
  const activeElementRef = React.useRef<HTMLElement | null>(null)
  const pendingElementRef = React.useRef<HTMLElement | null>(null)
  const hoveredElementRef = React.useRef<HTMLElement | null>(null)
  const focusedElementRef = React.useRef<HTMLElement | null>(null)
  const disabledElementsRef = React.useRef<HTMLElement[]>([])
  const showTimerRef = React.useRef<number | null>(null)
  const closeTimerRef = React.useRef<number | null>(null)

  const updateSnapshot = React.useCallback((snapshot: AttributeTooltipSnapshot) => {
    setActive(snapshot)
    setRenderedSnapshot(snapshot)
  }, [])

  const clearShowTimer = React.useCallback(() => {
    if (showTimerRef.current === null) return
    window.clearTimeout(showTimerRef.current)
    showTimerRef.current = null
  }, [])

  const clearCloseTimer = React.useCallback(() => {
    if (closeTimerRef.current === null) return
    window.clearTimeout(closeTimerRef.current)
    closeTimerRef.current = null
  }, [])

  const hide = React.useCallback(() => {
    clearShowTimer()
    clearCloseTimer()
    pendingElementRef.current = null
    activeElementRef.current = null
    setActive(null)
  }, [clearCloseTimer, clearShowTimer])

  const scheduleHide = React.useCallback(
    (element: HTMLElement) => {
      if (hoveredElementRef.current === element || focusedElementRef.current === element) return
      clearCloseTimer()
      closeTimerRef.current = window.setTimeout(() => {
        closeTimerRef.current = null
        if (hoveredElementRef.current === element || focusedElementRef.current === element) return
        if (activeElementRef.current === element || pendingElementRef.current === element) {
          hide()
        }
      }, 50)
    },
    [clearCloseTimer, hide],
  )

  const show = React.useCallback(
    (element: HTMLElement, immediate: boolean) => {
      clearCloseTimer()
      if (activeElementRef.current === element) {
        const snapshot = readAttributeTooltip(element)
        if (snapshot) updateSnapshot(snapshot)
        return
      }
      if (pendingElementRef.current === element && showTimerRef.current !== null) return

      clearShowTimer()
      pendingElementRef.current = element
      const commit = () => {
        showTimerRef.current = null
        if (hoveredElementRef.current !== element && focusedElementRef.current !== element) {
          pendingElementRef.current = null
          return
        }
        const snapshot = readAttributeTooltip(element)
        pendingElementRef.current = null
        if (!snapshot) return
        activeElementRef.current = element
        updateSnapshot(snapshot)
      }
      if (immediate) commit()
      else showTimerRef.current = window.setTimeout(commit, delayDuration)
    },
    [clearCloseTimer, clearShowTimer, delayDuration, updateSnapshot],
  )

  React.useEffect(() => {
    const onPointerOver = (event: PointerEvent) => {
      const element = findAttributeTooltipElement(event.target)
      if (!element || element === hoveredElementRef.current) return
      hoveredElementRef.current = element
      show(element, false)
    }
    const onPointerOut = (event: PointerEvent) => {
      const element = findAttributeTooltipElement(event.target)
      if (!element) return
      const relatedElement = findAttributeTooltipElement(event.relatedTarget)
      if (relatedElement === element) return
      if (hoveredElementRef.current === element) hoveredElementRef.current = null
      scheduleHide(element)
    }
    // shadcn buttons use `disabled:pointer-events-none`, so the disabled node
    // is not the pointer event target. Hit-test the small disabled subset to
    // preserve the explanatory tooltip that native `title` used to provide.
    const onPointerMove = (event: PointerEvent) => {
      if (findAttributeTooltipElement(event.target)) return
      const element = findDisabledAttributeTooltipAtPoint(
        disabledElementsRef.current,
        event.clientX,
        event.clientY,
      )
      const previous = hoveredElementRef.current
      if (element === previous) return
      if (previous?.matches(":disabled")) {
        hoveredElementRef.current = null
        scheduleHide(previous)
      }
      if (element) {
        hoveredElementRef.current = element
        show(element, false)
      }
    }
    const onFocusIn = (event: FocusEvent) => {
      const element = findAttributeTooltipElement(event.target)
      if (!element) return
      focusedElementRef.current = element
      show(element, true)
    }
    const onFocusOut = (event: FocusEvent) => {
      const element = findAttributeTooltipElement(event.target)
      if (!element) return
      const relatedElement = findAttributeTooltipElement(event.relatedTarget)
      if (relatedElement === element) return
      if (focusedElementRef.current === element) focusedElementRef.current = null
      scheduleHide(element)
    }
    const refreshPosition = () => {
      const element = activeElementRef.current
      if (!element) return
      const snapshot = readAttributeTooltip(element)
      if (snapshot) updateSnapshot(snapshot)
      else hide()
    }
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") hide()
    }
    const refreshDisabledElements = () => {
      disabledElementsRef.current = Array.from(
        document.querySelectorAll<HTMLElement>(`${ATTRIBUTE_TOOLTIP_SELECTOR}:disabled`),
      )
    }
    refreshDisabledElements()
    const mutationObserver = new MutationObserver((records) => {
      if (
        records.some(
          (record) =>
            record.type === "childList" ||
            record.attributeName === "disabled" ||
            record.attributeName === "data-ha-title-tip",
        )
      ) {
        refreshDisabledElements()
      }
      const activeElement = activeElementRef.current
      if (activeElement && !activeElement.isConnected) {
        if (hoveredElementRef.current === activeElement) hoveredElementRef.current = null
        if (focusedElementRef.current === activeElement) focusedElementRef.current = null
        hide()
        return
      }
      if (
        activeElement &&
        records.some(
          (record) =>
            record.type === "attributes" &&
            record.target === activeElement &&
            record.attributeName === "data-ha-title-tip",
        )
      ) {
        const snapshot = readAttributeTooltip(activeElement)
        if (snapshot) updateSnapshot(snapshot)
        else {
          if (hoveredElementRef.current === activeElement) hoveredElementRef.current = null
          if (focusedElementRef.current === activeElement) focusedElementRef.current = null
          hide()
        }
      }
    })
    mutationObserver.observe(document.body, {
      attributeFilter: ["data-ha-title-tip", "disabled"],
      attributes: true,
      childList: true,
      subtree: true,
    })

    document.addEventListener("pointerover", onPointerOver)
    document.addEventListener("pointerout", onPointerOut)
    document.addEventListener("pointermove", onPointerMove)
    document.addEventListener("focusin", onFocusIn)
    document.addEventListener("focusout", onFocusOut)
    document.addEventListener("keydown", onKeyDown)
    window.addEventListener("resize", refreshPosition)
    window.addEventListener("scroll", refreshPosition, true)
    window.addEventListener("blur", hide)
    return () => {
      clearShowTimer()
      clearCloseTimer()
      mutationObserver.disconnect()
      disabledElementsRef.current = []
      document.removeEventListener("pointerover", onPointerOver)
      document.removeEventListener("pointerout", onPointerOut)
      document.removeEventListener("pointermove", onPointerMove)
      document.removeEventListener("focusin", onFocusIn)
      document.removeEventListener("focusout", onFocusOut)
      document.removeEventListener("keydown", onKeyDown)
      window.removeEventListener("resize", refreshPosition)
      window.removeEventListener("scroll", refreshPosition, true)
      window.removeEventListener("blur", hide)
    }
  }, [clearCloseTimer, clearShowTimer, hide, scheduleHide, show, updateSnapshot])

  if (!renderedSnapshot) return null
  return (
    <Tooltip open={active !== null}>
      <TooltipTrigger asChild>
        <span
          aria-hidden="true"
          className="pointer-events-none fixed"
          style={{
            height: renderedSnapshot.rect.height,
            left: renderedSnapshot.rect.left,
            top: renderedSnapshot.rect.top,
            width: renderedSnapshot.rect.width,
          }}
        />
      </TooltipTrigger>
      <TooltipContent
        side={renderedSnapshot.side}
        className="max-w-[min(320px,calc(100vw-16px))] whitespace-pre-wrap break-words"
      >
        {renderedSnapshot.label}
      </TooltipContent>
    </Tooltip>
  )
}

const TooltipProvider = ({
  delayDuration = 100,
  skipDelayDuration = 50,
  children,
  ...props
}: React.ComponentPropsWithoutRef<typeof TooltipPrimitive.Provider>) => (
  <TooltipPrimitive.Provider
    delayDuration={delayDuration}
    skipDelayDuration={skipDelayDuration}
    {...props}
  >
    {children}
    <AttributeTooltipBridge delayDuration={delayDuration} />
  </TooltipPrimitive.Provider>
)
const Tooltip = TooltipPrimitive.Root
const TooltipTrigger = TooltipPrimitive.Trigger

function hasAccessibleText(node: React.ReactNode): boolean {
  if (typeof node === "string" || typeof node === "number") return String(node).trim().length > 0
  if (Array.isArray(node)) return node.some(hasAccessibleText)
  if (!React.isValidElement(node)) return false
  const props = node.props as { "aria-hidden"?: boolean | "true"; children?: React.ReactNode }
  if (props["aria-hidden"] === true || props["aria-hidden"] === "true") return false
  return hasAccessibleText(props.children)
}

const TooltipContent = React.forwardRef<
  React.ComponentRef<typeof TooltipPrimitive.Content>,
  React.ComponentPropsWithoutRef<typeof TooltipPrimitive.Content>
>(({ className, sideOffset = 6, ...props }, ref) => (
  <TooltipPrimitive.Portal>
    <TooltipPrimitive.Content
      ref={ref}
      sideOffset={sideOffset}
      className={cn(
        FLOATING_MENU_SURFACE_CLASS,
        FLOATING_MENU_RADIX_MOTION_CLASS,
        "z-50 rounded-md px-2.5 py-1 text-xs pointer-events-none [--ha-presence-enter-duration:120ms] [--ha-presence-exit-duration:100ms]",
        className,
      )}
      {...props}
    />
  </TooltipPrimitive.Portal>
))
TooltipContent.displayName = TooltipPrimitive.Content.displayName

/** Lightweight wrapper: wraps children in a tooltip. Renders inline to avoid layout shifts. */
function IconTip({
  label,
  side = "top",
  children,
}: {
  label?: string | null
  side?: "top" | "bottom" | "left" | "right"
  children: React.ReactNode
}) {
  if (!label) return <>{children}</>
  const text = String(label)
  const tipProps = {
    "data-ha-tip": text,
    "data-ha-tip-side": side,
  }

  if (React.isValidElement(children) && children.type !== React.Fragment) {
    type TipChildProps = {
      "aria-label"?: string
      className?: string
      children?: React.ReactNode
      title?: string
      "data-ha-tip"?: string
      "data-ha-tip-side"?: typeof side
    }
    const child = children as React.ReactElement<TipChildProps>
    const accessibleLabel =
      child.props["aria-label"] ?? (hasAccessibleText(child.props.children) ? undefined : text)
    const trigger = React.cloneElement(child, {
      ...tipProps,
      "aria-label": accessibleLabel,
      className: cn(child.props.className, "ha-icon-tip"),
      // A native title renders alongside the Radix tooltip after a longer
      // hover, producing two labels for the same control. IconTip owns the
      // hint surface, so suppress even an inherited/explicit native title.
      title: undefined,
    })
    return (
      <Tooltip>
        <TooltipTrigger asChild>{trigger}</TooltipTrigger>
        <TooltipContent side={side}>{text}</TooltipContent>
      </Tooltip>
    )
  }

  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <span className="ha-icon-tip" {...tipProps}>
          {children}
        </span>
      </TooltipTrigger>
      <TooltipContent side={side}>{text}</TooltipContent>
    </Tooltip>
  )
}

export { Tooltip, TooltipTrigger, TooltipContent, TooltipProvider, IconTip }
