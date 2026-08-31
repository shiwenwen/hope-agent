import { getTransport } from "@/lib/transport-provider"
import { openExternalUrl } from "@/lib/openExternalUrl"
import type { OllamaStatus } from "@/types/local-llm"
import type { LocalModelJobKind } from "@/types/local-model-jobs"

/** Check live prerequisites before creating a job, including retries of old jobs. */
export async function prepareLocalModelJob(kind: LocalModelJobKind): Promise<boolean> {
  if (kind === "memory_reembed" || kind === "knowledge_reembed") return true

  const status = await getTransport().call<OllamaStatus>("local_llm_detect_ollama")
  if (status.phase === "not-installed" && !status.installScriptSupported) {
    openExternalUrl("https://ollama.com/download")
    // Opening the download page is not a started/completed installation.
    return false
  }
  return true
}
