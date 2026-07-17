import type { TFunction } from "i18next"

export function knowledgeMaintenanceSkipReasonLabel(t: TFunction, note: string): string {
  switch (note) {
    case "already running":
      return t("knowledge.maintenance.skipReasons.alreadyRunning")
    case "manual disabled":
      return t("knowledge.maintenance.skipReasons.manualDisabled")
    case "disabled":
      return t("knowledge.maintenance.skipReasons.disabled")
    case "knowledge db not initialized":
      return t("knowledge.maintenance.skipReasons.databaseNotInitialized")
    default: {
      const listError = note.match(/^list kbs failed:\s*(.+)$/)
      if (listError) {
        return t("knowledge.maintenance.skipReasons.listFailed", { error: listError[1] })
      }
      // The backend note contract is open-ended. Preserve a future diagnostic
      // verbatim instead of replacing it with an inaccurate generic reason.
      return note
    }
  }
}
