import type { WebviewWindow } from "@tauri-apps/api/webviewWindow"

import { isTauriMode } from "@/lib/transport"
import { logger } from "@/lib/logger"
import type { KnowledgeFocusTarget } from "@/components/knowledge/knowledgeFocus"
import type { SettingsSection } from "@/components/settings/types"
import type { PetNavigationTarget } from "@/types/pet"

export type DetachableSpace = "knowledge" | "design"

export interface KnowledgeSpaceLocation {
  kbId: string | null
  path: string | null
}

export interface DesignSpaceLocation {
  projectId: string | null
  artifactId: string | null
}

export type SpaceWindowLocation =
  | { space: "knowledge"; location: KnowledgeSpaceLocation }
  | { space: "design"; location: DesignSpaceLocation }

export type SpaceWindowNavigationPayload =
  | SpaceWindowLocation
  | {
      space: "knowledge"
      knowledgeFocus: KnowledgeFocusTarget
    }
  | {
      space: "knowledge"
      petFocus: {
        target: Extract<PetNavigationTarget, { kind: "knowledge" }>
        nonce: number
      }
    }
  | {
      space: "design"
      petFocus: {
        target: Extract<PetNavigationTarget, { kind: "design" }>
        nonce: number
      }
    }

export interface SpaceNavigationRequest<T> {
  nonce: number
  location: T
}

export interface SpaceKnowledgeFocusRequest {
  nonce: number
  target: KnowledgeFocusTarget
}

export type SpaceWindowAction = "detach" | "close"

export interface SpaceWindowActionRequest {
  nonce: number
  action: SpaceWindowAction
}

export interface SpaceWindowSettingsRequest {
  section: SettingsSection
}

export interface SpaceWindowImplementRequest {
  sessionId: string
  message: string
}

export const SPACE_WINDOW_NAVIGATE_EVENT = "space-window:navigate"
export const SPACE_WINDOW_READY_EVENT = "space-window:ready"
export const SPACE_WINDOW_REATTACH_EVENT = "space-window:reattach"
export const SPACE_WINDOW_OPEN_SETTINGS_EVENT = "space-window:open-settings"
export const SPACE_WINDOW_IMPLEMENT_EVENT = "space-window:implement-to-code"

export interface SpaceWindowReadyPayload {
  space: DetachableSpace
  readyToken: string
}

const SPACE_WINDOW_LABELS: Record<DetachableSpace, string> = {
  knowledge: "knowledge-space-window",
  design: "design-space-window",
}

const SPACE_WINDOW_READY_TIMEOUT_MS = 15_000
const openingSpaceWindows: Partial<Record<DetachableSpace, Promise<WebviewWindow | null>>> = {}

export function usesSpaceWindowOverlayTitleBar(): boolean {
  return (
    isTauriMode() &&
    typeof navigator !== "undefined" &&
    navigator.platform.toUpperCase().includes("MAC")
  )
}

function spaceWindowUrl(target: SpaceWindowLocation, readyToken: string): string {
  const params = new URLSearchParams({ window: "space", space: target.space })
  params.set("readyToken", readyToken)
  if (target.space === "knowledge") {
    if (target.location.kbId) params.set("kbId", target.location.kbId)
    if (target.location.path) params.set("path", target.location.path)
  } else {
    if (target.location.projectId) params.set("projectId", target.location.projectId)
    if (target.location.artifactId) params.set("artifactId", target.location.artifactId)
  }
  return `index.html?${params.toString()}`
}

async function focusWindow(window: WebviewWindow): Promise<void> {
  await window.show()
  await window.unminimize()
  await window.setFocus()
}

export async function focusDetachedSpaceWindow(space: DetachableSpace): Promise<boolean> {
  if (!isTauriMode()) return false
  try {
    const { WebviewWindow } = await import("@tauri-apps/api/webviewWindow")
    const existing = await WebviewWindow.getByLabel(SPACE_WINDOW_LABELS[space])
    if (!existing) return false
    await focusWindow(existing)
    return true
  } catch (error) {
    logger.error("ui", "spaceWindow::focus", "Failed to focus detached space window", {
      space,
      error,
    })
    return false
  }
}

export async function navigateDetachedSpaceWindow(
  payload: SpaceWindowNavigationPayload,
): Promise<boolean> {
  if (!isTauriMode()) return false
  try {
    const { WebviewWindow } = await import("@tauri-apps/api/webviewWindow")
    const existing =
      (await openingSpaceWindows[payload.space]) ??
      (await WebviewWindow.getByLabel(SPACE_WINDOW_LABELS[payload.space]))
    if (!existing) return false
    await existing.emit(SPACE_WINDOW_NAVIGATE_EVENT, payload)
    await focusWindow(existing)
    return true
  } catch (error) {
    logger.error("ui", "spaceWindow::navigate", "Failed to navigate detached space window", {
      space: payload.space,
      error,
    })
    return false
  }
}

