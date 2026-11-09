import { describe, expect, it } from "vitest"
import { BATCH_CAPABLE_KINDS, STREAMING_KINDS } from "@/lib/stt"
import { STT_PRESETS } from "./presets"

describe("current STT preset transport contracts", () => {
  it("seeds the documented xAI model on the batch-only kind", () => {
    const preset = STT_PRESETS.find((p) => p.slug === "xai-stt")!
    expect(preset.defaultModels[0].id).toBe("grok-voice-transcribe-2.0")
    expect(preset.defaultBaseUrl).toBe("https://api.x.ai")
    expect(BATCH_CAPABLE_KINDS.has(preset.kind)).toBe(true)
    expect(STREAMING_KINDS.has(preset.kind)).toBe(false)
  })

  it("keeps current AssemblyAI models out of batch and IM fallback", () => {
    const preset = STT_PRESETS.find((p) => p.slug === "assemblyai-ws")!
    expect(preset.defaultModels.map((m) => m.id)).toEqual([
      "universal-3-6-pro",
      "universal-3-5-pro",
      "universal-streaming-english",
      "universal-streaming-multilingual",
    ])
    expect(preset.transport).toBe("ws")
    expect(BATCH_CAPABLE_KINDS.has(preset.kind)).toBe(false)
    expect(STREAMING_KINDS.has(preset.kind)).toBe(true)
  })
})
