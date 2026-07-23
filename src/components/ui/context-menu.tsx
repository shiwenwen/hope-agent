import * as React from "react"
import * as ContextMenuPrimitive from "@radix-ui/react-context-menu"
import { ChevronRight } from "lucide-react"
import {
  FLOATING_MENU_RADIX_MOTION_CLASS,
  FLOATING_MENU_SURFACE_CLASS,
} from "@/components/ui/floating-menu"
import { cn } from "@/lib/utils"

const ContextMenu = ContextMenuPrimitive.Root
const ContextMenuTrigger = ContextMenuPrimitive.Trigger
const ContextMenuSub = ContextMenuPrimitive.Sub
type ContextMenuVariant = "default" | "floating"

const ContextMenuVariantContext = React.createContext<ContextMenuVariant>("default")

const contentVariantClass: Record<ContextMenuVariant, string> = {
  default:
    "z-50 min-w-[8rem] overflow-hidden rounded-md border bg-popover p-1 text-popover-foreground shadow-md animate-in fade-in-80",
  floating: cn(
    "z-50 min-w-[9.5rem] overflow-visible p-1.5",
    FLOATING_MENU_SURFACE_CLASS,
    FLOATING_MENU_RADIX_MOTION_CLASS,
  ),
}

const subContentVariantClass: Record<ContextMenuVariant, string> = {
  default:
    "z-50 min-w-[10rem] max-h-[300px] overflow-y-auto rounded-md border bg-popover p-1 text-popover-foreground shadow-md animate-in fade-in-80",
  floating: cn(
    "z-50 min-w-[10rem] max-h-[300px] overflow-y-auto p-1.5",
    FLOATING_MENU_SURFACE_CLASS,
    FLOATING_MENU_RADIX_MOTION_CLASS,
  ),
}

const itemVariantClass: Record<ContextMenuVariant, string> = {
  default:
    "relative flex cursor-default select-none items-center rounded-sm px-2 py-1.5 text-xs outline-none focus:bg-accent focus:text-accent-foreground data-[disabled]:pointer-events-none data-[disabled]:opacity-50",
  floating:
    "relative flex cursor-default select-none items-center rounded-md px-2.5 py-1.5 text-[13px] leading-5 text-foreground/80 outline-none transition-colors duration-150 focus:bg-secondary/60 focus:text-foreground data-[state=open]:bg-secondary data-[state=open]:text-foreground data-[disabled]:pointer-events-none data-[disabled]:opacity-50 [&_svg]:shrink-0",
}

const subTriggerVariantClass: Record<ContextMenuVariant, string> = {
  default:
    "flex cursor-default select-none items-center rounded-sm px-2 py-1.5 text-xs outline-none focus:bg-accent data-[state=open]:bg-accent",
  floating:
    "flex cursor-default select-none items-center rounded-md px-2.5 py-1.5 text-[13px] leading-5 text-foreground/80 outline-none transition-colors duration-150 focus:bg-secondary/60 focus:text-foreground data-[state=open]:bg-secondary data-[state=open]:text-foreground [&_svg]:shrink-0",
}

const separatorVariantClass: Record<ContextMenuVariant, string> = {
  default: "-mx-1 my-1 h-px bg-border",
  floating: "-mx-1 my-1.5 h-px bg-border-soft",
}

const ContextMenuSubTrigger = React.forwardRef<
  React.ComponentRef<typeof ContextMenuPrimitive.SubTrigger>,
  React.ComponentPropsWithoutRef<typeof ContextMenuPrimitive.SubTrigger> & {
    inset?: boolean
    variant?: ContextMenuVariant
  }
