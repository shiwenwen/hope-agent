import { useState } from "react"
import { useTranslation } from "react-i18next"
import { cn } from "@/lib/utils"
import { Button } from "@/components/ui/button"
import {
  ArrowLeft,
  Bot,
  Brain,
  Code,
  Globe,
  Info,
  MessageSquare,
  Puzzle,
  HeartPulse,
  ScrollText,
  Server,
  Settings2,
  Shield,
  ShieldCheck,
  User,
  Wrench,
  Bell,
  Container,
  Cable,
  ClipboardList,
  MessageCircle,
  LineChart,
  Users2,
} from "lucide-react"
import type { ProviderConfig } from "@/components/settings/ProviderSettings"
import ProviderSetup from "@/components/settings/ProviderSetup"
import ProviderEditPage from "@/components/settings/ProviderEditPage"
import GeneralPanel from "@/components/settings/general-panel"
import ModelConfigPanel from "@/components/settings/ModelConfigPanel"
import ToolSettingsPanel from "@/components/settings/ToolSettingsPanel"
import ChatSettingsPanel from "@/components/settings/ChatSettingsPanel"
import PlanSettingsPanel from "@/components/settings/PlanSettingsPanel"
import RecapSettingsPanel from "@/components/settings/RecapSettingsPanel"
import SkillsPanel from "@/components/settings/skills-panel"
import AgentPanel from "@/components/settings/AgentPanel"
import TeamsPanel from "@/components/settings/teams-panel"
import UserProfilePanel from "@/components/settings/profile-panel"
import AboutPanel from "@/components/settings/AboutPanel"
import LogPanel from "@/components/settings/log-panel"
import MemoryPanel from "@/components/settings/MemoryPanel"
import PermissionsPanel from "@/components/settings/PermissionsPanel"
import CrashHistoryPanel from "@/components/settings/CrashHistoryPanel"
import NotificationPanel from "@/components/settings/NotificationPanel"
import DeveloperPanel from "@/components/settings/DeveloperPanel"
import SandboxPanel from "@/components/settings/SandboxPanel"
import AcpControlPanel from "@/components/settings/AcpControlPanel"
import ChannelPanel from "@/components/settings/channel-panel"
import ServerPanel from "@/components/settings/ServerPanel"
import SecurityPanel from "@/components/settings/SecurityPanel"
import BrowserPanel from "@/components/settings/BrowserPanel"
import type { SettingsSection, SettingsSectionItem } from "./types"

const SECTIONS: SettingsSectionItem[] = [
  {
    id: "profile",
    icon: <User className="h-4 w-4" />,
    labelKey: "settings.profile",
  },
  {
    id: "general",
    icon: <Settings2 className="h-4 w-4" />,
    labelKey: "settings.general",
  },
  {
    id: "modelConfig",
    icon: <Server className="h-4 w-4" />,
    labelKey: "settings.modelConfig",
  },
  {
    id: "agents",
    icon: <Bot className="h-4 w-4" />,
    labelKey: "settings.agents",
  },
  {
    id: "teams",
    icon: <Users2 className="h-4 w-4" />,
    labelKey: "settings.teams",
  },
  {
    id: "channels",
    icon: <MessageCircle className="h-4 w-4" />,
    labelKey: "settings.channels",
  },
  {
    id: "skills",
    icon: <Puzzle className="h-4 w-4" />,
    labelKey: "settings.skills",
  },
  {
    id: "tools",
    icon: <Wrench className="h-4 w-4" />,
    labelKey: "settings.tools",
  },
  {
    id: "memory",
    icon: <Brain className="h-4 w-4" />,
    labelKey: "settings.memory",
  },
  {
    id: "chat",
    icon: <MessageSquare className="h-4 w-4" />,
    labelKey: "settings.chat",
  },
  {
    id: "plan",
    icon: <ClipboardList className="h-4 w-4" />,
    labelKey: "settings.plan",
  },
  {
    id: "recap",
    icon: <LineChart className="h-4 w-4" />,
    labelKey: "settings.recap",
  },
  {
    id: "server",
    icon: <Globe className="h-4 w-4" />,
    labelKey: "settings.server",
  },
  {
    id: "sandbox",
    icon: <Container className="h-4 w-4" />,
    labelKey: "settings.sandbox",
  },
  {
    id: "browser",
    icon: <Globe className="h-4 w-4" />,
    labelKey: "settings.browser.title",
  },
  {
    id: "acp",
    icon: <Cable className="h-4 w-4" />,
    labelKey: "settings.acpControl",
  },
  {
    id: "notifications",
    icon: <Bell className="h-4 w-4" />,
    labelKey: "settings.notifications",
  },
  {
    id: "permissions",
    icon: <Shield className="h-4 w-4" />,
    labelKey: "settings.permissions",
  },
  {
    id: "security",
    icon: <ShieldCheck className="h-4 w-4" />,
    labelKey: "settings.security",
  },
  {
    id: "health",
    icon: <HeartPulse className="h-4 w-4" />,
    labelKey: "settings.health",
  },
  {
    id: "logs",
    icon: <ScrollText className="h-4 w-4" />,
    labelKey: "settings.logs",
  },
  {
    id: "about",
    icon: <Info className="h-4 w-4" />,
    labelKey: "settings.about",
  },
  {
    id: "developer",
    icon: <Code className="h-4 w-4" />,
    labelKey: "settings.developer",
  },
]

