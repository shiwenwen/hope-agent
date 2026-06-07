// Collapsible read-only outline view (Phase 3 G — D8 "native outline as an
// optional layer"). Renders a note as a foldable heading tree with each section's
// prose nested under its heading. Purely a render of the live `.md` text (via
// `buildOutline`); it never edits the document (D8 red line: outline never
// replaces the CM6 base). Clicking a heading jumps the source editor to it.

import { ChevronDown, ChevronRight } from "lucide-react"
import { memo, useMemo, useState } from "react"
import { useTranslation } from "react-i18next"

import MarkdownRenderer from "@/components/common/MarkdownRenderer"
import { IconTip } from "@/components/ui/tooltip"
import { cn } from "@/lib/utils"

import { buildOutline, type OutlineNode } from "./outline"

interface OutlineViewProps {
  content: string
  /** Jump the source editor to a 1-based line (the caller switches to an
   *  editable mode so the reveal is visible). */
  onJump?: (line: number) => void
}

function OutlineView({ content, onJump }: OutlineViewProps) {
  const { t } = useTranslation()
  const { preamble, nodes } = useMemo(() => buildOutline(content), [content])
  const [collapsed, setCollapsed] = useState<ReadonlySet<number>>(() => new Set<number>())

  const toggle = (line: number) =>
    setCollapsed((prev) => {
      const next = new Set(prev)
      if (next.has(line)) next.delete(line)
      else next.add(line)
      return next
    })

  if (!preamble && nodes.length === 0) {
    return (
      <div className="flex h-full items-center justify-center text-xs text-muted-foreground">
        {t("knowledge.outlineEmpty", "No headings")}
      </div>
    )
  }

  return (
    <div className="h-full overflow-auto px-3 py-2 text-sm">
      {preamble && (
        <div className="mb-2 border-l-2 border-border-soft/40 pl-2">
          <MarkdownRenderer content={preamble} />
        </div>
      )}
      {nodes.map((n) => (
        <OutlineNodeRow
          key={n.heading.line}
          node={n}
          depth={0}
          collapsed={collapsed}
          onToggle={toggle}
          onJump={onJump}
        />
      ))}
    </div>
  )
}

function OutlineNodeRow({
  node,
  depth,
  collapsed,
  onToggle,
  onJump,
}: {
  node: OutlineNode
  depth: number
  collapsed: ReadonlySet<number>
  onToggle: (line: number) => void
  onJump?: (line: number) => void
}) {
  const isCollapsed = collapsed.has(node.heading.line)
  const hasContent = node.body.length > 0 || node.children.length > 0
  return (
    <div>
      <div className="flex items-center gap-1" style={{ paddingLeft: `${depth * 14}px` }}>
        <button
          type="button"
          onClick={() => hasContent && onToggle(node.heading.line)}
          className={cn(
            "flex h-5 w-5 shrink-0 items-center justify-center rounded text-muted-foreground",
            hasContent ? "hover:bg-muted hover:text-foreground" : "opacity-0",
          )}
          aria-label={isCollapsed ? "expand" : "collapse"}
        >
          {isCollapsed ? (
            <ChevronRight className="h-3.5 w-3.5" />
          ) : (
            <ChevronDown className="h-3.5 w-3.5" />
          )}
        </button>
        <IconTip label={node.heading.text || null} side="left">
          <button
            type="button"
            onClick={() => onJump?.(node.heading.line)}
            className={cn(
              "truncate rounded px-1 py-0.5 text-left font-medium text-foreground/90 hover:text-primary",
              node.heading.level === 1 && "text-[15px]",
            )}
          >
            {node.heading.text || <span className="opacity-40">—</span>}
          </button>
        </IconTip>
      </div>
      {!isCollapsed && (
        <div>
          {node.body && (
            <div
              className="my-1 border-l-2 border-border-soft/30 pl-2 text-muted-foreground"
              style={{ marginLeft: `${depth * 14 + 22}px` }}
            >
              <MarkdownRenderer content={node.body} />
            </div>
          )}
          {node.children.map((c) => (
            <OutlineNodeRow
              key={c.heading.line}
              node={c}
              depth={depth + 1}
              collapsed={collapsed}
              onToggle={onToggle}
              onJump={onJump}
            />
          ))}
        </div>
      )}
    </div>
  )
}

export default memo(OutlineView)