>(({ className, inset, variant, children, ...props }, ref) => {
  const inheritedVariant = React.useContext(ContextMenuVariantContext)
  const resolvedVariant = variant ?? inheritedVariant
  return (
    <ContextMenuPrimitive.SubTrigger
      ref={ref}
      className={cn(
        subTriggerVariantClass[resolvedVariant],
        inset && (resolvedVariant === "floating" ? "pl-9" : "pl-8"),
        className,
      )}
      {...props}
    >
      {children}
      <ChevronRight
        className={cn(
          "ml-auto",
          resolvedVariant === "floating" ? "h-4 w-4 opacity-60" : "h-3.5 w-3.5",
        )}
      />
    </ContextMenuPrimitive.SubTrigger>
  )
})
ContextMenuSubTrigger.displayName = ContextMenuPrimitive.SubTrigger.displayName

const ContextMenuSubContent = React.forwardRef<
  React.ComponentRef<typeof ContextMenuPrimitive.SubContent>,
  React.ComponentPropsWithoutRef<typeof ContextMenuPrimitive.SubContent> & {
    variant?: ContextMenuVariant
  }
>(({ className, variant, ...props }, ref) => {
  const inheritedVariant = React.useContext(ContextMenuVariantContext)
  const resolvedVariant = variant ?? inheritedVariant
  return (
    <ContextMenuVariantContext.Provider value={resolvedVariant}>
      <ContextMenuPrimitive.SubContent
        ref={ref}
        className={cn(subContentVariantClass[resolvedVariant], className)}
        {...props}
      />
    </ContextMenuVariantContext.Provider>
  )
})
ContextMenuSubContent.displayName = ContextMenuPrimitive.SubContent.displayName

const ContextMenuContent = React.forwardRef<
  React.ComponentRef<typeof ContextMenuPrimitive.Content>,
  React.ComponentPropsWithoutRef<typeof ContextMenuPrimitive.Content> & {
    variant?: ContextMenuVariant
  }
>(({ className, variant = "default", ...props }, ref) => (
  <ContextMenuVariantContext.Provider value={variant}>
    <ContextMenuPrimitive.Portal>
      <ContextMenuPrimitive.Content
        ref={ref}
        className={cn(contentVariantClass[variant], className)}
        {...props}
      />
    </ContextMenuPrimitive.Portal>
  </ContextMenuVariantContext.Provider>
))
ContextMenuContent.displayName = ContextMenuPrimitive.Content.displayName

const ContextMenuItem = React.forwardRef<
  React.ComponentRef<typeof ContextMenuPrimitive.Item>,
  React.ComponentPropsWithoutRef<typeof ContextMenuPrimitive.Item> & {
    inset?: boolean
    variant?: ContextMenuVariant
  }
>(({ className, inset, variant, ...props }, ref) => {
  const inheritedVariant = React.useContext(ContextMenuVariantContext)
  const resolvedVariant = variant ?? inheritedVariant
  return (
    <ContextMenuPrimitive.Item
      ref={ref}
      className={cn(
        itemVariantClass[resolvedVariant],
        inset && (resolvedVariant === "floating" ? "pl-9" : "pl-8"),
        className,
      )}
      {...props}
    />
  )
})
ContextMenuItem.displayName = ContextMenuPrimitive.Item.displayName

const ContextMenuSeparator = React.forwardRef<
  React.ComponentRef<typeof ContextMenuPrimitive.Separator>,
  React.ComponentPropsWithoutRef<typeof ContextMenuPrimitive.Separator> & {
    variant?: ContextMenuVariant
  }
>(({ className, variant, ...props }, ref) => {
  const inheritedVariant = React.useContext(ContextMenuVariantContext)
  const resolvedVariant = variant ?? inheritedVariant
  return (
    <ContextMenuPrimitive.Separator
      ref={ref}
      className={cn(separatorVariantClass[resolvedVariant], className)}
      {...props}
    />
  )
})
ContextMenuSeparator.displayName = ContextMenuPrimitive.Separator.displayName

export {
  ContextMenu,
  ContextMenuTrigger,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuSub,
  ContextMenuSubTrigger,
  ContextMenuSubContent,
}
