// Whole-KB link graph view (WS1, Phase 2). Canvas force-directed layout via
// react-force-graph-2d (pure npm, offline, no CDN — CSP-safe). Nodes = notes
// (sized by degree, orphans coloured distinctly, the open note ringed), edges =
// resolved `[[ ]]`/`![[ ]]` links. Click a node to open it.

import { useCallback, useEffect, useMemo, useRef, useState } from "react"
import { useTranslation } from "react-i18next"
import ForceGraph2D, { type ForceGraphMethods } from "react-force-graph-2d"

import { getTransport } from "@/lib/transport-provider"
import type { KnowledgeGraph } from "@/types/knowledge"

const COLOR_NODE = "#6366f1" // indigo (connected note)
const COLOR_ORPHAN = "#f59e0b" // amber (no resolved links)
const COLOR_LINK = "rgba(130,130,150,0.28)"
const COLOR_RING = "#ec4899" // pink ring on the currently-open note

interface VizNode {
  id: number
  name: string
  relPath: string
  degree: number
  orphan: boolean
  active: boolean
  color: string
  // mutated by the force engine:
  x?: number
  y?: number
}

interface VizLink {
  source: number
  target: number
}

interface KnowledgeGraphViewProps {
  kbId: string
  /** Currently-open note rel-path (ringed in the graph). */
  activePath?: string | null
  /** Bumped on knowledge:changed to refetch the graph. */
  refreshKey: number
  onOpenNote: (relPath: string) => void
}

export default function KnowledgeGraphView({
  kbId,
  activePath,
  refreshKey,
  onOpenNote,
}: KnowledgeGraphViewProps) {
  const { t } = useTranslation()
  const containerRef = useRef<HTMLDivElement | null>(null)
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const fgRef = useRef<ForceGraphMethods<any, any> | undefined>(undefined)
  // Result tagged with the (kbId, refreshKey) it was fetched for, so a change
  // reads as "loading" via derivation rather than a synchronous setState.
  const [fetched, setFetched] = useState<{
    kbId: string
    refreshKey: number
    graph: KnowledgeGraph
  } | null>(null)
  const [size, setSize] = useState<{ w: number; h: number }>({ w: 0, h: 0 })

  // Fetch the graph for this KB (refetch on KB / knowledge change).
  useEffect(() => {
    let alive = true
    const tx = getTransport()
    tx.call<KnowledgeGraph>("kb_graph_cmd", { kbId })
      .then((g) => {
        if (alive) setFetched({ kbId, refreshKey, graph: g })
      })
      .catch((e) => {
        console.error("kb_graph failed", e)
        if (alive) {
          setFetched({ kbId, refreshKey, graph: { nodes: [], edges: [], truncated: false } })
        }
      })
    return () => {
      alive = false
    }
  }, [kbId, refreshKey])

  const graph =
    fetched && fetched.kbId === kbId && fetched.refreshKey === refreshKey ? fetched.graph : null
  const loading = graph === null

  // Track container size for the canvas.
  useEffect(() => {
    const el = containerRef.current
    if (!el) return
    const ro = new ResizeObserver((entries) => {
      const r = entries[0]?.contentRect
      if (r) setSize({ w: Math.floor(r.width), h: Math.floor(r.height) })
    })
    ro.observe(el)
    return () => ro.disconnect()
  }, [])

  const data = useMemo(() => {
    if (!graph) return { nodes: [] as VizNode[], links: [] as VizLink[] }
    const nodes: VizNode[] = graph.nodes.map((n) => {
      const degree = n.inDegree + n.outDegree
      const orphan = degree === 0
      return {
        id: n.id,
        name: n.title || n.relPath,
        relPath: n.relPath,
        degree,
        orphan,
        active: !!activePath && n.relPath === activePath,
        color: orphan ? COLOR_ORPHAN : COLOR_NODE,
      }
    })
    const links: VizLink[] = graph.edges.map((e) => ({ source: e.source, target: e.target }))
    return { nodes, links }
  }, [graph, activePath])

  const nodePaint = useCallback(
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    (node: any, ctx: CanvasRenderingContext2D, globalScale: number) => {
      const n = node as VizNode
      const r = 3 + Math.min(n.degree, 10) * 0.55
      ctx.beginPath()
      ctx.arc(n.x ?? 0, n.y ?? 0, r, 0, 2 * Math.PI)
      ctx.fillStyle = n.color
      ctx.fill()
      if (n.active) {
        ctx.lineWidth = 2 / globalScale
        ctx.strokeStyle = COLOR_RING
        ctx.stroke()
      }
      // Labels only when zoomed in enough to avoid clutter.
      if (globalScale > 1.6) {
        ctx.font = `${10 / globalScale}px ui-sans-serif, system-ui, sans-serif`
        ctx.fillStyle = "rgba(140,140,160,0.95)"
        ctx.textAlign = "center"
        ctx.textBaseline = "top"
        ctx.fillText(n.name, n.x ?? 0, (n.y ?? 0) + r + 1)
      }
    },
    [],
  )

  const handleClick = useCallback(
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    (node: any) => {
      const n = node as VizNode
      if (n?.relPath) onOpenNote(n.relPath)
    },
    [onOpenNote],
  )

  const empty = !loading && data.nodes.length === 0

  return (
    <div className="flex flex-1 min-w-0 flex-col">
      <div className="flex items-center gap-3 border-b border-border-soft/60 px-3 py-1.5 text-[11px] text-muted-foreground">
        <span>
          {t("knowledge.graph.stats", "{{nodes}} notes · {{edges}} links", {
            nodes: data.nodes.length,
            edges: data.links.length,
          })}
        </span>
        <span className="flex items-center gap-1">
          <span className="inline-block h-2 w-2 rounded-full" style={{ background: COLOR_ORPHAN }} />
          {t("knowledge.graph.orphanLegend", "Orphan")}
        </span>
        {graph?.truncated && (
          <span className="text-amber-500">
            {t("knowledge.graph.truncated", "Large graph — showing the most connected notes.")}
          </span>
        )}
      </div>
      <div ref={containerRef} className="relative min-h-0 flex-1 overflow-hidden">
        {empty ? (
          <div className="flex h-full items-center justify-center text-sm text-muted-foreground">
            {t("knowledge.graph.empty", "No notes to graph yet.")}
          </div>
        ) : (
          size.w > 0 &&
          size.h > 0 && (
            <ForceGraph2D
              ref={fgRef}
              width={size.w}
              height={size.h}
              graphData={data}
              nodeId="id"
              nodeLabel="name"
              nodeCanvasObject={nodePaint}
              nodePointerAreaPaint={(node, color, ctx) => {
                // eslint-disable-next-line @typescript-eslint/no-explicit-any
                const n = node as any as VizNode
                const r = 3 + Math.min(n.degree, 10) * 0.55 + 2
                ctx.fillStyle = color
                ctx.beginPath()
                ctx.arc(n.x ?? 0, n.y ?? 0, r, 0, 2 * Math.PI)
                ctx.fill()
              }}
              linkColor={() => COLOR_LINK}
              linkDirectionalArrowLength={2.5}
              linkDirectionalArrowRelPos={1}
              cooldownTicks={120}
              onNodeClick={handleClick}
              onEngineStop={() => fgRef.current?.zoomToFit(400, 40)}
            />
          )
        )}
        {loading && (
          <div className="absolute inset-0 flex items-center justify-center text-sm text-muted-foreground">
            {t("knowledge.graph.loading", "Building graph…")}
          </div>
        )}
      </div>
    </div>
  )
}
