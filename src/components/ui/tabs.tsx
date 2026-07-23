import * as React from "react"
import * as TabsPrimitive from "@radix-ui/react-tabs"
import { cn } from "@/lib/utils"
import { UI_EASING, UI_MOTION } from "./motion"

interface TabsMotionContextValue {
  activeValue: string | undefined
  getActiveRect: () => DOMRect | null
  setActiveRect: (rect: DOMRect) => void
  replaceAnimation: (animation: Animation) => void
  clearAnimation: (animation: Animation) => boolean
}

const TabsMotionContext = React.createContext<TabsMotionContextValue | null>(null)

const Tabs = React.forwardRef<
  React.ComponentRef<typeof TabsPrimitive.Root>,
  React.ComponentPropsWithoutRef<typeof TabsPrimitive.Root>
>(({ value, defaultValue, onValueChange, ...props }, forwardedRef) => {
  const [uncontrolledValue, setUncontrolledValue] = React.useState(defaultValue)
  const rootRef = React.useRef<React.ComponentRef<typeof TabsPrimitive.Root>>(null)
  const activeRectRef = React.useRef<DOMRect | null>(null)
  const animationRef = React.useRef<Animation | null>(null)
  const activeValue = value ?? uncontrolledValue

  const setRootRef = React.useCallback(
    (node: React.ComponentRef<typeof TabsPrimitive.Root> | null) => {
      rootRef.current = node
      if (typeof forwardedRef === "function") forwardedRef(node)
      else if (forwardedRef) forwardedRef.current = node
    },
    [forwardedRef],
  )

  const getActiveRect = React.useCallback(() => activeRectRef.current, [])
  const setActiveRect = React.useCallback((rect: DOMRect) => {
    activeRectRef.current = rect
  }, [])
  const replaceAnimation = React.useCallback((animation: Animation) => {
    animationRef.current?.cancel()
    animationRef.current = animation
  }, [])
  const clearAnimation = React.useCallback((animation: Animation) => {
    if (animationRef.current !== animation) return false
    animationRef.current = null
    return true
  }, [])

  const handleValueChange = React.useCallback(
    (nextValue: string) => {
      const currentTrigger = rootRef.current?.querySelector<HTMLElement>(
        '[role="tab"][data-state="active"]',
      )
      if (currentTrigger) activeRectRef.current = currentTrigger.getBoundingClientRect()
      if (value === undefined) setUncontrolledValue(nextValue)
      onValueChange?.(nextValue)
    },
    [onValueChange, value],
  )

  const motionContext = React.useMemo<TabsMotionContextValue>(
    () => ({ activeValue, getActiveRect, setActiveRect, replaceAnimation, clearAnimation }),
    [activeValue, clearAnimation, getActiveRect, replaceAnimation, setActiveRect],
  )

  return (
    <TabsMotionContext.Provider value={motionContext}>
      <TabsPrimitive.Root
        ref={setRootRef}
        value={value}
        defaultValue={defaultValue}
        onValueChange={handleValueChange}
        {...props}
      />
    </TabsMotionContext.Provider>
  )
})
Tabs.displayName = TabsPrimitive.Root.displayName

const TabsList = React.forwardRef<
  React.ComponentRef<typeof TabsPrimitive.List>,
  React.ComponentPropsWithoutRef<typeof TabsPrimitive.List>
>(({ className, ...props }, ref) => (
  <TabsPrimitive.List
    ref={ref}
    className={cn(
      "relative isolate inline-flex h-9 items-center justify-center rounded-lg bg-muted p-1 text-muted-foreground",
      className,
    )}
    {...props}
  />
))
TabsList.displayName = TabsPrimitive.List.displayName

const TabsTrigger = React.forwardRef<
  React.ComponentRef<typeof TabsPrimitive.Trigger>,
  React.ComponentPropsWithoutRef<typeof TabsPrimitive.Trigger>
