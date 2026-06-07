// Whole-KB link graph view (WS1, Phase 2; Batch J = drag-to-pin + persisted
// layout). Canvas force-directed layout via react-force-graph-2d (pure npm,
// offline, no CDN — CSP-safe). Nodes = notes (sized by degree, orphans coloured
// distinctly, the open note ringed), edges = resolved `[[ ]]`/`![[ ]]` links.
// Click a node to open it; drag a node to pin it (position persists per KB in
// sessions.db, keyed by rel_path so it survives index rebuilds); "Reset layout"
// unpins everything.
//
// `data` deliberately depends only on [graph, layout] — NOT activePath — so node
// objects are stable across note clicks (the force engine keeps their positions,
// no reshuffle, and we never read a ref during render). The active-note ring is
// painted from `activePathRef` (a canvas callback, not React render) and a
// `resumeAnimation()` nudge repaints it when the open note changes.

import { RotateCcw } from "lucide-react"
import { useCallback, useEffect, useMemo, useRef, useState } from "react"
import { useTranslation } from "react-i18next"
import ForceGraph2D, { type ForceGraphMethods } from "react-force-graph-2d"

import { IconTip } from "@/components/ui/tooltip"
import { getTransport } from "@/lib/transport-provider"
import type { GraphNodePosition, KnowledgeGraph } from "@/types/knowledge"

const COLOR_NODE = "#6366f1" // indigo (connected note)
const COLOR_ORPHAN = "#f59e0b" // amber (no resolved links)
const COLOR_LINK = "rgba(130,130,150,0.28)"
const COLOR_RING = "#ec4899" // pink ring on the currently-open note
const COLOR_PIN = "#10b981" // emerald ring on a pinned (fixed) node
const SAVE_DEBOUNCE_MS = 600

