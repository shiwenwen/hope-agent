import { useCallback, useEffect, useState } from "react"
import { useTranslation } from "react-i18next"
import { useMemoryData } from "./useMemoryData"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import EmbeddingView from "./EmbeddingView"
import MemoryFormView from "./MemoryFormView"
import MemoryListView from "./MemoryListView"
import MemorySettingsView from "./MemorySettingsView"
import CoreMemoryManager from "./CoreMemoryManager"
import PendingMemoryReview from "./PendingMemoryReview"
import DreamingPanel from "./DreamingPanel"
import ClaimsBetaView from "./ClaimsBetaView"
import ProfileSnapshotView from "./ProfileSnapshotView"
import MemoryOverviewView from "./MemoryOverviewView"
import {
  buildClaimFocusState,
  consumePendingMemoryFocus,
  subscribeMemoryFocus,
  type ClaimFocusState,
  type MemoryOverviewFocus,
  type MemoryFocusTarget,
} from "./memoryFocus"

type MemoryPanelTab = "overview" | "settings" | "manage" | "dreaming" | "profile" | "claims"

type ClaimFocus = ClaimFocusState

interface MemoryFocus {
  nonce: number
  memoryId: number
}

interface ExperienceFocus {
  nonce: number
  kind: "episode" | "procedure"
  id: string
}

interface OverviewFocus extends MemoryOverviewFocus {
  nonce: number
}

/**
 * MemoryPanel - Memory management UI.
 *
 * Two modes:
 * - **Standalone** (no agentId): Global view with agent scope filter dropdown.
 *   Used in Settings > Memory tab.
 * - **Embedded** (agentId provided): Agent-scoped view showing only that agent's
 *   memories + global memories. Used inside Agent edit panel's Memory tab.
 */
