import { Loader2 } from "lucide-react"
import { useTranslation } from "react-i18next"

/** Spinner shown while an office file's bytes are fetched / its renderer loads. */
export function OfficeLoading() {
  const { t } = useTranslation()
  return (
    <div className="flex items-center justify-center gap-2 py-12 text-sm text-muted-foreground">
      <Loader2 className="h-4 w-4 animate-spin" />
      {t("fileBrowser.officeLoading", "Rendering preview…")}
    </div>
  )
}
