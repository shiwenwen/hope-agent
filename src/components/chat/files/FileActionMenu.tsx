import type { ReactNode } from "react"
import { useTranslation } from "react-i18next"
import { MoreHorizontal } from "lucide-react"

import { cn } from "@/lib/utils"
import { IconTip } from "@/components/ui/tooltip"
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuTrigger,
} from "@/components/ui/context-menu"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import { FILE_ACTION_META, type FileAction } from "@/lib/fileActions"
import type { FileActionsOverrides } from "./useFileActions"
import { useFileResource } from "./useFileResource"
import type { PreviewTarget } from "./useFilePreview"

function actionLabel(t: ReturnType<typeof useTranslation>["t"], action: FileAction): string {
  const meta = FILE_ACTION_META[action]
  return t(meta.labelKey, meta.defaultLabel)
}

/**
 * Wrap any element so a right-click opens the unified file-action menu
 * (preview / open / download / reveal, resolved by kind × mode). Renders the
 * children unchanged when there's no target or no applicable action.
 */
export function FileContextMenu({
  target,
  overrides,
  children,
}: {
  target: PreviewTarget | null
  overrides?: FileActionsOverrides
  children: ReactNode
}) {
  const { t } = useTranslation()
  const { menu, run } = useFileResource(target, overrides)
  if (!target || menu.length === 0) return <>{children}</>
  return (
    <ContextMenu>
      <ContextMenuTrigger asChild>{children}</ContextMenuTrigger>
      <ContextMenuContent variant="floating">
        {menu.map((action) => {
          const Icon = FILE_ACTION_META[action].icon
          return (
            <ContextMenuItem key={action} onSelect={() => run(action)} className="gap-2">
              <Icon className="h-3.5 w-3.5 text-muted-foreground" />
              {actionLabel(t, action)}
            </ContextMenuItem>
          )
        })}
      </ContextMenuContent>
    </ContextMenu>
  )
}

/**
 * A "⋯" button that opens the same unified menu on left-click — the
 * discoverable affordance for file cards / rows (right-click alone is easy to
 * miss). Renders nothing when no action applies.
 */
export function FileActionsMoreButton({
  target,
  overrides,
  className,
}: {
  target: PreviewTarget | null
  overrides?: FileActionsOverrides
  className?: string
}) {
  const { t } = useTranslation()
  const { menu, run } = useFileResource(target, overrides)
  if (!target || menu.length === 0) return null
  return (
    <DropdownMenu>
      <IconTip label={t("fileActions.more", "More actions")}>
        <DropdownMenuTrigger asChild>
          <button
            type="button"
            aria-label={t("fileActions.more", "More actions")}
            className={cn(
              "rounded p-1 text-muted-foreground transition-colors hover:bg-muted hover:text-foreground",
              className,
            )}
          >
            <MoreHorizontal className="h-3.5 w-3.5" />
          </button>
        </DropdownMenuTrigger>
      </IconTip>
      <DropdownMenuContent variant="floating" align="end">
        {menu.map((action) => {
          const Icon = FILE_ACTION_META[action].icon
          return (
            <DropdownMenuItem key={action} onSelect={() => run(action)} className="gap-2">
              <Icon className="h-3.5 w-3.5 text-muted-foreground" />
              {actionLabel(t, action)}
            </DropdownMenuItem>
          )
        })}
      </DropdownMenuContent>
    </DropdownMenu>
  )
}
