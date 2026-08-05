import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type MouseEvent as ReactMouseEvent,
  type PointerEvent as ReactPointerEvent,
} from "react"
import { flushSync } from "react-dom"
import { emit } from "@tauri-apps/api/event"
import { getCurrentWindow } from "@tauri-apps/api/window"
import { ChevronDown } from "lucide-react"
import { useTranslation } from "react-i18next"
import type { ApprovalRequest } from "@/components/chat/ApprovalDialog"
import type { AskUserQuestionGroup } from "@/components/chat/ask-user/AskUserQuestionBlock"
import { useApprovals } from "@/components/chat/hooks/useApprovals"
import { Button } from "@/components/ui/button"
import { PetApprovalCard } from "@/components/pet/PetApprovalCard"
import { PetAskUserCard } from "@/components/pet/PetAskUserCard"
import { PetBubble, type PetReplyDispatch } from "@/components/pet/PetBubble"
import { AnimatedPetSprite } from "@/components/pet/PetSprite"
import {
  actionForStatus,
  lookTargetForPointer,
  type PetAction,
  type PetLookTarget,
} from "@/components/pet/hooks/usePetAnimator"
import { usePetActivity } from "@/components/pet/hooks/usePetActivity"
import { usePetAssetUrl } from "@/components/pet/hooks/usePetAssetUrl"
import { usePetStreamPreviews } from "@/components/pet/hooks/usePetStreamPreviews"
import {
  usePetInactivePointer,
  type PetInactiveHoverTarget,
} from "@/components/pet/hooks/usePetInactivePointer"
import { usePetWindowLayout, type PetOverlayMode } from "@/components/pet/hooks/usePetWindowLayout"
import { logger } from "@/lib/logger"
import { TRANSPORT_EVENT_RESYNC_REQUIRED, type ChatStartArgs } from "@/lib/transport"
import { getTransport } from "@/lib/transport-provider"
import { cn } from "@/lib/utils"
import type { PetActivity, PetConfig, PetLibrarySnapshot, PetNavigationTarget } from "@/types/pet"

type PetInteraction =
  | { kind: "approval"; request: ApprovalRequest }
  | { kind: "ask_user"; group: AskUserQuestionGroup }

const NATIVE_DRAG_PRESENTATION_MS = 34
const PET_NATIVE_DRAG_ENDED_EVENT = "pet:native_drag_ended"
const V2_HOVER_GREETING_DELAY_MS = 700

function sessionIdForTarget(target: PetNavigationTarget): string {
  return target.sessionId
}

function activityProjectionKey(activity: PetActivity): string {
  return `${activity.activityId}:${activity.status}:${activity.boundary ?? activity.updatedAt}`
}

function activityAutoExpandKey(activity: PetActivity): string {
  return `${activity.activityId}:${activity.status}:${activity.boundary ?? "active"}`
}

function interactionAutoExpandKey(interaction: PetInteraction): string {
  return interaction.kind === "approval"
    ? `approval:${interaction.request.request_id}`
    : `ask_user:${interaction.group.requestId}`
}