async function createDetachedSpaceWindow(
  target: SpaceWindowLocation,
  title: string,
): Promise<WebviewWindow | null> {
  if (!isTauriMode()) return null
  let stopReadyListener: (() => void) | null = null
  let readyTimeout: ReturnType<typeof setTimeout> | null = null
  try {
    const [{ WebviewWindow }, { LogicalPosition }, { listen }] = await Promise.all([
      import("@tauri-apps/api/webviewWindow"),
      import("@tauri-apps/api/dpi"),
      import("@tauri-apps/api/event"),
    ])
    const label = SPACE_WINDOW_LABELS[target.space]
    const existing = await WebviewWindow.getByLabel(label)
    if (existing) {
      await existing.emit(SPACE_WINDOW_NAVIGATE_EVENT, target)
      await focusWindow(existing)
      return existing
    }

    const readyToken = `${target.space}:${Date.now()}:${Math.random()}`
    let resolveRendererReady: (ready: boolean) => void = () => {}
    const rendererReadyPromise = new Promise<boolean>((resolve) => {
      resolveRendererReady = resolve
    })
    stopReadyListener = await listen<SpaceWindowReadyPayload>(
      SPACE_WINDOW_READY_EVENT,
      (event) => {
        if (
          event.payload?.space === target.space &&
          event.payload.readyToken === readyToken
        ) {
          resolveRendererReady(true)
        }
      },
    )

    const webview = new WebviewWindow(label, {
      url: spaceWindowUrl(target, readyToken),
      title,
      width: 1320,
      height: 860,
      minWidth: 760,
      minHeight: 520,
      center: true,
      acceptFirstMouse: true,
      ...(usesSpaceWindowOverlayTitleBar()
        ? {
            decorations: true,
            titleBarStyle: "overlay" as const,
            hiddenTitle: true,
            trafficLightPosition: new LogicalPosition(8, 18),
          }
        : {}),
    })

    const created = await new Promise<boolean>((resolve) => {
      let settled = false
      const settle = (value: boolean) => {
        if (settled) return
        settled = true
        resolve(value)
      }
      webview.once("tauri://created", () => settle(true))
      webview.once("tauri://error", (event) => {
        logger.error("ui", "spaceWindow::open", "Failed to create detached space window", {
          space: target.space,
          error: event.payload,
        })
        settle(false)
      })
    })
    if (!created) {
      stopReadyListener()
      stopReadyListener = null
      return null
    }

    const rendererReady = await Promise.race([
      rendererReadyPromise,
      new Promise<boolean>((resolve) => {
        readyTimeout = setTimeout(() => resolve(false), SPACE_WINDOW_READY_TIMEOUT_MS)
      }),
    ])
    if (readyTimeout) clearTimeout(readyTimeout)
    readyTimeout = null
    stopReadyListener()
    stopReadyListener = null
    if (!rendererReady) {
      logger.error(
        "ui",
        "spaceWindow::open",
        "Detached space renderer did not become ready",
        { space: target.space },
      )
      await webview.close().catch(() => undefined)
      return null
    }
    return webview
  } catch (error) {
    if (readyTimeout) clearTimeout(readyTimeout)
    stopReadyListener?.()
    logger.error("ui", "spaceWindow::open", "Failed to open detached space window", {
      space: target.space,
      error,
    })
    return null
  }
}

export function openDetachedSpaceWindow(
  target: SpaceWindowLocation,
  title: string,
): Promise<WebviewWindow | null> {
  const opening = openingSpaceWindows[target.space]
  if (opening) {
    return opening.then(async (webview) => {
      if (!webview) return null
      await webview.emit(SPACE_WINDOW_NAVIGATE_EVENT, target)
      await focusWindow(webview)
      return webview
    })
  }

  const request = createDetachedSpaceWindow(target, title)
  openingSpaceWindows[target.space] = request
  void request.finally(() => {
    if (openingSpaceWindows[target.space] === request) {
      delete openingSpaceWindows[target.space]
    }
  })
  return request
}

export function readSpaceWindowLocation(params: URLSearchParams): SpaceWindowLocation {
  if (params.get("space") === "design") {
    return {
      space: "design",
      location: {
        projectId: params.get("projectId"),
        artifactId: params.get("artifactId"),
      },
    }
  }
  return {
    space: "knowledge",
    location: {
      kbId: params.get("kbId"),
      path: params.get("path"),
    },
  }
}

export async function focusMainWindow(): Promise<void> {
  if (!isTauriMode()) return
  const { WebviewWindow } = await import("@tauri-apps/api/webviewWindow")
  const main = await WebviewWindow.getByLabel("main")
  if (main) await focusWindow(main)
}
