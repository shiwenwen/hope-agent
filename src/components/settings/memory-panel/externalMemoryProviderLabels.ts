import type { TFunction } from "i18next"

import type { ExternalMemoryProviderSyncBlockReason } from "./types"

export function externalMemoryProviderSyncBlockReasonLabel(
  reason: ExternalMemoryProviderSyncBlockReason | string,
  t: TFunction,
): string {
  switch (reason) {
    case "global_disabled":
      return t("settings.memoryExternalProviderBlockGlobalDisabled", "Global sync off")
    case "provider_disabled":
      return t("settings.memoryExternalProviderBlockProviderDisabled", "Provider off")
    case "policy_off":
      return t("settings.memoryExternalProviderBlockPolicyOff", "Policy off")
    case "endpoint_missing":
      return t("settings.memoryExternalProviderBlockEndpointMissing", "Endpoint missing")
    case "policy_unsupported":
      return t("settings.memoryExternalProviderBlockPolicyUnsupported", "Policy unsupported")
    case "adapter_unavailable":
      return t("settings.memoryExternalProviderBlockAdapterUnavailable", "Adapter unavailable")
    case "last_error":
      return t("settings.memoryExternalProviderBlockLastError", "Last error")
    default:
      return reason
  }
}
