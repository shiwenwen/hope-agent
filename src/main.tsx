import { StrictMode } from "react"
import { createRoot } from "react-dom/client"
import "./index.css"
import { i18nReady } from "./i18n/i18n"
import App from "./App.tsx"
import QuickChatWindow from "./QuickChatWindow.tsx"
import PlanDetachedWindow from "./PlanDetachedWindow.tsx"
import FileBrowserDetachedWindow from "./FileBrowserDetachedWindow.tsx"
import { logger } from "./lib/logger"
import { captureTokenFromUrl } from "./lib/api-key-storage"
import { installDesktopContextMenuGuard } from "./lib/contextMenuGuard"
import { installInvertedClickRecovery } from "./lib/inverted-click-recovery"

// Pull `?token=XXX` out of the URL into localStorage before the transport
// singleton is constructed. Standalone Web GUI mode (Docker / reverse
// proxy without auth header injection) bootstraps the Bearer token this
// way — see `src/lib/api-key-storage.ts`.
captureTokenFromUrl()
installDesktopContextMenuGuard()
installInvertedClickRecovery()

// Flush buffered logs before page unload to prevent data loss
window.addEventListener("beforeunload", () => {
  logger.flush()
})

const windowType = new URLSearchParams(window.location.search).get("window")

// 首屏前等初始语言 bundle 就位再渲染，避免非英语用户冷启动闪一帧英文（懒加载只
// await 当前一种 locale，毫秒级本地资源）。i18nReady 内部已 try/catch，chunk 失败
// 也会 resolve（回退 en），渲染绝不会被卡死。
void i18nReady.finally(async () => {
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
      {windowType === "quickchat" ? (
        <QuickChatWindow />
      ) : windowType === "plan" ? (
        <PlanDetachedWindow />
      ) : windowType === "files" ? (
        <FileBrowserDetachedWindow />
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
    </StrictMode>,
  )
})
