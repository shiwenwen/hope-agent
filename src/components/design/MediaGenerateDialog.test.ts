import { describe, expect, test } from "vitest"
import type { ImageEditCaps, MediaCandidateOverview } from "@/components/settings/media-gen/types"
import {
  inferAudioKindFromPrompt,
  resolveImageRequestCapability,
} from "./MediaGenerateDialog.logic"

function imageCandidate(providerId: string, edit: ImageEditCaps | null): MediaCandidateOverview {
  return {
    providerId,
    providerName: providerId,
    vendor: "fal",
    modelId: `${providerId}-model`,
    modelName: `${providerId} model`,
    supportsVoiceListing: false,
    image: {
      maxN: 1,
      supportsSize: false,
      supportsAspectRatio: true,
      supportsResolution: true,
      sizes: [],
      aspectRatios: ["1:1", "16:9"],
      resolutions: ["1K", "2K"],
      supportsMask: false,
      edit,
    },
  }
}

describe("MediaGenerateDialog request state", () => {
  test("infers an audio kind from a seeded prompt", () => {
    expect(inferAudioKindFromPrompt("  [MUSIC] ambient piano")).toBe("music")
    expect(inferAudioKindFromPrompt("[sfx] closing door")).toBe("sfx")
    expect(inferAudioKindFromPrompt("plain narration")).toBeNull()
  })

  test("uses generation capabilities when there are no reference images", () => {
    const candidate = imageCandidate("generation", {
      maxN: 1,
      maxInputImages: 1,
      supportsSize: false,
      supportsAspectRatio: false,
      supportsResolution: false,
    })

    const resolved = resolveImageRequestCapability([candidate], 0)

    expect(resolved.candidate?.providerId).toBe("generation")
    expect(resolved.caps?.supportsAspectRatio).toBe(true)
    expect(resolved.caps?.supportsResolution).toBe(true)
  })

  test("selects a compatible edit candidate and uses its narrower capabilities", () => {
    const generationOnly = imageCandidate("generation-only", null)
    const oneImage = imageCandidate("one-image", {
      maxN: 1,
      maxInputImages: 1,
      supportsSize: false,
      supportsAspectRatio: true,
      supportsResolution: true,
    })
    const multiImage = imageCandidate("multi-image", {
      maxN: 1,
      maxInputImages: 4,
      supportsSize: false,
      supportsAspectRatio: false,
      supportsResolution: false,
    })

    const resolved = resolveImageRequestCapability([generationOnly, oneImage, multiImage], 2)

    expect(resolved.candidate?.providerId).toBe("multi-image")
    expect(resolved.caps?.supportsAspectRatio).toBe(false)
    expect(resolved.caps?.supportsResolution).toBe(false)
  })
})