interface VizNode {
  id: number
  name: string
  relPath: string
  degree: number
  orphan: boolean
  color: string
  // mutated by the force engine; fx/fy fix a node in place (a user pin):
  x?: number
  y?: number
  fx?: number
  fy?: number
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
    layout: GraphNodePosition[]
  } | null>(null)
  const [size, setSize] = useState<{ w: number; h: number }>({ w: 0, h: 0 })
  // Bumped on drag (a pin mutates a node's fx/fy in place, off-render) to force
  // the derived `hasPins` to recompute.
  const [pinTick, setPinTick] = useState(0)

  const graphKey = `${kbId}:${refreshKey}`

  // Fetch the graph + saved layout for this KB (refetch on KB / knowledge change).
  useEffect(() => {
    let alive = true
    const tx = getTransport()
    Promise.all([
      tx.call<KnowledgeGraph>("kb_graph_cmd", { kbId }),
      tx.call<GraphNodePosition[]>("kb_graph_layout_get_cmd", { kbId }),
    ])
      .then(([g, layout]) => {
        if (alive) setFetched({ kbId, refreshKey, graph: g, layout: layout ?? [] })
      })
      .catch((e) => {
        console.error("kb_graph fetch failed", e)
        if (alive) {
          setFetched({
            kbId,
            refreshKey,
            graph: { nodes: [], edges: [], truncated: false },
            layout: [],
          })
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

  // Build the viz data once per graph/layout. Pinned nodes are seeded straight
  // from the saved layout (fx/fy) so the persisted arrangement restores on load;
  // unpinned nodes get no seed and the engine places them.
  const data = useMemo(() => {
    if (!graph) return { nodes: [] as VizNode[], links: [] as VizLink[] }
    const layoutMap = new Map((fetched?.layout ?? []).map((p) => [p.relPath, p]))
    const nodes: VizNode[] = graph.nodes.map((n) => {
      const degree = n.inDegree + n.outDegree
      const orphan = degree === 0
      const saved = layoutMap.get(n.relPath)
      return {
        id: n.id,
        name: n.title || n.relPath,
        relPath: n.relPath,
        degree,
        orphan,
        color: orphan ? COLOR_ORPHAN : COLOR_NODE,
        x: saved?.x,
        y: saved?.y,
        fx: saved?.x,
        fy: saved?.y,
      }
    })
    const links: VizLink[] = graph.edges.map((e) => ({ source: e.source, target: e.target }))
    return { nodes, links }
  }, [graph, fetched?.layout])

  // Mirror the current nodes + graph identity into refs for the event-time
  // handlers (drag / reset / debounced save) — read there, never during render.
  const nodesRef = useRef<VizNode[]>([])
  const graphKeyRef = useRef("")
  useEffect(() => {
    nodesRef.current = data.nodes
    graphKeyRef.current = graphKey
  }, [data, graphKey])

  // Reset button visibility — derived (a fresh `data` build from saved layout
  // seeds pins; `pinTick` re-derives after an in-session drag).
  const hasPins = useMemo(() => {
    void pinTick // recompute trigger: a drag mutates fx/fy off-render
    return data.nodes.some((n) => n.fx != null && n.fy != null)
  }, [data, pinTick])

  // Active-note ring: kept in a ref read by the canvas painter; a resume nudge
  // forces a repaint so the ring follows the open note without rebuilding nodes.
  const activePathRef = useRef<string | null | undefined>(activePath)
  useEffect(() => {
    activePathRef.current = activePath
    fgRef.current?.resumeAnimation()
  }, [activePath])

  // Persist the pinned set (debounced) — only nodes the user has fixed.
  const saveTimer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined)
  const persistLayout = useCallback(() => {
    clearTimeout(saveTimer.current)
    const forKey = graphKeyRef.current
    saveTimer.current = setTimeout(() => {
      // The graph identity changed under us (a knowledge:changed refresh during
      // the debounce) — skip this stale save rather than write another graph's
      // snapshot.
      if (graphKeyRef.current !== forKey) return
      const positions: GraphNodePosition[] = []
      for (const n of nodesRef.current) {
        if (n.fx != null && n.fy != null) positions.push({ relPath: n.relPath, x: n.fx, y: n.fy })
      }
      getTransport()
        .call("kb_graph_layout_save_cmd", { kbId, positions })
        .catch((e) => console.error("kb_graph_layout save failed", e))
    }, SAVE_DEBOUNCE_MS)
  }, [kbId])

  useEffect(() => () => clearTimeout(saveTimer.current), [])

  const handleDragEnd = useCallback(
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    (node: any) => {
      const n = node as VizNode
      // Keep the node fixed where it was dropped (the engine releases fx/fy on
      // drag end by default).
      n.fx = n.x
      n.fy = n.y
      setPinTick((t) => t + 1)
      persistLayout()
    },
    [persistLayout],
  )

  const handleReset = useCallback(() => {
    clearTimeout(saveTimer.current)
    // Drop the saved layout so `data` rebuilds with no fx/fy seed — fresh node
    // objects float free and the engine re-lays-out (no ref mutation needed);
    // the rebuilt `data` re-derives `hasPins` to false on its own.
    setFetched((prev) => (prev ? { ...prev, layout: [] } : prev))
    getTransport()
      .call("kb_graph_layout_save_cmd", { kbId, positions: [] })
      .catch((e) => console.error("kb_graph_layout reset failed", e))
  }, [kbId])

  const nodePaint = useCallback(
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    (node: any, ctx: CanvasRenderingContext2D, globalScale: number) => {
      const n = node as VizNode
      const r = 3 + Math.min(n.degree, 10) * 0.55
      ctx.beginPath()
      ctx.arc(n.x ?? 0, n.y ?? 0, r, 0, 2 * Math.PI)
      ctx.fillStyle = n.color
      ctx.fill()
      // The open note gets a pink ring; a pinned node an emerald one (active wins).
      const isActive = !!activePathRef.current && n.relPath === activePathRef.current
      if (isActive || (n.fx != null && n.fy != null)) {
        ctx.lineWidth = 2 / globalScale
        ctx.strokeStyle = isActive ? COLOR_RING : COLOR_PIN
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

  // Fit to view once per graph load (not on every settle, so a drag/refresh
  // doesn't yank the camera); never let an empty settle consume the one-shot fit.
  const didFitRef = useRef("")
  const handleEngineStop = useCallback(() => {
    if (data.nodes.length === 0) return
    if (didFitRef.current !== graphKey) {
      didFitRef.current = graphKey
      fgRef.current?.zoomToFit(400, 40)
    }
  }, [graphKey, data.nodes.length])

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
        {hasPins && (
          <IconTip label={t("knowledge.graph.resetLayout", "Reset layout (unpin all)")}>
            <button
              type="button"
              onClick={handleReset}
              className="ml-auto flex items-center gap-1 rounded px-1.5 py-0.5 hover:bg-muted hover:text-foreground"
            >
              <RotateCcw className="h-3 w-3" />
              {t("knowledge.graph.reset", "Reset layout")}
            </button>
          </IconTip>
        )}
      </div>
      <div ref={containerRef} className="relative min-h-0 flex-1 overflow-hidden">
        {empty ? (
          <div className="flex h-full items-center justify-center text-sm text-muted-foreground">
            {t("knowledge.graph.empty", "No notes to graph yet.")}
          </div>
        ) : (
          !loading &&
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
              onNodeDragEnd={handleDragEnd}
              onEngineStop={handleEngineStop}
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
