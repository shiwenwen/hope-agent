import {
  forwardRef,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ForwardedRef,
} from "react"
import { useTranslation } from "react-i18next"
import { toast } from "sonner"

import {
  SelectionActionMenu,
  type SelectionActionMenuPosition,
} from "@/components/common/SelectionActionMenu"
import { useTransport, useTransportRevision } from "@/lib/transport-provider"
import { cn } from "@/lib/utils"
import { fileResourceAdapterFor } from "@/components/chat/files/fileResourceAdapter"
import type { FileTarget } from "@/components/chat/files/types"

export interface ArtifactTextSelection {
  text: string
}

interface ArtifactViewerProps {
  artifactId: string
  projectPath?: string | null
  title: string
  refreshKey?: string | number
  className?: string
  /** Stages the selected excerpt in the owning composer; never sends it. */
  onQuoteSelection?: (selection: ArtifactTextSelection) => void
}

export interface ArtifactSelectionIframeProps {
  src: string
  title: string
  refreshKey?: string | number
  className?: string
  onQuoteSelection?: (selection: ArtifactTextSelection) => void
  /**
   * True only for projections whose CSP permits the exact app-authored bridge
   * and no author script. Tokens are correlation IDs, not an authentication
   * boundary for JavaScript running in the same iframe.
   */
  selectionBridgeTrusted?: boolean
}

const SELECTION_PROTOCOL_VERSION = 1
const SELECTION_MESSAGE_TYPE = "hope_artifact_text_selection"
const SELECTION_CLEAR_MESSAGE_TYPE = "hope_artifact_text_selection_clear"
const SELECTION_ACTIVATE_MESSAGE_TYPE = "hope_artifact_selection_activate"
const MAX_SELECTION_TEXT_LENGTH = 20_000
const MENU_MARGIN = 8
const MENU_HEIGHT = 42
let selectionChannelTokenCounter = 0

interface SelectionMessageRect {
  left: number
  top: number
  right: number
  bottom: number
}

interface SelectionMenuState {
  contextKey: string
  text: string
  position: SelectionActionMenuPosition
}

function assignForwardedRef<T>(ref: ForwardedRef<T>, value: T | null) {
  if (typeof ref === "function") {
    ref(value)
  } else if (ref) {
    ref.current = value
  }
}

function createSelectionChannelToken(): string {
  const cryptoApi = globalThis.crypto
  const uuid = cryptoApi?.randomUUID?.()
  if (uuid) return uuid
  if (cryptoApi?.getRandomValues) {
    const bytes = new Uint32Array(4)
    cryptoApi.getRandomValues(bytes)
    return Array.from(bytes, (value) => value.toString(16).padStart(8, "0")).join("")
  }
  selectionChannelTokenCounter += 1
  return `fallback-${Date.now().toString(36)}-${selectionChannelTokenCounter.toString(36)}`
}

function isSelectionRect(value: unknown): value is SelectionMessageRect {
  if (!value || typeof value !== "object") return false
  const rect = value as Record<string, unknown>
  return (
    typeof rect.left === "number" &&
    Number.isFinite(rect.left) &&
    typeof rect.top === "number" &&
    Number.isFinite(rect.top) &&
    typeof rect.right === "number" &&
    Number.isFinite(rect.right) &&
    typeof rect.bottom === "number" &&
    Number.isFinite(rect.bottom) &&
    rect.right >= rect.left &&
    rect.bottom >= rect.top
  )
}

function clamp(value: number, min: number, max: number): number {
  return Math.min(Math.max(value, min), Math.max(min, max))
}

