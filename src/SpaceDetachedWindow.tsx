import { lazy, Suspense, useCallback, useEffect, useRef, useState } from "react"
import { emitTo, listen } from "@tauri-apps/api/event"
import { getCurrentWindow } from "@tauri-apps/api/window"

import { TooltipProvider } from "@/components/ui/tooltip"
import { Toaster } from "@/components/ui/sonner"
import { LightboxProvider } from "@/components/common/ImageLightbox"
import ErrorBoundary from "@/components/common/ErrorBoundary"
import DangerousModeBanner from "@/components/common/DangerousModeBanner"
import { initLanguageFromConfig, listenLanguageConfigChange } from "@/i18n/i18n"
import { initThemeFromConfig, listenThemeConfigChange } from "@/hooks/useTheme"
import { logger } from "@/lib/logger"
import {
  SPACE_WINDOW_IMPLEMENT_EVENT,
  SPACE_WINDOW_NAVIGATE_EVENT,
  SPACE_WINDOW_OPEN_SETTINGS_EVENT,
  SPACE_WINDOW_READY_EVENT,
  SPACE_WINDOW_REATTACH_EVENT,
  focusMainWindow,
  readSpaceWindowLocation,
  type DesignSpaceLocation,
  type KnowledgeSpaceLocation,
  type SpaceNavigationRequest,
  type SpaceKnowledgeFocusRequest,
  type SpaceWindowAction,
  type SpaceWindowActionRequest,
  type SpaceWindowImplementRequest,
  type SpaceWindowLocation,
  type SpaceWindowNavigationPayload,
  type SpaceWindowReadyPayload,
  type SpaceWindowSettingsRequest,
} from "@/lib/spaceWindow"
import type { PetNavigationTarget } from "@/types/pet"

const KnowledgeView = lazy(() => import("@/components/knowledge/KnowledgeView"))
const DesignView = lazy(() => import("@/components/design/DesignView"))

