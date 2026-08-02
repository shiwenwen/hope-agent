import { type FormEvent, type ReactNode, useCallback, useEffect, useState } from "react"
import { AlertCircle, KeyRound, Loader2, ShieldCheck } from "lucide-react"
import { useTranslation } from "react-i18next"

import logoUrl from "@/assets/logo.png"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import {
  AUTH_REQUIRED_EVENT,
  clearStoredApiKey,
  getStoredApiKey,
} from "@/lib/api-key-storage"
import { isTauriMode } from "@/lib/transport"
import {
  authenticateWebOwnerToken,
  configuredHttpBase,
  isConfiguredHttpBaseSameOrigin,
} from "@/lib/transport-provider"

type GateState = "checking" | "ready" | "login" | "unavailable"

interface AuthStatus {
  authRequired: boolean
  authenticated: boolean
}

export function AuthGate({ children }: { children: ReactNode }) {
  const { t } = useTranslation()
  const [state, setState] = useState<GateState>(isTauriMode() ? "ready" : "checking")
  const [token, setToken] = useState("")
  const [error, setError] = useState<string | null>(null)
  const [submitting, setSubmitting] = useState(false)

  const exchangeToken = useCallback(
    (candidate: string) => authenticateWebOwnerToken(candidate),
    [],
  )

  const check = useCallback(async () => {
    if (isTauriMode()) {
      setState("ready")
      return
    }
    setState("checking")
    setError(null)
    try {
      const response = await fetch(`${configuredHttpBase()}/api/auth/status`, {
        credentials: isConfiguredHttpBaseSameOrigin() ? "same-origin" : "omit",
        cache: "no-store",
      })
      if (!response.ok) throw new Error(`status ${response.status}`)
      const status = (await response.json()) as AuthStatus
      if (!status.authRequired || status.authenticated) {
        clearStoredApiKey()
        setState("ready")
        return
      }

      // One-release migration for installations that used the legacy
      // localStorage Bearer token. Exchange it once, then erase it whether
      // accepted or stale so the long-lived secret cannot linger in JS state.
      const legacyToken = getStoredApiKey()
      if (legacyToken) {
        const migrated = await exchangeToken(legacyToken).catch(() => false)
        clearStoredApiKey()
        if (migrated) {
          setState("ready")
          return
        }
      }
      setState("login")
    } catch {
      setState("unavailable")
    }
  }, [exchangeToken])

  useEffect(() => {
    void check()
  }, [check])

  useEffect(() => {
    if (isTauriMode()) return
    const requireAuth = () => {
      setToken("")
      setError(null)
      setState("login")
    }
    window.addEventListener(AUTH_REQUIRED_EVENT, requireAuth)
    return () => window.removeEventListener(AUTH_REQUIRED_EVENT, requireAuth)
  }, [])

  const submit = async (event: FormEvent) => {
    event.preventDefault()
    const candidate = token.replace(/[\r\n]+$/, "")
    if (!candidate) return
    setSubmitting(true)
    setError(null)
    try {
      if (await exchangeToken(candidate)) {
        setToken("")
        clearStoredApiKey()
        setState("ready")
      } else {
        setError(t("auth.tokenInvalid"))
      }
    } catch {
      setError(t("auth.serverUnavailable"))
    } finally {
      setSubmitting(false)
    }
  }

  if (state === "ready") return children

  return (
    <main className="flex min-h-screen items-center justify-center bg-background p-4">
      <section className="w-full max-w-md rounded-xl border bg-card text-card-foreground shadow-lg">
        <div className="flex flex-col items-center space-y-1.5 p-6 text-center">
          <img src={logoUrl} alt="Hope Agent" className="mb-2 size-14 rounded-xl" />
          <div className="mb-1 flex size-10 items-center justify-center rounded-full bg-primary/10 text-primary">
            {state === "login" ? <KeyRound className="size-5" /> : <ShieldCheck className="size-5" />}
          </div>
          <h1 className="text-2xl font-semibold leading-none tracking-tight">
            {t("auth.tokenRequiredTitle")}
          </h1>
          <p className="text-sm text-muted-foreground">{t("auth.tokenRequiredBody")}</p>
        </div>
        <div className="p-6 pt-0">
          {state === "checking" ? (
            <div className="flex items-center justify-center gap-2 py-6 text-sm text-muted-foreground">
              <Loader2 className="size-4 animate-spin" />
              {t("auth.checking")}
            </div>
          ) : state === "unavailable" ? (
            <div className="space-y-4">
              <div className="flex gap-2 rounded-lg bg-destructive/10 p-3 text-sm text-destructive">
                <AlertCircle className="mt-0.5 size-4 shrink-0" />
                <span>{t("auth.serverUnavailable")}</span>
              </div>
              <Button className="w-full" onClick={() => void check()}>
                {t("auth.retry")}
              </Button>
            </div>
          ) : (
            <form className="space-y-4" onSubmit={submit}>
              <div className="space-y-2">
                <Label htmlFor="ha-owner-token">{t("auth.tokenLabel")}</Label>
                <Input
                  id="ha-owner-token"
                  type="password"
                  autoComplete="current-password"
                  autoCapitalize="none"
                  autoCorrect="off"
                  spellCheck={false}
                  value={token}
                  placeholder={t("auth.tokenPlaceholder")}
                  onChange={(event) => setToken(event.target.value)}
                  autoFocus
                />
              </div>
              {error ? (
                <div className="flex gap-2 rounded-lg bg-destructive/10 p-3 text-sm text-destructive">
                  <AlertCircle className="mt-0.5 size-4 shrink-0" />
                  <span>{error}</span>
                </div>
              ) : null}
              <Button className="w-full" type="submit" disabled={!token || submitting}>
                {submitting ? <Loader2 className="size-4 animate-spin" /> : null}
                {t("auth.tokenContinue")}
              </Button>
              <p className="text-center text-xs text-muted-foreground">
                {t("auth.sessionSecurityHint")}
              </p>
            </form>
          )}
        </div>
      </section>
    </main>
  )
}
