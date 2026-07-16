import * as React from "react"
import {
  EMBEDDED_CONTROL_SURFACE_CLASS,
  FLAT_CONTROL_SURFACE_CLASS,
  type ControlSurface,
} from "@/components/ui/control-surface"
import { cn } from "@/lib/utils"

export interface InputProps extends React.InputHTMLAttributes<HTMLInputElement> {
  surface?: ControlSurface
}

const Input = React.forwardRef<HTMLInputElement, InputProps>(
  ({ className, type, surface = "default", ...props }, ref) => {
    return (
      <input
        type={type}
        className={cn(
          surface === "default"
            ? FLAT_CONTROL_SURFACE_CLASS
            : EMBEDDED_CONTROL_SURFACE_CLASS,
          "flex h-9 w-full px-3 py-1 text-sm text-foreground placeholder:text-muted-foreground focus:outline-none disabled:cursor-not-allowed disabled:opacity-50",
          className,
        )}
        ref={ref}
        {...props}
      />
    )
  },
)
Input.displayName = "Input"

export { Input }
