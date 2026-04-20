import type { TFunction } from "i18next"
import type { ChannelAccountConfig, WeChatConnection } from "./types"

// Must match DUPLICATE_CREDENTIAL_ERROR_PREFIX in
// crates/ha-core/src/channel/accounts.rs.
export const DUPLICATE_CREDENTIAL_ERROR_PREFIX = "DUPLICATE_CREDENTIAL"

export function parseChannelSaveError(e: unknown, t: TFunction): string {
  const raw = String(e)
  const marker = `${DUPLICATE_CREDENTIAL_ERROR_PREFIX}:`
  const idx = raw.indexOf(marker)
  if (idx === -1) return raw
  const label = raw.slice(idx + marker.length).trim()
  return t("channels.duplicateCredential", { label })
}

export function formatUptime(secs: number): string {
  if (secs < 60) return `${secs}s`
  if (secs < 3600) return `${Math.floor(secs / 60)}m`
  if (secs < 86400) return `${Math.floor(secs / 3600)}h ${Math.floor((secs % 3600) / 60)}m`
  return `${Math.floor(secs / 86400)}d ${Math.floor((secs % 86400) / 3600)}h`
}

export function getWeChatConnectionFromAccount(account: ChannelAccountConfig | null): WeChatConnection | null {
  if (!account || account.channelId !== "wechat") return null

  const credentials = account.credentials as Record<string, string | undefined>
  const settings = account.settings as Record<string, string | undefined>
  const botToken = credentials.token?.trim()
  const baseUrl = settings.baseUrl?.trim() || credentials.baseUrl?.trim()

  if (!botToken || !baseUrl) return null

  return {
    botToken,
    baseUrl,
    remoteAccountId: credentials.remoteAccountId ?? null,
    userId: credentials.userId ?? null,
  }
}

export function defaultWeChatLabel(connection: WeChatConnection): string {
  const identity = connection.userId?.trim() || connection.remoteAccountId?.trim()
  return identity ? `WeChat ${identity}` : "WeChat"
}