export default function MemoryPanel({ agentId, compact }: { agentId?: string; compact?: boolean }) {
  const { t } = useTranslation()
  const isAgentMode = !!agentId
  const [tab, setTab] = useState<MemoryPanelTab>("overview")
  const [claimFocus, setClaimFocus] = useState<ClaimFocus | null>(null)
  const [memoryFocus, setMemoryFocus] = useState<MemoryFocus | null>(null)
  const [experienceFocus, setExperienceFocus] = useState<ExperienceFocus | null>(null)
  const [overviewFocus, setOverviewFocus] = useState<OverviewFocus | null>(null)

  const data = useMemoryData({ agentId, isAgentMode })
  const { effectiveExtractClaims, setView } = data

  const openClaims = useCallback((focus?: Omit<ClaimFocus, "nonce">) => {
    setClaimFocus((prev) => buildClaimFocusState(focus, prev?.nonce ?? 0))
    setTab("claims")
  }, [])

  const applyFocus = useCallback(
    (target: MemoryFocusTarget) => {
      if (target.kind === "overview") {
        setView("list")
        setOverviewFocus((prev) => ({
          nonce: (prev?.nonce ?? 0) + 1,
          auditOpen: target.auditOpen,
          auditAction: target.auditAction,
          auditQuery: target.auditQuery,
        }))
        setTab("overview")
        return
      }
      if (target.kind === "claim") {
        if (!isAgentMode && effectiveExtractClaims) {
          setView("list")
          openClaims({ ...target, selectedId: target.id })
        }
        return
      }
      if (target.kind === "claims") {
        if (!isAgentMode && effectiveExtractClaims) {
          setView("list")
          openClaims(target)
        }
        return
      }
      if (target.kind === "memory") {
        setView("list")
        setMemoryFocus((prev) => ({
          nonce: (prev?.nonce ?? 0) + 1,
          memoryId: target.id,
        }))
        setTab("manage")
        return
      }
      if (target.kind === "episode" || target.kind === "procedure") {
        setView("list")
        setExperienceFocus((prev) => ({
          nonce: (prev?.nonce ?? 0) + 1,
          kind: target.kind,
          id: target.id,
        }))
        setTab("overview")
        return
      }
      if (target.kind === "profile") {
        if (!isAgentMode) {
          setView("list")
          setTab("profile")
        }
      }
    },
    [effectiveExtractClaims, isAgentMode, openClaims, setView],
  )

  useEffect(() => {
    let cancelled = false
    const pending = consumePendingMemoryFocus()
    if (pending) {
      queueMicrotask(() => {
        if (!cancelled) applyFocus(pending)
      })
    }
    const unsubscribe = subscribeMemoryFocus(applyFocus)
    return () => {
      cancelled = true
      unsubscribe()
    }
  }, [applyFocus])

  // If the structured-memory tab was selected and the flag is then turned off
  // elsewhere (ha-settings skill / another window), fall back to settings so
  // the panel never renders a blank body for a tab with no content. Derived
  // during render (no setState-in-effect).
  const activeTab = tab === "claims" && !data.effectiveExtractClaims ? "settings" : tab

  // ── Embedding Config View ──
  if (data.view === "embedding") {
    return <EmbeddingView data={data} />
  }

  // ── Add / Edit View ──
  if (data.view === "add" || data.view === "edit") {
    return <MemoryFormView data={data} />
  }

  return (
    <Tabs
      value={activeTab}
      onValueChange={(value) => setTab(value as MemoryPanelTab)}
      className="flex-1 flex flex-col min-h-0"
    >
      <div className="px-6 pt-2 shrink-0">
        <TabsList>
          <TabsTrigger value="overview">{t("settings.tabOverview")}</TabsTrigger>
          <TabsTrigger value="settings">{t("settings.memoryTabs.settings")}</TabsTrigger>
          <TabsTrigger value="manage">{t("settings.memoryTabs.manage")}</TabsTrigger>
          {!isAgentMode && (
            <TabsTrigger value="dreaming">{t("settings.memoryTabs.dreaming")}</TabsTrigger>
          )}
          {!isAgentMode && (
            <TabsTrigger value="profile">{t("settings.memoryTabs.profile")}</TabsTrigger>
          )}
          {!isAgentMode && data.effectiveExtractClaims && (
            <TabsTrigger value="claims">{t("settings.memoryTabs.claims")}</TabsTrigger>
          )}
        </TabsList>
      </div>

      <TabsContent value="overview" className="flex-1 min-h-0 outline-none">
        <MemoryOverviewView
          data={data}
          isAgentMode={isAgentMode}
          onSelectTab={setTab}
          onOpenClaims={openClaims}
          focus={experienceFocus}
          auditFocus={overviewFocus}
        />
      </TabsContent>

      <TabsContent value="settings" className="flex-1 min-h-0 outline-none">
        <MemorySettingsView data={data} isAgentMode={isAgentMode} />
      </TabsContent>

      <TabsContent value="manage" className="flex-1 min-h-0 outline-none">
        <div className="flex-1 overflow-y-auto p-6">
          <div className="w-full">
            {!isAgentMode && <CoreMemoryManager agents={data.agents} />}
            {!isAgentMode && <PendingMemoryReview agents={data.agents} />}
            <MemoryListView
              data={data}
              isAgentMode={isAgentMode}
              compact={compact}
              embedded
              focus={memoryFocus}
              onOpenClaims={!isAgentMode && data.effectiveExtractClaims ? openClaims : undefined}
            />
          </div>
        </div>
      </TabsContent>

      {!isAgentMode && (
        <TabsContent value="dreaming" className="flex-1 min-h-0 outline-none">
          <DreamingPanel />
        </TabsContent>
      )}

      {!isAgentMode && (
        <TabsContent value="profile" className="flex-1 min-h-0 outline-none">
          <ProfileSnapshotView />
        </TabsContent>
      )}

      {!isAgentMode && data.effectiveExtractClaims && (
        <TabsContent value="claims" className="flex-1 min-h-0 outline-none">
          <ClaimsBetaView focus={claimFocus} />
        </TabsContent>
      )}
    </Tabs>
  )
}
