import { useState, useEffect } from "react"
import { invoke } from "@tauri-apps/api/core"
import { logger } from "@/lib/logger"
import ProviderSetup from "@/components/settings/ProviderSetup"
import SettingsView from "@/components/settings/SettingsView"
import IconSidebar from "@/components/common/IconSidebar"
import ChatScreen from "@/components/chat/ChatScreen"
import CronCalendarView from "@/components/cron/CronCalendarView"

export default function App() {
  const [view, setView] = useState<
    "loading" | "setup" | "chat" | "settings" | "skills" | "profile" | "agents" | "calendar"
  >("loading")
  const [agentIdForSettings, setAgentIdForSettings] = useState<string | undefined>(undefined)
  const [userAvatar, setUserAvatar] = useState<string | null>(null)

  // Load user avatar
  async function fetchUserAvatar() {
    try {
      const config = await invoke<{ avatar?: string | null }>("get_user_config")
      return config.avatar ?? null
    } catch {
      return null
    }
  }

  // Reload avatar when switching back to chat
  useEffect(() => {
    if (view === "chat") {
      let cancelled = false
      fetchUserAvatar().then((avatar) => {
        if (!cancelled) setUserAvatar(avatar)
      })
      return () => { cancelled = true }
    }
  }, [view])

  // Try to restore previous session on mount
  useEffect(() => {
    ;(async () => {
      try {
        const avatar = await fetchUserAvatar()
        setUserAvatar(avatar)
        const restored = await invoke<boolean>("try_restore_session")
        if (restored) {
          setView("chat")
        } else {
          const has = await invoke<boolean>("has_providers")
          setView(has ? "chat" : "setup")
        }
      } catch (e) {
        logger.error("app", "App::init", "Failed to restore session", e)
        setView("setup")
      }
    })()
  }, [])

  async function handleCodexAuth() {
    await invoke("start_codex_auth")

    const poll = async (): Promise<void> => {
      for (let i = 0; i < 300; i++) {
        await new Promise((r) => setTimeout(r, 1000))
        const status = await invoke<{
          authenticated: boolean
          error: string | null
        }>("check_auth_status")
        if (status.authenticated) {
          await invoke("finalize_codex_auth")
          setView("chat")
          return
        }
        if (status.error) {
          throw new Error(status.error)
        }
      }
      throw new Error("Login timed out")
    }

    await poll()
  }

  if (view === "loading") {
    return (
      <div className="flex items-center justify-center h-screen">
        <div className="animate-spin h-6 w-6 border-2 border-foreground border-t-transparent rounded-full" />
      </div>
    )
  }

  if (view === "setup") {
    return (
      <div className="h-screen overflow-hidden">
        <ProviderSetup
          onComplete={() => setView("chat")}
          onCodexAuth={handleCodexAuth}
        />
      </div>
    )
  }

  return (
    <div className="flex h-screen overflow-hidden bg-background">
      <IconSidebar
        view={view === "settings" ? "settings" : view === "skills" ? "skills" : view === "profile" ? "profile" : view === "agents" ? "agents" : view === "calendar" ? "calendar" : "chat"}
        onOpenSettings={() => setView("settings")}
        onOpenChat={() => setView("chat")}
        onOpenAgents={() => { setAgentIdForSettings(undefined); setView("agents") }}
        onOpenSkills={() => setView("skills")}
        onOpenProfile={() => { setView("profile") }}
        onOpenCalendar={() => setView("calendar")}
        userAvatar={userAvatar}
      />
      {view === "settings" ? (
        <SettingsView
          onBack={() => setView("chat")}
          onCodexAuth={handleCodexAuth}
          onCodexReauth={handleCodexAuth}
        />
      ) : view === "skills" ? (
        <SettingsView
          onBack={() => setView("chat")}
          onCodexAuth={handleCodexAuth}
          onCodexReauth={handleCodexAuth}
          initialSection="skills"
        />
      ) : view === "profile" ? (
        <SettingsView
          onBack={() => setView("chat")}
          onCodexAuth={handleCodexAuth}
          onCodexReauth={handleCodexAuth}
          initialSection="profile"
        />
      ) : view === "agents" ? (
        <SettingsView
          onBack={() => { setView("chat"); setAgentIdForSettings(undefined) }}
          onCodexAuth={handleCodexAuth}
          onCodexReauth={handleCodexAuth}
          initialSection="agents"
          initialAgentId={agentIdForSettings}
        />
      ) : view === "calendar" ? (
        <CronCalendarView onBack={() => setView("chat")} />
      ) : (
        <ChatScreen onOpenAgentSettings={(agentId) => { setAgentIdForSettings(agentId); setView("agents") }} />
      )}
    </div>
  )
}
