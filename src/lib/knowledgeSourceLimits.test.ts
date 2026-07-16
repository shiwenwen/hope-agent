import { describe, expect, it, vi } from "vitest"

import type { Transport } from "./transport"
import {
  DEFAULT_KNOWLEDGE_SOURCE_LIMITS,
  normalizeKnowledgeSourceLimits,
  readKnowledgeSourceLimits,
  writeKnowledgeSourceLimits,
} from "./knowledgeSourceLimits"

describe("knowledge source limits", () => {
  it("uses defaults for missing legacy fields and clamps configured values", () => {
    expect(normalizeKnowledgeSourceLimits()).toEqual(DEFAULT_KNOWLEDGE_SOURCE_LIMITS)
    expect(
      normalizeKnowledgeSourceLimits({
        maxTextSourceMb: 0,
        maxBinarySourceMb: 1000,
        maxUrlResponseMb: 2.6,
      }),
    ).toEqual({
      maxTextSourceMb: 1,
      maxBinarySourceMb: 100,
      maxUrlResponseMb: 3,
    })
  })

  it("normalizes both transport reads and writes", async () => {
    const call = vi
      .fn()
      .mockResolvedValueOnce({ maxTextSourceMb: 50 })
      .mockResolvedValueOnce({ maxBinarySourceMb: 0 })
    const transport = { call } as unknown as Transport

    await expect(readKnowledgeSourceLimits(transport)).resolves.toEqual({
      maxTextSourceMb: 20,
      maxBinarySourceMb: 24,
      maxUrlResponseMb: 2,
    })
    await expect(
      writeKnowledgeSourceLimits(transport, {
        maxTextSourceMb: 30,
        maxBinarySourceMb: 101,
        maxUrlResponseMb: 0,
      }),
    ).resolves.toEqual({
      maxTextSourceMb: 5,
      maxBinarySourceMb: 1,
      maxUrlResponseMb: 2,
    })
    expect(call).toHaveBeenLastCalledWith("knowledge_source_limits_config_set_cmd", {
      config: {
        maxTextSourceMb: 20,
        maxBinarySourceMb: 100,
        maxUrlResponseMb: 1,
      },
    })
  })
})
