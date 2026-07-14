import * as React from "react"

import { Input, type InputProps } from "@/components/ui/input"
import { cn } from "@/lib/utils"

export type SearchInputProps = InputProps

/** Flat search surface shared across list, panel, and settings search bars. */
const SearchInput = React.forwardRef<HTMLInputElement, SearchInputProps>(
  ({ className, type = "search", ...props }, ref) => (
    <Input
      ref={ref}
      type={type}
      className={cn(
        "border-0 bg-muted/50 shadow-none placeholder:text-muted-foreground/60 hover:bg-muted/70 forced-colors:border forced-colors:border-[CanvasText] [&::-webkit-search-cancel-button]:hidden",
        className,
      )}
      {...props}
    />
  ),
)

SearchInput.displayName = "SearchInput"

export { SearchInput }
