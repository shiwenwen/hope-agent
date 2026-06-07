import { useState } from "react"
import { useTranslation } from "react-i18next"
import { useMemoryData } from "./useMemoryData"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import EmbeddingView from "./EmbeddingView"
import MemoryFormView from "./MemoryFormView"
import MemoryListView from "./MemoryListView"
import MemorySettingsView from "./MemorySettingsView"
import CoreMemoryEditor from "./CoreMemoryEditor"
import DreamingPanel from "./DreamingPanel"
import ClaimsBetaView from "./ClaimsBetaView"
import ProfileSnapshotView from "./ProfileSnapshotView"

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
  const [tab, setTab] = useState<"settings" | "manage" | "dreaming" | "profile" | "claims">("settings")

  const data = useMemoryData({ agentId, isAgentMode })

  // If the Claims (beta) tab was selected and the flag is then turned off
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
      onValueChange={(value) =>
        setTab(value as "settings" | "manage" | "dreaming" | "profile" | "claims")
      }
      className="flex-1 flex flex-col min-h-0"
    >
      <div className="px-6 pt-2 shrink-0">
        <TabsList>
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

      <TabsContent value="settings" className="flex-1 min-h-0 outline-none">
        <MemorySettingsView data={data} isAgentMode={isAgentMode} />
      </TabsContent>

      <TabsContent value="manage" className="flex-1 min-h-0 outline-none">
        <div className="flex-1 overflow-y-auto p-6">
          <div className="w-full">
            {!isAgentMode && <CoreMemoryEditor scope="global" />}
            <MemoryListView
              data={data}
              isAgentMode={isAgentMode}
              compact={compact}
              embedded
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
          <ClaimsBetaView />
        </TabsContent>
      )}
    </Tabs>
  )
}
