import { AlertCircle, AlertTriangle, Check, Loader2 } from "lucide-react"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"

export default function TelegramConnectionFields({
  token,
  apiRoot,
  validating,
  validationResult,
  validationError,
  onTokenChange,
  onApiRootChange,
  onValidate,
}: {
  token: string
  apiRoot: string
  validating: boolean
  validationResult: string | null
  validationError: string | null
  onTokenChange: (value: string) => void
  onApiRootChange: (value: string) => void
  onValidate: () => void
}) {
  const { t } = useTranslation()

  return (
    <div className="space-y-4">
      <div className="space-y-2">
        <Label>{t("channels.botToken")}</Label>
        <div className="flex gap-2">
          <Input
            type="password"
            placeholder="123456:ABC-DEF..."
            value={token}
            onChange={(event) => onTokenChange(event.target.value)}
            className="flex-1"
          />
          <Button
            variant="outline"
            size="sm"
            onClick={onValidate}
            disabled={!token.trim() || validating}
            className="shrink-0"
          >
            {validating ? (
              <Loader2 className="h-4 w-4 animate-spin" />
            ) : (
              t("channels.testConnection")
            )}
          </Button>
        </div>
        <p className="text-xs text-muted-foreground">{t("channels.telegramTokenHint")}</p>
      </div>

      <div className="space-y-2">
        <Label>{t("channels.telegramApiRoot")}</Label>
        <Input
          type="url"
          inputMode="url"
          placeholder="https://api.telegram.org"
          value={apiRoot}
          onChange={(event) => onApiRootChange(event.target.value)}
        />
        <p className="text-xs text-muted-foreground">{t("channels.telegramApiRootHint")}</p>
        {apiRoot.trim() && (
          <div className="flex items-start gap-1.5 text-xs text-amber-600 dark:text-amber-400">
            <AlertTriangle className="mt-0.5 h-3.5 w-3.5 shrink-0" />
            <span>{t("channels.telegramApiRootSecurityHint")}</span>
          </div>
        )}
      </div>

      {validationResult && (
        <div className="flex items-center gap-1 text-sm text-green-600">
          <Check className="h-3.5 w-3.5" />
          {validationResult}
        </div>
      )}
      {validationError && (
        <div className="flex items-start gap-1 text-sm text-destructive">
          <AlertCircle className="mt-0.5 h-3.5 w-3.5 shrink-0" />
          <span className="break-all">{validationError}</span>
        </div>
      )}
    </div>
  )
}