export default function SpaceDetachedWindow() {
  const [initialTarget] = useState<SpaceWindowLocation>(() =>
    readSpaceWindowLocation(new URLSearchParams(window.location.search)),
  )
  const [readyToken] = useState(() =>
    new URLSearchParams(window.location.search).get("readyToken"),
  )
  const space = initialTarget.space
  const locationRef = useRef(initialTarget.location)
  const reattachingRef = useRef(false)
  const allowNativeCloseRef = useRef(false)
  const navigationNonceRef = useRef(1)
  const windowActionNonceRef = useRef(0)
  const [knowledgeNavigation, setKnowledgeNavigation] =
    useState<SpaceNavigationRequest<KnowledgeSpaceLocation> | null>(
      initialTarget.space === "knowledge"
        ? { nonce: 1, location: initialTarget.location }
        : null,
    )
  const [designNavigation, setDesignNavigation] =
    useState<SpaceNavigationRequest<DesignSpaceLocation> | null>(
      initialTarget.space === "design"
        ? { nonce: 1, location: initialTarget.location }
        : null,
    )
  const [knowledgeWindowActionRequest, setKnowledgeWindowActionRequest] =
    useState<SpaceWindowActionRequest | null>(null)
  const [knowledgeFocus, setKnowledgeFocus] = useState<SpaceKnowledgeFocusRequest | null>(null)
  const [knowledgePetFocus, setKnowledgePetFocus] = useState<{
    target: Extract<PetNavigationTarget, { kind: "knowledge" }>
    nonce: number
  } | null>(null)
  const [designPetFocus, setDesignPetFocus] = useState<{
    target: Extract<PetNavigationTarget, { kind: "design" }>
    nonce: number
  } | null>(null)

  useEffect(() => {
    void initLanguageFromConfig()
    void initThemeFromConfig()
    const stopLanguage = listenLanguageConfigChange()
    const stopTheme = listenThemeConfigChange()
    return () => {
      stopLanguage()
      stopTheme()
    }
  }, [])

  useEffect(() => {
    let cancelled = false
    let unlisten: (() => void) | null = null
    void (async () => {
      const stop = await listen<SpaceWindowNavigationPayload>(
        SPACE_WINDOW_NAVIGATE_EVENT,
        (event) => {
          const payload = event.payload
          if (!payload || payload.space !== space) return
          if ("petFocus" in payload) {
            if (payload.space === "knowledge") setKnowledgePetFocus(payload.petFocus)
            else setDesignPetFocus(payload.petFocus)
            return
          }
          if ("knowledgeFocus" in payload) {
            setKnowledgeFocus({
              nonce: ++navigationNonceRef.current,
              target: payload.knowledgeFocus,
            })
            return
          }
          const nonce = ++navigationNonceRef.current
          if (payload.space === "knowledge") {
            setKnowledgeNavigation({ nonce, location: payload.location })
          } else {
            setDesignNavigation({ nonce, location: payload.location })
          }
        },
      )
      if (cancelled) {
        stop()
        return
      }
      unlisten = stop
      if (readyToken) {
        await emitTo<SpaceWindowReadyPayload>("main", SPACE_WINDOW_READY_EVENT, {
          space,
          readyToken,
        })
      }
    })().catch((error) => {
      logger.error(
        "ui",
        "SpaceDetachedWindow::ready",
        "Failed to register detached space navigation",
        { space, error },
      )
    })
    return () => {
      cancelled = true
      unlisten?.()
    }
  }, [readyToken, space])

  useEffect(() => {
    if (space !== "knowledge") return
    let cancelled = false
    let unlisten: (() => void) | null = null
    void getCurrentWindow()
      .onCloseRequested((event) => {
        if (allowNativeCloseRef.current) return
        event.preventDefault()
        setKnowledgeWindowActionRequest({
          nonce: ++windowActionNonceRef.current,
          action: "close",
        })
      })
      .then((stop) => {
        if (cancelled) stop()
        else unlisten = stop
      })
    return () => {
      cancelled = true
      unlisten?.()
    }
  }, [space])

  const closeDetachedWindow = useCallback(async (location: KnowledgeSpaceLocation) => {
    locationRef.current = location
    allowNativeCloseRef.current = true
    try {
      await getCurrentWindow().close()
    } catch (error) {
      allowNativeCloseRef.current = false
      logger.error(
        "ui",
        "SpaceDetachedWindow::close",
        "Failed to close detached space window",
        error,
      )
    }
  }, [])

  const handleReattach = useCallback(
    async (location?: KnowledgeSpaceLocation | DesignSpaceLocation) => {
      if (reattachingRef.current) return
      reattachingRef.current = true
      if (location) locationRef.current = location
      try {
        await emitTo("main", SPACE_WINDOW_REATTACH_EVENT, {
          space,
          location: locationRef.current,
        } as SpaceWindowLocation)
        await focusMainWindow()
        allowNativeCloseRef.current = true
        await getCurrentWindow().close()
      } catch (error) {
        reattachingRef.current = false
        allowNativeCloseRef.current = false
        logger.error("ui", "SpaceDetachedWindow::reattach", "Failed to reattach space window", {
          space,
          error,
        })
      }
    },
    [space],
  )

  const openMainSettings = useCallback(async (section: "knowledge" | "design") => {
    await emitTo<SpaceWindowSettingsRequest>("main", SPACE_WINDOW_OPEN_SETTINGS_EVENT, {
      section,
    })
    await focusMainWindow()
  }, [])

  const implementToCode = useCallback(async (sessionId: string, message: string) => {
    await emitTo<SpaceWindowImplementRequest>("main", SPACE_WINDOW_IMPLEMENT_EVENT, {
      sessionId,
      message,
    })
    await focusMainWindow()
  }, [])

  return (
    <ErrorBoundary>
      <TooltipProvider>
        <LightboxProvider>
          <div className="flex h-screen min-h-0 min-w-0 flex-col bg-surface-app">
            <DangerousModeBanner />
            <div className="flex min-h-0 min-w-0 flex-1">
              <Suspense
                fallback={
                  <div className="flex flex-1 items-center justify-center">
                    <div className="h-6 w-6 animate-spin rounded-full border-2 border-foreground border-t-transparent" />
                  </div>
                }
              >
                {space === "knowledge" ? (
                  <KnowledgeView
                    isViewVisible
                    windowMode="detached"
                    windowNavigation={knowledgeNavigation}
                    knowledgeFocus={knowledgeFocus}
                    onKnowledgeFocusHandled={(nonce) =>
                      setKnowledgeFocus((current) => (current?.nonce === nonce ? null : current))
                    }
                    windowActionRequest={knowledgeWindowActionRequest}
                    onWindowLocationChange={(location) => {
                      locationRef.current = location
                    }}
                    onWindowActionReady={(
                      action: SpaceWindowAction,
                      location: KnowledgeSpaceLocation,
                    ) => {
                      if (action === "close") void closeDetachedWindow(location)
                    }}
                    onToggleWindowMode={(location) => void handleReattach(location)}
                    onOpenSettings={() => void openMainSettings("knowledge")}
                    petFocus={knowledgePetFocus}
                    onPetFocusHandled={(nonce) =>
                      setKnowledgePetFocus((current) => (current?.nonce === nonce ? null : current))
                    }
                  />
                ) : (
                  <DesignView
                    isViewVisible
                    windowMode="detached"
                    windowNavigation={designNavigation}
                    onWindowLocationChange={(location) => {
                      locationRef.current = location
                    }}
                    onToggleWindowMode={(location) => void handleReattach(location)}
                    onOpenSettings={() => void openMainSettings("design")}
                    onImplementToCode={(sessionId, message) =>
                      void implementToCode(sessionId, message)
                    }
                    petFocus={designPetFocus}
                    onPetFocusHandled={(nonce) =>
                      setDesignPetFocus((current) => (current?.nonce === nonce ? null : current))
                    }
                  />
                )}
              </Suspense>
            </div>
            <Toaster />
          </div>
        </LightboxProvider>
      </TooltipProvider>
    </ErrorBoundary>
  )
}
