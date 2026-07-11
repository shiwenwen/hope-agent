import * as React from "react"
import * as TooltipPrimitive from "@radix-ui/react-tooltip"
import { cn } from "@/lib/utils"

const TooltipProvider = ({
  delayDuration = 100,
  skipDelayDuration = 50,
  ...props
}: React.ComponentPropsWithoutRef<typeof TooltipPrimitive.Provider>) => (
  <TooltipPrimitive.Provider
    delayDuration={delayDuration}
    skipDelayDuration={skipDelayDuration}
    {...props}
  />
)
const Tooltip = TooltipPrimitive.Root
const TooltipTrigger = TooltipPrimitive.Trigger

const TooltipContent = React.forwardRef<
  React.ComponentRef<typeof TooltipPrimitive.Content>,
  React.ComponentPropsWithoutRef<typeof TooltipPrimitive.Content>
>(({ className, sideOffset = 6, ...props }, ref) => (
  <TooltipPrimitive.Portal>
    <TooltipPrimitive.Content
      ref={ref}
      sideOffset={sideOffset}
      className={cn(
        "z-50 rounded-md bg-popover px-2.5 py-1 text-xs text-popover-foreground shadow-md border border-border/50 animate-in fade-in-0 zoom-in-95 pointer-events-none",
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
    title: text,
  }

  if (React.isValidElement(children) && children.type !== React.Fragment) {
    type TipChildProps = {
      className?: string
      title?: string
      "data-ha-tip"?: string
      "data-ha-tip-side"?: typeof side
    }
    const child = children as React.ReactElement<TipChildProps>
    const trigger = React.cloneElement(child, {
      ...tipProps,
      className: cn(child.props.className, "ha-icon-tip"),
      title: child.props.title ?? text,
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