function selectionMenuPosition(
  iframe: HTMLIFrameElement,
  rect: SelectionMessageRect,
  hasQuoteAction: boolean,
): SelectionActionMenuPosition | null {
  const frameRect = iframe.getBoundingClientRect()
  if (frameRect.width <= 0 || frameRect.height <= 0) return null

  // Range coordinates are relative to the iframe viewport. Account for a CSS
  // transform on an ancestor so the portal remains aligned with the selection.
  const innerWidth = iframe.clientWidth > 0 ? iframe.clientWidth : frameRect.width
  const innerHeight = iframe.clientHeight > 0 ? iframe.clientHeight : frameRect.height
  const scaleX = frameRect.width / innerWidth
  const scaleY = frameRect.height / innerHeight
  const left = clamp(rect.left, 0, innerWidth)
  const right = clamp(rect.right, left, innerWidth)
  const top = clamp(rect.top, 0, innerHeight)
  const bottom = clamp(rect.bottom, top, innerHeight)
  const menuWidth = hasQuoteAction ? 252 : 140
  const centeredX = frameRect.left + ((left + right) / 2) * scaleX - menuWidth / 2
  const selectionTop = frameRect.top + top * scaleY
  const selectionBottom = frameRect.top + bottom * scaleY
  const aboveY = selectionTop - MENU_HEIGHT - MENU_MARGIN
  const belowY = selectionBottom + MENU_MARGIN

  return {
    x: clamp(centeredX, MENU_MARGIN, window.innerWidth - menuWidth - MENU_MARGIN),
    y: clamp(
      aboveY >= MENU_MARGIN ? aboveY : belowY,
      MENU_MARGIN,
      window.innerHeight - MENU_HEIGHT - MENU_MARGIN,
    ),
  }
}

/**
 * Sandboxed iframe host for any managed HTML projection that carries the Hope
 * selection bridge. It owns the source/token checks and copy/quote toolbar;
 * callers only provide a credential-safe URL and an optional staging callback.
 */
export const ArtifactSelectionIframe = forwardRef<HTMLIFrameElement, ArtifactSelectionIframeProps>(
  (
    {
      src,
      title,
      refreshKey = 0,
      className,
      onQuoteSelection,
      selectionBridgeTrusted = false,
    },
    forwardedRef,
  ) => {
    const { t } = useTranslation()
    const iframeRef = useRef<HTMLIFrameElement | null>(null)
    const [selectionMenu, setSelectionMenu] = useState<SelectionMenuState | null>(null)
    // An empty iframe `src` resolves to the embedding page in some WebViews.
    // Keep unresolved/failed previews on an inert document instead.
    const resolvedSrc = src || "about:blank"
    const contextKey = `${resolvedSrc}-${refreshKey}`
    const selectionChannelToken = useMemo(
      () => createSelectionChannelToken(),
      // A remount/navigation gets a fresh correlation token so delayed messages
      // from the previous document cannot reopen the menu.
      // eslint-disable-next-line react-hooks/exhaustive-deps
      [contextKey],
    )
    const menuOpen = selectionBridgeTrusted && selectionMenu?.contextKey === contextKey

    const setIframeRef = useCallback(
      (node: HTMLIFrameElement | null) => {
        iframeRef.current = node
        assignForwardedRef(forwardedRef, node)
      },
      [forwardedRef],
    )

    useEffect(() => {
      if (!menuOpen) return
      const close = () => setSelectionMenu(null)
      const onKeyDown = (event: KeyboardEvent) => {
        if (event.key === "Escape") close()
      }
      window.addEventListener("pointerdown", close)
      window.addEventListener("keydown", onKeyDown)
      window.addEventListener("resize", close)
      window.addEventListener("blur", close)
      window.addEventListener("scroll", close, true)
      return () => {
        window.removeEventListener("pointerdown", close)
        window.removeEventListener("keydown", onKeyDown)
        window.removeEventListener("resize", close)
        window.removeEventListener("blur", close)
        window.removeEventListener("scroll", close, true)
      }
    }, [menuOpen])

    useEffect(() => {
      if (!selectionBridgeTrusted) return
      const handler = (event: MessageEvent) => {
        const iframe = iframeRef.current
        // sandbox="allow-scripts" deliberately gives the document an opaque
        // origin (event.origin is "null"), so WindowProxy identity is the
        // authoritative sender boundary.
        if (!iframe?.contentWindow || event.source !== iframe.contentWindow) return
        if (!event.data || typeof event.data !== "object") return
        const data = event.data as Record<string, unknown>
        if (data.version !== SELECTION_PROTOCOL_VERSION || data.token !== selectionChannelToken) {
          return
        }
        if (data.type === SELECTION_CLEAR_MESSAGE_TYPE) {
          setSelectionMenu(null)
          return
        }
        if (data.type !== SELECTION_MESSAGE_TYPE) return
        if (
          typeof data.text !== "string" ||
          data.text.length === 0 ||
          data.text.length > MAX_SELECTION_TEXT_LENGTH ||
          data.truncated === true ||
          !data.text.trim() ||
          !isSelectionRect(data.rect)
        ) {
          return
        }
        const position = selectionMenuPosition(iframe, data.rect, Boolean(onQuoteSelection))
        if (!position) return
        setSelectionMenu({ contextKey, text: data.text, position })
      }

      window.addEventListener("message", handler)
      return () => window.removeEventListener("message", handler)
    }, [contextKey, onQuoteSelection, selectionBridgeTrusted, selectionChannelToken])

    const activateSelectionBridge = useCallback(() => {
      iframeRef.current?.contentWindow?.postMessage(
        {
          type: SELECTION_ACTIVATE_MESSAGE_TYPE,
          version: SELECTION_PROTOCOL_VERSION,
          token: selectionChannelToken,
        },
        "*",
      )
    }, [selectionChannelToken])

    const copySelection = useCallback(
      async (text: string) => {
        try {
          await navigator.clipboard.writeText(text)
          toast.success(t("fileBrowser.copied", "Copied"))
        } catch {
          toast.error(t("fileBrowser.copyFailed", "Copy failed"))
        }
      },
      [t],
    )

    return (
      <>
        <iframe
          ref={setIframeRef}
          key={contextKey}
          src={resolvedSrc}
          sandbox="allow-scripts"
          referrerPolicy="no-referrer"
          onLoad={selectionBridgeTrusted ? activateSelectionBridge : undefined}
          className={cn(
            "block h-full min-h-0 w-full min-w-0 max-w-full border-0 bg-white dark:bg-surface-app",
            className,
          )}
          title={title}
        />
        <SelectionActionMenu
          open={menuOpen}
          position={selectionMenu?.position ?? null}
          text={selectionMenu?.text ?? ""}
          onCopy={copySelection}
          onQuote={
            onQuoteSelection
              ? (text) => {
                  onQuoteSelection({ text })
                }
              : undefined
          }
          onClose={() => setSelectionMenu(null)}
          className="z-[100]"
        />
      </>
    )
  },
)

