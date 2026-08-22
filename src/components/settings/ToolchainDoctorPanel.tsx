import { useCallback, useEffect, useMemo, useState } from "react"
import { useTranslation } from "react-i18next"
import { AlertTriangle, CheckCircle2, CircleDot, Loader2, RefreshCw, ShieldX } from "lucide-react"
import { Button } from "@/components/ui/button"
import { sanitizeDiagnosticText } from "@/lib/diagnosticRedaction"
import { getTransport } from "@/lib/transport-provider"
import type {
  ToolchainDoctorCheck,
  ToolchainDoctorReport,
  ToolchainDoctorStatus,
} from "@/lib/transport"

const STATUS_CLASS: Record<ToolchainDoctorStatus, string> = {
  detected: "border-sky-500/20 bg-sky-500/10 text-sky-700 dark:text-sky-300",
  supported: "border-emerald-500/20 bg-emerald-500/10 text-emerald-700 dark:text-emerald-300",
  degraded: "border-amber-500/20 bg-amber-500/10 text-amber-700 dark:text-amber-300",
  blocked: "border-destructive/20 bg-destructive/10 text-destructive",
}

const COMPONENT_LABELS: Record<string, string> = {
  operating_system: "Operating system",
  docker: "Docker",
  chrome: "Chrome / Chromium",
  ffmpeg: "FFmpeg",
  github_cli: "GitHub CLI",
  ollama: "Ollama",
  python: "Python",
  rust_analyzer: "rust-analyzer",
  typescript_language_server: "TypeScript Language Server",
  clangd: "clangd",
  libreoffice: "LibreOffice",
  poppler: "Poppler",
}

function StatusIcon({ status }: { status: ToolchainDoctorStatus }) {
  if (status === "supported") return <CheckCircle2 className="h-3.5 w-3.5" />
  if (status === "degraded") return <AlertTriangle className="h-3.5 w-3.5" />
  if (status === "blocked") return <ShieldX className="h-3.5 w-3.5" />
  return <CircleDot className="h-3.5 w-3.5" />
}

function describeError(error: unknown): string {
  if (error instanceof Error) return sanitizeDiagnosticText(error.message)
  if (typeof error === "string") return sanitizeDiagnosticText(error)
  return sanitizeDiagnosticText(String(error))
}

export default function ToolchainDoctorPanel() {
  const { t } = useTranslation()
  const [report, setReport] = useState<ToolchainDoctorReport | null>(null)
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)

  const refresh = useCallback(async () => {
    setLoading(true)
    setError(null)
    try {
      const next = await getTransport().call<ToolchainDoctorReport>(
        "get_toolchain_doctor_report",
        {},
      )
      setReport(next)
    } catch (cause) {
      setError(describeError(cause))
    } finally {
      setLoading(false)
    }
  }, [])

  useEffect(() => {
    void refresh()
  }, [refresh])

  const checks = useMemo(
    () => report?.checks.slice().sort((left, right) => left.id.localeCompare(right.id)) ?? [],
    [report],
  )

  return (
    <section className="rounded-[24px] border border-border/70 bg-card px-6 py-5">
      <div className="flex flex-wrap items-start justify-between gap-4">
        <div>
          <h3 className="text-base font-semibold tracking-tight text-foreground">
            {t("toolchainDoctor.title")}
          </h3>
          <p className="mt-1 max-w-3xl text-sm leading-6 text-muted-foreground">
            {t("toolchainDoctor.description")}
          </p>
        </div>
        <Button variant="outline" size="sm" onClick={() => void refresh()} disabled={loading}>
          {loading ? (
            <Loader2 className="mr-1.5 h-3.5 w-3.5 animate-spin" />
          ) : (
            <RefreshCw className="mr-1.5 h-3.5 w-3.5" />
          )}
          {t("toolchainDoctor.refresh")}
        </Button>
      </div>

      {report && (
        <div className="mt-4 flex flex-wrap gap-2 text-xs">
          {(["supported", "detected", "degraded", "blocked"] as const).map((status) => (
            <span
              key={status}
              className={`inline-flex items-center gap-1.5 rounded-full border px-2.5 py-1 ${STATUS_CLASS[status]}`}
            >
              <StatusIcon status={status} />
              {t(`toolchainDoctor.status.${status}`)} · {report.summary[status]}
            </span>
          ))}
        </div>
      )}

      {error && (
        <div className="mt-4 rounded-xl border border-destructive/20 bg-destructive/5 px-4 py-3 text-sm text-destructive">
          <p className="font-medium">{t("toolchainDoctor.loadFailed")}</p>
          <p className="mt-1 break-all text-xs text-muted-foreground">{error}</p>
        </div>
      )}

      {loading && !report ? (
        <div className="mt-5 flex items-center gap-2 text-sm text-muted-foreground">
          <Loader2 className="h-4 w-4 animate-spin" />
          {t("toolchainDoctor.loading")}
        </div>
      ) : (
        <div className="mt-5 grid gap-3 md:grid-cols-2">
          {checks.map((check) => (
            <DoctorCheckCard key={check.id} check={check} />
          ))}
        </div>
      )}

      <p className="mt-4 text-xs leading-5 text-muted-foreground">
        {t("toolchainDoctor.readOnlyNotice")}
      </p>
    </section>
  )
}

function DoctorCheckCard({ check }: { check: ToolchainDoctorCheck }) {
  const { t } = useTranslation()
  const related = Object.entries(check.relatedVersions ?? {})
  return (
    <div className="rounded-2xl border border-border/60 bg-secondary/20 p-4">
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          <p className="text-sm font-medium text-foreground">
            {COMPONENT_LABELS[check.id] ?? check.id}
          </p>
          {(check.detectedVersion || check.minimumVersion) && (
            <p className="mt-1 text-xs text-muted-foreground">
              {check.detectedVersion
                ? t("toolchainDoctor.detectedVersion", { version: check.detectedVersion })
                : t("toolchainDoctor.notDetected")}
              {check.minimumVersion
                ? ` · ${t("toolchainDoctor.minimumVersion", { version: check.minimumVersion })}`
                : ""}
            </p>
          )}
        </div>
        <span
          className={`inline-flex shrink-0 items-center gap-1.5 rounded-full border px-2.5 py-1 text-xs ${STATUS_CLASS[check.status]}`}
        >
          <StatusIcon status={check.status} />
          {t(`toolchainDoctor.status.${check.status}`)}
        </span>
      </div>
      {related.length > 0 && (
        <div className="mt-2 flex flex-wrap gap-1.5">
          {related.map(([name, version]) => (
            <span
              key={name}
              className="rounded-md border border-border/60 bg-background/60 px-2 py-0.5 font-mono text-[11px] text-muted-foreground"
            >
              {name} {version}
            </span>
          ))}
        </div>
      )}
      {(check.facts?.length ?? 0) > 0 && (
        <div className="mt-2 text-[11px] leading-5 text-muted-foreground">
          <span>{t("toolchainDoctor.diagnosticFacts")}: </span>
          <span className="font-mono">{check.facts?.join(" · ")}</span>
        </div>
      )}
    </div>
  )
}