export default function PetWindow() {
  const { t } = useTranslation()
  const { snapshot, initialized } = usePetActivity()
  const { approvalRequests, handleApprovalResponse } = useApprovals(undefined)
  const [config, setConfig] = useState<PetConfig | null>(null)
  const [library, setLibrary] = useState<PetLibrarySnapshot | null>(null)
  const [stackExpanded, setStackExpanded] = useState(false)
  const [expandedReplyId, setExpandedReplyId] = useState<string | null>(null)
  const [menuOpen, setMenuOpen] = useState(false)
  const [dragging, setDragging] = useState(false)
  const [dragAction, setDragAction] = useState<PetAction | null>(null)
  const [pointerAction, setPointerAction] = useState<PetAction | null>(null)
  const [lookTarget, setLookTarget] = useState<PetLookTarget>(null)
  const [askGroups, setAskGroups] = useState<Map<string, AskUserQuestionGroup>>(new Map())
  const [dismissedActivityKeys, setDismissedActivityKeys] = useState<Set<string>>(new Set())
  const measureRef = useRef<HTMLDivElement>(null)
  const liveOverlayRef = useRef<HTMLDivElement>(null)
  const petButtonRef = useRef<HTMLButtonElement>(null)
  const menuItemRef = useRef<HTMLButtonElement>(null)
  const observedActivityKeys = useRef<Set<string>>(new Set())
  const observedInteractionKeys = useRef<Set<string>>(new Set())
  const readActivityRequests = useRef<Map<string, Promise<boolean>>>(new Map())
  const readActivitySucceeded = useRef<Set<string>>(new Set())
  const pointerGesture = useRef<{ x: number; y: number; dragged: boolean } | null>(null)
  const dragWindowX = useRef<number | null>(null)
  const nativeDragFrame = useRef<number | null>(null)
  const nativeDragTimer = useRef<ReturnType<typeof setTimeout> | null>(null)
  const hoverGreetingTimer = useRef<ReturnType<typeof setTimeout> | null>(null)
  const suppressNextPetClick = useRef(false)
  const domPetHovered = useRef(false)
  const inactivePetHovered = useRef(false)
  const inactivePetWasHovered = useRef(false)

  const selectedPet = useMemo(() => {
    const selected = library?.pets.find((pet) => pet.petRef === config?.selectedPetRef)
    return selected ?? library?.pets.find((pet) => pet.builtin) ?? null
  }, [config?.selectedPetRef, library])
  const petAsset = usePetAssetUrl(selectedPet?.assetId ?? null)
  const isV1Pet = selectedPet?.manifest.spriteVersionNumber === 1
  const isV2Pet = selectedPet?.manifest.spriteVersionNumber === 2
  const clearHoverGreeting = useCallback(() => {
    if (hoverGreetingTimer.current === null) return
    clearTimeout(hoverGreetingTimer.current)
    hoverGreetingTimer.current = null
  }, [])
  const scheduleV2HoverGreeting = useCallback(() => {
    clearHoverGreeting()
    hoverGreetingTimer.current = setTimeout(() => {
      hoverGreetingTimer.current = null
      if (!domPetHovered.current && !inactivePetHovered.current) return
      if (pointerGesture.current?.dragged) return
      setPointerAction((current) => current ?? "wave")
    }, V2_HOVER_GREETING_DELAY_MS)
  }, [clearHoverGreeting])
  const streamPreviews = usePetStreamPreviews(snapshot.activities)
  const visibleActivities = useMemo(
    () =>
      snapshot.activities.filter(
        (activity) => !dismissedActivityKeys.has(activityProjectionKey(activity)),
      ),
    [dismissedActivityKeys, snapshot.activities],
  )
  const dismissedVisibleCount = snapshot.activities.reduce(
    (count, activity) =>
      count + (dismissedActivityKeys.has(activityProjectionKey(activity)) ? 1 : 0),
    0,
  )

  const loadLibrary = useCallback(async () => {
    const transport = getTransport()
    const [nextConfig, nextLibrary] = await Promise.all([
      transport.call<PetConfig>("get_pet_config_cmd"),
      transport.call<PetLibrarySnapshot>("pet_list_cmd"),
    ])
    setConfig(nextConfig)
    setLibrary(nextLibrary)
  }, [])

  const refreshLibrary = useCallback(() => {
    void loadLibrary().catch((error) => {
      logger.warn("pet", "library_refresh", "Failed to refresh the pet library", error)
    })
  }, [loadLibrary])

  useEffect(() => {
    const initialRefresh = setTimeout(refreshLibrary, 0)
    const transport = getTransport()
    const unlisteners = [
      transport.listen("pet:library_changed", refreshLibrary),
      transport.listen("pet:config_changed", refreshLibrary),
      transport.listen(TRANSPORT_EVENT_RESYNC_REQUIRED, refreshLibrary),
    ]
    return () => {
      clearTimeout(initialRefresh)
      for (const unlisten of unlisteners) unlisten()
    }
  }, [refreshLibrary])

  useEffect(() => {
    if (!initialized) return
    const nextKeys = new Set(visibleActivities.map(activityAutoExpandKey))
    const hasNewActivity = [...nextKeys].some((key) => !observedActivityKeys.current.has(key))
    const timer = setTimeout(() => {
      observedActivityKeys.current = nextKeys
      if (hasNewActivity) setStackExpanded(true)
      if (nextKeys.size === 0) {
        setStackExpanded(false)
        setExpandedReplyId(null)
      }
    }, 0)
    return () => clearTimeout(timer)
  }, [initialized, visibleActivities])

  const needsInputSessionIds = useMemo(
    () =>
      snapshot.activities
        .filter((activity) => activity.status === "needs_input")
        .map((activity) => activity.activityId),
    [snapshot.activities],
  )
  const needsInputKey = needsInputSessionIds.join("|")

  useEffect(() => {
    let cancelled = false
    const ids = needsInputKey ? needsInputKey.split("|") : []
    if (ids.length === 0) {
      const timer = setTimeout(() => {
        if (!cancelled) setAskGroups(new Map())
      }, 0)
      return () => {
        cancelled = true
        clearTimeout(timer)
      }
    }
    void Promise.all(
      ids.map(async (sessionId) => {
        const group = await getTransport().call<AskUserQuestionGroup | null>(
          "get_pending_ask_user_group",
          { sessionId },
        )
        return [sessionId, group] as const
      }),
    )
      .then((entries) => {
        if (cancelled) return
        setAskGroups(
          new Map(
            entries.filter(
              (entry): entry is readonly [string, AskUserQuestionGroup] => entry[1] !== null,
            ),
          ),
        )
      })
      .catch((error) => {
        logger.warn("pet", "ask_user_snapshot", "Failed to load Pet ask-user cards", error)
      })
    return () => {
      cancelled = true
    }
    // Snapshot revision changes when an interaction is created or resolved,
    // even if the owning activity remains in needs_input throughout.
  }, [needsInputKey, snapshot.revision])

  const activityBySession = useMemo(
    () => new Map(snapshot.activities.map((activity) => [activity.activityId, activity])),
    [snapshot.activities],
  )
  const interactions = useMemo(() => {
    const approvalsBySession = new Map<string, ApprovalRequest[]>()
    for (const request of approvalRequests) {
      const sessionId = request.session_id
      if (!sessionId || !activityBySession.has(sessionId)) continue
      const queue = approvalsBySession.get(sessionId) ?? []
      queue.push(request)
      approvalsBySession.set(sessionId, queue)
    }
    const ordered: PetInteraction[] = []
    for (const activity of snapshot.activities) {
      for (const request of approvalsBySession.get(activity.activityId) ?? []) {
        ordered.push({ kind: "approval", request })
      }
      const group = askGroups.get(activity.activityId)
      if (group) {
        ordered.push({
          kind: "ask_user",
          group,
        })
      }
    }
    return ordered
  }, [activityBySession, approvalRequests, askGroups, snapshot.activities])
  const currentInteraction = interactions[0] ?? null

  useEffect(() => {
    if (!initialized) return
    const nextKeys = new Set(interactions.map(interactionAutoExpandKey))
    const hasNewInteraction = [...nextKeys].some((key) => !observedInteractionKeys.current.has(key))
    const timer = setTimeout(() => {
      observedInteractionKeys.current = nextKeys
      if (hasNewInteraction) setStackExpanded(true)
    }, 0)
    return () => clearTimeout(timer)
  }, [initialized, interactions])

  const markActivityRead = useCallback((activity: PetActivity): Promise<boolean> => {
    if (!activity.boundary || !["ready", "blocked"].includes(activity.status)) {
      return Promise.resolve(true)
    }
    const key = activityProjectionKey(activity)
    if (readActivitySucceeded.current.has(key)) return Promise.resolve(true)
    const existing = readActivityRequests.current.get(key)
    if (existing) return existing

    const request = getTransport()
      .call("mark_session_read_cmd", {
        sessionId: activity.activityId,
        throughMessageId: activity.boundary,
      })
      .then(() => {
        readActivitySucceeded.current.add(key)
        return true
      })
      .catch((error) => {
        logger.warn("pet", "activity_read", "Failed to mark a viewed Pet activity as read", error)
        return false
      })
      .finally(() => {
        readActivityRequests.current.delete(key)
      })
    readActivityRequests.current.set(key, request)
    return request
  }, [])

  const openTarget = async (activity: PetActivity) => {
    try {
      await getTransport().call("pet_focus_target_cmd", { target: activity.target })
      setStackExpanded(false)
      setExpandedReplyId(null)
      setMenuOpen(false)
    } catch (error) {
      logger.warn("pet", "focus_target", "Failed to focus a pet activity", error)
    }
  }

  const dispatchReply = useCallback(
    async (activity: PetActivity, text: string): Promise<PetReplyDispatch> => {
      const transport = getTransport()
      if (activity.status === "running") {
        await transport.call("queue_turn_user_message", {
          requestId: crypto.randomUUID(),
          sessionId: sessionIdForTarget(activity.target),
          message: text,
          attachments: [],
        })
        return "queued"
      }
      const args: ChatStartArgs = {
        message: text,
        attachments: [],
        sessionId: sessionIdForTarget(activity.target),
        uiSurface: "pet_chat",
        ...(activity.target.kind === "knowledge"
          ? { toolScope: "knowledge" as const }
          : activity.target.kind === "design"
            ? { toolScope: "design" as const }
            : {}),
      }
      const start = transport.startChat(args, () => undefined)
      const outcome = await new Promise<PetReplyDispatch>((resolve, reject) => {
        let acknowledged = false
        const timer = setTimeout(() => {
          acknowledged = true
          resolve("sent")
        }, 180)
        void start.then(
          () => {
            if (acknowledged) return
            clearTimeout(timer)
            acknowledged = true
            resolve("sent")
          },
          (error) => {
            if (acknowledged) {
              logger.warn("pet", "quick_reply", "Pet reply turn failed after dispatch", error)
              return
            }
            clearTimeout(timer)
            acknowledged = true
            reject(error)
          },
        )
      })
      void markActivityRead(activity)
      return outcome
    },
    [markActivityRead],
  )

  const stopActivity = useCallback(async (activity: PetActivity) => {
    try {
      await getTransport().call("stop_chat", {
        sessionId: sessionIdForTarget(activity.target),
        turnId: null,
      })
    } catch (error) {
      logger.warn("pet", "stop_activity", "Failed to stop a Pet activity", error)
    }
  }, [])

  const dismissActivity = useCallback(
    async (activity: PetActivity) => {
      const key = activityProjectionKey(activity)
      const currentKeys = new Set(snapshot.activities.map(activityProjectionKey))
      setDismissedActivityKeys((current) => {
        const next = new Set([...current].filter((dismissedKey) => currentKeys.has(dismissedKey)))
        next.add(key)
        return next
      })
      setExpandedReplyId((current) => (current === activity.activityId ? null : current))
      if (!(await markActivityRead(activity))) {
        setDismissedActivityKeys((current) => {
          const next = new Set(current)
          next.delete(key)
          return next
        })
      }
    },
    [markActivityRead, snapshot.activities],
  )

  const handlePetClick = () => {
    clearHoverGreeting()
    const suppress = suppressNextPetClick.current || pointerGesture.current?.dragged
    suppressNextPetClick.current = false
    pointerGesture.current = null
    if (suppress) return
    if (menuOpen) {
      setMenuOpen(false)
      return
    }
    setPointerAction("jump")
    setStackExpanded(false)
    setExpandedReplyId(null)
    void getTransport()
      .call("pet_focus_target_cmd", { target: null })
      .catch((error) => {
        logger.warn("pet", "focus_main", "Failed to focus the main window from Pet", error)
      })
  }

  const handlePointerDown = (event: ReactPointerEvent<HTMLButtonElement>) => {
    if (event.button !== 0) return
    clearHoverGreeting()
    suppressNextPetClick.current = false
    pointerGesture.current = { x: event.clientX, y: event.clientY, dragged: false }
    event.currentTarget.setPointerCapture(event.pointerId)
  }

  const finishPetDrag = useCallback(() => {
    if (!pointerGesture.current?.dragged) return
    if (nativeDragFrame.current !== null) {
      cancelAnimationFrame(nativeDragFrame.current)
      nativeDragFrame.current = null
    }
    if (nativeDragTimer.current !== null) {
      clearTimeout(nativeDragTimer.current)
      nativeDragTimer.current = null
    }
    pointerGesture.current = null
    dragWindowX.current = null
    setDragging(false)
    setDragAction(null)
  }, [])

  const updateLookTarget = useCallback(
    (clientX: number, clientY: number): PetLookTarget => {
      if (!isV2Pet || dragging || pointerGesture.current?.dragged) {
        setLookTarget(null)
        return null
      }
      const rect = petButtonRef.current?.getBoundingClientRect()
      if (!rect) return null
      const next = lookTargetForPointer(clientX, clientY, rect)
      setLookTarget((current) => (current === next ? current : next))
      return next
    },
    [dragging, isV2Pet],
  )
  const handleV2PointerMotion = useCallback(
    (clientX: number, clientY: number, overPet: boolean) => {
      const next = updateLookTarget(clientX, clientY)
      if (!isV2Pet) return
      setPointerAction((current) => (current === "wave" ? null : current))
      if (overPet && next === "neutral" && !dragging && !pointerGesture.current?.dragged) {
        scheduleV2HoverGreeting()
      } else {
        clearHoverGreeting()
      }
    },
    [clearHoverGreeting, dragging, isV2Pet, scheduleV2HoverGreeting, updateLookTarget],
  )
  const inactivePointerLook = useCallback(
    (x: number | null, y: number | null, target: PetInactiveHoverTarget) => {
      inactivePetHovered.current = target.pet
      if (x === null || y === null) {
        clearHoverGreeting()
        if (isV2Pet) setPointerAction(null)
        setLookTarget(domPetHovered.current && isV2Pet ? "neutral" : null)
        return
      }
      handleV2PointerMotion(x, y, target.pet)
    },
    [clearHoverGreeting, handleV2PointerMotion, isV2Pet],
  )
  const inactiveHover = usePetInactivePointer(inactivePointerLook)

  useEffect(
    () => getTransport().listen(PET_NATIVE_DRAG_ENDED_EVENT, finishPetDrag),
    [finishPetDrag],
  )

  useEffect(() => {
    let disposed = false
    let unlisten: (() => void) | null = null
    void getCurrentWindow()
      .onMoved(({ payload }) => {
        if (!pointerGesture.current?.dragged) {
          dragWindowX.current = null
          return
        }
        const previousX = dragWindowX.current
        dragWindowX.current = payload.x
        if (previousX === null || payload.x === previousX) return
        setDragAction(payload.x < previousX ? "run_left" : "run_right")
      })
      .then((dispose) => {
        if (disposed) dispose()
        else unlisten = dispose
      })
      .catch((error) => {
        logger.warn("pet", "native_drag_direction", "Failed to observe Pet window movement", error)
      })
    return () => {
      disposed = true
      unlisten?.()
    }
  }, [])

  const handlePointerMove = (event: ReactPointerEvent<HTMLButtonElement>) => {
    const gesture = pointerGesture.current
    if (!gesture || gesture.dragged) return
    const dx = event.clientX - gesture.x
    const dy = event.clientY - gesture.y
    if (Math.hypot(dx, dy) < 4) return
    clearHoverGreeting()
    gesture.dragged = true
    dragWindowX.current = null
    suppressNextPetClick.current = true
    flushSync(() => {
      setPointerAction(null)
      setLookTarget(null)
      setMenuOpen(false)
      setDragging(true)
      setDragAction(dx < 0 ? "run_left" : "run_right")
    })
    // macOS snapshots the last submitted WebView texture before entering its
    // blocking native drag loop. Commit synchronously, cross a paint boundary,
    // then leave two display intervals for that texture to reach WindowServer.
    nativeDragFrame.current = requestAnimationFrame(() => {
      nativeDragFrame.current = null
      nativeDragTimer.current = setTimeout(() => {
        nativeDragTimer.current = null
        void getCurrentWindow()
          .startDragging()
          .catch((error) => {
            logger.warn("pet", "native_drag", "Failed to start native pet dragging", error)
            finishPetDrag()
          })
      }, NATIVE_DRAG_PRESENTATION_MS)
    })
  }

  useEffect(
    () => () => {
      if (nativeDragFrame.current !== null) cancelAnimationFrame(nativeDragFrame.current)
      if (nativeDragTimer.current !== null) clearTimeout(nativeDragTimer.current)
      clearHoverGreeting()
    },
    [clearHoverGreeting],
  )

  const handlePetContextMenu = (event: ReactMouseEvent<HTMLButtonElement>) => {
    event.preventDefault()
    event.stopPropagation()
    clearHoverGreeting()
    pointerGesture.current = null
    suppressNextPetClick.current = false
    setPointerAction(null)
    setStackExpanded(false)
    setExpandedReplyId(null)
    setMenuOpen(true)
  }

  const tuckAway = async () => {
    try {
      setMenuOpen(false)
      await getTransport().call("pet_set_enabled_cmd", { enabled: false, source: "pet-window" })
    } catch (error) {
      logger.warn("pet", "tuck_away", "Failed to tuck away the pet", error)
    }
  }

  const openPetSettings = async () => {
    setMenuOpen(false)
    try {
      await getTransport().call("pet_focus_target_cmd", { target: null })
      await emit("open-settings", { section: "pets" })
    } catch (error) {
      logger.warn("pet", "open_settings", "Failed to open pet settings", error)
    }
  }

  const closeMenu = useCallback(() => {
    setMenuOpen(false)
    requestAnimationFrame(() => petButtonRef.current?.focus())
  }, [])

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return
      if (menuOpen) {
        event.preventDefault()
        closeMenu()
      } else if (expandedReplyId) {
        event.preventDefault()
        setExpandedReplyId(null)
      } else if (stackExpanded) {
        event.preventDefault()
        setStackExpanded(false)
      }
    }
    document.addEventListener("keydown", onKeyDown)
    return () => document.removeEventListener("keydown", onKeyDown)
  }, [closeMenu, expandedReplyId, menuOpen, stackExpanded])

  const overlayVisible = stackExpanded && (!!currentInteraction || visibleActivities.length > 0)
  const desiredMode: PetOverlayMode = overlayVisible ? "bubble" : "none"
  const committedLayout = usePetWindowLayout(desiredMode, measureRef, dragging, liveOverlayRef)
  const action = dragAction ?? pointerAction ?? actionForStatus(snapshot.dominant)

  useEffect(() => {
    const enteredPet = inactiveHover.pet && !inactivePetWasHovered.current
    inactivePetWasHovered.current = inactiveHover.pet
    if (dragging) {
      clearHoverGreeting()
      return
    }
    if (!inactiveHover.pet || !enteredPet || !isV1Pet) return
    const frame = requestAnimationFrame(() => setPointerAction((current) => current ?? "wave"))
    return () => cancelAnimationFrame(frame)
  }, [clearHoverGreeting, dragging, inactiveHover.pet, isV1Pet])

  useEffect(() => {
    if (!menuOpen) return
    const frame = requestAnimationFrame(() => menuItemRef.current?.focus())
    return () => cancelAnimationFrame(frame)
  }, [menuOpen])

  const renderInteraction = (interaction: PetInteraction, measuring: boolean) => {
    const queuePosition = { current: 1, total: interactions.length }
    if (interaction.kind === "approval") {
      return (
        <PetApprovalCard
          key={interaction.request.request_id}
          request={interaction.request}
          queuePosition={queuePosition}
          measuring={measuring}
          onRespond={handleApprovalResponse}
        />
      )
    }
    return (
      <PetAskUserCard
        key={interaction.group.requestId}
        group={interaction.group}
        queuePosition={queuePosition}
        measuring={measuring}
        onSubmitted={() => {
          setAskGroups((current) => {
            const next = new Map(current)
            next.delete(interaction.group.sessionId)
            return next
          })
        }}
      />
    )
  }

  const renderOverlay = (mode: Exclude<PetOverlayMode, "none">, measuring: boolean) => {
    if (mode === "bubble") {
      return (
        <div className="p-1">
          <div
            className={cn(
              "max-h-[496px] overflow-y-auto px-4 pb-7 pt-4",
              currentInteraction ? "w-[392px]" : "w-[376px]",
            )}
          >
            <div className="space-y-2">
              {currentInteraction && renderInteraction(currentInteraction, measuring)}
              {visibleActivities.map((activity) => (
                <PetBubble
                  key={`${activity.activityId}:${activity.boundary ?? 0}`}
                  activity={activity}
                  expanded={expandedReplyId === activity.activityId}
                  interactionPending={activity.status === "needs_input"}
                  livePreview={streamPreviews.get(activity.activityId)}
                  measuring={measuring}
                  onOpen={() => void openTarget(activity)}
                  onDismiss={() => void dismissActivity(activity)}
                  onExpandReply={() => setExpandedReplyId(activity.activityId)}
                  onCollapseReply={() => setExpandedReplyId(null)}
                  onReply={(text) => dispatchReply(activity, text)}
                  onStop={() => stopActivity(activity)}
                  onViewed={() => void markActivityRead(activity)}
                  nativeHovered={inactiveHover.activityId === activity.activityId}
                  nativeHoveredAction={
                    inactiveHover.activityId === activity.activityId ? inactiveHover.action : null
                  }
                />
              ))}
            </div>
          </div>
        </div>
      )
    }
    return null
  }

  const committedOverlay =
    committedLayout.mode !== "none" ? renderOverlay(committedLayout.mode, false) : null
  const measureOverlay = desiredMode !== "none" ? renderOverlay(desiredMode, true) : null
  const overlayBeforePet = committedLayout.vertical === "above"
  const activityCount = Math.max(
    0,
    (snapshot.total || snapshot.activities.length) - dismissedVisibleCount,
  )
  const pendingConversationCount = new Set(
    interactions.map((interaction) =>
      interaction.kind === "approval"
        ? interaction.request.session_id
        : interaction.group.sessionId,
    ),
  ).size
  const collapsedNeedsAction = !stackExpanded && pendingConversationCount > 0
  const expandLabel = t("pet.stack.expand", {
    count: activityCount,
    defaultValue: `Show ${activityCount} conversations`,
  })
  const stackControlLabel = stackExpanded
    ? t("pet.stack.collapse", { defaultValue: "Collapse conversations" })
    : collapsedNeedsAction
      ? `${expandLabel}. ${t("pet.bubble.needsInput", {
          count: pendingConversationCount,
          defaultValue: `${pendingConversationCount} conversations need your input.`,
        })}`
      : expandLabel

  return (
    <main
      className="relative h-screen w-screen overflow-hidden bg-transparent"
      onPointerMove={(event) =>
        handleV2PointerMotion(event.clientX, event.clientY, domPetHovered.current)
      }
      onPointerLeave={() => {
        domPetHovered.current = false
        clearHoverGreeting()
        if (isV2Pet) setPointerAction(null)
        setLookTarget(null)
      }}
      onPointerDown={(event) => {
        if (menuOpen && event.target === event.currentTarget) closeMenu()
      }}
    >
      <div
        className={cn(
          "flex h-full w-full pointer-events-none",
          overlayBeforePet ? "flex-col justify-end" : "flex-col justify-start",
          committedLayout.horizontal === "left" ? "items-end" : "items-start",
        )}
      >
        {overlayBeforePet && committedOverlay && (
          <div
            ref={liveOverlayRef}
            className={cn(
              "pointer-events-auto mb-2 transform-gpu transition-[opacity,transform] duration-[180ms] ease-out motion-reduce:transition-none",
              committedLayout.visible ? "translate-y-0 opacity-100" : "translate-y-1 opacity-0",
            )}
          >
            {committedOverlay}
          </div>
        )}
        <div className="relative flex h-[128px] w-[120px] shrink-0 transform-gpu items-end justify-center pointer-events-auto">
          {activityCount > 0 && !menuOpen && (
            <Button
              type="button"
              variant="secondary"
              size="icon"
              onClick={() => {
                if (stackExpanded) {
                  for (const activity of visibleActivities) void markActivityRead(activity)
                }
                setStackExpanded((expanded) => !expanded)
                setExpandedReplyId(null)
              }}
              aria-label={stackControlLabel}
              className={cn(
                "absolute right-1 top-0 z-10 size-7 min-w-7 rounded-full border border-border/70 p-0 text-xs shadow-sm",
                collapsedNeedsAction
                  ? "bg-amber-400/90 text-amber-950 hover:bg-amber-400"
                  : "bg-muted/90 text-muted-foreground hover:bg-muted",
              )}
            >
              {stackExpanded ? (
                <ChevronDown className="h-4 w-4" />
              ) : activityCount > 9 ? (
                "9+"
              ) : (
                activityCount
              )}
            </Button>
          )}
          <Button
            ref={petButtonRef}
            data-pet-sprite
            type="button"
            variant="ghost"
            onClick={handlePetClick}
            onPointerEnter={(event) => {
              domPetHovered.current = true
              if (isV2Pet) {
                handleV2PointerMotion(event.clientX, event.clientY, true)
              } else if (!dragging && !pointerAction && isV1Pet) {
                setPointerAction("wave")
              }
            }}
            onPointerLeave={(event) => {
              domPetHovered.current = false
              handleV2PointerMotion(event.clientX, event.clientY, false)
            }}
            onPointerDown={handlePointerDown}
            onPointerMove={handlePointerMove}
            onPointerUp={finishPetDrag}
            onContextMenu={handlePetContextMenu}
            aria-label={t("pet.window.interact", { defaultValue: "Interact with Nori" })}
            className="h-auto w-auto cursor-grab rounded-2xl bg-transparent p-1 hover:bg-transparent active:cursor-grabbing"
          >
            <AnimatedPetSprite
              src={petAsset.src}
              action={action}
              rowCount={isV1Pet ? 9 : 11}
              lookTarget={action === "idle" ? lookTarget : null}
              dimmed={petAsset.loading || petAsset.failed || snapshot.stale}
              onActionComplete={(completed) => {
                setPointerAction((current) => (current === completed ? null : current))
              }}
            />
          </Button>
          {menuOpen && (
            <div
              role="menu"
              aria-label={t("pet.window.interact", { defaultValue: "Pet actions" })}
              className="pointer-events-auto absolute left-1/2 top-1/2 z-20 flex max-w-[116px] -translate-x-1/2 -translate-y-1/2 items-center gap-0.5 rounded-full border border-border/60 bg-popover/90 p-0.5 shadow-md backdrop-blur-md"
            >
              <Button
                ref={menuItemRef}
                type="button"
                role="menuitem"
                variant="ghost"
                onClick={() => void openPetSettings()}
                className="h-6 min-w-0 flex-1 rounded-full px-2 text-[11px] font-normal text-muted-foreground hover:bg-muted hover:text-foreground"
              >
                <span className="truncate">
                  {t("common.settings", { defaultValue: "Settings" })}
                </span>
              </Button>
              <Button
                type="button"
                role="menuitem"
                variant="ghost"
                onClick={() => void tuckAway()}
                className="h-6 min-w-0 flex-1 rounded-full px-2 text-[11px] font-normal text-muted-foreground hover:bg-muted hover:text-foreground"
              >
                <span className="truncate">{t("common.close", { defaultValue: "Close" })}</span>
              </Button>
            </div>
          )}
        </div>
        {!overlayBeforePet && committedOverlay && (
          <div
            ref={liveOverlayRef}
            className={cn(
              "pointer-events-auto mt-2 transform-gpu transition-[opacity,transform] duration-[180ms] ease-out motion-reduce:transition-none",
              committedLayout.visible ? "translate-y-0 opacity-100" : "-translate-y-1 opacity-0",
            )}
          >
            {committedOverlay}
          </div>
        )}
      </div>
      {measureOverlay && (
        <div
          ref={measureRef}
          className="invisible pointer-events-none absolute left-0 top-0"
          aria-hidden="true"
        >
          {measureOverlay}
        </div>
      )}
    </main>
  )
}
