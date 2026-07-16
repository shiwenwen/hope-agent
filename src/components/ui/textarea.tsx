import * as React from "react"
import {
  EMBEDDED_CONTROL_SURFACE_CLASS,
  FLAT_CONTROL_SURFACE_CLASS,
  type ControlSurface,
} from "@/components/ui/control-surface"
import { cn } from "@/lib/utils"

export interface TextareaProps extends React.TextareaHTMLAttributes<HTMLTextAreaElement> {
  surface?: ControlSurface
}

const Textarea = React.forwardRef<HTMLTextAreaElement, TextareaProps>(
  ({ className, surface = "default", ...props }, ref) => {
    return (
      <textarea
        className={cn(
          surface === "default"
            ? FLAT_CONTROL_SURFACE_CLASS
            : EMBEDDED_CONTROL_SURFACE_CLASS,
          "flex min-h-[60px] w-full px-3 py-2 text-sm text-foreground placeholder:text-muted-foreground focus:outline-none disabled:cursor-not-allowed disabled:opacity-50",
          className,
        )}
        ref={ref}
        {...props}
      />
    )
  },
)
Textarea.displayName = "Textarea"

export { Textarea }
