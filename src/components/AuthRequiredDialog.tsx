/**
 * Modal that lets the user paste a Bearer token after the server rejects
 * the one we currently have. Listens for `ha:auth-required` events
 * dispatched by `HttpTransport` on a 401 response (see
 * `src/lib/transport-http.ts`).
 *
 * Only renders when running outside Tauri — the desktop app has its own
 * native flows for managing auth and never gets 401 from its embedded
 * server.
 *
 * Flow:
 * 1. HttpTransport gets 401 → dispatches AUTH_REQUIRED_EVENT.
 * 2. This component opens the dialog.
 * 3. User pastes token → click Save → token written to localStorage.
 * 4. Page reload picks up the new token through `getTransport()`.
 *
 * Page reload is the simplest path to a clean state — the WebSocket,
 * any in-flight fetches, and React's transport singleton all need to
 * be torn down and rebuilt after the key changes.
 */

import { useEffect, useState } from "react"
import { useTranslation } from "react-i18next"

import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { isTauriMode } from "@/lib/transport"
import {
  AUTH_REQUIRED_EVENT,
  consumeAuthRequiredSticky,
  setStoredApiKey,
} from "@/lib/api-key-storage"

export function AuthRequiredDialog() {
  const { t } = useTranslation()
  const [open, setOpen] = useState(false)
  const [token, setToken] = useState("")

  useEffect(() => {
    // Tauri desktop never produces 401 from its embedded server (the key
    // is loaded out of native config before requests fly), so don't even
    // attach the listener — it would just be dead weight.
    if (isTauriMode()) return
    const handler = () => {
      setToken("")
      setOpen(true)
    }
    window.addEventListener(AUTH_REQUIRED_EVENT, handler)
    // First-boot races: the very first protected API call often fails
    // with 401 before this component has mounted (the boot effect runs
    // while the loading splash is showing). `dispatchAuthRequired`
    // raises a sticky flag for exactly that case — replay the event
    // so the listener we just attached picks it up via the same path
    // as a fresh 401 (keeps state-update flow out of the effect body
    // and react-hooks/set-state-in-effect happy).
    if (consumeAuthRequiredSticky()) {
      window.dispatchEvent(new CustomEvent(AUTH_REQUIRED_EVENT))
    }
    return () => window.removeEventListener(AUTH_REQUIRED_EVENT, handler)
  }, [])

  const submit = () => {
    const trimmed = token.trim()
    if (!trimmed) return
    setStoredApiKey(trimmed)
    setOpen(false)
    // Reload is intentional — rebuilds the transport singleton, the
    // event WebSocket, and any in-flight component state so the new
    // key replaces the old one everywhere with no half-state.
    window.location.reload()
  }

  return (
    <Dialog open={open} onOpenChange={setOpen}>
      <DialogContent className="max-w-md">
        <DialogHeader>
          <DialogTitle>{t("auth.tokenRequiredTitle")}</DialogTitle>
          <DialogDescription>{t("auth.tokenRequiredBody")}</DialogDescription>
        </DialogHeader>
        <div className="grid gap-2 py-2">
          <Label htmlFor="ha-auth-token">{t("auth.tokenLabel")}</Label>
          <Input
            id="ha-auth-token"
            type="password"
            value={token}
            placeholder={t("auth.tokenPlaceholder")}
            onChange={(e) => setToken(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") submit()
            }}
            autoFocus
          />
        </div>
        <DialogFooter>
          <Button variant="outline" onClick={() => setOpen(false)}>
            {t("common.cancel")}
          </Button>
          <Button onClick={submit} disabled={!token.trim()}>
            {t("auth.tokenSave")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
