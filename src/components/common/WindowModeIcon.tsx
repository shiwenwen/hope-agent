import { SquareArrowDownLeft, SquareArrowOutUpRight, type LucideProps } from "lucide-react"

interface WindowModeIconProps extends LucideProps {
  action: "detach" | "reattach"
}

/** Shared visual language for moving a workspace or panel out of / back into its host window. */
export function WindowModeIcon({ action, ...props }: WindowModeIconProps) {
  const Icon = action === "detach" ? SquareArrowOutUpRight : SquareArrowDownLeft
  return <Icon aria-hidden="true" {...props} />
}
