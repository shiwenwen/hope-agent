export type ApprovalReasonKind =
  | "edit_tool"
  | "edit_command"
  | "dangerous_command"
  | "protected_path"
  | "agent_custom_list"
  | "smart_judge"
  | "browser_evaluate"
  | "browser_raw_cdp"
  | "browser_chrome_access"
  | "browser_download_action"
  | "mac_control_action"
  | "mac_control_dangerous_action"
  | "external_connector_action"
  | "plan_mode_ask"
  | "cron_delete"

/** Mirrors backend `ApprovalReasonKind::is_strict()`. */
export function isStrictApprovalReason(kind: ApprovalReasonKind | undefined): boolean {
  return (
    kind === "protected_path" ||
    kind === "dangerous_command" ||
    kind === "browser_raw_cdp" ||
    kind === "mac_control_dangerous_action" ||
    kind === "external_connector_action" ||
    kind === "plan_mode_ask"
  )
}

/** Strict reasons and cron deletion may never create a standing grant. */
export function approvalBarsAllowAlways(kind: ApprovalReasonKind | undefined): boolean {
  return isStrictApprovalReason(kind) || kind === "cron_delete"
}
