import type { TFunction } from "i18next"

export type BackupMetadataKind = "kind" | "category" | "source"

export function backupMetadataLabel(
  t: TFunction,
  kind: BackupMetadataKind,
  value: string,
): string {
  const normalized = value.trim().toLowerCase()
  if (!normalized || normalized === "unknown") return t("common.unknown")

  if (kind === "category" && normalized.startsWith("rollback-to-")) {
    return t("configRecovery.categoryValues.rollback_to", {
      timestamp: value.slice("rollback-to-".length),
    })
  }

  const key = normalized.replace(/[^a-z0-9]+/g, "_")
  return t(`configRecovery.${kind}Values.${key}`, {
    // category/source are deliberately open config identifiers. Preserve a
    // future value exactly instead of turning it into misleading English UI.
    defaultValue: value,
  })
}
