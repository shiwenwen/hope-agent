import { useCallback, useRef, type ReactNode } from "react"
import { useTranslation } from "react-i18next"
import { toast } from "sonner"

import { SelectionActionMenu } from "@/components/common/SelectionActionMenu"
import { cn } from "@/lib/utils"
import type { CodeSelection } from "./ShikiCodeView"
import { selectionWithLineRange } from "./selectionLineRange"
import { readDomTextSelection, useSelectionActionMenu } from "./useSelectionActionMenu"

export function SelectableDomPreview({
  children,
  sourceText,
  copyAllText,
  onQuote,
  className,
}: {
  children: ReactNode
  /** Original source used only to infer line numbers for a rendered preview. */
  sourceText?: string
  /** Enables the legacy right-click copy-all action for this surface. */
  copyAllText?: string
  onQuote?: (selection: CodeSelection) => void
  className?: string
}) {
  const { t } = useTranslation()
  const rootRef = useRef<HTMLDivElement>(null)
  const readSelection = useCallback(() => {
    const selected = readDomTextSelection(rootRef.current)
    return selected ? selectionWithLineRange(selected.text, sourceText) : null
  }, [sourceText])
  const getCopyAllText = useCallback(() => copyAllText ?? "", [copyAllText])
  const { menu, closeMenu, onContextMenuCapture } = useSelectionActionMenu({
    rootRef,
    readSelection,
    ...(copyAllText !== undefined ? { getCopyAllText } : {}),
  })

  const copyText = useCallback(
    (text: string) => {
      navigator.clipboard.writeText(text).then(
        () => toast.success(t("fileBrowser.copied", "Copied")),
        () => toast.error(t("fileBrowser.copyFailed", "Copy failed")),
      )
    },
    [t],
  )

  return (
    <>
      <div
        ref={rootRef}
        className={cn("relative", className)}
        onContextMenuCapture={onContextMenuCapture}
      >
        {children}
      </div>
      <SelectionActionMenu
        open={menu !== null}
        position={menu?.position ?? null}
        text={menu?.text ?? ""}
        copyMode={menu?.copyMode}
        onCopy={copyText}
        quoteDisabled={!menu?.value}
        onQuote={
          onQuote
            ? () => {
                if (menu?.value) onQuote(menu.value)
              }
            : undefined
        }
        onClose={closeMenu}
      />
    </>
  )
}
