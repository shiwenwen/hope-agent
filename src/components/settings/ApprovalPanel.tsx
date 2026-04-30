import { useTranslation } from "react-i18next"
import GlobalYoloSection from "./approval-panel/GlobalYoloSection"
import PatternListEditor from "./approval-panel/PatternListEditor"
import SmartModeSection from "./approval-panel/SmartModeSection"
import ApprovalTimeoutSection from "./approval-panel/ApprovalTimeoutSection"

/**
 * "权限" / "Approval" settings panel — central home for the permission v2
 * system: Global YOLO toggle, three editable pattern lists, Smart mode
 * configuration, and approval-dialog timeout settings.
 *
 * Per-tool approval (per-agent) lives on the Agent settings page in
 * `agent-panel/tabs/ApprovalTab.tsx` — this panel is global only.
 */
export default function ApprovalPanel() {
  const { t } = useTranslation()

  return (
    <div className="flex-1 overflow-y-auto px-6 pb-8">
      <div className="max-w-3xl mx-auto py-4 space-y-4">
        <header>
          <h2 className="text-base font-semibold text-foreground">
            {t("settings.approvalPanel.title")}
          </h2>
          <p className="text-xs text-muted-foreground mt-1">
            {t("settings.approvalPanel.intro")}
          </p>
        </header>

        <GlobalYoloSection />

        <PatternListEditor
          title={t("settings.approvalPanel.protectedPathsTitle")}
          description={t("settings.approvalPanel.protectedPathsDesc")}
          inputPlaceholder={t("settings.approvalPanel.protectedPathsPlaceholder")}
          getCmd="get_protected_paths"
          setCmd="set_protected_paths"
          resetCmd="reset_protected_paths"
        />

        <PatternListEditor
          title={t("settings.approvalPanel.editCommandsTitle")}
          description={t("settings.approvalPanel.editCommandsDesc")}
          inputPlaceholder={t("settings.approvalPanel.editCommandsPlaceholder")}
          getCmd="get_edit_commands"
          setCmd="set_edit_commands"
          resetCmd="reset_edit_commands"
        />

        <PatternListEditor
          title={t("settings.approvalPanel.dangerousCommandsTitle")}
          description={t("settings.approvalPanel.dangerousCommandsDesc")}
          inputPlaceholder={t("settings.approvalPanel.dangerousCommandsPlaceholder")}
          getCmd="get_dangerous_commands"
          setCmd="set_dangerous_commands"
          resetCmd="reset_dangerous_commands"
        />

        <SmartModeSection />

        <ApprovalTimeoutSection />
      </div>
    </div>
  )
}
