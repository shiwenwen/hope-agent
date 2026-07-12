import * as React from "react"
import * as DropdownMenuPrimitive from "@radix-ui/react-dropdown-menu"
import {
  FLOATING_MENU_RADIX_MOTION_CLASS,
  FLOATING_MENU_SURFACE_CLASS,
} from "@/components/ui/floating-menu"
import { cn } from "@/lib/utils"

const DropdownMenu = DropdownMenuPrimitive.Root
const DropdownMenuTrigger = DropdownMenuPrimitive.Trigger
const DropdownMenuGroup = DropdownMenuPrimitive.Group
type DropdownMenuVariant = "default" | "floating"

const DropdownMenuVariantContext = React.createContext<DropdownMenuVariant>("default")

const contentVariantClass: Record<DropdownMenuVariant, string> = {
  default:
    "z-50 min-w-[8rem] overflow-hidden rounded-md border bg-popover p-1 text-popover-foreground shadow-md animate-in fade-in-80",
  floating: cn(
    "z-50 min-w-[9.5rem] overflow-hidden p-1.5",
    FLOATING_MENU_SURFACE_CLASS,
    FLOATING_MENU_RADIX_MOTION_CLASS,
  ),
}

const itemVariantClass: Record<DropdownMenuVariant, string> = {
  default:
    "relative flex cursor-default select-none items-center rounded-sm px-2 py-1.5 text-xs outline-none transition-colors focus:bg-accent focus:text-accent-foreground data-[disabled]:pointer-events-none data-[disabled]:opacity-50",
  floating:
    "relative flex cursor-default select-none items-center rounded-md px-2.5 py-1.5 text-[13px] leading-5 text-foreground/80 outline-none transition-colors duration-150 focus:bg-secondary/60 focus:text-foreground data-[disabled]:pointer-events-none data-[disabled]:opacity-50 [&_svg]:shrink-0",
}

const separatorVariantClass: Record<DropdownMenuVariant, string> = {
  default: "-mx-1 my-1 h-px bg-border",
  floating: "-mx-1 my-1.5 h-px bg-border-soft",
}

const DropdownMenuContent = React.forwardRef<
  React.ComponentRef<typeof DropdownMenuPrimitive.Content>,
  React.ComponentPropsWithoutRef<typeof DropdownMenuPrimitive.Content> & {
    variant?: DropdownMenuVariant
  }
>(({ className, sideOffset = 4, variant = "default", ...props }, ref) => (
  <DropdownMenuVariantContext.Provider value={variant}>
    <DropdownMenuPrimitive.Portal>
      <DropdownMenuPrimitive.Content
        ref={ref}
        sideOffset={sideOffset}
        className={cn(contentVariantClass[variant], className)}
        {...props}
      />
    </DropdownMenuPrimitive.Portal>
  </DropdownMenuVariantContext.Provider>
))
DropdownMenuContent.displayName = DropdownMenuPrimitive.Content.displayName

const DropdownMenuItem = React.forwardRef<
  React.ComponentRef<typeof DropdownMenuPrimitive.Item>,
  React.ComponentPropsWithoutRef<typeof DropdownMenuPrimitive.Item> & {
    inset?: boolean
    variant?: DropdownMenuVariant
  }
>(({ className, inset, variant, ...props }, ref) => {
  const inheritedVariant = React.useContext(DropdownMenuVariantContext)
  const resolvedVariant = variant ?? inheritedVariant
  return (
    <DropdownMenuPrimitive.Item
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
DropdownMenuItem.displayName = DropdownMenuPrimitive.Item.displayName

const DropdownMenuSeparator = React.forwardRef<
  React.ComponentRef<typeof DropdownMenuPrimitive.Separator>,
  React.ComponentPropsWithoutRef<typeof DropdownMenuPrimitive.Separator> & {
    variant?: DropdownMenuVariant
  }
>(({ className, variant, ...props }, ref) => {
  const inheritedVariant = React.useContext(DropdownMenuVariantContext)
  const resolvedVariant = variant ?? inheritedVariant
  return (
    <DropdownMenuPrimitive.Separator
      ref={ref}
      className={cn(separatorVariantClass[resolvedVariant], className)}
      {...props}
    />
  )
})
DropdownMenuSeparator.displayName = DropdownMenuPrimitive.Separator.displayName

export {
  DropdownMenu,
  DropdownMenuTrigger,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuGroup,
  DropdownMenuSeparator,
}
