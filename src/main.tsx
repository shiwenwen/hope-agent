import { StrictMode } from "react"
import { createRoot } from "react-dom/client"
import "./index.css"
import "./i18n/i18n"
import App from "./App.tsx"
import QuickChatWindow from "./QuickChatWindow.tsx"
import PlanDetachedWindow from "./PlanDetachedWindow.tsx"

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
