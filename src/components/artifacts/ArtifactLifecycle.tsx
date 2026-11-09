import { useTranslation } from "react-i18next"
import type { ArtifactRecord } from "@/lib/transport"

/** Lifecycle identity comes from durable versions, not the current chat turn. */
export function ArtifactLifecycle({
  artifact,
}: {
  artifact: Pick<ArtifactRecord, "currentVersion" | "createdAt" | "updatedAt">
}) {
  const { t } = useTranslation()
  const updated = artifact.currentVersion > 1
  const timestamp = updated ? artifact.updatedAt : artifact.createdAt
  return (
    <span className="inline-flex flex-wrap items-center gap-1.5 text-xs text-muted-foreground">
      <span className="rounded-md bg-muted px-1.5 py-0.5 font-medium">
        {t(
          updated ? "artifacts.registration.updatedVersion" : "artifacts.registration.firstVersion",
        )}
      </span>
      <span>
        {t(updated ? "artifacts.registration.updatedAt" : "artifacts.registration.createdAt")}
      </span>
      <time
        dateTime={timestamp}
        data-ha-title-tip={`${t("artifacts.registration.createdAt")} ${new Date(artifact.createdAt).toLocaleString()} · ${t("artifacts.registration.updatedAt")} ${new Date(artifact.updatedAt).toLocaleString()}`}
      >
        {new Date(timestamp).toLocaleDateString()}
      </time>
    </span>
  )
}
