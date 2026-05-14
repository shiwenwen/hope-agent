import { StrictMode } from "react"
import { createRoot } from "react-dom/client"
import "./index.css"
import "./i18n/i18n"
import App from "./App.tsx"
import QuickChatWindow from "./QuickChatWindow.tsx"
import PlanDetachedWindow from "./PlanDetachedWindow.tsx"
import { logger } from "./lib/logger"
import { captureTokenFromUrl } from "./lib/api-key-storage"

// Pull `?token=XXX` out of the URL into localStorage before the transport
// singleton is constructed. Standalone Web GUI mode (Docker / reverse
// proxy without auth header injection) bootstraps the Bearer token this
// way — see `src/lib/api-key-storage.ts`.
captureTokenFromUrl()

// Flush buffered logs before page unload to prevent data loss
window.addEventListener("beforeunload", () => {
  logger.flush()
})

const windowType = new URLSearchParams(window.location.search).get("window")

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    {windowType === "quickchat" ? (
      <QuickChatWindow />
    ) : windowType === "plan" ? (
      <PlanDetachedWindow />
    ) : (
      <App />
    )}
  </StrictMode>,
)
