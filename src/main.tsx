import { StrictMode } from "react"
import { createRoot } from "react-dom/client"
import "./index.css"
import "./i18n/i18n"
import App from "./App.tsx"
import QuickChatWindow from "./QuickChatWindow.tsx"

const isQuickChat = new URLSearchParams(window.location.search).get("window") === "quickchat"

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    {isQuickChat ? <QuickChatWindow /> : <App />}
  </StrictMode>,
)