export default function SettingsView({
  onBack,
  onCodexAuth,
  onCodexReauth,
  initialSection,
  initialAgentId,
  initialChannelId,
  onProfileSaved,
}: {
  onBack: () => void
  onCodexAuth: () => Promise<void>
  onCodexReauth?: () => void
  initialSection?: SettingsSection
  initialAgentId?: string
  /** When `initialSection === "channels"`, pre-open the Add dialog with
   *  this channel pre-selected. Used by the onboarding wizard. */
  initialChannelId?: string
  onProfileSaved?: () => void
}) {
  const { t } = useTranslation()
  const [activeSection, setActiveSection] = useState<SettingsSection>(
    initialSection ?? "modelConfig",
  )
  const [addingProvider, setAddingProvider] = useState(false)
  const [editingProvider, setEditingProvider] = useState<ProviderConfig | null>(null)

  return (
    <div className="flex flex-1 h-full overflow-hidden bg-background">
      {/* Left Sidebar — Settings Navigation */}
      <div className="w-[220px] shrink-0 border-r border-border bg-secondary/20 flex flex-col">
        {/* Header with back button + drag region */}
        <div className="h-10 flex items-end px-4 gap-2 shrink-0" data-tauri-drag-region>
          <Button
            variant="ghost"
            size="sm"
            onClick={onBack}
            className="gap-1.5 text-muted-foreground hover:text-foreground pb-1.5"
          >
            <ArrowLeft className="h-4 w-4" />
            <span className="text-sm font-semibold text-foreground">{t("settings.title")}</span>
          </Button>
        </div>

        {/* Navigation Items */}
        <div className="flex-1 overflow-y-auto p-2 space-y-0.5">
          {SECTIONS.map((section) => (
            <button
              key={section.id}
              className={cn(
                "flex items-center gap-2.5 w-full px-3 py-2 rounded-lg text-sm transition-all duration-150",
                activeSection === section.id
                  ? "bg-secondary text-foreground font-medium shadow-sm"
                  : "text-muted-foreground hover:bg-secondary/60 hover:text-foreground",
              )}
              onClick={() => setActiveSection(section.id)}
            >
              <span
                className={cn(
                  "shrink-0",
                  activeSection === section.id ? "text-primary" : "text-muted-foreground",
                )}
              >
                {section.icon}
              </span>
              {t(section.labelKey)}
            </button>
          ))}
        </div>
      </div>

      {/* Right Content Panel */}
      <div className="flex-1 flex flex-col min-w-0 overflow-hidden">
        {/* Content Header + drag region */}
        <div className="h-10 flex items-end px-6 shrink-0" data-tauri-drag-region>
          <span className="text-sm font-semibold text-foreground pb-1.5">
            {t(SECTIONS.find((s) => s.id === activeSection)?.labelKey ?? "settings.title")}
          </span>
        </div>

        {/* Content Area */}
        <div className="flex-1 flex flex-col min-h-0 overflow-hidden">
          <div
            key={activeSection}
            className="flex-1 flex flex-col min-h-0 overflow-hidden animate-in fade-in-0 duration-150"
          >
            {activeSection === "general" && <GeneralPanel />}
            {activeSection === "modelConfig" &&
              (addingProvider ? (
                <ProviderSetup
                  onComplete={() => setAddingProvider(false)}
                  onCodexAuth={onCodexAuth}
                  onCancel={() => setAddingProvider(false)}
                />
              ) : editingProvider ? (
                <ProviderEditPage
                  provider={editingProvider}
                  onSave={() => setEditingProvider(null)}
                  onCancel={() => setEditingProvider(null)}
                  onCodexReauth={onCodexReauth}
                />
              ) : (
                <ModelConfigPanel
                  onAddProvider={() => setAddingProvider(true)}
                  onEditProvider={(p) => setEditingProvider(p)}
                  onCodexReauth={onCodexReauth}
                />
              ))}
            {activeSection === "skills" && <SkillsPanel />}
            {activeSection === "agents" && <AgentPanel initialAgentId={initialAgentId} />}
            {activeSection === "teams" && <TeamsPanel />}
            {activeSection === "profile" && <UserProfilePanel onSaved={onProfileSaved} />}
            {activeSection === "memory" && <MemoryPanel />}
            {activeSection === "notifications" && <NotificationPanel />}
            {activeSection === "tools" && <ToolSettingsPanel />}
            {activeSection === "sandbox" && <SandboxPanel />}
            {activeSection === "browser" && <BrowserPanel />}
            {activeSection === "acp" && <AcpControlPanel />}
            {activeSection === "channels" && (
              <ChannelPanel initialChannelId={initialChannelId} />
            )}
            {activeSection === "permissions" && <PermissionsPanel />}
            {activeSection === "security" && <SecurityPanel />}
            {activeSection === "chat" && <ChatSettingsPanel />}
            {activeSection === "plan" && <PlanSettingsPanel />}
            {activeSection === "recap" && <RecapSettingsPanel />}
            {activeSection === "health" && <CrashHistoryPanel />}
            {activeSection === "logs" && <LogPanel />}
            {activeSection === "about" && <AboutPanel />}
            {activeSection === "server" && <ServerPanel />}
            {activeSection === "developer" && <DeveloperPanel />}
          </div>
        </div>
      </div>
    </div>
  )
}