ArtifactSelectionIframe.displayName = "ArtifactSelectionIframe"

/** Shared sandboxed reading surface for CanvasPanel and the Artifact Gallery. */
const ArtifactViewer = forwardRef<HTMLIFrameElement, ArtifactViewerProps>(
  ({ artifactId, projectPath, title, refreshKey = 0, className, onQuoteSelection }, ref) => {
    const { t } = useTranslation()
    const transport = useTransport()
    const transportRevision = useTransportRevision()
    const target = useMemo<Extract<FileTarget, { kind: "artifact" }>>(
      () => ({
        kind: "artifact",
        artifactId,
        name: `${title || t("artifacts.defaultName")}.html`,
        projectPath,
      }),
      [artifactId, projectPath, t, title],
    )
    const sourceKey = `${artifactId}\u0000${projectPath ?? ""}\u0000${refreshKey}\u0000${transportRevision}`
    const [resolvedSource, setResolvedSource] = useState<{
      key: string
      url: string
      selectionBridgeTrusted: boolean
    }>({
      key: "",
      url: "",
      selectionBridgeTrusted: false,
    })
    // Never render the previous Artifact's URL under the next Artifact's title
    // and quote callback while async URL resolution catches up.
    const src = resolvedSource.key === sourceKey ? resolvedSource.url : ""

    useEffect(() => {
      let cancelled = false
      const source = fileResourceAdapterFor(target).previewSource(target, { transport })
      void Promise.all([
        source.rawUrl(),
        source.selectionBridgeTrusted?.().catch(() => false) ?? Promise.resolve(false),
      ])
        .then(([url, selectionBridgeTrusted]) => {
          if (!cancelled) {
            setResolvedSource({
              key: sourceKey,
              url: url ?? "",
              selectionBridgeTrusted,
            })
          }
        })
        .catch(() => {
          if (!cancelled) {
            setResolvedSource({ key: sourceKey, url: "", selectionBridgeTrusted: false })
          }
        })
      return () => {
        cancelled = true
      }
    }, [sourceKey, target, transport])

    return (
      <ArtifactSelectionIframe
        ref={ref}
        src={src}
        title={title}
        refreshKey={`${artifactId}-${refreshKey}-${transportRevision}`}
        className={className}
        onQuoteSelection={onQuoteSelection}
        selectionBridgeTrusted={
          resolvedSource.key === sourceKey && resolvedSource.selectionBridgeTrusted
        }
      />
    )
  },
)

ArtifactViewer.displayName = "ArtifactViewer"

export default ArtifactViewer
