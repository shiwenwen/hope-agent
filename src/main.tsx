import { StrictMode } from "react"
import { createRoot } from "react-dom/client"
import "./index.css"
import { i18nReady } from "./i18n/i18n"
import App from "./App.tsx"
import QuickChatWindow from "./QuickChatWindow.tsx"
import PlanDetachedWindow from "./PlanDetachedWindow.tsx"
import FileBrowserDetachedWindow from "./FileBrowserDetachedWindow.tsx"
import { logger } from "./lib/logger"
import { AuthGate } from "./components/AuthGate"
import { discardTokenFromUrl } from "./lib/api-key-storage"
import { installDesktopContextMenuGuard } from "./lib/contextMenuGuard"
import { installInvertedClickRecovery } from "./lib/inverted-click-recovery"
import { installFocusVisibilityTracker } from "./lib/focus-visibility"
import {
  listenEnhancedFocusIndicators,
  loadEnhancedFocusIndicators,
} from "./lib/focus-indicator-preference"

discardTokenFromUrl()
installDesktopContextMenuGuard()
installInvertedClickRecovery()
installFocusVisibilityTracker()

// Flush buffered logs before page unload to prevent data loss
window.addEventListener("beforeunload", () => {
  logger.flush()
})

const windowType = new URLSearchParams(window.location.search).get("window")

// The shared base stylesheet paints `body` with the application background.
// A transparent native PetWindow must clear that surface before the async i18n
// bootstrap and React's first render, otherwise the whole WebView appears as a
// white rectangle behind the sprite. Keep the override in the static Tailwind
// utility system so the renderer does not mutate presentation styles directly.
if (windowType === "pet") {
  for (const element of [
    document.documentElement,
    document.body,
    document.getElementById("root"),
  ]) {
    element?.classList.add("bg-transparent")
  }
}

// 首屏前等初始语言 bundle 就位再渲染，避免非英语用户冷启动闪一帧英文（懒加载只
// await 当前一种 locale，毫秒级本地资源）。i18nReady 内部已 try/catch，chunk 失败
// 也会 resolve（回退 en），渲染绝不会被卡死。
void i18nReady.finally(async () => {
  await loadEnhancedFocusIndicators()
  listenEnhancedFocusIndicators()

  // Dynamic import keeps the Help surface out of the main chunk (same
  // pattern as the DEV smoke windows below).
  const HelpWindow = windowType === "help" ? (await import("./HelpWindow.tsx")).default : null
  const PetWindow = windowType === "pet" ? (await import("./PetWindow.tsx")).default : null
  const SpaceDetachedWindow =
    windowType === "space" ? (await import("./SpaceDetachedWindow.tsx")).default : null

  const WorkflowSmokeWindow =
    windowType === "workflow-smoke" && import.meta.env.DEV
      ? (await import("./dev/WorkflowSmokeWindow.tsx")).default
      : null
  const LoopSmokeWindow =
    windowType === "loop-smoke" && import.meta.env.DEV
      ? (await import("./dev/LoopSmokeWindow.tsx")).default
      : null
  const GoalSmokeWindow =
    windowType === "goal-smoke" && import.meta.env.DEV
      ? (await import("./dev/GoalSmokeWindow.tsx")).default
      : null
  const WorkspaceSmokeWindow =
    windowType === "workspace-smoke" && import.meta.env.DEV
      ? (await import("./dev/WorkspaceSmokeWindow.tsx")).default
      : null
  const ChatInputSmokeWindow =
    windowType === "chat-input-smoke" && import.meta.env.DEV
      ? (await import("./dev/ChatInputSmokeWindow.tsx")).default
      : null

  createRoot(document.getElementById("root")!).render(
    <StrictMode>
      <AuthGate>
        {windowType === "quickchat" ? (
          <QuickChatWindow />
        ) : PetWindow ? (
          <PetWindow />
        ) : windowType === "plan" ? (
          <PlanDetachedWindow />
        ) : windowType === "files" ? (
          <FileBrowserDetachedWindow />
        ) : SpaceDetachedWindow ? (
          <SpaceDetachedWindow />
        ) : HelpWindow ? (
          <HelpWindow />
        ) : WorkflowSmokeWindow ? (
          <WorkflowSmokeWindow />
        ) : LoopSmokeWindow ? (
          <LoopSmokeWindow />
        ) : GoalSmokeWindow ? (
          <GoalSmokeWindow />
        ) : WorkspaceSmokeWindow ? (
          <WorkspaceSmokeWindow />
        ) : ChatInputSmokeWindow ? (
          <ChatInputSmokeWindow />
        ) : (
          <App />
        )}
      </AuthGate>
    </StrictMode>,
  )
})