>(({ className, value, children, ...props }, forwardedRef) => {
  const motionContext = React.useContext(TabsMotionContext)
  const triggerRef = React.useRef<React.ComponentRef<typeof TabsPrimitive.Trigger>>(null)
  const indicatorRef = React.useRef<HTMLSpanElement>(null)
  const isActive = motionContext?.activeValue === value

  const setTriggerRef = React.useCallback(
    (node: React.ComponentRef<typeof TabsPrimitive.Trigger> | null) => {
      triggerRef.current = node
      if (typeof forwardedRef === "function") forwardedRef(node)
      else if (forwardedRef) forwardedRef.current = node
    },
    [forwardedRef],
  )

  React.useLayoutEffect(() => {
    const trigger = triggerRef.current
    const indicator = indicatorRef.current
    if (!isActive || !trigger || !indicator || !motionContext) return

    const currentRect = trigger.getBoundingClientRect()
    const previousRect = motionContext.getActiveRect()
    motionContext.setActiveRect(currentRect)

    const reducedMotion = window.matchMedia?.("(prefers-reduced-motion: reduce)")?.matches ?? false
    if (
      previousRect &&
      currentRect.width > 0 &&
      currentRect.height > 0 &&
      !reducedMotion &&
      typeof indicator.animate === "function"
    ) {
      const animation = indicator.animate(
        [
          {
            transform: `translate3d(${previousRect.left - currentRect.left}px, ${previousRect.top - currentRect.top}px, 0) scale(${previousRect.width / currentRect.width}, ${previousRect.height / currentRect.height})`,
            transformOrigin: "top left",
          },
          { transform: "translate3d(0, 0, 0) scale(1, 1)", transformOrigin: "top left" },
        ],
        {
          duration: UI_MOTION.tabIndicator,
          easing: UI_EASING.emphasized,
          fill: "both",
        },
      )
      motionContext.replaceAnimation(animation)
      animation.onfinish = () => {
        if (!motionContext.clearAnimation(animation)) return
        animation.cancel()
      }
      animation.oncancel = () => {
        motionContext.clearAnimation(animation)
      }
    }

    if (typeof ResizeObserver === "undefined") return
    const resizeObserver = new ResizeObserver(() => {
      motionContext.setActiveRect(trigger.getBoundingClientRect())
    })
    resizeObserver.observe(trigger)
    return () => resizeObserver.disconnect()
  }, [isActive, motionContext])

  return (
    <TabsPrimitive.Trigger
      ref={setTriggerRef}
      value={value}
      className={cn(
        "relative inline-flex items-center justify-center whitespace-nowrap rounded-md px-3 py-1 text-sm font-medium transition-colors focus:outline-none motion-reduce:transition-none disabled:pointer-events-none disabled:opacity-50 data-[state=active]:text-foreground",
        !motionContext && "data-[state=active]:bg-background",
        className,
      )}
      {...props}
    >
      {isActive && (
        <span
          ref={indicatorRef}
          aria-hidden="true"
          data-tabs-indicator=""
          className="pointer-events-none absolute inset-0 -z-10 rounded-[inherit] bg-background will-change-transform"
        />
      )}
      {children}
    </TabsPrimitive.Trigger>
  )
})
TabsTrigger.displayName = TabsPrimitive.Trigger.displayName

const TabsContent = React.forwardRef<
  React.ComponentRef<typeof TabsPrimitive.Content>,
  React.ComponentPropsWithoutRef<typeof TabsPrimitive.Content>
>(({ className, ...props }, ref) => (
  <TabsPrimitive.Content
    ref={ref}
    className={cn(
      "mt-2 outline-none data-[state=active]:flex data-[state=active]:flex-col",
      className,
    )}
    {...props}
  />
))
TabsContent.displayName = TabsPrimitive.Content.displayName

export { Tabs, TabsList, TabsTrigger, TabsContent }
