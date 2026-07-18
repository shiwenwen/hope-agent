// Shared data hook for the media-generation settings surfaces: loads the
// full config (providers + chains + defaults), the vendor templates, and
// exposes provider CRUD helpers (immediate saves — no dirty state) plus a
// reload for the chain/defaults consumers.

import { useCallback, useEffect, useMemo, useState } from "react"
import { getTransport } from "@/lib/transport-provider"
import { logger } from "@/lib/logger"
import type {
  MediaFunctionKey,
  MediaGenConfigView,
  MediaGenOverview,
  MediaProviderConfig,
  MediaProviderTemplate,
} from "./types"
import { toAvailableModels } from "./types"

export function useMediaGenData() {
  const [config, setConfig] = useState<MediaGenConfigView | null>(null)
  const [templates, setTemplates] = useState<MediaProviderTemplate[]>([])
  const [loading, setLoading] = useState(true)

  const reload = useCallback(async () => {
    try {
      const cfg = await getTransport().call<MediaGenConfigView>("get_media_gen_config")
      setConfig(cfg)
    } catch (e) {
      logger.error("settings", "useMediaGenData", `load config failed: ${e}`)
      // Tolerate an empty/new backend: fall to an empty default view.
      setConfig({
        providers: [],
        chains: {},
        imageDefaults: {
          enabled: true,
          timeoutSeconds: 180,
          defaultSize: "1024x1024",
        },
        audioDefaults: { enabled: true, timeoutSeconds: 300 },
      })
    } finally {
      setLoading(false)
    }
  }, [])

  useEffect(() => {
    void reload()
    getTransport()
      .call<MediaProviderTemplate[]>("get_media_provider_templates")
      .then(setTemplates)
      .catch((e) =>
        logger.error("settings", "useMediaGenData", `load templates failed: ${e}`),
      )
  }, [reload])

  const addProvider = useCallback(
    async (provider: MediaProviderConfig) => {
      const stored = await getTransport().call<MediaProviderConfig>("add_media_provider", {
        provider,
      })
      await reload()
      return stored
    },
    [reload],
  )

  const updateProvider = useCallback(
    async (provider: MediaProviderConfig) => {
      await getTransport().call("update_media_provider", {
        providerId: provider.id,
        provider,
      })
      await reload()
    },
    [reload],
  )

  const deleteProvider = useCallback(
    async (providerId: string) => {
      const chainsTouched = await getTransport().call<boolean | { chainsTouched: boolean }>(
        "delete_media_provider",
        { providerId },
      )
      await reload()
      return typeof chainsTouched === "boolean"
        ? chainsTouched
        : Boolean(chainsTouched?.chainsTouched)
    },
    [reload],
  )

  const reorderProviders = useCallback(
    async (providerIds: string[]) => {
      // Optimistic: reorder locally first so dnd doesn't snap back.
      setConfig((prev) => {
        if (!prev) return prev
        const byId = new Map(prev.providers.map((p) => [p.id, p]))
        const next = providerIds
          .map((id) => byId.get(id))
          .filter((p): p is MediaProviderConfig => Boolean(p))
        for (const p of prev.providers) {
          if (!providerIds.includes(p.id)) next.push(p)
        }
        return { ...prev, providers: next }
      })
      await getTransport().call("reorder_media_providers", { providerIds })
      await reload()
    },
    [reload],
  )

  return {
    config,
    templates,
    loading,
    reload,
    addProvider,
    updateProvider,
    deleteProvider,
    reorderProviders,
  }
}

/** Chain-editor data source for one function key, memoized. */
export function useAvailableMediaModels(
  config: MediaGenConfigView | null,
  fn: MediaFunctionKey,
) {
  return useMemo(
    () => (config ? toAvailableModels(config.providers, fn) : []),
    [config, fn],
  )
}

/** Fetch the sanitized overview (design dialogs / chain hints). */
export async function fetchMediaGenOverview(): Promise<MediaGenOverview | null> {
  try {
    return await getTransport().call<MediaGenOverview>("get_media_gen_overview")
  } catch (e) {
    logger.error("settings", "useMediaGenData", `load overview failed: ${e}`)
    return null
  }
}
