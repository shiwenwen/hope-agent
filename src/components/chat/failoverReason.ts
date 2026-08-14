const REASON_KEYS: Record<string, string> = {
  rate_limit: "reasonRateLimit",
  overloaded: "reasonOverloaded",
  timeout: "reasonTimeout",
  auth: "reasonAuth",
  billing: "reasonBilling",
  model_not_found: "reasonModelNotFound",
  context_overflow: "reasonContextOverflow",
  current_tool_group_overflow: "reasonContextOverflow",
  dispatch_unknown: "reasonDispatchUnknown",
  unknown: "reasonUnknown",
}

export function failoverReasonKey(reason?: string): string {
  return REASON_KEYS[reason ?? ""] ?? REASON_KEYS.unknown
}
